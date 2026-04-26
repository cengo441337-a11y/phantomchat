//! PhantomChat desktop — Tauri 2 backend.
//!
//! Exposes a small command surface that mirrors the CLI's keygen / send /
//! listen flow but pushes UI updates to the React frontend through Tauri's
//! event bus instead of writing to stdout.
//!
//! All state (identity, contacts, sessions) is persisted under the platform's
//! standard app-data dir via `tauri::Manager::path().app_data_dir()` —
//! `%APPDATA%\de.dc-infosec.phantomchat\` on Windows, the corresponding
//! `~/.local/share/...` on Linux for `cargo check` purposes.

use std::{
    fs,
    fs::OpenOptions,
    io::Write as _,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, OnceLock},
};

use anyhow::{anyhow, Context};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use tauri_plugin_updater::UpdaterExt;
use phantomchat_core::{
    address::PhantomAddress,
    keys::{IdentityKey, PhantomSigningKey, SpendKey, ViewKey},
    mls::{self, GroupId, PhantomMlsGroup, PhantomMlsMember},
    privacy::{PrivacyConfig, PrivacyMode, ProxyConfig, ProxyKind},
    secure_storage::{detect_best_storage, key_id_for_path, SecureStorage, SecureStorageError},
    session::SessionStore,
};
use zeroize::Zeroizing;
use phantomchat_relays::{
    make_multi_relay, BridgeProvider, ConnectionEvent, EnvelopeHandler, StateHandler,
};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Mutex as AsyncMutex;
use x25519_dalek::{PublicKey, StaticSecret};

const KEYS_FILE: &str = "keys.json";
const CONTACTS_FILE: &str = "contacts.json";
const SESSIONS_FILE: &str = "sessions.json";
const MESSAGES_FILE: &str = "messages.json";
const MLS_DIRECTORY_FILE: &str = "mls_directory.json";
const MLS_STATE_DIR: &str = "mls_state";
const ME_FILE: &str = "me.json";
const RELAYS_FILE: &str = "relays.json";
const PRIVACY_FILE: &str = "privacy.json";
const AUDIT_LOG_FILE: &str = "audit.log";
/// Append-only JSONL crash log next to keys.json. One JSON record per
/// captured panic (`CrashReport`). Populated by the panic hook installed
/// in `setup()`; read by `list_crash_reports`; cleared by
/// `clear_crash_reports`. NEVER written from anywhere else — the rest of
/// the codebase routes errors through `Result<_, String>` and does not
/// panic on user-driven paths.
const CRASHES_FILE: &str = "crashes.jsonl";
/// Per-app sentinel marking that the user has explicitly opted in to
/// uploading crash reports. Stored as an empty file (its presence IS the
/// "yes"). Default ABSENT — `dispatch_crash_report` refuses to POST
/// anything if the file is missing.
const CRASH_OPTIN_FILE: &str = "crash_reporting_opted_in.flag";
/// Endpoint for opt-in crash uploads. Matches the nginx vhost rule on
/// `updates.dc-infosec.de` (return 202 on POST today, real persistence
/// follow-up). Self-hosters can override by editing this constant + a
/// rebuild — orgs running their own collector point at their own URL.
const CRASH_REPORT_ENDPOINT: &str = "https://updates.dc-infosec.de/crash-report";
/// Persistence target for the desktop main-window geometry (position, size,
/// maximized flag, last monitor identity). See the `window_state` module
/// section further down for the on-disk schema and restore policy.
const WINDOW_STATE_FILE: &str = "window_state.json";
const POW_DIFFICULTY: u32 = 8;

/// Bootstrap relay set written to `relays.json` if the file is missing.
/// Three well-known Nostr relays — gives the user redundancy out of the box
/// without forcing them to configure anything before first use.
const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.snort.social",
];

/// 8-byte ASCII tag prefixed to legacy MLS Welcome bytes before sealed-
/// sender wrap. Carries no inviter metadata, so the joiner displays the
/// inviter as `?<8hex>` until they reply through their own contact path.
/// Kept around for backwards compatibility with in-flight v1 envelopes.
const MLS_WLC_PREFIX_V1: &[u8; 8] = b"MLS-WLC1";
/// 8-byte ASCII tag prefixed to v2 MLS Welcome bytes. Wire format:
///
///   MLS-WLC2 || varint(meta_len) || meta_json || welcome_bytes
///
/// `meta_json` is a [`MlsWelcomeMetaV2`] (`inviter_label`,
/// `inviter_address`, `inviter_signing_pub_hex`). The joiner inserts the
/// inviter into its `member_addresses` directory before processing the
/// Welcome, so the very first incoming MLS app message resolves to the
/// human label instead of the placeholder.
const MLS_WLC_PREFIX_V2: &[u8; 8] = b"MLS-WLC2";
/// 8-byte ASCII tag prefixed to MLS application/commit wire bytes before
/// sealed-sender wrap. See [`MLS_WLC_PREFIX_V1`] for the migration story.
const MLS_APP_PREFIX: &[u8; 8] = b"MLS-APP1";

/// 7-byte ASCII tag prefixed to delivery / read receipt envelopes. Wire:
///
///   RCPT-1: || ULEB128(meta_len) || meta_json
///
/// `meta_json` is a [`ReceiptMetaV1`] (`msg_id`, `kind`). Sealed-sender-
/// wrapped like every other 1:1 payload so the relay can't observe who is
/// reading whose messages.
const RCPT_PREFIX_V1: &[u8; 7] = b"RCPT-1:";
/// 7-byte ASCII tag prefixed to typing-indicator envelopes. Wire:
///
///   TYPN-1: || ULEB128(meta_len) || meta_json
///
/// `meta_json` is a [`TypingMetaV1`] (`contact_label`, `ttl_secs`). Same
/// sealed-sender wrap as RCPT — typing pings carry no payload, just the
/// presence signal.
const TYPN_PREFIX_V1: &[u8; 7] = b"TYPN-1:";
/// Default TTL for typing pings — matches the InputBar's 1.5s debounce
/// with enough headroom (≈3 cycles) to bridge a brief pause.
const TYPING_TTL_SECS: u32 = 5;

/// 7-byte ASCII tag prefixed to reply envelopes. Wire:
///
///   REPL-1: || ULEB128(meta_len) || meta_json || plaintext_body
///
/// `meta_json` is a [`ReplyMetaV1`] (`in_reply_to_msg_id`, `quoted_preview`).
/// The body that follows the meta is the new message's plaintext — the
/// receiver displays it like a regular text row but with an inline
/// "↪ <quoted_preview>" header that scrolls to the quoted row when clicked.
const REPL_PREFIX_V1: &[u8; 7] = b"REPL-1:";

/// 7-byte ASCII tag prefixed to reaction envelopes. Wire:
///
///   RACT-1: || ULEB128(meta_len) || meta_json
///
/// `meta_json` is a [`ReactionMetaV1`] (`target_msg_id`, `emoji`, `action`).
/// No body — pure metadata. Reactions are accumulated client-side and
/// stored on the target message's `reactions` array; we never persist the
/// per-reaction stream on the wire (only the action stream replays it).
const RACT_PREFIX_V1: &[u8; 7] = b"RACT-1:";

/// 7-byte ASCII tag prefixed to disappearing-messages TTL setting envelopes.
/// Wire:
///
///   DISA-1: || ULEB128(meta_len) || meta_json
///
/// `meta_json` is a [`DisappearingMetaV1`] (`contact_label`, `ttl_secs`).
/// `ttl_secs == None` disables the timer for the conversation. Both peers
/// must agree (one sets, the other receives + applies to their own copy).
const DISA_PREFIX_V1: &[u8; 7] = b"DISA-1:";

/// Per-contact disappearing-messages settings file. Lives next to
/// `contacts.json`. Map of `contact_label -> ttl_secs`. A missing entry
/// means "no auto-disappear for this conversation".
const DISAPPEARING_FILE: &str = "disappearing.json";

/// Auto-purge tick interval. Every 60s the background task scans
/// `messages.json` for rows whose `expires_at <= now()` and prunes them.
const PURGE_INTERVAL_SECS: u64 = 60;

// ── Wire types shared with the React frontend ────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityInfo {
    pub address: String,
    pub view_pub_short: String,
    pub spend_pub_short: String,
    pub signing_pub_short: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Contact {
    pub label: String,
    pub address: String,
    /// Hex-encoded Ed25519 public key used by sealed-sender attribution.
    /// `None` until the user binds it via `bind_last_unbound_sender` (or
    /// edits the contacts file out-of-band). Optional + skip-if-none keeps
    /// the on-disk format backwards-compatible with pre-attribution
    /// contact files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_pub: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContactBook {
    #[serde(default)]
    pub contacts: Vec<Contact>,
}

/// Payload pushed to the frontend via `app.emit("message", ...)` whenever a
/// relay envelope decrypts successfully. Extended with sealed-sender
/// attribution fields so the React side can render the right contact name
/// (or flag a tampered / unbound sender).
///
/// Also doubles as the on-disk `messages.json` record format
/// (`save_history` / `load_history`). For history rows the frontend may set
/// `kind` = "outgoing" / "system" — defaults to "incoming" so older saved
/// payloads (pre-history-feature) round-trip cleanly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IncomingMessage {
    pub plaintext: String,
    pub timestamp: String,
    /// Resolved label: contact name, "INBOX" (no attribution), "INBOX!"
    /// (signature failed), or "?<8-hex>" (attribution OK but no matching
    /// contact yet).
    pub sender_label: String,
    /// Whether the sealed-sender signature verified. `true` for the
    /// no-attribution case (vacuously trusted) and the matched-contact
    /// case; `false` only when an attribution was present and tampered.
    pub sig_ok: bool,
    /// Hex-encoded Ed25519 public key of the sender, if the envelope
    /// carried sealed-sender attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_pub_hex: Option<String>,
    /// Direction marker for persistence. One of "incoming" | "outgoing" |
    /// "system". Defaults to "incoming" so existing live-emit payloads stay
    /// shape-compatible.
    #[serde(default = "default_direction")]
    pub direction: String,
    /// Row kind. Defaults to `"text"` for backwards-compat with pre-file
    /// history rows. `"file"` rows carry a populated `file_meta`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// File metadata (filename / size / sha256 / saved_path). Present iff
    /// `kind == "file"`. Optional so text rows stay shape-compatible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_meta: Option<FileMeta>,
    /// Stable per-message identifier. Computed via the same SHA256-based
    /// recipe on both sender and receiver so receipts can match outgoing
    /// rows back to their delivery state. Present on outgoing rows (stamped
    /// at send time) and on incoming rows (computed at decode time so the
    /// receiver knows which `msg_id` to echo back in `RCPT-1:`).
    /// 16 hex chars = 64 bits — collision-safe inside a single conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
    /// Outgoing-row delivery state machine. One of `"sent"` / `"delivered"`
    /// / `"read"`. Only set on outgoing rows; the React reducer escalates
    /// monotonically (never downgrades read → delivered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_state: Option<String>,
    /// Reply-thread metadata, populated when the row was sent via
    /// `send_reply` (or arrived in a `REPL-1:` envelope). The frontend
    /// renders a magenta-tinted quote block above the body that scrolls
    /// to the quoted row on click.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ReplyMeta>,
    /// Accumulated reactions for this row. Mutated in-place by the
    /// `RACT-1:` listener path: `add` appends a `ReactionEntry`,
    /// `remove` drops the matching `(sender_label, emoji)` pair. The UI
    /// groups by emoji for the pill display.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<ReactionEntry>,
    /// Unix-epoch second after which this row is auto-purged. Set on
    /// both outgoing (sender stamps from local TTL) and incoming
    /// (receiver stamps from local TTL) rows. The 60s purge sweep
    /// drops rows whose value is `<= now()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

/// Reply-thread metadata. Carried inline in REPL-1: envelopes and
/// persisted on the row so re-opens still show the quote block.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplyMeta {
    pub in_reply_to_msg_id: String,
    /// First ~80 chars of the quoted message. We carry the preview
    /// inline rather than looking it up by msg_id at render time so the
    /// recipient sees the SENDER'S view of the quoted text (defends
    /// against a sender forging a quote that doesn't match what the
    /// receiver actually has on record).
    pub quoted_preview: String,
}

/// One emoji-reaction entry on a message row. Aggregated client-side
/// from `RACT-1:` events.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReactionEntry {
    pub sender_label: String,
    pub emoji: String,
}

fn default_direction() -> String {
    "incoming".to_string()
}

/// Metadata describing one file payload — mirrored 1:1 with the TS
/// `FileMeta` interface in `desktop/src/types.ts`. Stored on history rows
/// and emitted live via the `file_received` event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileMeta {
    pub filename: String,
    pub size: u64,
    /// Absolute path on the receiver's disk where the bytes were saved.
    /// `None` for outgoing-side history rows (the sender doesn't keep a
    /// copy under PhantomChat's Downloads dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_path: Option<String>,
    pub sha256_hex: String,
    /// `Some(true)` if the receiver re-hashed the bytes and matched the
    /// manifest's `sha256_hex`; `Some(false)` for a mismatched receive
    /// (UI shows red ⚠ tint); `None` for outgoing rows where the sender
    /// vacuously trusts its own bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256_ok: Option<bool>,
}

// ── MLS wire types ───────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
pub struct MlsAddResult {
    pub commit_b64: String,
    pub welcome_b64: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MlsDecryptResult {
    pub plaintext: Option<String>,
    pub control_only: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MlsStatus {
    pub initialized: bool,
    pub in_group: bool,
    pub member_count: u32,
    pub identity_label: Option<String>,
    /// Snapshot of the auto-transport directory entries known to this
    /// bundle. Empty unless `mls_add_member` has been called at least once
    /// (or the directory was rehydrated from `mls_directory.json`).
    #[serde(default)]
    pub members: Vec<MlsMemberRef>,
}

/// Per-MLS-member transport pointer — what the auto-transport layer needs
/// to ship welcomes/app messages over the existing sealed-sender 1:1 pipe
/// AND to attribute incoming MLS payloads back to a human label.
///
/// Persisted as part of `mls_directory.json` so the directory survives
/// app restarts even though the OpenMLS provider's tree itself is RAM-only
/// in this MVP.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MlsMemberRef {
    /// Human-readable label, e.g. "alice".
    pub label: String,
    /// Recipient PhantomAddress in the canonical `phantom:view:spend` form,
    /// used as the `recipient` for `SessionStore::send_sealed`.
    pub address: String,
    /// Hex-encoded Ed25519 sealed-sender pubkey of the OTHER member —
    /// mirrors `Contact::signing_pub` so we can look up which directory
    /// entry an incoming MLS payload came from.
    pub signing_pub_hex: String,
}

/// V2 Welcome envelope metadata. Carried inline before the welcome bytes
/// so the joiner can promote the inviter into its directory atomically
/// with the join, avoiding the `?<8hex>` placeholder for the first
/// incoming app message.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MlsWelcomeMetaV2 {
    inviter_label: String,
    inviter_address: String,
    inviter_signing_pub_hex: String,
}

/// One row returned by `mls_list_members` — pairs an MLS leaf-credential
/// label with the member's signing pubkey + cross-references the bundle's
/// transport directory so the UI can show "alice (you)" / "bob → contact"
/// rather than raw hex.
#[derive(Clone, Debug, Serialize)]
pub struct MlsMemberInfo {
    /// UTF-8 label captured from the member's BasicCredential identity.
    /// Falls back to `?<8hex>` when the bytes aren't valid UTF-8 (which
    /// shouldn't happen for PhantomChat-created bundles, but openmls
    /// itself doesn't enforce UTF-8 on credential identities).
    pub credential_label: String,
    /// Hex-encoded MLS leaf signature pubkey (Ed25519 in our ciphersuite).
    pub signature_pub_hex: String,
    /// `true` when this leaf's signing pubkey matches our local member's
    /// signing pubkey — the React side uses this to render "(you)".
    pub is_self: bool,
    /// `Some(label)` when the leaf's signing pubkey appears in the
    /// bundle's `member_addresses` directory. `None` for a self-row or
    /// for a member that joined before we cached their transport pointer.
    pub mapped_contact_label: Option<String>,
}

/// Wire-side privacy configuration shipped to / loaded from the React
/// frontend. Mirrors `phantomchat_core::privacy::PrivacyConfig` but uses
/// flat string-tagged enums so the JSON is trivially editable + matches
/// what `SettingsPanel.tsx` already speaks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrivacyConfigDto {
    /// `"DailyUse"` | `"MaximumStealth"`.
    pub mode: String,
    pub proxy_addr: String,
    /// `"Tor"` | `"Nym"`.
    pub proxy_kind: String,
}

impl Default for PrivacyConfigDto {
    fn default() -> Self {
        Self::from(&PrivacyConfig::default())
    }
}

impl From<&PrivacyConfig> for PrivacyConfigDto {
    fn from(cfg: &PrivacyConfig) -> Self {
        Self {
            mode: match cfg.mode {
                PrivacyMode::DailyUse => "DailyUse".to_string(),
                PrivacyMode::MaximumStealth => "MaximumStealth".to_string(),
            },
            proxy_addr: cfg.proxy.addr.clone(),
            proxy_kind: match cfg.proxy.kind {
                ProxyKind::Tor => "Tor".to_string(),
                ProxyKind::Nym => "Nym".to_string(),
            },
        }
    }
}

impl PrivacyConfigDto {
    /// Convert into the canonical core `PrivacyConfig`. Validation rejects
    /// unknown enum tags so a malformed `privacy.json` (e.g. one
    /// hand-edited to `"mode": "Foo"`) surfaces a clear error instead of
    /// silently falling back to defaults.
    fn to_core(&self) -> Result<PrivacyConfig, String> {
        let mode = match self.mode.as_str() {
            "DailyUse" => PrivacyMode::DailyUse,
            "MaximumStealth" => PrivacyMode::MaximumStealth,
            other => return Err(format!("unknown privacy mode '{}'", other)),
        };
        let kind = match self.proxy_kind.as_str() {
            "Tor" => ProxyKind::Tor,
            "Nym" => ProxyKind::Nym,
            other => return Err(format!("unknown proxy kind '{}'", other)),
        };
        let addr = self.proxy_addr.trim().to_string();
        if addr.is_empty() {
            return Err("proxy_addr must not be empty".into());
        }
        Ok(PrivacyConfig {
            mode,
            proxy: ProxyConfig { addr, kind },
            cover_traffic_enabled: true,
        })
    }
}

/// Persisted under `me.json` so the inviter side can supply a stable
/// self-label across restarts (used in the V2 Welcome metadata).
/// Optional — if missing we fall back to the first 8 chars of our own
/// signing-pub hex.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct MeDisk {
    #[serde(default)]
    label: String,
}

fn me_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(ME_FILE))
}

fn load_me(app: &AppHandle) -> MeDisk {
    me_path(app)
        .ok()
        .and_then(|p| fs::read(&p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_me(app: &AppHandle, me: &MeDisk) -> anyhow::Result<()> {
    let path = me_path(app)?;
    fs::write(&path, serde_json::to_vec_pretty(me)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Resolve the user's preferred self-label for outbound MLS Welcomes.
/// Order: (1) explicit `me.json` label, (2) first 8 chars of signing-pub
/// hex (so we always emit *something* deterministic). Returns `None` only
/// if the keyfile itself is missing, which would already have blocked
/// the upstream send.
fn resolve_self_label(app: &AppHandle) -> Option<String> {
    let me = load_me(app);
    if !me.label.trim().is_empty() {
        return Some(me.label.trim().to_string());
    }
    let pub_hex = auto_init_identity_label(app)?;
    Some(pub_hex.chars().take(8).collect())
}

/// Encode a v2 Welcome wrapping payload:
///   MLS-WLC2 || varint(meta_len) || meta_json || welcome_bytes
///
/// `varint` uses unsigned-LEB128 to keep the meta length compact and
/// future-proof against >127 B JSON blobs without forcing a fixed-width
/// length field. `meta_len` covers `meta_json` only — the welcome bytes
/// fill the remainder of the payload.
fn encode_wlc_v2(meta: &MlsWelcomeMetaV2, welcome: &[u8]) -> Vec<u8> {
    let meta_json = serde_json::to_vec(meta).expect("MlsWelcomeMetaV2 ser is infallible");
    let mut out = Vec::with_capacity(MLS_WLC_PREFIX_V2.len() + 5 + meta_json.len() + welcome.len());
    out.extend_from_slice(MLS_WLC_PREFIX_V2);
    write_uleb128(&mut out, meta_json.len() as u64);
    out.extend_from_slice(&meta_json);
    out.extend_from_slice(welcome);
    out
}

/// Inverse of [`encode_wlc_v2`]. Returns the parsed meta + a slice into
/// `body` pointing at the embedded welcome bytes. Body is the payload
/// AFTER the 8-byte `MLS-WLC2` prefix has already been stripped.
fn decode_wlc_v2(body: &[u8]) -> Result<(MlsWelcomeMetaV2, &[u8]), String> {
    let (meta_len, consumed) = read_uleb128(body).ok_or("truncated meta length")?;
    let meta_end = consumed
        .checked_add(meta_len as usize)
        .ok_or("meta_len overflow")?;
    if body.len() < meta_end {
        return Err(format!(
            "meta_len {} exceeds body of {} bytes",
            meta_len,
            body.len()
        ));
    }
    let meta_bytes = &body[consumed..meta_end];
    let meta: MlsWelcomeMetaV2 = serde_json::from_slice(meta_bytes)
        .map_err(|e| format!("meta JSON: {e}"))?;
    Ok((meta, &body[meta_end..]))
}

fn write_uleb128(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_uleb128(input: &[u8]) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    for (i, b) in input.iter().enumerate() {
        let chunk = (b & 0x7F) as u64;
        value |= chunk.checked_shl(shift)?;
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

// ── MLS bundle (safe per-call `MlsGroup::load` pattern) ───────────────────
//
// `PhantomMlsGroup<'m>` borrows `&mut PhantomMlsMember`, so we cannot keep
// one alive across Tauri command invocations alongside the member it
// borrows from. Instead we persist only the lightweight `group_id` bytes
// here. Each command:
//
//   1. takes `&mut self` on the bundle,
//   2. calls `mls::load_group(&self.member, &GroupId::from_slice(id))` to
//      reconstruct the openmls `MlsGroup` from the provider's storage,
//   3. wraps it in a temporary `PhantomMlsGroup` via `from_parts`,
//   4. performs exactly one MLS op (which mutates the group + auto-persists
//      the new state into the provider's storage), then drops the wrapper.
//
// Openmls' `MlsGroup` auto-persists after every mutating call — `add_members`
// + `merge_pending_commit`, `create_message`, `process_message` +
// `merge_staged_commit` — so the next command sees the updated tree when it
// reloads. `member_count` is cached after each op so `mls_status` is O(1).
pub struct MlsBundle {
    member: PhantomMlsMember,
    /// `Some(id_bytes)` iff we're in a group. Stable byte view of the
    /// openmls `GroupId`, fed back into `GroupId::from_slice` + `load_group`.
    group_id: Option<Vec<u8>>,
    /// Cached from the most-recent op so `mls_status` doesn't have to
    /// reload + walk the tree just to count members.
    member_count: u32,
    /// Original identity label, retained for status reporting.
    identity_label: String,
    /// Auto-transport directory: per-MLS-member pointers used by
    /// `mls_add_member` / `mls_encrypt` to ship wire bytes via the existing
    /// sealed-sender 1:1 pipe, and by `start_listener` to attribute
    /// incoming MLS payloads back to a human label.
    ///
    /// Persisted to `mls_directory.json` next to `keys.json`/`contacts.json`.
    /// The MlsBundle's MLS state internals (group tree, epoch, member
    /// count) are NOT here — those live inside the openmls provider.
    member_addresses: Vec<MlsMemberRef>,
}

impl MlsBundle {
    /// File-backed constructor used by every desktop entry point. The
    /// MLS provider's KV state survives restarts via
    /// [`PhantomMlsMember::new_with_storage_dir`]; if the supplied
    /// `storage_dir` already contains a previous session's state the
    /// bundle picks up its `group_id` from disk transparently. The
    /// member-count cache is recomputed lazily by reloading the group on
    /// first read so we don't have to walk the tree on every launch.
    fn new(identity: String, storage_dir: &std::path::Path) -> Result<Self, String> {
        let member = PhantomMlsMember::new_with_storage_dir(
            identity.as_bytes().to_vec(),
            storage_dir,
        )
        .map_err(|e| format!("PhantomMlsMember::new_with_storage_dir: {e}"))?;

        // Rehydrate the active group id (if any) so `with_group` /
        // `mls_status` work without a fresh `create_group` call.
        let group_id = member.active_group_id().map(|s| s.to_vec());
        let mut bundle = Self {
            member,
            group_id,
            member_count: 0,
            identity_label: identity,
            member_addresses: Vec::new(),
        };
        // Touch the group once to refresh `member_count`. If the group
        // can't be loaded (e.g. the `mls_meta.json` survived but the
        // `mls_state.bin` was wiped) we treat it as "not in a group" so
        // the next `create_group` / `join_via_welcome` proceeds cleanly.
        if bundle.group_id.is_some() {
            // `with_group` updates `bundle.member_count` itself via the
            // post-op refresh, so we just discard the closure return.
            if bundle.with_group(|_g| Ok(())).is_err() {
                bundle.group_id = None;
                let _ = bundle.member.clear_active_group_id();
            }
        }
        Ok(bundle)
    }

    fn publish_key_package(&self) -> Result<Vec<u8>, String> {
        self.member
            .publish_key_package()
            .map_err(|e| format!("publish_key_package: {e}"))
    }

    /// Load the group from storage, hand a temporary `PhantomMlsGroup`
    /// wrapper to `op`, then drop. Refreshes the cached `member_count`
    /// from the post-op wrapper before returning.
    fn with_group<R>(
        &mut self,
        op: impl FnOnce(&mut PhantomMlsGroup<'_>) -> Result<R, String>,
    ) -> Result<R, String> {
        let id_bytes = self
            .group_id
            .as_ref()
            .ok_or_else(|| "not in a group — call create_group/join_via_welcome first".to_string())?
            .clone();
        let gid = GroupId::from_slice(&id_bytes);
        let group = mls::load_group(&self.member, &gid)
            .map_err(|e| format!("load_group: {e}"))?;
        let mut wrapper = PhantomMlsGroup::from_parts(&mut self.member, group);
        let out = op(&mut wrapper)?;
        self.member_count = wrapper.member_count() as u32;
        Ok(out)
    }

    fn create_group(&mut self) -> Result<(), String> {
        if self.group_id.is_some() {
            return Err("already in a group (single-group MVP)".into());
        }
        // Borrow self.member only inside this block — capture the id +
        // count from the wrapper before it drops, releasing the borrow.
        // Openmls has already persisted the group into our provider's
        // storage by the time `create_group` returns.
        let (id_bytes, count) = {
            let wrapper = self
                .member
                .create_group()
                .map_err(|e| format!("create_group: {e}"))?;
            (wrapper.group_id_bytes(), wrapper.member_count() as u32)
        };
        self.group_id = Some(id_bytes);
        self.member_count = count;
        Ok(())
    }

    fn join_via_welcome(&mut self, welcome: &[u8]) -> Result<(), String> {
        if self.group_id.is_some() {
            return Err("already in a group (single-group MVP)".into());
        }
        let (id_bytes, count) = {
            let wrapper = self
                .member
                .join_via_welcome(welcome)
                .map_err(|e| format!("join_via_welcome: {e}"))?;
            (wrapper.group_id_bytes(), wrapper.member_count() as u32)
        };
        self.group_id = Some(id_bytes);
        self.member_count = count;
        Ok(())
    }

    fn add_member(&mut self, kp_bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
        self.with_group(|g| g.add_member(kp_bytes).map_err(|e| format!("add_member: {e}")))
    }

    fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        self.with_group(|g| g.encrypt(plaintext).map_err(|e| format!("encrypt: {e}")))
    }

    fn decrypt(&mut self, wire: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.with_group(|g| g.decrypt(wire).map_err(|e| format!("decrypt: {e}")))
    }

    fn member_count(&self) -> u32 {
        self.member_count
    }

    fn in_group(&self) -> bool {
        self.group_id.is_some()
    }
}

// ── App state held in `tauri::State` ─────────────────────────────────────────

#[derive(Default)]
pub struct AppState {
    /// Set once `start_listener` has spawned its subscriber so a second
    /// invocation is a no-op.
    pub listener_started: StdMutex<bool>,
    /// Most-recently observed sealed-sender pubkey that did NOT match any
    /// contact's `signing_pub`. The frontend can call
    /// `bind_last_unbound_sender(label)` to attach it to a specific contact.
    pub last_unbound_sender: StdMutex<Option<[u8; 32]>>,
    /// In-memory MLS bundle (single-group MVP, no disk persistence — the
    /// `OpenMlsRustCrypto` provider stores everything in RAM and is lost
    /// across desktop restarts).
    pub mls: StdMutex<Option<MlsBundle>>,
    /// Most-recently emitted relay connection status. Stored so the frontend
    /// can ask for the current value at any time (and so we don't double-emit
    /// the same state). Values: "connecting" | "connected" | "disconnected".
    pub connection_status: StdMutex<String>,
    /// Serializes concurrent `save_history` writes so we never half-write the
    /// `messages.json` file when the React debouncer fires twice in a row.
    pub history_lock: StdMutex<()>,
    /// Control handle for the running relay-subscriber tokio task. `Some`
    /// once `start_listener` has spawned. `restart_listener` signals a
    /// graceful shutdown via the embedded oneshot, awaits the task with a
    /// 3s timeout, and only falls back to `abort()` if the task refuses
    /// to exit. The graceful path lets the inner `subscribe_with_state`
    /// future drop the relay handle cleanly so the per-relay WebSocket
    /// connections close their TCP halves with a proper close-frame
    /// instead of a half-open hang. AsyncMutex so we can hold it across
    /// the (async) take + spawn.
    pub subscriber: AsyncMutex<Option<ListenerControl>>,
    /// Set once `spawn_purge_task` has spawned the disappearing-messages
    /// auto-purge timer. Guard against React.StrictMode double-mounts
    /// that would otherwise stack two writers on `messages.json`.
    pub purge_started: StdMutex<bool>,
    /// Live LAN-org broadcaster + browser state. `None` until
    /// `lan_org_create` / `lan_org_join` (or the auto-rehydrate path on
    /// startup) brings up the mDNS daemon. Held behind an `AsyncMutex`
    /// so the join/leave commands — which await disk I/O and the daemon
    /// shutdown — can hold the lock across `.await` points without
    /// poisoning a std mutex.
    pub lan_org: AsyncMutex<Option<LanOrg>>,
}

/// Control handle for one in-flight relay-subscriber task. `shutdown_tx`
/// is consumed on `send` (oneshot semantics), and `handle` is awaited
/// after the signal so we know the task — and the `MultiRelay` it owns
/// inside the spawned future — has fully unwound before we spawn a
/// replacement.
pub struct ListenerControl {
    pub handle: JoinHandle<()>,
    pub shutdown_tx: oneshot::Sender<()>,
}

// ── Path helpers ─────────────────────────────────────────────────────────────

fn app_data(app: &AppHandle) -> anyhow::Result<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| anyhow!("resolve app_data_dir: {}", e))?;
    fs::create_dir_all(&dir).with_context(|| format!("mkdir -p {}", dir.display()))?;
    Ok(dir)
}

fn keys_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(KEYS_FILE))
}

fn contacts_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(CONTACTS_FILE))
}

fn sessions_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(SESSIONS_FILE))
}

fn relays_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(RELAYS_FILE))
}

/// Read the persisted relay list from `relays.json`, or seed it with
/// `DEFAULT_RELAYS` on first run. Lenient: corrupt file → fall back to
/// defaults rather than refusing to start the listener.
fn load_relays(app: &AppHandle) -> Vec<String> {
    let path = match relays_path(app) {
        Ok(p) => p,
        Err(_) => return DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
    };
    if let Ok(raw) = fs::read(&path) {
        if let Ok(list) = serde_json::from_slice::<Vec<String>>(&raw) {
            if !list.is_empty() {
                return list;
            }
        }
    }
    let defaults: Vec<String> = DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect();
    if let Ok(buf) = serde_json::to_vec_pretty(&defaults) {
        let _ = fs::write(&path, buf);
    }
    defaults
}

fn save_relays(app: &AppHandle, urls: &[String]) -> anyhow::Result<()> {
    let path = relays_path(app)?;
    let buf = serde_json::to_vec_pretty(&urls)?;
    fs::write(&path, buf).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn privacy_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(PRIVACY_FILE))
}

fn audit_log_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(AUDIT_LOG_FILE))
}

// ── Audit log (ISO27001 / ISMS append-only JSONL) ───────────────────────────
//
// Single file `app_data_dir/audit.log`. One JSON object per line:
//
//   {"ts":"2026-04-25T13:42:00Z","category":"identity","event":"created","details":{}}
//
// `audit(...)` is best-effort — every call is invoked from a security-relevant
// command path, but a write failure must NEVER bubble up and break the user's
// action. We log the failure to stderr and move on. Categories are
// deliberately coarse so the compliance auditor can filter quickly without a
// schema lookup. Details are categorical metadata only — never any private
// key material, contact address, or message plaintext.

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: String,
    pub category: String,
    pub event: String,
    #[serde(default)]
    pub details: serde_json::Value,
}

/// Append a single audit entry to `audit.log`. Best-effort — never returns
/// `Err` to the caller; stderr-logs any I/O failure so the audit trail can
/// be repaired later without breaking the user's flow.
fn audit(app: &AppHandle, category: &str, event: &str, details: serde_json::Value) {
    let path = match audit_log_path(app) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("audit: cannot resolve audit_log_path: {e}");
            return;
        }
    };
    let entry = AuditEntry {
        ts: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        category: category.to_string(),
        event: event.to_string(),
        details,
    };
    let line = match serde_json::to_string(&entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("audit: serialize failed: {e}");
            return;
        }
    };
    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("audit: open {} failed: {e}", path.display());
            return;
        }
    };
    if let Err(e) = writeln!(file, "{}", line) {
        eprintln!("audit: write {} failed: {e}", path.display());
    }
}

// ── Window-state persistence (multi-monitor aware) ──────────────────────────
//
// On-disk schema (`$APPDATA/de.dc-infosec.phantomchat/window_state.json`):
//
//   {
//     "schema_version": 1,
//     "x": 100,
//     "y": 50,
//     "width": 1320,
//     "height": 820,
//     "maximized": false,
//     "monitor_label": "DELL U2722DE",
//     "monitor_position": { "x": 1920, "y": 0 }
//   }
//
// All units are LOGICAL pixels — Tauri's `LogicalPosition`/`LogicalSize`
// abstract over DPI, so a window persisted on a 1.5x scaled display
// restores correctly on a 1.0x display without manual math.
//
// `monitor_label` + `monitor_position` form a soft fingerprint of the
// display the user last had the window on. On restore we walk
// `available_monitors()` and only re-apply the persisted geometry if a
// matching monitor is still attached (label equal AND top-left position
// within ±20 px). Otherwise we fall back to the platform default — which
// is the centered 1100×720 the WindowBuilder gave us in tauri.conf.json.
//
// Off-screen rescue: even when the monitor matches, we double-check that
// the restored rect lies entirely inside that monitor's bounds. A user
// who unplugs a 4K display and reattaches a 1080p one with the same name
// would otherwise get a window stranded outside the visible area.
//
// We deliberately hand-roll this instead of pulling in
// `tauri-plugin-window-state` because the upstream plugin's restore
// policy is "always restore last geometry" with no monitor-identity
// check — bad UX on an Anwalt's docking-station + portable laptop combo
// where a missing display means the window comes back invisible.

/// Tolerance (in logical pixels) for matching a persisted monitor against
/// the currently-attached set. Two monitors that report the same label
/// and a top-left position within this many pixels are considered the
/// same physical display. ±20 px absorbs minor DPI-rounding drift while
/// still distinguishing a monitor reattached at a different desktop
/// coordinate after a layout change.
const MONITOR_MATCH_TOLERANCE_PX: f64 = 20.0;

/// Debounce window for save_window_state writes. A drag emits a flood of
/// `Moved` events at every cursor tick; coalescing to one write per 500 ms
/// keeps `window_state.json` from being rewritten thousands of times per
/// gesture without losing the final position when the user releases.
const WINDOW_STATE_DEBOUNCE_MS: u64 = 500;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MonitorPos {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowStateDisk {
    #[serde(default = "window_state_schema_default")]
    pub schema_version: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub monitor_label: String,
    #[serde(default)]
    pub monitor_position: MonitorPos,
}

impl Default for MonitorPos {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

fn window_state_schema_default() -> u32 {
    1
}

fn window_state_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(WINDOW_STATE_FILE))
}

/// Lenient loader: missing or corrupt file → `None`, never an error.
/// First launch must not warn the user; a hand-edited file that fails to
/// parse falls back to defaults (same stance as `load_privacy`).
fn load_window_state(app: &AppHandle) -> Option<WindowStateDisk> {
    let path = window_state_path(app).ok()?;
    let raw = fs::read(&path).ok()?;
    serde_json::from_slice::<WindowStateDisk>(&raw).ok()
}

/// Write the window state atomically-enough for our purposes (single
/// `fs::write` — the file is small and a torn write just means we lose
/// the latest gesture, never corrupt earlier app data).
fn save_window_state(app: &AppHandle, state: WindowStateDisk) -> Result<(), String> {
    let path = window_state_path(app).map_err(|e| e.to_string())?;
    let buf = serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?;
    fs::write(&path, buf).map_err(|e| format!("write {}: {}", path.display(), e))?;
    Ok(())
}

/// Capture the current window geometry into a `WindowStateDisk`. Reads
/// physical units off the OS, converts to logical via the window's scale
/// factor so persistence is DPI-independent.
fn capture_window_state(
    win: &tauri::WebviewWindow,
) -> Result<WindowStateDisk, String> {
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    let pos = win.outer_position().map_err(|e| e.to_string())?;
    let size = win.outer_size().map_err(|e| e.to_string())?;
    let maximized = win.is_maximized().unwrap_or(false);
    let monitor = win.current_monitor().ok().flatten();
    let (label, mpos) = match monitor {
        Some(m) => {
            let mp = m.position();
            (
                m.name().cloned().unwrap_or_default(),
                MonitorPos {
                    x: mp.x as f64 / scale,
                    y: mp.y as f64 / scale,
                },
            )
        }
        None => (String::new(), MonitorPos::default()),
    };
    Ok(WindowStateDisk {
        schema_version: 1,
        x: pos.x as f64 / scale,
        y: pos.y as f64 / scale,
        width: size.width as f64 / scale,
        height: size.height as f64 / scale,
        maximized,
        monitor_label: label,
        monitor_position: mpos,
    })
}

/// Walk `available_monitors()` and pick the one matching the persisted
/// monitor identity. `None` ⇒ the user disconnected or rearranged
/// displays; caller falls back to the centered default.
fn find_matching_monitor(
    win: &tauri::WebviewWindow,
    state: &WindowStateDisk,
) -> Option<tauri::Monitor> {
    let monitors = win.available_monitors().ok()?;
    let scale = win.scale_factor().unwrap_or(1.0);
    monitors.into_iter().find(|m| {
        let label_match = m
            .name()
            .map(|n| n == &state.monitor_label)
            .unwrap_or(state.monitor_label.is_empty());
        let mp = m.position();
        let mx = mp.x as f64 / scale;
        let my = mp.y as f64 / scale;
        let pos_match = (mx - state.monitor_position.x).abs() <= MONITOR_MATCH_TOLERANCE_PX
            && (my - state.monitor_position.y).abs() <= MONITOR_MATCH_TOLERANCE_PX;
        label_match && pos_match
    })
}

/// Verify the persisted rect fits inside `monitor`. Catches the
/// "saved off-screen" / "monitor downgraded to lower resolution" cases
/// before we apply geometry that would leave the window unreachable.
fn rect_fits_monitor(
    win: &tauri::WebviewWindow,
    state: &WindowStateDisk,
    monitor: &tauri::Monitor,
) -> bool {
    let scale = win.scale_factor().unwrap_or(1.0);
    let mp = monitor.position();
    let ms = monitor.size();
    let m_x = mp.x as f64 / scale;
    let m_y = mp.y as f64 / scale;
    let m_w = ms.width as f64 / scale;
    let m_h = ms.height as f64 / scale;
    state.x >= m_x
        && state.y >= m_y
        && state.x + state.width <= m_x + m_w
        && state.y + state.height <= m_y + m_h
}

/// Restore window geometry from disk if a matching monitor is attached
/// AND the persisted rect still fits on it. Returns `Some(label)` on a
/// successful restore so the caller can audit-log the monitor name; the
/// audit entry deliberately omits the actual coordinates (per the spec
/// these could leak desktop layout via the audit-log export).
fn restore_window_state(app: &AppHandle, win: &tauri::WebviewWindow) -> Option<String> {
    let state = load_window_state(app)?;
    let monitor = match find_matching_monitor(win, &state) {
        Some(m) => m,
        None => {
            audit(
                app,
                "display",
                "window_restore_skipped",
                serde_json::json!({ "reason": "monitor_missing" }),
            );
            return None;
        }
    };
    if !rect_fits_monitor(win, &state, &monitor) {
        audit(
            app,
            "display",
            "window_restore_skipped",
            serde_json::json!({ "reason": "off_screen" }),
        );
        return None;
    }
    let _ = win.set_position(tauri::LogicalPosition::new(state.x, state.y));
    let _ = win.set_size(tauri::LogicalSize::new(state.width, state.height));
    if state.maximized {
        let _ = win.maximize();
    }
    Some(state.monitor_label.clone())
}

/// Read `privacy.json`, or fall back to `PrivacyConfigDto::default()` if
/// the file is missing/corrupt. Lenient — same stance as `load_relays`:
/// a hand-edited config that fails to parse should not brick the app.
fn load_privacy(app: &AppHandle) -> PrivacyConfigDto {
    privacy_path(app)
        .ok()
        .and_then(|p| fs::read(&p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_privacy(app: &AppHandle, dto: &PrivacyConfigDto) -> anyhow::Result<()> {
    let path = privacy_path(app)?;
    let buf = serde_json::to_vec_pretty(dto)?;
    fs::write(&path, buf).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Build a `BridgeProvider` honouring both the persisted relay list AND
/// the persisted privacy config. Used by every send / subscribe path so
/// MaximumStealth flips Tor/Nym SOCKS5 routing on uniformly. A corrupt
/// `privacy.json` quietly degrades to `DailyUse` defaults — same lenient
/// behaviour as `load_privacy`.
fn current_relay(app: &AppHandle) -> Box<dyn BridgeProvider> {
    let urls = load_relays(app);
    let url_refs: Vec<&str> = urls.iter().map(|s| s.as_str()).collect();
    let dto = load_privacy(app);
    let cfg = dto.to_core().unwrap_or_default();
    let stealth = cfg.mode == PrivacyMode::MaximumStealth;
    let proxy = if stealth { Some(cfg.proxy.addr.as_str()) } else { None };
    make_multi_relay(&url_refs, stealth, proxy)
}

fn mls_directory_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(MLS_DIRECTORY_FILE))
}

/// Resolve the directory backing `PhantomMlsMember`'s persistent KV
/// store (`$APPDATA/de.dc-infosec.phantomchat/mls_state/`). Created on
/// demand so the very first `mls_init` doesn't have to defensively mkdir.
fn mls_storage_dir(app: &AppHandle) -> anyhow::Result<PathBuf> {
    let dir = app_data(app)?.join(MLS_STATE_DIR);
    fs::create_dir_all(&dir).with_context(|| format!("mkdir -p {}", dir.display()))?;
    Ok(dir)
}

// ── MLS directory persistence ────────────────────────────────────────────────
//
// On-disk format (`$APPDATA/de.dc-infosec.phantomchat/mls_directory.json`):
//
//   {
//     "identity_label": "alice",
//     "members": [
//       {
//         "label":           "bob",
//         "address":         "phantom:<view-hex>:<spend-hex>",
//         "signing_pub_hex": "deadbeef…"
//       },
//       ...
//     ]
//   }
//
// Auto-loaded on `mls_init` so a previously-built directory survives a
// desktop restart even though the OpenMLS provider's tree itself does not.
// (The bundle still has to be re-initialized by `mls_init` or auto-init —
// we just rehydrate the transport pointers, not the cryptographic state.)
//
// Best-effort: a corrupt file is treated as "no directory" rather than a
// hard failure, mirroring `load_history`'s lenient stance.

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct MlsDirectoryDisk {
    #[serde(default)]
    identity_label: String,
    #[serde(default)]
    members: Vec<MlsMemberRef>,
}

fn load_mls_directory(path: &std::path::Path) -> MlsDirectoryDisk {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_mls_directory(
    path: &std::path::Path,
    identity_label: &str,
    members: &[MlsMemberRef],
) -> anyhow::Result<()> {
    let disk = MlsDirectoryDisk {
        identity_label: identity_label.to_string(),
        members: members.to_vec(),
    };
    fs::write(path, serde_json::to_vec_pretty(&disk)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

// ── Identity persistence (mirrors `cli::cmd_keygen` JSON layout) ─────────────
//
// Wave 8H — OS-secure keystore
// ----------------------------
// The on-disk `keys.json` no longer carries `view_private` /
// `spend_private` / `signing_private` plaintext. Instead it carries
// `view_private_ref` / `spend_private_ref` / `signing_private_ref`
// — opaque key_ids that resolve to byte blobs in the host's secure
// keystore (DPAPI on Windows, Keychain on macOS, libsecret on Linux,
// or — on hosts with no keystore — the in-process fallback).
//
// Migration is automatic and one-way: the first time `load_identity`
// sees a legacy plaintext keyfile it stores the secrets in the keystore,
// rewrites `keys.json` atomically (`.tmp` + fsync + rename), and emits an
// `audit("identity", "migrated_to_secure_storage", …)` line. If anything
// fails before the rename, the original plaintext keys.json survives so
// the user never gets locked out of their identity.
//
// `signing_public` and `view_public` / `spend_public` continue to live
// inline as hex — they are by design shareable.

/// Lazily-initialised handle onto the host's secure keystore. We initialise
/// once per process and reuse the same `Box<dyn SecureStorage>` for every
/// migration / load / write. Wrapped in `OnceLock` so the first identity
/// access on the hot path doesn't pay a dispatch cost on every subsequent
/// call.
static SECURE_STORAGE: std::sync::OnceLock<Box<dyn SecureStorage>> = std::sync::OnceLock::new();

fn secure_storage() -> &'static dyn SecureStorage {
    SECURE_STORAGE.get_or_init(detect_best_storage).as_ref()
}

/// Compose the per-secret key_id from a path-derived prefix. Keeps the
/// three secrets under one identity-file logically grouped in the OS
/// keystore (`secret-tool search service phantomchat` lists them together)
/// and lets `wipe_all_data` purge them by enumerating the suffixes.
fn secret_key_id(path: &std::path::Path, suffix: &str) -> String {
    format!("{}:{}", key_id_for_path(path), suffix)
}

const SECRET_SUFFIXES: &[&str] = &["view", "spend", "signing", "identity"];

/// Atomic write helper: serialise `value` to `path.tmp`, fsync, rename
/// onto `path`. If anything errors before the rename, the original file
/// (if any) is preserved bit-for-bit. Used by the migration path so a
/// power-loss between "stash secrets in keystore" and "rewrite keys.json"
/// never leaves a half-converted file on disk.
fn atomic_write_json(path: &std::path::Path, value: &serde_json::Value) -> anyhow::Result<()> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        let bytes = serde_json::to_vec_pretty(value)?;
        f.write_all(&bytes).with_context(|| format!("write {}", tmp.display()))?;
        f.sync_all().with_context(|| format!("fsync {}", tmp.display()))?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Stash a single secret (already base64-decoded) in the host keystore
/// under the canonical per-path key_id, and return the key_id string the
/// caller should write into the rewritten keys.json.
fn stash_secret(
    path: &std::path::Path,
    suffix: &str,
    secret: &[u8],
) -> Result<String, SecureStorageError> {
    let id = secret_key_id(path, suffix);
    secure_storage().store(&id, secret)?;
    Ok(id)
}

/// Pull a secret back from the keystore by its `_ref` value. Returned in a
/// `Zeroizing<Vec<u8>>` so the plaintext is wiped from RAM as soon as the
/// caller's deref-and-copy into a typed key struct returns.
fn fetch_secret(key_id: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    let bytes = secure_storage()
        .load(key_id)
        .with_context(|| format!("secure-storage load '{key_id}'"))?;
    Ok(Zeroizing::new(bytes))
}

/// Convert a keys.json that still carries `*_private` plaintext into the
/// new `*_private_ref` form. Returns `Ok(true)` iff a migration ran.
///
/// Atomicity contract: if any single keystore-store call fails (or the
/// final atomic rewrite fails), the on-disk keys.json is left untouched
/// and the caller can retry on the next launch.
fn migrate_keys_json_if_legacy(
    app: &AppHandle,
    path: &std::path::Path,
) -> anyhow::Result<bool> {
    let raw = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut json: serde_json::Value = serde_json::from_slice(&raw)?;

    let has_legacy = ["view_private", "spend_private", "signing_private", "identity_private"]
        .iter()
        .any(|f| json.get(*f).and_then(|v| v.as_str()).is_some());
    if !has_legacy {
        return Ok(false);
    }

    // Migrate every plaintext field that exists. `identity_private` is
    // legacy and may not appear on freshly-keygen'd files, so be lenient.
    for (field, suffix) in [
        ("view_private", "view"),
        ("spend_private", "spend"),
        ("signing_private", "signing"),
        ("identity_private", "identity"),
    ] {
        if let Some(s) = json.get(field).and_then(|v| v.as_str()) {
            // Wrap the decoded plaintext in `Zeroizing` so it's wiped from
            // RAM as soon as the keystore-store call returns, even on the
            // error path.
            let plaintext: Zeroizing<Vec<u8>> = Zeroizing::new(
                B64.decode(s).with_context(|| format!("decode {field} base64"))?,
            );
            let id = stash_secret(path, suffix, &plaintext)
                .map_err(|e| anyhow!("stash {field}: {e}"))?;
            if let Some(obj) = json.as_object_mut() {
                obj.remove(field);
                obj.insert(format!("{field}_ref"), serde_json::Value::String(id));
            }
        }
    }

    // Stamp which backend we used so a later operator audit can
    // differentiate a real keychain from the fallback plaintext store.
    let backend = secure_storage().name();
    if let Some(obj) = json.as_object_mut() {
        obj.insert(
            "storage_backend".to_string(),
            serde_json::Value::String(backend.to_string()),
        );
    }

    atomic_write_json(path, &json)?;

    audit(
        app,
        "identity",
        "migrated_to_secure_storage",
        serde_json::json!({
            "backend": backend,
        }),
    );
    if backend == "fallback-plaintext" {
        audit(
            app,
            "identity",
            "secure_storage_fallback_warning",
            serde_json::json!({
                "reason": "no OS keystore detected; secrets live in process memory only"
            }),
        );
    }

    Ok(true)
}

/// Load view + spend + signing keys from the on-disk keyfile.
///
/// Handles three input shapes:
///
/// 1. **Legacy plaintext** (`view_private`, etc.) — auto-migrated to
///    the secure-storage form on first sight, then loaded as case 2.
/// 2. **Secure-storage refs** (`view_private_ref`, etc.) — the
///    canonical post-Wave-8H format; secrets fetched from
///    `secure_storage()`.
/// 3. **Mixed / partial** (e.g. `signing_private` absent on
///    pre-attribution keyfiles) — the missing piece is generated
///    fresh, written to the keystore, and the keyfile is upgraded
///    in-place. The fourth tuple element flags such an upgrade so
///    the caller can emit a one-time `status` event.
fn load_identity(
    app: &AppHandle,
    path: &std::path::Path,
) -> anyhow::Result<(ViewKey, SpendKey, PhantomSigningKey, bool)> {
    // Step 1 — best-effort auto-migration. A migration failure surfaces
    // via the load below (the *_private fields will still be there, the
    // *_private_ref fields will be missing) so we don't gate the entire
    // load on it; just log if it errored.
    if let Err(e) = migrate_keys_json_if_legacy(app, path) {
        eprintln!("identity migration to secure-storage failed: {e:#}");
    }

    let raw = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut json: serde_json::Value = serde_json::from_slice(&raw)?;

    // Helper: load a single secret either by `_ref` (preferred) or by
    // legacy plaintext (fallback if the migration above failed).
    let load_secret = |json: &serde_json::Value,
                       field: &str|
     -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let ref_field = format!("{field}_ref");
        if let Some(id) = json.get(&ref_field).and_then(|v| v.as_str()) {
            return fetch_secret(id);
        }
        if let Some(s) = json.get(field).and_then(|v| v.as_str()) {
            return Ok(Zeroizing::new(B64.decode(s)?));
        }
        Err(anyhow!("missing {field} (or {ref_field})"))
    };

    let view_bytes = load_secret(&json, "view_private")?;
    let view_secret = StaticSecret::from(
        <[u8; 32]>::try_from(view_bytes.as_slice()).map_err(|_| anyhow!("bad view key"))?,
    );
    let view_key = ViewKey {
        public: PublicKey::from(&view_secret),
        secret: view_secret,
    };

    let spend_bytes = load_secret(&json, "spend_private")?;
    let spend_secret = StaticSecret::from(
        <[u8; 32]>::try_from(spend_bytes.as_slice()).map_err(|_| anyhow!("bad spend key"))?,
    );
    let spend_key = SpendKey {
        public: PublicKey::from(&spend_secret),
        secret: spend_secret,
    };

    // Signing key handling: prefer the `_ref` form, then plaintext, then
    // generate-and-persist if the keyfile predates sealed-sender
    // attribution. The `upgraded` flag fires only in that last case.
    let (signing, upgraded) = match load_secret(&json, "signing_private") {
        Ok(bytes) => {
            let arr: [u8; 32] = <[u8; 32]>::try_from(bytes.as_slice())
                .map_err(|_| anyhow!("bad signing key"))?;
            (PhantomSigningKey::from_bytes(arr), false)
        }
        Err(_) => {
            let sk = PhantomSigningKey::generate();
            // Persist directly into the keystore + a `_ref` in keys.json.
            // Best-effort: a failure here just means the next launch will
            // generate a different signing key (cosmetic — sealed-sender
            // attribution will show a different signing_pub).
            let plaintext: Zeroizing<[u8; 32]> = Zeroizing::new(sk.to_bytes());
            if let Ok(id) = stash_secret(path, "signing", plaintext.as_ref()) {
                if let Some(obj) = json.as_object_mut() {
                    obj.insert(
                        "signing_private_ref".to_string(),
                        serde_json::Value::String(id),
                    );
                    obj.insert(
                        "signing_public".to_string(),
                        serde_json::Value::String(hex::encode(sk.public_bytes())),
                    );
                    obj.remove("signing_private");
                    let _ = atomic_write_json(path, &json);
                }
            }
            (sk, true)
        }
    };

    Ok((view_key, spend_key, signing, upgraded))
}

fn write_identity(path: &std::path::Path) -> anyhow::Result<IdentityInfo> {
    let id = IdentityKey::generate();
    let view = ViewKey::generate();
    let spend = SpendKey::generate();
    let signing = PhantomSigningKey::generate();

    // Stash every secret in the host keystore first, build the on-disk
    // keys.json out of the returned key_ids. Identity-private is also
    // stashed even though current call sites don't read it back — keeps
    // the wire-format symmetry with the CLI keyfile (so an export/import
    // round-trip survives a future feature that needs the identity key).
    //
    // `Zeroizing` wraps each plaintext blob so it's wiped from RAM as
    // soon as `stash_secret` returns. The struct fields themselves
    // already zeroize-on-drop via the `Zeroize` derives in `core::keys`.
    let view_priv: Zeroizing<[u8; 32]> = Zeroizing::new(view.secret.to_bytes());
    let spend_priv: Zeroizing<[u8; 32]> = Zeroizing::new(spend.secret.to_bytes());
    let signing_priv: Zeroizing<[u8; 32]> = Zeroizing::new(signing.to_bytes());
    let identity_priv: Zeroizing<[u8; 32]> = Zeroizing::new(id.private);

    let view_ref = stash_secret(path, "view", view_priv.as_ref())
        .map_err(|e| anyhow!("stash view: {e}"))?;
    let spend_ref = stash_secret(path, "spend", spend_priv.as_ref())
        .map_err(|e| anyhow!("stash spend: {e}"))?;
    let signing_ref = stash_secret(path, "signing", signing_priv.as_ref())
        .map_err(|e| anyhow!("stash signing: {e}"))?;
    let identity_ref = stash_secret(path, "identity", identity_priv.as_ref())
        .map_err(|e| anyhow!("stash identity: {e}"))?;

    // Field names + encodings of the public-side fields match
    // `cli/src/main.rs::cmd_keygen` exactly so the address/QR shape is
    // identical to the CLI keyfile. The `*_ref` fields are new and only
    // make sense to a desktop install on the same host.
    let json = serde_json::json!({
        "identity_private_ref": identity_ref,
        "identity_public":      B64.encode(id.public),
        "view_private_ref":     view_ref,
        "view_public":          hex::encode(view.public.as_bytes()),
        "spend_private_ref":    spend_ref,
        "spend_public":         hex::encode(spend.public.as_bytes()),
        "signing_private_ref":  signing_ref,
        "signing_public":       hex::encode(signing.public_bytes()),
        "storage_backend":      secure_storage().name(),
    });

    atomic_write_json(path, &json)?;

    Ok(IdentityInfo {
        address: format!(
            "phantom:{}:{}",
            hex::encode(view.public.as_bytes()),
            hex::encode(spend.public.as_bytes())
        ),
        view_pub_short: hex::encode(view.public.as_bytes())[..16].to_string(),
        spend_pub_short: hex::encode(spend.public.as_bytes())[..16].to_string(),
        signing_pub_short: hex::encode(signing.public_bytes())[..16].to_string(),
    })
}

fn address_from_keyfile(path: &std::path::Path) -> anyhow::Result<String> {
    let raw = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let json: serde_json::Value = serde_json::from_slice(&raw)?;
    let view_pub = json["view_public"]
        .as_str()
        .context("missing view_public")?;
    let spend_pub = json["spend_public"]
        .as_str()
        .context("missing spend_public")?;
    Ok(format!("phantom:{}:{}", view_pub, spend_pub))
}

fn load_contacts(path: &std::path::Path) -> ContactBook {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_contacts(path: &std::path::Path, book: &ContactBook) -> anyhow::Result<()> {
    fs::write(path, serde_json::to_vec_pretty(book)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

// ── Tauri commands ───────────────────────────────────────────────────────────
// Each command returns `Result<T, String>` because Tauri requires the error
// type to be `Serialize`; `anyhow::Error` isn't, so we stringify at the
// boundary.

#[tauri::command]
fn generate_identity(app: AppHandle) -> Result<IdentityInfo, String> {
    let path = keys_path(&app).map_err(|e| e.to_string())?;
    if path.exists() {
        // Don't accidentally clobber an existing identity. The frontend
        // should call `get_address` first.
        return Err("identity already exists; refusing to overwrite".into());
    }
    let info = write_identity(&path).map_err(|e| e.to_string())?;
    audit(
        &app,
        "identity",
        "created",
        serde_json::json!({
            "view_pub_short": info.view_pub_short,
            "spend_pub_short": info.spend_pub_short,
            "signing_pub_short": info.signing_pub_short,
        }),
    );
    Ok(info)
}

#[tauri::command]
fn get_address(app: AppHandle) -> Result<String, String> {
    let path = keys_path(&app).map_err(|e| e.to_string())?;
    if !path.exists() {
        return Err("no identity yet — call generate_identity".into());
    }
    address_from_keyfile(&path).map_err(|e| e.to_string())
}

/// Render the user's `phantom:view:spend` address as a minimal SVG QR code.
///
/// Uses `qrcodegen` with medium error correction — the same setting the CLI
/// uses for `phantom pair`, so a QR scanned from the desktop app and one
/// scanned from CLI output decode identically.
///
/// The SVG is intentionally style-free: a single `viewBox`-anchored `<svg>`
/// root and one `<rect>` per dark module. The frontend's `AddressQR`
/// component applies CSS to color the modules `neon-green` against a
/// transparent background, which keeps the inline render small + themeable.
#[tauri::command]
fn address_qr_svg(app: AppHandle) -> Result<String, String> {
    let path = keys_path(&app).map_err(|e| e.to_string())?;
    if !path.exists() {
        return Err("no identity yet — call generate_identity".into());
    }
    let address = address_from_keyfile(&path).map_err(|e| e.to_string())?;
    let qr = qrcodegen::QrCode::encode_text(&address, qrcodegen::QrCodeEcc::Medium)
        .map_err(|e| format!("qr encode failed: {}", e))?;
    Ok(qr_to_svg(&qr, 4))
}

/// Convert a `QrCode` into a minimal SVG string. `border` is the quiet-zone
/// width in modules (the QR spec recommends 4). One `<rect>` per dark
/// module, no inline styles — coloring is the frontend's job.
fn qr_to_svg(qr: &qrcodegen::QrCode, border: i32) -> String {
    let size = qr.size();
    let dim = size + border * 2;
    let mut svg = String::with_capacity(2048 + (size as usize) * (size as usize) * 16);
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {dim} {dim}\" \
         shape-rendering=\"crispEdges\">",
        dim = dim
    ));
    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                svg.push_str(&format!(
                    "<rect x=\"{x}\" y=\"{y}\" width=\"1\" height=\"1\"/>",
                    x = x + border,
                    y = y + border
                ));
            }
        }
    }
    svg.push_str("</svg>");
    svg
}

#[tauri::command]
fn list_contacts(app: AppHandle) -> Result<Vec<Contact>, String> {
    let path = contacts_path(&app).map_err(|e| e.to_string())?;
    Ok(load_contacts(&path).contacts)
}

#[tauri::command]
fn add_contact(app: AppHandle, label: String, address: String) -> Result<(), String> {
    // PhantomAddress::parse returns Option, NOT Result — the CLI's tui.rs
    // got this wrong once, so be careful here.
    if PhantomAddress::parse(&address).is_none() {
        return Err(format!(
            "invalid address — expected 'phantom:view:spend' (got {} chars)",
            address.len()
        ));
    }
    let path = contacts_path(&app).map_err(|e| e.to_string())?;
    let mut book = load_contacts(&path);
    if book.contacts.iter().any(|c| c.label == label) {
        return Err(format!("contact '{}' already exists", label));
    }
    let label_for_audit = label.clone();
    book.contacts.push(Contact {
        label,
        address,
        signing_pub: None,
    });
    save_contacts(&path, &book).map_err(|e| e.to_string())?;
    audit(
        &app,
        "contact",
        "added",
        serde_json::json!({ "label": label_for_audit }),
    );
    Ok(())
}

#[tauri::command]
async fn send_message(
    app: AppHandle,
    contact_label: String,
    body: String,
) -> Result<String, String> {
    // Compute the msg_id BEFORE shipping so the frontend can stamp the
    // outgoing row with the same id the receiver will derive at decode
    // time. Returns the 16-hex id so React can store it on the outgoing
    // MsgLine + escalate `delivery_state` when receipts arrive.
    let msg_id = compute_msg_id("", body.as_bytes());
    send_message_inner(app, contact_label, body)
        .await
        .map_err(|e| e.to_string())?;
    Ok(msg_id)
}

/// Wrap arbitrary bytes in a sealed-sender envelope and publish them to the
/// default relay using the caller's identity. Shared by both the 1:1 send
/// path (`send_message_inner`) and the MLS auto-transport path
/// (`mls_add_member` / `mls_encrypt`).
///
/// Loads identity + sessions from disk on every call. Persists the updated
/// session state back before returning. Errors bubble up via anyhow.
async fn send_sealed_to_address(
    app: &AppHandle,
    recipient_addr: &str,
    payload: &[u8],
) -> anyhow::Result<()> {
    let keys_path = keys_path(app)?;
    let sessions_path = sessions_path(app)?;

    let recipient = PhantomAddress::parse(recipient_addr)
        .ok_or_else(|| anyhow!("invalid recipient address '{}'", recipient_addr))?;

    let (_view, _spend, signing_key, _upgraded) = load_identity(app, &keys_path)?;

    let mut store = SessionStore::load(&sessions_path)
        .with_context(|| format!("loading sessions from {}", sessions_path.display()))?;

    let envelope = store.send_sealed(&recipient, payload, &signing_key, POW_DIFFICULTY);

    // Fan publish out across the persisted relay set. `current_relay`
    // honours both `relays.json` and `privacy.json` — a single-URL
    // setup short-circuits to the direct path inside `make_multi_relay`,
    // and MaximumStealth re-routes everything via the configured SOCKS5
    // proxy. Same handle shape used by `start_listener`.
    let relay = current_relay(app);
    relay
        .publish(envelope)
        .await
        .map_err(|e| anyhow!("relay publish: {}", e))?;

    store
        .save(&sessions_path)
        .with_context(|| format!("saving sessions to {}", sessions_path.display()))?;

    Ok(())
}

async fn send_message_inner(
    app: AppHandle,
    contact_label: String,
    body: String,
) -> anyhow::Result<()> {
    let contacts_path = contacts_path(&app)?;

    // Load + locate contact.
    let book = load_contacts(&contacts_path);
    let contact = book
        .contacts
        .iter()
        .find(|c| c.label == contact_label)
        .ok_or_else(|| anyhow!("unknown contact '{}'", contact_label))?;

    // Sealed-sender send: stamps the envelope with our Ed25519 signature so
    // the recipient can attribute the message to a known contact (and so
    // anyone in possession of an unrelated spend key cannot forge our
    // identity). PoW difficulty 8 — instant on a desktop, still > zero on
    // the wire.
    send_sealed_to_address(&app, &contact.address, body.as_bytes()).await
}

#[tauri::command]
async fn start_listener(
    app: AppHandle,
    state: State<'_, AppState>,
    relay_url: Option<String>,
) -> Result<String, String> {
    {
        let mut started = state.listener_started.lock().map_err(|e| e.to_string())?;
        if *started {
            return Ok("listener already running".into());
        }
        *started = true;
    }
    spawn_listener_task(app, &state, relay_url).await
}

/// Build the relay handle + subscriber task for the current privacy /
/// relay config and store the resulting `JoinHandle` in `AppState` so
/// `restart_listener` can abort it later. Shared by both
/// [`start_listener`] (first launch) and [`restart_listener`] (privacy
/// or relay-set change).
async fn spawn_listener_task(
    app: AppHandle,
    state: &State<'_, AppState>,
    relay_url: Option<String>,
) -> Result<String, String> {
    // Resolve the active relay set. Order of precedence:
    //   1. Explicit `relay_url` arg (single relay — legacy CLI/test path).
    //   2. Persisted `relays.json` (multi-relay default).
    //   3. `DEFAULT_RELAYS` baked-in three-relay set (only if relays.json is
    //      empty/missing/corrupt — handled inside `load_relays`).
    let relay_urls: Vec<String> = match relay_url.clone() {
        Some(u) => vec![u],
        None => load_relays(&app),
    };

    let keys_path = keys_path(&app).map_err(|e| e.to_string())?;
    let contacts_path = contacts_path(&app).map_err(|e| e.to_string())?;
    let sessions_path = sessions_path(&app).map_err(|e| e.to_string())?;

    let (view_key, spend_key, _signing, upgraded) =
        load_identity(&app, &keys_path).map_err(|e| e.to_string())?;
    if upgraded {
        let _ = app.emit(
            "status",
            "Upgraded keyfile: generated fresh Ed25519 signing key for sealed-sender attribution."
                .to_string(),
        );
    }

    let store = SessionStore::load(&sessions_path).map_err(|e| e.to_string())?;
    let store = Arc::new(AsyncMutex::new(store));

    // Bridge the synchronous `BridgeProvider::subscribe` callback — which
    // can't await — onto a tokio task per envelope so we can lock the
    // async store and emit events back to the window.
    let app_for_handler = app.clone();
    let store_for_handler = Arc::clone(&store);
    let view_for_handler = view_key.clone();
    let spend_for_handler = spend_key.clone();
    let save_path = sessions_path.clone();
    let contacts_path_for_handler = contacts_path.clone();

    let handler: EnvelopeHandler = Box::new(move |env| {
        let app = app_for_handler.clone();
        let store = Arc::clone(&store_for_handler);
        let view = view_for_handler.clone();
        let spend = spend_for_handler.clone();
        let save_path = save_path.clone();
        let contacts_path = contacts_path_for_handler.clone();

        tokio::spawn(async move {
            let mut guard = store.lock().await;
            match guard.receive_full(&env, &view, &spend, None) {
                Ok(Some(msg)) => {
                    let _ = guard.save(&save_path);
                    drop(guard);

                    let (sender_pub, sig_ok) = match msg.sender {
                        Some((attr, ok)) => (Some(attr.sender_pub), ok),
                        None => (None, true),
                    };

                    // ── MLS auto-transport: peek the magic prefix BEFORE
                    // running the 1:1 attribution + emit path. The MLS
                    // branches consume the message entirely (their own
                    // emits) so the 1:1 fallthrough only fires for non-MLS
                    // payloads. We accept BOTH the legacy `MLS-WLC1`
                    // (placeholder inviter) and the new `MLS-WLC2`
                    // (inviter metadata inline) formats so any in-flight
                    // v1 envelope still lands.
                    if msg.plaintext.len() >= MLS_WLC_PREFIX_V1.len() {
                        let prefix = &msg.plaintext[..MLS_WLC_PREFIX_V1.len()];
                        if prefix == &MLS_WLC_PREFIX_V2[..] {
                            handle_incoming_mls_welcome_v2(
                                &app,
                                &msg.plaintext[MLS_WLC_PREFIX_V2.len()..],
                                sender_pub,
                                sig_ok,
                            );
                            return;
                        }
                        if prefix == &MLS_WLC_PREFIX_V1[..] {
                            handle_incoming_mls_welcome_v1(
                                &app,
                                &msg.plaintext[MLS_WLC_PREFIX_V1.len()..],
                                sender_pub,
                                sig_ok,
                            );
                            return;
                        }
                        if prefix == &MLS_APP_PREFIX[..] {
                            handle_incoming_mls_app(
                                &app,
                                &msg.plaintext[MLS_APP_PREFIX.len()..],
                                sender_pub,
                                sig_ok,
                            );
                            return;
                        }
                        if prefix == &FILE_PREFIX_V1[..] {
                            // 1:1 sender attribution mirrors the text path
                            // below — the file-receive helper resolves
                            // contact label / INBOX / "?<8hex>" itself so the
                            // emitted `file_received` event carries the
                            // human-friendly `from_label`.
                            handle_incoming_file_v1(
                                &app,
                                &msg.plaintext[FILE_PREFIX_V1.len()..],
                                sender_pub,
                                sig_ok,
                                &contacts_path,
                            );
                            return;
                        }
                    }
                    // Receipts + typing are 7-byte prefixes. Place them
                    // alongside the existing 8-byte branches above without
                    // disturbing the legacy dispatch order.
                    if msg.plaintext.len() >= RCPT_PREFIX_V1.len() {
                        let prefix7 = &msg.plaintext[..RCPT_PREFIX_V1.len()];
                        if prefix7 == &RCPT_PREFIX_V1[..] {
                            handle_incoming_receipt_v1(
                                &app,
                                &msg.plaintext[RCPT_PREFIX_V1.len()..],
                                sender_pub,
                                sig_ok,
                                &contacts_path,
                            );
                            return;
                        }
                        if prefix7 == &TYPN_PREFIX_V1[..] {
                            handle_incoming_typing_v1(
                                &app,
                                &msg.plaintext[TYPN_PREFIX_V1.len()..],
                                sender_pub,
                                sig_ok,
                                &contacts_path,
                            );
                            return;
                        }
                        if prefix7 == &REPL_PREFIX_V1[..] {
                            handle_incoming_reply_v1(
                                &app,
                                &msg.plaintext[REPL_PREFIX_V1.len()..],
                                sender_pub,
                                sig_ok,
                                &contacts_path,
                            );
                            return;
                        }
                        if prefix7 == &RACT_PREFIX_V1[..] {
                            handle_incoming_reaction_v1(
                                &app,
                                &msg.plaintext[RACT_PREFIX_V1.len()..],
                                sender_pub,
                                sig_ok,
                                &contacts_path,
                            );
                            return;
                        }
                        if prefix7 == &DISA_PREFIX_V1[..] {
                            handle_incoming_disappearing_v1(
                                &app,
                                &msg.plaintext[DISA_PREFIX_V1.len()..],
                                sender_pub,
                                sig_ok,
                                &contacts_path,
                            );
                            return;
                        }
                    }

                    // Resolve sender attribution against the contact list.
                    //   - No attribution           → "INBOX"
                    //   - Attribution + bad sig    → "INBOX!"   (don't trust pub)
                    //   - Attribution + matched    → contact label
                    //   - Attribution + unmatched  → "?<8-hex>" + remember
                    //     the pubkey in app state so the frontend can call
                    //     `bind_last_unbound_sender(label)`.
                    let (sender_label, sender_pub_hex) = match sender_pub {
                        None => ("INBOX".to_string(), None),
                        Some(_) if !sig_ok => ("INBOX!".to_string(), None),
                        Some(pub_bytes) => {
                            let pub_hex = hex::encode(pub_bytes);
                            let book = load_contacts(&contacts_path);
                            let matched = book
                                .contacts
                                .iter()
                                .find(|c| {
                                    c.signing_pub
                                        .as_deref()
                                        .map(|h| h.eq_ignore_ascii_case(&pub_hex))
                                        .unwrap_or(false)
                                })
                                .map(|c| c.label.clone());
                            match matched {
                                Some(lbl) => (lbl, Some(pub_hex)),
                                None => {
                                    // Stash for the upcoming `bind_last_unbound_sender`
                                    // call. Best-effort: a poisoned mutex
                                    // shouldn't kill the listener.
                                    if let Some(state) =
                                        app.try_state::<AppState>()
                                    {
                                        if let Ok(mut slot) =
                                            state.last_unbound_sender.lock()
                                        {
                                            *slot = Some(pub_bytes);
                                        }
                                    }
                                    let label = format!("?{}", &pub_hex[..8]);
                                    (label, Some(pub_hex))
                                }
                            }
                        }
                    };

                    // Compute the stable msg_id over (timestamp || plaintext_hex)
                    // so the sender (which derived the same id at send time)
                    // can match the upcoming `delivered` / `read` receipts
                    // back to the right outgoing row.
                    let ts = chrono::Local::now().format("%H:%M:%S").to_string();
                    let msg_id = compute_msg_id(&ts, &msg.plaintext);

                    // Apply per-contact disappearing TTL on plain text
                    // rows so the receiver's auto-purge clock starts at
                    // local now() (sender + receiver clocks may drift,
                    // small delta is acceptable per the spec).
                    let expires_at = match sender_label.as_str() {
                        "INBOX" | "INBOX!" => None,
                        l if l.starts_with('?') => None,
                        l => {
                            let disk = load_disappearing(&app);
                            disk.entries.get(l).map(|secs| now_unix_secs() + (*secs as u64))
                        }
                    };

                    let payload = IncomingMessage {
                        plaintext: String::from_utf8_lossy(&msg.plaintext).to_string(),
                        timestamp: ts.clone(),
                        sender_label: sender_label.clone(),
                        sig_ok,
                        sender_pub_hex,
                        direction: "incoming".to_string(),
                        kind: None,
                        file_meta: None,
                        msg_id: Some(msg_id.clone()),
                        delivery_state: None,
                        reply_to: None,
                        reactions: Vec::new(),
                        expires_at,
                    };
                    maybe_notify(
                        &app,
                        &format!("From {}", payload.sender_label),
                        &payload.plaintext,
                    );
                    let _ = app.emit("message", payload);

                    // Auto-emit a `delivered` receipt to the sender so they
                    // see ✓✓ even if the recipient never scrolls. We only
                    // do this for sender-attributed (matched-contact) 1:1
                    // text payloads — `INBOX` / `INBOX!` / `?<hex>` rows
                    // have no resolvable contact to ship a receipt back to.
                    if !sender_label.starts_with('?')
                        && sender_label != "INBOX"
                        && sender_label != "INBOX!"
                    {
                        let app_for_rcpt = app.clone();
                        let label_for_rcpt = sender_label.clone();
                        let id_for_rcpt = msg_id.clone();
                        tokio::spawn(async move {
                            if let Err(e) = send_receipt(
                                &app_for_rcpt,
                                &label_for_rcpt,
                                &id_for_rcpt,
                                "delivered",
                            )
                            .await
                            {
                                let _ = app_for_rcpt
                                    .emit("error", format!("send delivered receipt: {e}"));
                            }
                        });
                    }
                }
                Ok(None) => {
                    // Not for us — emit a low-volume "scanned" tick so the
                    // frontend's footer counter can advance.
                    let _ = app.emit("scanned", ());
                }
                Err(e) => {
                    let _ = app.emit("error", format!("decrypt: {}", e));
                }
            }
        });
    });

    // Emit an initial "connecting" pill so the StatusFooter can show
    // greyed-out state during the first subscribe round-trip.
    emit_connection(&app, "connecting", None);

    // Spawn the actual relay subscribe loop in a detached task. We use the
    // multi-relay factory + `subscribe_with_state` so the StatusFooter pill
    // gets live updates as underlying relays connect/disconnect/reconnect.
    let relay_urls_for_task = relay_urls.clone();
    let app_for_task = app.clone();
    let app_for_state = app.clone();
    // Snapshot the privacy DTO BEFORE building the relay so the subscriber
    // task captures the same mode the rest of the send paths see at this
    // moment. A subsequent `set_privacy_config` -> `restart_listener` will
    // rebuild the task with the new mode; the previous task is aborted.
    let privacy_dto = load_privacy(&app);
    let privacy_cfg = privacy_dto.to_core().unwrap_or_default();
    let stealth = privacy_cfg.mode == PrivacyMode::MaximumStealth;
    let proxy_addr_owned: Option<String> =
        if stealth { Some(privacy_cfg.proxy.addr.clone()) } else { None };

    // State handler bridges relay-layer `ConnectionEvent`s onto the existing
    // 3-string vocabulary the frontend already speaks. `Reconnecting` carries
    // a richer payload (attempt + backoff) for tooltip surface — the
    // frontend's existing reducer just looks at `status` so the extra `note`
    // field is benign.
    let state_handler: StateHandler = Box::new(move |ev: ConnectionEvent| {
        match ev {
            ConnectionEvent::Connecting => emit_connection(&app_for_state, "connecting", None),
            ConnectionEvent::Connected => emit_connection(&app_for_state, "connected", None),
            ConnectionEvent::Disconnected(reason) => {
                emit_connection(&app_for_state, "disconnected", Some(reason));
            }
            ConnectionEvent::Reconnecting { attempt, backoff_secs } => {
                emit_connection(
                    &app_for_state,
                    "connecting",
                    Some(format!("retry #{} in {}s", attempt, backoff_secs)),
                );
            }
        }
    });

    let relay_count = relay_urls.len();
    // Graceful-shutdown channel: `restart_listener` sends on `shutdown_tx`,
    // which lights up the `shutdown_rx` arm of the inner `tokio::select!`.
    // The select picks whichever future completes first; on shutdown the
    // subscribe_with_state future is dropped (cancellation), and the
    // `relay` binding then goes out of scope so its owned per-relay
    // reconnect/dispatcher tasks shut down before the outer task returns.
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let task: JoinHandle<()> = tokio::spawn(async move {
        let url_refs: Vec<&str> = relay_urls_for_task.iter().map(|s| s.as_str()).collect();
        let relay = make_multi_relay(&url_refs, stealth, proxy_addr_owned.as_deref());
        tokio::select! {
            // Normal subscribe path. Returns Err on transport failure,
            // Ok if the inner loop ever exits cleanly (currently never —
            // the relay parks forever on its own).
            res = relay.subscribe_with_state(handler, state_handler) => {
                if let Err(e) = res {
                    let detail = format!("subscribe: {}", e);
                    let _ = app_for_task.emit("error", detail.clone());
                    emit_connection(&app_for_task, "disconnected", Some(detail));
                }
            }
            // Graceful shutdown: `restart_listener` (or app teardown)
            // signalled. The select arm completing here cancels the
            // subscribe future above. A dropped sender (Err on rx) is
            // treated identically — either way we exit the task.
            _ = shutdown_rx => {
                // Intentional no-op body — falling through to the explicit
                // `drop(relay)` below is what closes the WS connections.
            }
        }
        // Drop the relay handle EXPLICITLY before the task returns so
        // the per-relay WS connections see their owners disappear and
        // close their TCP halves with a proper close-frame instead of
        // a half-open hang. (The let-binding would drop on scope exit
        // anyway, but the explicit call documents the contract.)
        drop(relay);
    });

    // Persist the control handle so `restart_listener` can signal us.
    {
        let mut slot = state.subscriber.lock().await;
        *slot = Some(ListenerControl { handle: task, shutdown_tx });
    }

    Ok(format!("listening on {} relay(s)", relay_count))
}

/// Resolve an MLS sender pubkey against the bundle's transport directory.
///
/// Mirrors the 1:1 path's "?<8-hex>" placeholder convention so unfamiliar
/// senders surface visibly in the group log instead of silently appearing
/// as the raw pubkey hex.
fn resolve_mls_from_label(
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
    members: &[MlsMemberRef],
) -> String {
    match sender_pub {
        None => "INBOX".to_string(),
        Some(_) if !sig_ok => "INBOX!".to_string(),
        Some(pub_bytes) => {
            let pub_hex = hex::encode(pub_bytes);
            members
                .iter()
                .find(|m| m.signing_pub_hex.eq_ignore_ascii_case(&pub_hex))
                .map(|m| m.label.clone())
                .unwrap_or_else(|| format!("?{}", &pub_hex[..8]))
        }
    }
}

/// Look up the user's `signing_public` hex from `keys.json`. Used as the
/// auto-init identity label so a fresh receiver who's never called
/// `mls_init` can still join an incoming Welcome without manual setup.
fn auto_init_identity_label(app: &AppHandle) -> Option<String> {
    let path = keys_path(app).ok()?;
    let raw = fs::read(&path).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    json["signing_public"].as_str().map(|s| s.to_string())
}

/// Bootstrap an in-memory `MlsBundle` slot if absent. Mirrors the
/// auto-init logic of the original `mls_init`-less Welcome path, but
/// returns the slot guard so callers can keep mutating it.
///
/// On a fresh launch the bundle's storage dir is read so any persisted
/// state (group_id, member_count, member directory) lands in lockstep.
fn ensure_mls_bundle(app: &AppHandle, slot: &mut Option<MlsBundle>) -> Result<(), String> {
    if slot.is_some() {
        return Ok(());
    }
    let label = auto_init_identity_label(app).unwrap_or_else(|| "self".to_string());
    let storage_dir = mls_storage_dir(app).map_err(|e| e.to_string())?;
    let mut bundle = MlsBundle::new(label.clone(), &storage_dir)?;
    // Rehydrate any persisted directory matching this identity. The
    // identity label may have been promoted from the on-disk meta
    // (`mls_meta.json`), so honour the bundle's CURRENT label rather
    // than the auto-init one when matching.
    if let Ok(path) = mls_directory_path(app) {
        let disk = load_mls_directory(&path);
        if disk.identity_label == bundle.identity_label {
            bundle.member_addresses = disk.members;
        }
    }
    *slot = Some(bundle);
    Ok(())
}

/// Handle an incoming legacy `MLS-WLC1` payload: auto-init the bundle if
/// needed, then `join_via_welcome` with NO inviter metadata available.
/// Inviter shows up as `?<8hex>` until the user binds the contact.
fn handle_incoming_mls_welcome_v1(
    app: &AppHandle,
    welcome_bytes: &[u8],
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
) {
    let state = match app.try_state::<AppState>() {
        Some(s) => s,
        None => return,
    };
    let mut slot = match state.mls.lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    if let Err(e) = ensure_mls_bundle(app, &mut slot) {
        let _ = app.emit("error", format!("MLS auto-init failed: {}", e));
        return;
    }
    let bundle = match slot.as_mut() {
        Some(b) => b,
        None => return,
    };

    // V1 has no inline metadata, so we resolve against the existing
    // directory only — typically yields `?<8hex>` for an unfamiliar inviter.
    let from_label = resolve_mls_from_label(sender_pub, sig_ok, &bundle.member_addresses);
    let identity_label = bundle.identity_label.clone();

    if let Err(e) = bundle.join_via_welcome(welcome_bytes) {
        let _ = app.emit("error", format!("MLS join_via_welcome: {}", e));
        return;
    }
    let member_count = bundle.member_count();
    let directory_snapshot = bundle.member_addresses.clone();
    drop(slot);
    if let Ok(path) = mls_directory_path(app) {
        let _ = save_mls_directory(&path, &identity_label, &directory_snapshot);
    }

    let _ = app.emit(
        "mls_joined",
        serde_json::json!({
            "from_label": from_label,
            "group_member_count": member_count,
        }),
    );
}

/// Handle an incoming v2 `MLS-WLC2` payload: parse the inline inviter
/// metadata, push the inviter into our directory BEFORE joining (so the
/// emit's `from_label` and any next app message resolve to the human
/// label), then `join_via_welcome`.
fn handle_incoming_mls_welcome_v2(
    app: &AppHandle,
    body: &[u8],
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
) {
    let (meta, welcome_bytes) = match decode_wlc_v2(body) {
        Ok(t) => t,
        Err(e) => {
            let _ = app.emit("error", format!("MLS-WLC2 decode: {}", e));
            return;
        }
    };

    // Light validation. We don't reject on signing-pub-hex mismatch with
    // the sealed-sender pub since that's caller-asserted info; we DO
    // demand a parseable PhantomAddress so the directory entry stays
    // round-trippable through `send_sealed_to_address`.
    if PhantomAddress::parse(&meta.inviter_address).is_none() {
        let _ = app.emit(
            "error",
            format!(
                "MLS-WLC2 inviter_address malformed: '{}'",
                meta.inviter_address
            ),
        );
        return;
    }

    let state = match app.try_state::<AppState>() {
        Some(s) => s,
        None => return,
    };
    let mut slot = match state.mls.lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    if let Err(e) = ensure_mls_bundle(app, &mut slot) {
        let _ = app.emit("error", format!("MLS auto-init failed: {}", e));
        return;
    }
    let bundle = match slot.as_mut() {
        Some(b) => b,
        None => return,
    };

    // Insert inviter into the directory if not already present (idempotent
    // by signing-pub hex; label collisions are tolerated since the inviter
    // chose their own label and we're the joiner).
    let inviter_pub_hex = meta.inviter_signing_pub_hex.to_lowercase();
    if !bundle
        .member_addresses
        .iter()
        .any(|m| m.signing_pub_hex.eq_ignore_ascii_case(&inviter_pub_hex))
    {
        bundle.member_addresses.push(MlsMemberRef {
            label: meta.inviter_label.clone(),
            address: meta.inviter_address.clone(),
            signing_pub_hex: inviter_pub_hex,
        });
    }

    // Always trust the inline meta's label for the join event since it
    // lets the UI render "alice joined you" instead of "?abcd joined".
    // Sealed-sender attribution is still used to flag sig-fail.
    let from_label = match (sender_pub, sig_ok) {
        (Some(_), false) => "INBOX!".to_string(),
        _ => meta.inviter_label.clone(),
    };
    let identity_label = bundle.identity_label.clone();

    if let Err(e) = bundle.join_via_welcome(welcome_bytes) {
        let _ = app.emit("error", format!("MLS join_via_welcome: {}", e));
        return;
    }
    let member_count = bundle.member_count();
    let directory_snapshot = bundle.member_addresses.clone();
    drop(slot);
    if let Ok(path) = mls_directory_path(app) {
        let _ = save_mls_directory(&path, &identity_label, &directory_snapshot);
    }

    let _ = app.emit(
        "mls_joined",
        serde_json::json!({
            "from_label": from_label,
            "group_member_count": member_count,
        }),
    );
}

/// Handle an incoming `MLS-APP1` payload: `decrypt` and either emit
/// `mls_message` (with plaintext) or `mls_epoch` (when the wire was a
/// control message — the Some/None split mirrors the legacy `mls_decrypt`
/// command's `control_only` flag).
fn handle_incoming_mls_app(
    app: &AppHandle,
    wire_bytes: &[u8],
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
) {
    let state = match app.try_state::<AppState>() {
        Some(s) => s,
        None => return,
    };

    let mut slot = match state.mls.lock() {
        Ok(s) => s,
        Err(_) => return,
    };

    let bundle = match slot.as_mut() {
        Some(b) => b,
        None => {
            let _ = app.emit(
                "error",
                "MLS app-msg received but bundle not initialized — wait for a Welcome first"
                    .to_string(),
            );
            return;
        }
    };

    let from_label = resolve_mls_from_label(sender_pub, sig_ok, &bundle.member_addresses);

    match bundle.decrypt(wire_bytes) {
        Ok(Some(plain)) => {
            let member_count = bundle.member_count();
            drop(slot);
            let plaintext = String::from_utf8_lossy(&plain).to_string();
            maybe_notify(
                app,
                &format!("Group: {}", from_label),
                &plaintext,
            );
            let _ = app.emit(
                "mls_message",
                serde_json::json!({
                    "from_label": from_label,
                    "plaintext": plaintext,
                    "ts": chrono::Local::now().format("%H:%M:%S").to_string(),
                    "member_count": member_count,
                }),
            );
        }
        Ok(None) => {
            let member_count = bundle.member_count();
            drop(slot);
            let _ = app.emit(
                "mls_epoch",
                serde_json::json!({
                    "member_count": member_count,
                }),
            );
        }
        Err(e) => {
            let _ = app.emit("error", format!("MLS decrypt: {}", e));
        }
    }
}

/// Emit a `connection` event and cache the latest status in `AppState`. Safe
/// to call before `AppState` is initialized — the cache update is best-effort.
fn emit_connection(app: &AppHandle, status: &str, detail: Option<String>) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut slot) = state.connection_status.lock() {
            *slot = status.to_string();
        }
    }
    let _ = app.emit(
        "connection",
        serde_json::json!({ "status": status, "detail": detail }),
    );
}

/// Bind the most-recently-seen unbound sealed-sender pubkey to `contact_label`.
///
/// Frontend usage: when an incoming `message` event arrives with
/// `sender_label = "?<hex>"`, surface a "bind to contact" button for the
/// user. On click, call this command with the chosen contact's label. The
/// pubkey is persisted to `contacts.json` and subsequent messages from that
/// sender will resolve to the contact's name instead of the placeholder.
#[tauri::command]
fn bind_last_unbound_sender(
    app: AppHandle,
    state: State<'_, AppState>,
    contact_label: String,
) -> Result<(), String> {
    // Take (not just peek) — once the user binds, the slot is consumed so
    // an accidental double-click doesn't bind the same key to two contacts.
    let pub_bytes = {
        let mut slot = state
            .last_unbound_sender
            .lock()
            .map_err(|e| e.to_string())?;
        slot.take()
            .ok_or_else(|| "no unbound sender pending — wait for an incoming sealed message tagged ?<hex>".to_string())?
    };
    let pub_hex = hex::encode(pub_bytes);

    let path = contacts_path(&app).map_err(|e| e.to_string())?;
    let mut book = load_contacts(&path);
    let contact = book
        .contacts
        .iter_mut()
        .find(|c| c.label == contact_label)
        .ok_or_else(|| {
            // Restore the slot so the user can retry with the right label.
            if let Ok(mut slot) = state.last_unbound_sender.lock() {
                *slot = Some(pub_bytes);
            }
            format!("unknown contact '{}'", contact_label)
        })?;
    contact.signing_pub = Some(pub_hex);
    save_contacts(&path, &book).map_err(|e| e.to_string())?;
    audit(
        &app,
        "contact",
        "bound",
        serde_json::json!({ "label": contact_label }),
    );
    Ok(())
}

// ── MLS commands ─────────────────────────────────────────────────────────────
//
// All wire bytes (key packages, welcomes, application messages, commits)
// are base64-encoded so they round-trip cleanly through Tauri's JSON IPC.
//
// Out of scope here: hooking these into the relay transport. The user
// copy/pastes base64 strings between machines for now — same UX as the
// CLI's `mls-export-kp` / `mls-import-welcome` pair.

/// Persist a self-chosen human label to `me.json`. Used by
/// [`mls_add_member`] to populate the inviter metadata in V2 Welcome
/// envelopes so the joiner sees `alice` instead of `?abcd`.
///
/// Empty input clears the on-disk label (next call to
/// [`resolve_self_label`] will fall back to the first 8 chars of the
/// signing pub hex).
#[tauri::command]
fn set_my_label(app: AppHandle, label: String) -> Result<(), String> {
    let me = MeDisk { label: label.trim().to_string() };
    save_me(&app, &me).map_err(|e| e.to_string())
}

/// Read the persisted self-label, or `""` if none has been set yet.
/// Surfaced for the Settings UI so the user can see / edit the value
/// the inviter side will broadcast in V2 Welcomes.
#[tauri::command]
fn get_my_label(app: AppHandle) -> Result<String, String> {
    Ok(load_me(&app).label)
}

#[tauri::command]
fn mls_init(
    app: AppHandle,
    state: State<'_, AppState>,
    identity_label: String,
) -> Result<(), String> {
    let mut slot = state.mls.lock().map_err(|e| e.to_string())?;
    if slot.is_some() {
        // Idempotent.
        return Ok(());
    }
    let label = identity_label.trim().to_string();
    if label.is_empty() {
        return Err("identity_label must not be empty".into());
    }
    let storage_dir = mls_storage_dir(&app).map_err(|e| e.to_string())?;
    // Construct (or rehydrate) the file-backed MLS bundle. If
    // `mls_state.bin` + `mls_meta.json` already exist in `storage_dir`,
    // the bundle picks up the prior session's group_id / signing key
    // transparently — the user-supplied `identity_label` may be promoted
    // from the on-disk meta in that case.
    let mut bundle = MlsBundle::new(label.clone(), &storage_dir)?;
    // Rehydrate the auto-transport directory companion (this lives next
    // to keys.json, NOT inside mls_state/, to keep the human-readable
    // member list trivially editable).
    if let Ok(path) = mls_directory_path(&app) {
        let disk = load_mls_directory(&path);
        if disk.identity_label == bundle.identity_label {
            bundle.member_addresses = disk.members;
        }
    }
    *slot = Some(bundle);
    Ok(())
}

#[tauri::command]
fn mls_publish_key_package(state: State<'_, AppState>) -> Result<String, String> {
    let slot = state.mls.lock().map_err(|e| e.to_string())?;
    let bundle = slot
        .as_ref()
        .ok_or_else(|| "MLS not initialized — call mls_init first".to_string())?;
    let bytes = bundle.publish_key_package()?;
    Ok(B64.encode(bytes))
}

#[tauri::command]
fn mls_create_group(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut slot = state.mls.lock().map_err(|e| e.to_string())?;
    let bundle = slot
        .as_mut()
        .ok_or_else(|| "MLS not initialized".to_string())?;
    bundle.create_group()?;
    drop(slot);
    audit(&app, "mls", "created", serde_json::json!({}));
    Ok(())
}

/// Invite a new MLS member by their key package, AND register their
/// transport pointers so future welcomes / app messages auto-fly via the
/// existing sealed-sender relay path.
///
/// Argument shape (vs. legacy):
///   - `key_package_b64`         — opaque MLS KP, same as before.
///   - `member_label`            — human-readable directory entry name.
///   - `member_address`          — peer's PhantomAddress (`phantom:view:spend`).
///   - `member_signing_pub_hex`  — peer's Ed25519 sealed-sender pubkey.
///
/// On success: the resulting Welcome is sealed-sender-shipped to the new
/// member, the resulting Commit is sealed-sender-shipped to ALL existing
/// directory members AND the new member (so every epoch advances together
/// and new joiners see consistent state on their next app message). The
/// new member is then appended to the bundle's `member_addresses` and the
/// directory persisted to disk. No payload is returned — the frontend no
/// longer needs to display the welcome bytes.
#[tauri::command]
async fn mls_add_member(
    app: AppHandle,
    state: State<'_, AppState>,
    key_package_b64: String,
    member_label: String,
    member_address: String,
    member_signing_pub_hex: String,
) -> Result<(), String> {
    let kp_bytes = B64
        .decode(key_package_b64.trim())
        .map_err(|e| format!("base64 decode key_package: {e}"))?;

    let label = member_label.trim().to_string();
    let address = member_address.trim().to_string();
    let signing_pub_hex = member_signing_pub_hex.trim().to_lowercase();

    if label.is_empty() {
        return Err("member_label must not be empty".into());
    }
    if PhantomAddress::parse(&address).is_none() {
        return Err(format!(
            "invalid member_address — expected 'phantom:view:spend' (got {} chars)",
            address.len()
        ));
    }
    if signing_pub_hex.len() != 64 || hex::decode(&signing_pub_hex).is_err() {
        return Err("member_signing_pub_hex must be 32 bytes hex (64 chars)".into());
    }

    // Hold the lock just long enough to mutate the MLS group + capture the
    // wire bytes + updated directory, then drop before any await. Tauri
    // command tasks must be Send across awaits, and `StdMutex` guards
    // aren't.
    let identity_label;
    let new_member = MlsMemberRef {
        label: label.clone(),
        address: address.clone(),
        signing_pub_hex: signing_pub_hex.clone(),
    };
    let (commit_bytes, welcome_bytes, existing_recipients, directory_snapshot) = {
        let mut slot = state.mls.lock().map_err(|e| e.to_string())?;
        let bundle = slot
            .as_mut()
            .ok_or_else(|| "MLS not initialized".to_string())?;

        // Reject duplicate directory entries — `add_member` would still
        // succeed in MLS, but the per-recipient send loop would fan out
        // twice to the same address.
        if bundle
            .member_addresses
            .iter()
            .any(|m| m.label == label || m.signing_pub_hex == signing_pub_hex)
        {
            return Err(format!(
                "member '{}' (or that signing pubkey) already in directory",
                label
            ));
        }

        // Snapshot the EXISTING recipient list BEFORE pushing the new
        // member. The commit is meaningful only to those who already have
        // a group state to fast-forward — the new joiner ingests the
        // group state via the Welcome itself, NOT via a separate commit
        // (the commit would reference them as a brand-new leaf, which
        // openmls flags as nonsense from their just-joined perspective).
        let existing = bundle.member_addresses.clone();

        let (commit, welcome) = bundle.add_member(&kp_bytes)?;
        bundle.member_addresses.push(new_member.clone());

        identity_label = bundle.identity_label.clone();
        (
            commit,
            welcome,
            existing,
            bundle.member_addresses.clone(),
        )
    };

    // Persist the directory before any network I/O — if the relay is down
    // we still want the new member captured so a retry can re-fan-out.
    if let Ok(path) = mls_directory_path(&app) {
        let _ = save_mls_directory(&path, &identity_label, &directory_snapshot);
    }

    // Welcome → only the new joiner. Use the v2 wrapping so the joiner's
    // `mls_joined` event fires with our human label, not `?<8hex>`. The
    // self-label comes from `me.json` if the user has set one, else
    // first-8-hex of our signing pubkey for a stable fallback.
    let inviter_label = resolve_self_label(&app)
        .unwrap_or_else(|| identity_label.clone());
    let inviter_signing_pub_hex = auto_init_identity_label(&app).unwrap_or_default();
    let inviter_address = address_from_keyfile(
        &keys_path(&app).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("read self address: {e}"))?;
    let meta = MlsWelcomeMetaV2 {
        inviter_label,
        inviter_address,
        inviter_signing_pub_hex,
    };
    let welcome_payload = encode_wlc_v2(&meta, &welcome_bytes);
    if let Err(e) = send_sealed_to_address(&app, &address, &welcome_payload).await {
        return Err(format!("send welcome to '{}': {}", label, e));
    }

    // Commit → fan out to EXISTING members only. The commit advances
    // their epoch + rekeys around the new leaf. Do NOT send to the new
    // joiner — they consume the same epoch transition implicitly via the
    // Welcome's GroupInfo and would reject a duplicate add of themselves.
    //
    // Failures here are logged via the `error` event but do NOT roll
    // back the commit — the sender's group state has already advanced.
    // A peer that misses one commit can recover from a subsequent app
    // message (openmls re-keys on the next sync round).
    if !existing_recipients.is_empty() {
        let commit_payload = [&MLS_APP_PREFIX[..], &commit_bytes[..]].concat();
        for member in existing_recipients {
            if let Err(e) =
                send_sealed_to_address(&app, &member.address, &commit_payload).await
            {
                let _ = app.emit(
                    "error",
                    format!("MLS commit fan-out to '{}' failed: {}", member.label, e),
                );
            }
        }
    }

    // Member-count after the add is the size of the now-extended directory
    // plus self (we never appear in our own directory).
    let member_count_after = directory_snapshot.len() + 1;
    audit(
        &app,
        "mls",
        "added",
        serde_json::json!({
            "member_label": label,
            "member_count_after": member_count_after,
        }),
    );

    Ok(())
}

#[tauri::command]
fn mls_join_via_welcome(
    state: State<'_, AppState>,
    welcome_b64: String,
) -> Result<(), String> {
    let welcome = B64
        .decode(welcome_b64.trim())
        .map_err(|e| format!("base64 decode welcome: {e}"))?;
    let mut slot = state.mls.lock().map_err(|e| e.to_string())?;
    let bundle = slot
        .as_mut()
        .ok_or_else(|| "MLS not initialized".to_string())?;
    bundle.join_via_welcome(&welcome)
}

/// Encrypt a plaintext for the current MLS group AND auto-ship the
/// resulting wire bytes to every directory member via sealed-sender.
///
/// Returns `()` — the frontend no longer needs to display ciphertext; it
/// hits "Send to Group" and the message goes out. Failures to reach
/// individual recipients surface as `error` events but do NOT roll back
/// the group encryption (openmls has already advanced our local epoch by
/// the time this function returns from `bundle.encrypt`).
#[tauri::command]
async fn mls_encrypt(
    app: AppHandle,
    state: State<'_, AppState>,
    plaintext: String,
) -> Result<(), String> {
    // Same lock-then-drop pattern as `mls_add_member`: encrypt + snapshot
    // the recipient list while holding the StdMutex, drop before await.
    let (wire_bytes, recipients) = {
        let mut slot = state.mls.lock().map_err(|e| e.to_string())?;
        let bundle = slot
            .as_mut()
            .ok_or_else(|| "MLS not initialized".to_string())?;
        let wire = bundle.encrypt(plaintext.as_bytes())?;
        (wire, bundle.member_addresses.clone())
    };

    if recipients.is_empty() {
        // Solo group — encryption succeeded but nobody to ship to. This is
        // a no-op success; the frontend can still echo the outgoing line.
        return Ok(());
    }

    let payload = [&MLS_APP_PREFIX[..], &wire_bytes[..]].concat();
    for member in recipients {
        if let Err(e) = send_sealed_to_address(&app, &member.address, &payload).await {
            let _ = app.emit(
                "error",
                format!("MLS app-msg to '{}' failed: {}", member.label, e),
            );
        }
    }
    Ok(())
}

#[tauri::command]
fn mls_decrypt(
    state: State<'_, AppState>,
    wire_b64: String,
) -> Result<MlsDecryptResult, String> {
    let wire = B64
        .decode(wire_b64.trim())
        .map_err(|e| format!("base64 decode wire: {e}"))?;
    let mut slot = state.mls.lock().map_err(|e| e.to_string())?;
    let bundle = slot
        .as_mut()
        .ok_or_else(|| "MLS not initialized".to_string())?;
    match bundle.decrypt(&wire)? {
        Some(bytes) => {
            let text = String::from_utf8_lossy(&bytes).to_string();
            Ok(MlsDecryptResult {
                plaintext: Some(text),
                control_only: false,
            })
        }
        None => Ok(MlsDecryptResult {
            plaintext: None,
            control_only: true,
        }),
    }
}

#[tauri::command]
fn mls_status(state: State<'_, AppState>) -> Result<MlsStatus, String> {
    let slot = state.mls.lock().map_err(|e| e.to_string())?;
    Ok(match slot.as_ref() {
        None => MlsStatus {
            initialized: false,
            in_group: false,
            member_count: 0,
            identity_label: None,
            members: Vec::new(),
        },
        Some(b) => MlsStatus {
            initialized: true,
            in_group: b.in_group(),
            member_count: b.member_count(),
            identity_label: Some(b.identity_label.clone()),
            members: b.member_addresses.clone(),
        },
    })
}

// ── MLS group lifecycle commands ─────────────────────────────────────────────
//
// `mls_list_members` enumerates the MLS leaf-tree, `mls_leave_group` shuts our
// own leaf down and wipes local state, `mls_remove_member` evicts another
// leaf. All three drop down into raw openmls APIs (the `PhantomMlsGroup`
// wrapper doesn't surface them yet) using the public accessors
// `member.provider()` + `member.signer()` from `core::mls`.
//
// openmls 0.8 method names used:
//   - `MlsGroup::members(&self) -> impl Iterator<Item = Member>`
//   - `MlsGroup::leave_group(provider, signer) -> Result<MlsMessageOut, _>`
//   - `MlsGroup::remove_members(provider, signer, &[LeafNodeIndex]) ->
//        Result<(MlsMessageOut, Option<MlsMessageOut>, _), _>`
//   - `MlsGroup::merge_pending_commit(provider) -> Result<(), _>`

#[tauri::command]
fn mls_list_members(state: State<'_, AppState>) -> Result<Vec<MlsMemberInfo>, String> {
    let mut slot = state.mls.lock().map_err(|e| e.to_string())?;
    let bundle = slot
        .as_mut()
        .ok_or_else(|| "MLS not initialized".to_string())?;

    let id_bytes = bundle
        .group_id
        .as_ref()
        .ok_or_else(|| "not in a group".to_string())?
        .clone();
    let gid = GroupId::from_slice(&id_bytes);
    let group = mls::load_group(&bundle.member, &gid)
        .map_err(|e| format!("load_group: {e}"))?;

    // Our own signing pubkey: the openmls API exposes the raw bytes via
    // `to_public_vec` on `SignatureKeyPair`. Compare against each leaf's
    // `signature_key` to flag the self-row.
    let self_sig_pub = bundle.member.signer().to_public_vec();
    let directory = bundle.member_addresses.clone();

    let mut out = Vec::new();
    for member in group.members() {
        // Leaf signature pubkey is already raw bytes (Vec<u8>) — hex it
        // straight without an intermediate TLS codec round-trip.
        let signature_pub_hex = hex::encode(&member.signature_key);
        let is_self = member.signature_key == self_sig_pub;

        // BasicCredential stores the identity bytes inline — `serialized_content`
        // is the raw VLBytes payload, which for a `BasicCredential::new(label)`
        // call is just the label bytes (no extra framing). Falls back to a
        // hex placeholder if the bytes aren't valid UTF-8 (defensive — every
        // PhantomChat-created credential is a UTF-8 label).
        let credential_label = match std::str::from_utf8(member.credential.serialized_content()) {
            Ok(s) => s.to_string(),
            Err(_) => format!("?{}", &signature_pub_hex[..8]),
        };

        let mapped_contact_label = directory
            .iter()
            .find(|m| m.signing_pub_hex.eq_ignore_ascii_case(&signature_pub_hex))
            .map(|m| m.label.clone());

        out.push(MlsMemberInfo {
            credential_label,
            signature_pub_hex,
            is_self,
            mapped_contact_label,
        });
    }

    Ok(out)
}

/// Walk our local leaf out of the group: `MlsGroup::leave_group` produces a
/// Remove proposal that any remaining member can commit. We ship that
/// proposal to every directory recipient so they observe our exit, then
/// wipe ALL local MLS state (provider blob + meta JSON + transport
/// directory + in-memory bundle) so a subsequent `mls_create_group`
/// starts cleanly. Emits `mls_left` so the frontend can react.
#[tauri::command]
async fn mls_leave_group(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Hold the StdMutex only long enough to capture the wire bytes +
    // recipient list, then drop before any await (Tauri command tasks must
    // be Send across awaits).
    let (proposal_bytes, recipients) = {
        let mut slot = state.mls.lock().map_err(|e| e.to_string())?;
        let bundle = slot
            .as_mut()
            .ok_or_else(|| "MLS not initialized".to_string())?;
        let id_bytes = bundle
            .group_id
            .as_ref()
            .ok_or_else(|| "not in a group".to_string())?
            .clone();
        let gid = GroupId::from_slice(&id_bytes);
        let mut group = mls::load_group(&bundle.member, &gid)
            .map_err(|e| format!("load_group: {e}"))?;

        let proposal = group
            .leave_group(bundle.member.provider(), bundle.member.signer())
            .map_err(|e| format!("leave_group: {e:?}"))?;

        use tls_codec::Serialize as _;
        let bytes = proposal
            .tls_serialize_detached()
            .map_err(|e| format!("leave proposal ser: {e:?}"))?;

        (bytes, bundle.member_addresses.clone())
    };

    // Best-effort fan-out: even if a peer is offline, we still wipe local
    // state below so the user can rejoin / restart fresh. Errors surface as
    // `error` events but do NOT block the wipe.
    if !recipients.is_empty() {
        let payload = [&MLS_APP_PREFIX[..], &proposal_bytes[..]].concat();
        for member in &recipients {
            if let Err(e) = send_sealed_to_address(&app, &member.address, &payload).await {
                let _ = app.emit(
                    "error",
                    format!("MLS leave proposal to '{}' failed: {}", member.label, e),
                );
            }
        }
    }

    // Wipe local MLS state. We delete the on-disk artifacts directly
    // (rather than going through the bundle) so a corrupt provider
    // doesn't block the cleanup. The in-memory slot is then cleared so
    // the next `mls_create_group` constructs a fresh bundle.
    if let Ok(dir) = mls_storage_dir(&app) {
        let _ = fs::remove_file(dir.join("mls_state.bin"));
        let _ = fs::remove_file(dir.join("mls_meta.json"));
    }
    if let Ok(path) = mls_directory_path(&app) {
        let _ = fs::remove_file(path);
    }
    {
        let mut slot = state.mls.lock().map_err(|e| e.to_string())?;
        *slot = None;
    }

    let _ = app.emit("mls_left", ());
    audit(&app, "mls", "left", serde_json::json!({}));
    Ok(())
}

/// Evict another member from the group by their MLS signature pubkey hex.
/// Calls `MlsGroup::remove_members` (which emits a Commit) followed by
/// `merge_pending_commit` to advance our own epoch, then ships the commit
/// to every remaining directory member (skipping the just-removed one).
/// The removed peer is dropped from `member_addresses` and the directory
/// is persisted.
///
/// MLS has no built-in admin role — any member can call this, mirroring
/// openmls' permission model. Coordinate out-of-band who's allowed to
/// remove whom.
#[tauri::command]
async fn mls_remove_member(
    app: AppHandle,
    state: State<'_, AppState>,
    target_signing_pub_hex: String,
) -> Result<(), String> {
    let target_hex = target_signing_pub_hex.trim().to_lowercase();
    if target_hex.len() != 64 || hex::decode(&target_hex).is_err() {
        return Err("target_signing_pub_hex must be 32 bytes hex (64 chars)".into());
    }
    let target_bytes = hex::decode(&target_hex).expect("validated above");

    let (commit_bytes, remaining_recipients, identity_label, directory_snapshot, member_count) = {
        let mut slot = state.mls.lock().map_err(|e| e.to_string())?;
        let bundle = slot
            .as_mut()
            .ok_or_else(|| "MLS not initialized".to_string())?;

        let id_bytes = bundle
            .group_id
            .as_ref()
            .ok_or_else(|| "not in a group".to_string())?
            .clone();
        let gid = GroupId::from_slice(&id_bytes);
        let mut group = mls::load_group(&bundle.member, &gid)
            .map_err(|e| format!("load_group: {e}"))?;

        // Locate the leaf by signature pubkey.
        let target_leaf = group
            .members()
            .find(|m| m.signature_key == target_bytes)
            .ok_or_else(|| {
                format!(
                    "no member with signing pubkey {} in current group",
                    &target_hex[..16]
                )
            })?;
        let leaf_index = target_leaf.index;

        // Refuse to remove ourselves via this path — that's what
        // `mls_leave_group` is for, and openmls treats self-removal as a
        // distinct flow (no resulting Welcome to fan out, etc.).
        let self_pub = bundle.member.signer().to_public_vec();
        if target_leaf.signature_key == self_pub {
            return Err("cannot remove self via mls_remove_member — use mls_leave_group".into());
        }

        let (commit_msg, _welcome_opt, _gi) = group
            .remove_members(
                bundle.member.provider(),
                bundle.member.signer(),
                &[leaf_index],
            )
            .map_err(|e| format!("remove_members: {e:?}"))?;
        // Advance our own state to the post-remove epoch. openmls leaves
        // the commit pending until we explicitly merge.
        group
            .merge_pending_commit(bundle.member.provider())
            .map_err(|e| format!("merge_pending_commit: {e:?}"))?;

        use tls_codec::Serialize as _;
        let commit_bytes = commit_msg
            .tls_serialize_detached()
            .map_err(|e| format!("commit ser: {e:?}"))?;

        // Drop the evicted member from our directory + cache the
        // remaining recipients (excluding self, who isn't in the directory).
        bundle
            .member_addresses
            .retain(|m| !m.signing_pub_hex.eq_ignore_ascii_case(&target_hex));
        let recipients = bundle.member_addresses.clone();
        bundle.member_count = group.members().count() as u32;

        (
            commit_bytes,
            recipients,
            bundle.identity_label.clone(),
            bundle.member_addresses.clone(),
            bundle.member_count,
        )
    };

    // Persist the trimmed directory before any network I/O so a relay
    // failure can't leave a stale ghost entry on disk.
    if let Ok(path) = mls_directory_path(&app) {
        let _ = save_mls_directory(&path, &identity_label, &directory_snapshot);
    }

    // Fan the commit out to every REMAINING member so they advance to
    // the same epoch. The just-removed peer is intentionally excluded —
    // they can't decrypt the new epoch's messages anyway, and openmls
    // won't accept the commit for a leaf that's been evicted.
    let payload = [&MLS_APP_PREFIX[..], &commit_bytes[..]].concat();
    for member in &remaining_recipients {
        if let Err(e) = send_sealed_to_address(&app, &member.address, &payload).await {
            let _ = app.emit(
                "error",
                format!("MLS remove commit to '{}' failed: {}", member.label, e),
            );
        }
    }

    let _ = app.emit(
        "mls_member_removed",
        serde_json::json!({
            "signing_pub_hex": target_hex,
            "member_count": member_count,
        }),
    );

    audit(
        &app,
        "mls",
        "removed",
        serde_json::json!({
            "target_signing_pub_short": &target_hex[..16.min(target_hex.len())],
            "member_count_after": member_count,
        }),
    );

    Ok(())
}

// ── History persistence + connection-status commands ────────────────────────
//
// On-disk format (`$APPDATA/de.dc-infosec.phantomchat/messages.json`):
//
//   [
//     {
//       "plaintext":      "hello",
//       "timestamp":      "12:34:56",
//       "sender_label":   "alice",
//       "sig_ok":         true,
//       "sender_pub_hex": "deadbeef…",   // optional
//       "direction":      "incoming"     // "incoming" | "outgoing" | "system"
//     },
//     ...
//   ]
//
// We reuse the existing `IncomingMessage` struct; the `direction` field was
// added with `#[serde(default)]` so older records (and live event payloads)
// stay shape-compatible. Rolling our own `MessageRecord` would have
// duplicated five fields — extension is the cheaper, BC-friendly call.

fn messages_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(MESSAGES_FILE))
}

#[tauri::command]
fn save_history(
    app: AppHandle,
    state: State<'_, AppState>,
    messages: Vec<IncomingMessage>,
) -> Result<(), String> {
    let _guard = state.history_lock.lock().map_err(|e| e.to_string())?;
    let path = messages_path(&app).map_err(|e| e.to_string())?;
    let buf = serde_json::to_vec_pretty(&messages).map_err(|e| e.to_string())?;
    fs::write(&path, buf).map_err(|e| format!("write {}: {}", path.display(), e))?;
    Ok(())
}

#[tauri::command]
fn load_history(app: AppHandle) -> Result<Vec<IncomingMessage>, String> {
    let path = messages_path(&app).map_err(|e| e.to_string())?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read(&path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    // Be lenient: a corrupted history shouldn't brick the app. Treat parse
    // failures as "no history" rather than bubbling up an error to the UI.
    Ok(serde_json::from_slice(&raw).unwrap_or_default())
}

/// One match returned by `search_messages`. `msg_idx` is the position in
/// the on-disk `messages.json` array (0-based), so the frontend can
/// scroll-to-row + pulse-highlight the corresponding `MessageStream`
/// entry. `match_ranges` are byte-offset half-open intervals into
/// `plaintext` for every case-insensitive occurrence of the query — the
/// React side renders each range with a magenta background highlight.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub msg_idx: usize,
    pub timestamp: String,
    pub direction: String,
    pub sender_label: String,
    pub plaintext: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub match_ranges: Vec<(usize, usize)>,
}

const SEARCH_LIMIT_DEFAULT: u32 = 100;

/// Locate every byte-offset range in `haystack` where `needle_lower`
/// occurs, comparing case-insensitively. Walks the haystack one ASCII-
/// lowercased pass and matches against the (already-lowercased) needle.
/// We do the lowercasing per-call rather than caching since the message
/// volumes a single user has are tiny — readability beats micro-opt.
///
/// Note: case-folding is performed via `char::to_ascii_lowercase` so
/// non-ASCII bytes are compared raw. That matches what most chat
/// search UIs do (incl. Slack/Telegram for non-Latin queries) and
/// avoids the alloc cost of `.to_lowercase()` on every substring.
fn find_match_ranges(haystack: &str, needle_lower: &str) -> Vec<(usize, usize)> {
    if needle_lower.is_empty() {
        return Vec::new();
    }
    let hay_lower = haystack.to_ascii_lowercase();
    let needle_len = needle_lower.len();
    let mut out = Vec::new();
    let mut start = 0usize;
    while let Some(rel) = hay_lower[start..].find(needle_lower) {
        let abs = start + rel;
        out.push((abs, abs + needle_len));
        // Advance by 1 (not needle_len) so overlapping matches don't slip
        // through — `aaaa` searched for `aa` should yield 3 matches at
        // 0/1/2, mirroring grep -o semantics on overlapping inputs.
        start = abs + 1;
        if start > hay_lower.len() {
            break;
        }
    }
    out
}

/// Linear-scan search across the persisted message history. Optional
/// `sender_filter` restricts to one contact label (exact match). Results
/// are returned newest-first (history is appended chronologically, so we
/// reverse the iteration order) and capped at `limit` (default 100).
///
/// Complexity: O(N * M) where N = #messages and M = total chars per row.
/// That's fine for the message volumes a single user accumulates — even
/// 50k rows × 200 chars finishes in a few ms on commodity hardware. If
/// users ever exceed that we'd index, but for an MVP this is the cheaper
/// bet (no schema migration, no on-disk index file to keep in sync).
///
/// File rows (`kind == "file"`) additionally search the `file_meta`
/// filename so a user looking for "report.pdf" still finds it even
/// though the row's `plaintext` is a humanized "received report.pdf
/// (12.4 KiB)" caption.
#[tauri::command]
async fn search_messages(
    app: AppHandle,
    query: String,
    sender_filter: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<SearchHit>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let q_lower = q.to_ascii_lowercase();
    let cap = limit.unwrap_or(SEARCH_LIMIT_DEFAULT) as usize;

    let history = load_history(app)?;
    let mut hits: Vec<SearchHit> = Vec::new();

    // Iterate newest-first so we can early-exit at `cap`. Use enumerate
    // BEFORE reversing so `msg_idx` stays the position in the on-disk
    // array (== the React stream's row index).
    for (idx, msg) in history.iter().enumerate().rev() {
        if let Some(ref want) = sender_filter {
            if &msg.sender_label != want {
                continue;
            }
        }
        // Search the message body first.
        let mut ranges = find_match_ranges(&msg.plaintext, &q_lower);
        // Then, for file rows, look at the filename. We don't try to
        // splice filename matches into the body's offset space — that
        // would lie about where the highlight goes. Instead we treat
        // any filename hit as a single-row hit with no per-char ranges
        // (the React side falls back to highlighting the whole row).
        if ranges.is_empty() {
            if msg.kind.as_deref() == Some("file") {
                if let Some(meta) = msg.file_meta.as_ref() {
                    if meta.filename.to_ascii_lowercase().contains(&q_lower) {
                        // Empty ranges = filename-only match. Frontend
                        // shows the row but skips substring highlights.
                        ranges = Vec::new();
                        hits.push(SearchHit {
                            msg_idx: idx,
                            timestamp: msg.timestamp.clone(),
                            direction: msg.direction.clone(),
                            sender_label: msg.sender_label.clone(),
                            plaintext: msg.plaintext.clone(),
                            kind: msg.kind.clone(),
                            match_ranges: ranges,
                        });
                        if hits.len() >= cap {
                            break;
                        }
                    }
                }
            }
            continue;
        }
        hits.push(SearchHit {
            msg_idx: idx,
            timestamp: msg.timestamp.clone(),
            direction: msg.direction.clone(),
            sender_label: msg.sender_label.clone(),
            plaintext: msg.plaintext.clone(),
            kind: msg.kind.clone(),
            match_ranges: ranges,
        });
        if hits.len() >= cap {
            break;
        }
    }
    Ok(hits)
}

#[tauri::command]
fn get_connection_status(state: State<'_, AppState>) -> Result<String, String> {
    let slot = state.connection_status.lock().map_err(|e| e.to_string())?;
    Ok(if slot.is_empty() {
        "connecting".to_string()
    } else {
        slot.clone()
    })
}

// ── Entrypoint ───────────────────────────────────────────────────────────────

// ── Relay-management commands ────────────────────────────────────────────────
//
// Persisted to `relays.json` next to keys.json/contacts.json. The new URL list
// only takes effect on the *next* `start_listener` invocation — so the
// frontend should prompt the user to reconnect after a `set_relays` call.
// Validation: each URL must parse via `url::Url` and use a `ws`/`wss` scheme.
// We deliberately don't probe for reachability here — `start_listener` will
// surface that via the `connection` event channel.

/// Lightweight URL validation. We avoid pulling in the `url` crate as a
/// direct dep just for this — a scheme-prefix check + non-empty host segment
/// catches typos and mis-pasted addresses, and any deeper structural error
/// will surface at `connect_async` time inside the relay layer.
fn validate_relay_url(u: &str) -> Result<(), String> {
    let trimmed = u.trim();
    if trimmed.is_empty() {
        return Err("relay URL must not be empty".into());
    }
    let rest = if let Some(r) = trimmed.strip_prefix("wss://") {
        r
    } else if let Some(r) = trimmed.strip_prefix("ws://") {
        r
    } else {
        return Err(format!("relay '{}' must start with ws:// or wss://", u));
    };
    // Must have at least one char of host before any path/`:` segment.
    let host = rest.split('/').next().unwrap_or("");
    let host_only = host.split(':').next().unwrap_or("");
    if host_only.is_empty() {
        return Err(format!("relay '{}' is missing a host", u));
    }
    Ok(())
}

#[tauri::command]
fn list_relays(app: AppHandle) -> Result<Vec<String>, String> {
    Ok(load_relays(&app))
}

#[tauri::command]
fn set_relays(app: AppHandle, urls: Vec<String>) -> Result<(), String> {
    if urls.is_empty() {
        return Err("relay list must not be empty".into());
    }
    for u in &urls {
        validate_relay_url(u)?;
    }
    let count = urls.len();
    save_relays(&app, &urls).map_err(|e| e.to_string())?;
    audit(
        &app,
        "relay",
        "changed",
        serde_json::json!({ "count": count }),
    );
    Ok(())
}

// ── Privacy config + listener-restart commands ───────────────────────────────
//
// `privacy.json` lives next to `relays.json` / `keys.json` and stores the DTO
// shape directly so the frontend can round-trip it without a transform. A
// missing file = `PrivacyConfigDto::default()` (DailyUse + Tor on
// 127.0.0.1:9050). Calling `set_privacy_config` only persists; the user must
// call `restart_listener` afterwards to recreate the subscriber task with the
// new mode (the Settings panel does this automatically after Save).

#[tauri::command]
fn get_privacy_config(app: AppHandle) -> Result<PrivacyConfigDto, String> {
    Ok(load_privacy(&app))
}

#[tauri::command]
fn set_privacy_config(app: AppHandle, cfg: PrivacyConfigDto) -> Result<(), String> {
    // Round-trip through the core type so we reject malformed enum tags
    // up front rather than persisting garbage.
    let _validated = cfg.to_core()?;
    let mode = cfg.mode.clone();
    save_privacy(&app, &cfg).map_err(|e| e.to_string())?;
    audit(
        &app,
        "privacy",
        "changed",
        serde_json::json!({ "mode": mode }),
    );
    Ok(())
}

/// Gracefully tear down the current relay-subscriber task and spawn a
/// fresh one. Used by the Settings panel after `set_privacy_config` so
/// the new mode (Tor / direct) takes effect without forcing the user to
/// restart the app.
///
/// Shutdown protocol: send the oneshot, then `join` the handle with a
/// 3s timeout so the inner `select!` arm can drop the relay and let
/// each WebSocket close cleanly. Falls back to `JoinHandle::abort()` if
/// the task is wedged past the deadline (e.g. a stuck blocking sub-call)
/// so a hung subscriber never bricks the privacy-mode swap.
///
/// Idempotent — if no listener is running we just spawn one. The
/// `listener_started` flag stays `true` so callers cannot accidentally
/// double-spawn via a parallel `start_listener`.
#[tauri::command]
async fn restart_listener(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    // Take the existing control handle out of the mutex so we don't
    // hold the lock across the await on `handle`. The oneshot sender
    // consumes self on `send`, so taking ownership matches its API.
    let prev = {
        let mut slot = state.subscriber.lock().await;
        slot.take()
    };
    if let Some(ListenerControl { mut handle, shutdown_tx }) = prev {
        // Signal shutdown. Ignoring the Err is intentional — if the
        // receiver was already dropped (task crashed) we still want to
        // proceed with the join + spawn-replacement path.
        let _ = shutdown_tx.send(());
        // Bound the wait so a wedged subscriber can't stall the restart.
        // 3s is generous for a clean WS close-frame round trip. We
        // borrow `handle` mutably so the timeout doesn't consume it —
        // that way the abort fallback below still has the JoinHandle
        // available if we time out.
        match tokio::time::timeout(std::time::Duration::from_secs(3), &mut handle).await {
            Ok(Ok(())) => { /* clean exit */ }
            Ok(Err(join_err)) => {
                // Task panicked or was previously aborted — log and
                // move on. We're about to spawn a fresh one anyway.
                eprintln!("restart_listener: prior task join error: {}", join_err);
            }
            Err(_elapsed) => {
                eprintln!(
                    "restart_listener: prior subscriber did not exit within 3s — falling back to abort()"
                );
                handle.abort();
            }
        }
    }
    // Make sure the started flag is set before spawning so a parallel
    // `start_listener` becomes the documented no-op. (The flag is left
    // `true` across the abort/spawn sequence — restart never tears the
    // user-visible "listener active" indicator down.)
    {
        let mut started = state.listener_started.lock().map_err(|e| e.to_string())?;
        *started = true;
    }
    spawn_listener_task(app, &state, None).await
}

// ── Notification helper ──────────────────────────────────────────────────────
//
// Fires a native OS notification iff the main window is hidden OR not focused.
// Title + body are truncated to keep the system shelf legible (body capped at
// 80 grapheme-ish chars; we use char-count for simplicity since the upper
// bound is a soft UX limit, not a security boundary).
fn maybe_notify(app: &AppHandle, title: &str, body: &str) {
    let win = match app.get_webview_window("main") {
        Some(w) => w,
        None => return,
    };
    let focused = win.is_focused().unwrap_or(false);
    let visible = win.is_visible().unwrap_or(true);
    if focused && visible {
        return;
    }
    let truncated: String = if body.chars().count() > 80 {
        let mut s: String = body.chars().take(80).collect();
        s.push('\u{2026}');
        s
    } else {
        body.to_string()
    };
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(&truncated)
        .show();
}

// ── Settings + onboarding commands ───────────────────────────────────────────

const ONBOARDED_MARKER: &str = "onboarded";

/// Return the absolute path of the on-disk keyfile, primarily for the
/// "Backup keyfile" Settings button so the user can grab it via their
/// platform's file manager. Errors if no identity exists yet.
#[tauri::command]
fn export_keyfile(app: AppHandle) -> Result<String, String> {
    let path = keys_path(&app).map_err(|e| e.to_string())?;
    if !path.exists() {
        return Err("no identity yet — generate one first".into());
    }
    Ok(path.to_string_lossy().to_string())
}

/// Validate + write a pasted keyfile JSON into `app_data_dir/keys.json`.
/// Overwrites any existing file (the wizard's "Restore" branch is the only
/// caller; the user is informed it'll replace the current identity).
///
/// Imported keyfiles still arrive in the legacy plaintext format (that's
/// the export shape — `*_ref` fields would be useless on the importing
/// host since they only resolve in the originating keystore). Right after
/// writing the plaintext to disk we run the standard migration so the
/// just-imported `*_private` fields get pushed into the host keystore and
/// the on-disk file is rewritten with `*_private_ref` placeholders. The
/// process is idempotent: a re-import overwrites both the file AND the
/// keystore entries since they're keyed off the same path.
#[tauri::command]
fn import_keyfile(app: AppHandle, json_text: String) -> Result<(), String> {
    let json: serde_json::Value =
        serde_json::from_str(json_text.trim()).map_err(|e| format!("invalid JSON: {e}"))?;
    let required = [
        "view_private",
        "view_public",
        "spend_private",
        "spend_public",
        "signing_private",
        "signing_public",
        "identity_private",
        "identity_public",
    ];
    for field in required {
        if !json.get(field).map(|v| v.is_string()).unwrap_or(false) {
            return Err(format!("keyfile missing required field '{field}'"));
        }
    }
    let path = keys_path(&app).map_err(|e| e.to_string())?;
    // Atomic write of the plaintext form so we never have a half-written
    // keys.json next to a half-stashed keystore.
    atomic_write_json(&path, &json).map_err(|e| format!("write {}: {}", path.display(), e))?;
    audit(&app, "identity", "restored", serde_json::json!({}));
    // Immediately migrate the just-imported plaintext into the host
    // keystore. Best-effort: a migration failure leaves the plaintext
    // keys.json on disk (the user can still use the app — every read path
    // falls back to the inline `*_private` field if `*_private_ref` is
    // missing) and the next launch will retry the migration.
    if let Err(e) = migrate_keys_json_if_legacy(&app, &path) {
        eprintln!("import: secure-storage migration failed: {e:#}");
    }
    Ok(())
}

/// Anti-forensic file-overwrite threshold. Files **larger** than this skip
/// the zero-overwrite pass and are unlinked directly with a WARN audit
/// entry — primarily to keep `wipe_all_data` from spending O(GB) of disk
/// I/O on user-staged backups that the user is independently responsible
/// for. SSD TRIM will do its own thing on those over time.
const WIPE_OVERWRITE_MAX_BYTES: u64 = 100 * 1024 * 1024;

/// Best-effort cryptographic-shred of a single file: open for write,
/// overwrite the byte range with zeros, fsync, truncate, then unlink.
///
/// On SSDs the underlying NAND blocks are usually only TRIM'd on delete,
/// so the in-place zero-write is partially redundant — but on spinning
/// rust it materially raises the bar against forensic recovery, and on
/// SSDs it at least pre-empts wear-leveling shenanigans that might keep
/// a stale copy of the original bytes in an unmapped erase block.
///
/// All errors are swallowed (logged to stderr) — the goal is "delete the
/// file, scrubbed if possible, plain-deleted as a fallback". A scrub
/// failure must not block the unlink.
fn shred_file(path: &std::path::Path) {
    let size = match fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => {
            eprintln!("shred: stat {} failed: {e}", path.display());
            // Even with no metadata, attempt the unlink as a last resort.
            let _ = fs::remove_file(path);
            return;
        }
    };

    if size > WIPE_OVERWRITE_MAX_BYTES {
        // Probably a backup the user staged into app_data; honour the
        // size threshold and just unlink. We log so a compliance auditor
        // can see why this file wasn't scrubbed.
        eprintln!(
            "shred: skipping zero-overwrite of {} ({} > {}); unlinking only",
            path.display(),
            size,
            WIPE_OVERWRITE_MAX_BYTES
        );
        let _ = fs::remove_file(path);
        return;
    }

    if size > 0 {
        match OpenOptions::new().write(true).truncate(false).open(path) {
            Ok(mut f) => {
                // Stream the zero-pass in 64 KiB chunks rather than
                // allocating a `vec![0u8; size as usize]` outright — keeps
                // wipe of a 95 MiB file from spiking RSS to 95 MiB.
                let chunk = vec![0u8; 64 * 1024];
                let mut remaining = size as usize;
                while remaining > 0 {
                    let n = remaining.min(chunk.len());
                    if let Err(e) = f.write_all(&chunk[..n]) {
                        eprintln!("shred: write {} failed: {e}", path.display());
                        break;
                    }
                    remaining -= n;
                }
                let _ = f.sync_all();
                // Truncate after the overwrite so the directory entry
                // points at a 0-byte file before unlink — narrows the
                // forensic window further on filesystems that delay free.
                let _ = f.set_len(0);
                let _ = f.sync_all();
            }
            Err(e) => {
                eprintln!("shred: open {} for overwrite failed: {e}", path.display());
            }
        }
    }

    if let Err(e) = fs::remove_file(path) {
        eprintln!("shred: unlink {} failed: {e}", path.display());
    }
}

/// Recursively shred every file under `dir`, then remove the (now-empty)
/// directory tree. Symlinks are unlinked without following — defence
/// against a hostile or buggy preconfig that points app_data at `/`.
fn shred_directory(dir: &std::path::Path) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("shred: read_dir {} failed: {e}", dir.display());
            // Still try to remove the dir itself — it may already be empty.
            let _ = fs::remove_dir_all(dir);
            return;
        }
    };
    for entry in entries.flatten() {
        let p = entry.path();
        // Lstat (symlink_metadata) so we don't follow into another tree.
        let meta = match fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            let _ = fs::remove_file(&p);
        } else if meta.is_dir() {
            shred_directory(&p);
        } else {
            shred_file(&p);
        }
    }
    let _ = fs::remove_dir(dir);
}

/// Wipe the entire app-data directory (keys, contacts, sessions, history,
/// MLS directory, relays.json, onboarding marker) AND the host-keystore
/// secrets keyed off the current identity file, then exit. Caller is
/// expected to confirm the destructive intent before invoking.
///
/// Pre-delete pass scrubs each file with a single zero-overwrite +
/// truncate to defeat naive forensic recovery on spinning disks. Files
/// larger than `WIPE_OVERWRITE_MAX_BYTES` (100 MiB) skip the overwrite
/// and are unlinked directly — see `shred_file` for the rationale.
#[tauri::command]
fn wipe_all_data(app: AppHandle) -> Result<(), String> {
    // Best-effort audit BEFORE the wipe — the on-disk audit.log itself is
    // about to be deleted, but the line appears in any in-flight stderr
    // capture and is the right thing to do for compliance review of the
    // command sequence.
    audit(&app, "data", "wiped", serde_json::json!({}));
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve app_data_dir: {e}"))?;

    // Step 1 — purge keystore entries belonging to this identity. The
    // path-derived key_id scheme means a stale `keys.json` is enough to
    // know which entries to drop. Best-effort: a missing entry is fine.
    if dir.exists() {
        let kp = dir.join(KEYS_FILE);
        for suffix in SECRET_SUFFIXES {
            let id = secret_key_id(&kp, suffix);
            if let Err(e) = secure_storage().delete(&id) {
                eprintln!("wipe: keystore delete '{id}' failed: {e}");
            }
        }
    }

    // Step 2 — anti-forensic shred + remove every file in app_data_dir.
    if dir.exists() {
        shred_directory(&dir);
    }

    // Hard exit: we just nuked our own state; staying in the process risks
    // background tasks (relay subscribe loop, session writes) recreating
    // some of the files we just deleted.
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ── Audit log Tauri commands ────────────────────────────────────────────────

/// Read the last `limit` (default 100) entries from `audit.log`. Each line
/// is parsed as a single JSON `AuditEntry`; malformed lines are silently
/// skipped so a partial write at process-kill time can't lock the auditor
/// out of every other entry. Newest entries first.
#[tauri::command]
fn read_audit_log(app: AppHandle, limit: Option<u32>) -> Result<Vec<AuditEntry>, String> {
    let cap = limit.unwrap_or(100) as usize;
    let path = audit_log_path(&app).map_err(|e| e.to_string())?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;
    let mut entries: Vec<AuditEntry> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<AuditEntry>(l).ok())
        .collect();
    // Newest entries first — file is append-order so the tail is the latest.
    entries.reverse();
    entries.truncate(cap);
    Ok(entries)
}

/// Return the absolute path to `audit.log` so a compliance auditor can grab
/// it via the platform's file manager (similar to `export_keyfile`).
#[tauri::command]
fn export_audit_log(app: AppHandle) -> Result<String, String> {
    let path = audit_log_path(&app).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

// ── Auto-updater Tauri commands ─────────────────────────────────────────────
//
// Thin wrappers around `tauri-plugin-updater`. The plugin's signature
// verification hangs off the `pubkey` set in `tauri.conf.json` — for an MVP
// release the placeholder pubkey there will reject every update payload, so
// the install path is a no-op until the real signing keypair is generated
// via `tauri signer generate`.

#[derive(Clone, Debug, Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub version: Option<String>,
    pub release_notes: Option<String>,
}

/// Check the configured updater endpoint for a newer release. Returns
/// `available = false` on any error so the frontend can simply show
/// "you're up to date" — a transient network failure shouldn't escalate
/// to a noisy user-visible error in the Settings panel.
#[tauri::command]
async fn check_for_updates(app: AppHandle) -> Result<UpdateInfo, String> {
    let updater = app
        .updater()
        .map_err(|e| format!("updater init: {}", e))?;
    match updater.check().await {
        Ok(Some(update)) => Ok(UpdateInfo {
            available: true,
            version: Some(update.version.clone()),
            release_notes: update.body.clone(),
        }),
        Ok(None) => Ok(UpdateInfo {
            available: false,
            version: None,
            release_notes: None,
        }),
        Err(e) => Err(format!("update check failed: {}", e)),
    }
}

/// Download + install the available update. The plugin's `download_and_install`
/// applies the update via the platform's installer (NSIS on Windows in
/// `passive` mode per `tauri.conf.json`, .app bundle replace on macOS, AppImage
/// rewrite on Linux). Errors bubble back to the UI so the user can retry.
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let updater = app
        .updater()
        .map_err(|e| format!("updater init: {}", e))?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("update check failed: {}", e))?
        .ok_or_else(|| "no update available".to_string())?;
    update
        .download_and_install(|_chunk_len, _content_len| {}, || {})
        .await
        .map_err(|e| format!("install failed: {}", e))?;
    Ok(())
}

fn onboarded_marker_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(ONBOARDED_MARKER))
}

#[tauri::command]
fn is_onboarded(app: AppHandle) -> bool {
    match onboarded_marker_path(&app) {
        Ok(p) => p.exists(),
        Err(_) => false,
    }
}

#[tauri::command]
fn mark_onboarded(app: AppHandle) -> Result<(), String> {
    let path = onboarded_marker_path(&app).map_err(|e| e.to_string())?;
    fs::write(&path, b"1").map_err(|e| format!("write {}: {}", path.display(), e))?;
    audit(&app, "data", "onboarded", serde_json::json!({}));
    Ok(())
}

// ── Wave 7C: enterprise pre-seeded bootstrap ────────────────────────────────
//
// `bootstrap.json` is a per-org pre-seed file that the templater
// (`tools/phantom-build-org-msi`) bakes into an enterprise deploy artifact.
// On first launch the desktop reads it (admin-pushed via Group Policy /
// Intune file deploy OR bundled inside the MSI install dir), generates a
// fresh per-install identity, pre-populates the contact directory + relay
// list, and skips the onboarding wizard entirely.
//
// Schema mirrors `tools/phantom-build-org-msi/src/main.rs::BootstrapFile`
// 1:1 — bump `BOOTSTRAP_SCHEMA_VERSION` on the templater side AND here in
// lock-step.

const BOOTSTRAP_SCHEMA_VERSION: u32 = 1;
const BOOTSTRAP_FILE: &str = "bootstrap.json";

#[derive(Debug, Deserialize)]
struct BootstrapFile {
    schema_version: u32,
    #[serde(default)]
    org_name: String,
    #[serde(default)]
    org_id: String,
    /// Reserved for future use (signing org-internal directory updates).
    /// Persisted into `org_bootstrap.json` but not yet wired into any
    /// send/receive path. Kept here so the desktop side knows about the
    /// field and round-trips it cleanly when the templater bumps schema.
    #[serde(default)]
    org_secret: String,
    #[serde(default)]
    default_relays: Vec<String>,
    #[serde(default)]
    directory: Vec<BootstrapDirectoryEntry>,
    #[serde(default)]
    auto_join_lan_org_code: Option<String>,
    #[serde(default)]
    branding: BootstrapBranding,
}

#[derive(Debug, Deserialize)]
struct BootstrapDirectoryEntry {
    label: String,
    address: String,
    #[serde(default)]
    signing_pub_hex: String,
}

#[derive(Debug, Default, Deserialize)]
struct BootstrapBranding {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    primary_color: Option<String>,
}

/// Search path for the pre-seeded bootstrap file. Order matters: the
/// per-user `app_data_dir` location is checked first because it's the
/// canonical drop-zone for admin-pushed config (Group Policy file deploy,
/// Intune copy, deploy.ps1 from the templater) AND it survives MSI
/// upgrades (per-user data is not touched by `msiexec /i`). The
/// `<exe_parent>` location is the fallback for true MSI re-bundling
/// (tauri's MSI files dir) — useful once Wave 7C-followup ships a real
/// WiX shim.
fn bootstrap_search_paths(app: &AppHandle) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(dir) = app_data(app) {
        out.push(dir.join(BOOTSTRAP_FILE));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            out.push(parent.join(BOOTSTRAP_FILE));
        }
    }
    out
}

/// Look for `bootstrap.json` and, if found + valid, apply it. Returns
/// `Ok(true)` when a bootstrap was successfully applied (caller should
/// then auto-mark the install as onboarded so the wizard never shows);
/// `Ok(false)` when no bootstrap was found (normal path — let the wizard
/// run); `Err(...)` on a hard validation/IO error against a present-but-
/// malformed file.
///
/// Hard constraint: this function ONLY runs when no identity exists yet
/// (`keys.json` missing) AND the install is not already onboarded. That
/// keeps a malicious bootstrap dropped on a populated install from
/// clobbering existing state.
fn try_apply_bootstrap(app: AppHandle) -> Result<bool, String> {
    // Don't even consider the bootstrap if the user is already onboarded
    // OR if a keyfile already exists. This is the single source of truth
    // for "fresh install" — anything else is treated as a normal startup.
    if is_onboarded(app.clone()) {
        return Ok(false);
    }
    let keyfile = keys_path(&app).map_err(|e| e.to_string())?;
    if keyfile.exists() {
        return Ok(false);
    }

    // Locate + parse bootstrap. A missing file is the normal case → just
    // return false. A present-but-broken file is loud → return Err so
    // the operator notices in `audit.log` + stderr.
    let mut found_path: Option<PathBuf> = None;
    let mut raw_bytes: Option<Vec<u8>> = None;
    for candidate in bootstrap_search_paths(&app) {
        if candidate.exists() {
            match fs::read(&candidate) {
                Ok(b) => {
                    found_path = Some(candidate);
                    raw_bytes = Some(b);
                    break;
                }
                Err(e) => {
                    eprintln!(
                        "bootstrap: read {} failed: {} — trying next path",
                        candidate.display(),
                        e
// ── Crash reporting ─────────────────────────────────────────────────────────
//
// MVP design:
//   - Local: a process-wide panic hook captures (msg, location, backtrace,
//     ts, version, OS) and appends one JSONL line to `crashes.jsonl` in the
//     app-data dir. The hook NEVER POSTs from inside itself — panics
//     happen in unsafe states, so we only do file I/O before delegating to
//     the default abort handler.
//   - Surface: three Tauri commands (`list_crash_reports`,
//     `clear_crash_reports`, `dispatch_crash_report`) so the Settings →
//     Diagnostics section can render a list, let the user send one report
//     at a time, or delete them all.
//   - Privacy: `dispatch_crash_report` ALWAYS refuses unless
//     `crash_reporting_opted_in.flag` exists in app-data — a one-shot
//     opt-in the user toggles via the Diagnostics checkbox. Reports
//     contain ONLY the panic information; never any contact list, message
//     content, key material, or relay URLs.
//
// The HTTP collector at `CRASH_REPORT_ENDPOINT` currently logs receipt and
// returns 202; persistent server-side storage is a follow-up.

/// Cached app-data directory captured at `setup()` time. The panic hook
/// has no `AppHandle` available, so we have to know where to write the
/// JSONL file BEFORE a panic actually happens. Set exactly once via
/// `set_panic_hook`; reads after that are lock-free.
static CRASH_LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// One row written to `crashes.jsonl` (and read back by
/// `list_crash_reports`). Same shape regardless of whether the report
/// was generated by an actual panic or hand-written for tests.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrashReport {
    /// ISO-8601 UTC timestamp the panic was captured at.
    pub ts: String,
    /// `CARGO_PKG_VERSION` at build time so a remote operator can correlate
    /// a crash with a specific release.
    pub version: String,
    /// `std::env::consts::OS` — `"linux"` / `"macos"` / `"windows"`.
    pub os: String,
    /// First line of the panic payload (typically the formatted `panic!`
    /// argument). Trimmed to the first newline so `list_crash_reports`
    /// returns a compact summary line per row.
    pub panic_msg: String,
    /// `file:line:col` of the panic origin, or `"<unknown>"` if rust gave
    /// us no `Location`.
    pub location: String,
    /// Captured via `Backtrace::force_capture()` so we get frames even when
    /// `RUST_BACKTRACE` is unset. Multi-line; the UI formats it as `<pre>`.
    pub backtrace: String,
    /// Set to `true` once the user POSTs this row via
    /// `dispatch_crash_report` so the Diagnostics modal can show "already
    /// sent" instead of offering a re-send.
    #[serde(default)]
    pub user_dispatched: bool,
}

fn crashes_path_from_dir(dir: &std::path::Path) -> PathBuf {
    dir.join(CRASHES_FILE)
}

fn crashes_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(CRASHES_FILE))
}

fn crash_optin_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(CRASH_OPTIN_FILE))
}

/// Install the process-wide panic hook. Must be called exactly once from
/// `setup()`. Captures the app-data dir into [`CRASH_LOG_DIR`] so the
/// hook closure has somewhere to write without needing an `AppHandle`.
///
/// The hook chains the default Rust panic-printer AFTER our JSONL write,
/// so the user still sees the usual stderr "thread 'main' panicked at"
/// trace AND we get a persisted record. We don't `process::abort()`
/// ourselves — the runtime's default behavior (unwind-or-abort depending
/// on `panic = ...` in Cargo.toml) is preserved.
fn set_panic_hook(app: AppHandle) {
    // Resolve + cache the app-data dir on entry. If we can't even create
    // the dir there's nothing the hook can do — fall through and accept
    // that crashes won't be persisted on this run.
    let dir = match app_data(&app) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("set_panic_hook: cannot resolve app_data_dir: {e}");
            return;
        }
    };
    if CRASH_LOG_DIR.set(dir).is_err() {
        // Already initialized — `setup()` was somehow called twice. The
        // existing hook is fine; nothing to do.
        return;
    }

    // Stash the default hook so we still get a stderr backtrace after
    // logging — useful in `tauri dev` and for downstream supervisors.
    let default = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        // Format payload defensively — every step here runs INSIDE a
        // panic, so we cannot panic again ourselves. Avoid `unwrap`,
        // `expect`, indexing, or anything else that could double-panic.
        let payload_str = if let Some(s) = info.payload().downcast_ref::<&'static str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let panic_msg = payload_str
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();

        let report = CrashReport {
            ts: chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            panic_msg,
            location,
            backtrace,
            user_dispatched: false,
        };

        if let Some(dir) = CRASH_LOG_DIR.get() {
            let path = crashes_path_from_dir(dir);
            // Best-effort append. If serialization or open() fails we
            // silently skip — the default hook below still prints the
            // panic to stderr.
            if let Ok(line) = serde_json::to_string(&report) {
                if let Ok(mut f) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    let _ = writeln!(f, "{}", line);
                }
            }
        }

        // Chain the default behaviour — process aborts (or unwinds)
        // exactly as it would have without our hook installed.
        default(info);
    }));
}

/// Read up to `limit` (default 50) crash records from `crashes.jsonl`.
/// Newest entries first; malformed lines are silently skipped so a
/// partial write at process-kill time doesn't lock the user out of the
/// rest of the file.
#[tauri::command]
fn list_crash_reports(
    app: AppHandle,
    limit: Option<u32>,
) -> Result<Vec<CrashReport>, String> {
    let cap = limit.unwrap_or(50) as usize;
    let path = crashes_path(&app).map_err(|e| e.to_string())?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;
    let mut entries: Vec<CrashReport> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<CrashReport>(l).ok())
        .collect();
    entries.reverse();
    entries.truncate(cap);
    Ok(entries)
}

/// Delete the on-disk `crashes.jsonl`. No-op if the file doesn't exist.
#[tauri::command]
fn clear_crash_reports(app: AppHandle) -> Result<(), String> {
    let path = crashes_path(&app).map_err(|e| e.to_string())?;
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| format!("remove {}: {}", path.display(), e))?;
    }
    audit(&app, "data", "crashes_cleared", serde_json::json!({}));
    Ok(())
}

/// Read the user's crash-report opt-in. Returns `true` iff the sentinel
/// file `crash_reporting_opted_in.flag` exists in app-data. Used by the
/// Settings → Diagnostics checkbox to render its initial state.
#[tauri::command]
fn get_crash_reporting_opt_in(app: AppHandle) -> Result<bool, String> {
    let path = crash_optin_path(&app).map_err(|e| e.to_string())?;
    Ok(path.exists())
}

/// Toggle the crash-report opt-in. `true` creates the sentinel,
/// `false` removes it. Idempotent in both directions.
#[tauri::command]
fn set_crash_reporting_opt_in(app: AppHandle, enabled: bool) -> Result<(), String> {
    let path = crash_optin_path(&app).map_err(|e| e.to_string())?;
    if enabled {
        if !path.exists() {
            fs::write(&path, b"1")
                .map_err(|e| format!("write {}: {}", path.display(), e))?;
        }
        audit(
            &app,
            "privacy",
            "crash_reporting_opted_in",
            serde_json::json!({}),
        );
    } else {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| format!("remove {}: {}", path.display(), e))?;
        }
        audit(
            &app,
            "privacy",
            "crash_reporting_opted_out",
            serde_json::json!({}),
        );
    }
    Ok(())
}

/// POST a single crash report (identified by its `ts`) to
/// `CRASH_REPORT_ENDPOINT`. Returns the server's status line so the UI
/// can surface "202 accepted" or similar.
///
/// Hard-gated behind the opt-in sentinel: if the user hasn't ticked the
/// Diagnostics checkbox, this command refuses to send anything and
/// returns an error string. The caller in the React panel surfaces that
/// error inline so the user knows why nothing happened.
///
/// Updates the matching row in `crashes.jsonl` to set
/// `user_dispatched = true` after a successful POST so the Diagnostics
/// modal can render an "already sent" indicator on subsequent re-opens.
#[tauri::command]
async fn dispatch_crash_report(
    app: AppHandle,
    crash_id: String,
) -> Result<String, String> {
    // Hard opt-in gate — never POST without the sentinel.
    let optin = crash_optin_path(&app).map_err(|e| e.to_string())?;
    if !optin.exists() {
        return Err("crash reporting is not opted in".to_string());
    }

    let path = crashes_path(&app).map_err(|e| e.to_string())?;
    if !path.exists() {
        return Err("no crash log on disk".to_string());
    }

    // Locate the target row by `ts` (which acts as our crash_id).
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;
    let mut all: Vec<CrashReport> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<CrashReport>(l).ok())
        .collect();
    let target_idx = all
        .iter()
        .position(|r| r.ts == crash_id)
        .ok_or_else(|| format!("no crash report with id '{crash_id}'"))?;

    // POST the report — and ONLY the report. No other state. Body is the
    // serialized CrashReport JSON object as-is.
    let body = serde_json::to_string(&all[target_idx])
        .map_err(|e| format!("serialize report: {e}"))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client init: {e}"))?;
    let resp = client
        .post(CRASH_REPORT_ENDPOINT)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("crash POST failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("crash POST returned {}", status));
    }

    // Mark the row as dispatched and rewrite the entire file. The file is
    // tiny (50 records max in the UI; even 500 records is < 1 MiB), so
    // the rewrite cost is irrelevant compared to a multi-second HTTP
    // round-trip.
    all[target_idx].user_dispatched = true;
    let rewritten: String = all
        .iter()
        .filter_map(|r| serde_json::to_string(r).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let mut buf = rewritten;
    buf.push('\n');
    fs::write(&path, buf).map_err(|e| format!("rewrite {}: {}", path.display(), e))?;

    audit(
        &app,
        "privacy",
        "crash_dispatched",
        serde_json::json!({ "status": status.as_u16() }),
    );

    Ok(status.to_string())
// ── LAN org: zero-touch mDNS discovery for office deployments ───────────────
//
// Goal: turn 30-PC enrollment from "all 30 users manually exchange
// `phantom:view:spend` addresses" (870 manual steps) into 30 × (paste 6-digit
// code → done). The first install runs `lan_org_create` which generates a
// 6-character org code (`XXX-XXX`, alphabet excludes 0/O/1/I/L) and starts an
// mDNS broadcast on `_phantomchat._tcp.local.` with TXT records:
//
//     org=<code>          // shared secret — also the discovery filter
//     label=<my_label>    // human-readable name (from me.json)
//     addr=<phantom:…>    // 1:1 contact address
//     sigpub=<hex>        // Ed25519 sealed-sender pubkey
//
// Subsequent installs run `lan_org_join(code)` which broadcasts the SAME
// service (so the whole org converges into a mesh) AND continuously browses
// for matching peers. Each match is deduped by `signing_pub_hex`, persisted
// to `lan_org.json` next to `keys.json`, and auto-added as a 1:1 contact via
// the same code path used by `add_contact` (idempotent — skipped when the
// label is already taken). Discovered peers are NOT auto-added as MLS group
// members; that stays an explicit user action.
//
// Authentication model: shared-secret only. Anyone with the org code who is
// on the same broadcast domain can announce themselves as a peer. There is
// NO cryptographic verification of org membership beyond "you typed the
// right code". The Wizard surfaces this explicitly so a user doesn't enable
// mDNS on a hostile network (cafés, hotels, conferences). MVP is single
// broadcast domain — multi-LAN scenarios (e.g. two offices over a VPN) are
// out of scope.
//
// `lan_org.json` on-disk shape:
//
//     { "code": "X9K-3PT",
//       "discovered": [
//         { "label": "alice",
//           "address": "phantom:view:spend",
//           "signing_pub_hex": "deadbeef…",
//           "last_seen": 1712345678 }, … ] }
//
// On every app start, if `lan_org.json` exists we re-arm the broadcast +
// browse from `setup()` so the office mesh keeps reconverging without user
// intervention. Audit log entries: `lan/created`, `lan/joined`,
// `lan/peer_added`, `lan/left`.

const LAN_ORG_FILE: &str = "lan_org.json";
const LAN_ORG_SERVICE_TYPE: &str = "_phantomchat._tcp.local.";
/// Unambiguous-character alphabet for the 6-char org code. Excludes
/// `0/O/1/I/L` to prevent dictation/typing mistakes when an admin reads
/// the code out loud to a colleague over the office. 31 chars; 6 chars
/// gives 31^6 ≈ 8.87 × 10^8 codes which is ample for the office-sized
/// scope (a full room of users would have to randomly collide on a 6-char
/// code, which is far below birthday risk for a small org).
const LAN_ORG_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

/// One discovered LAN peer. Minimal copy of what we need to add the peer
/// as a 1:1 contact + render in the Settings panel. Persisted to
/// `lan_org.json` so a restart doesn't show "0 peers" until the next
/// browse round-trip lands.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub label: String,
    pub address: String,
    pub signing_pub_hex: String,
    /// Unix epoch seconds of the most-recent mDNS announcement we saw.
    /// Used by the Settings panel's "last discovery" surface.
    pub last_seen: u64,
}

/// Live LAN-org state. The `broadcaster` field holds the running
/// `ServiceDaemon` if we're broadcasting; `None` means we tore it down.
/// `discovered` is the deduplicated peer set, persisted to disk on every
/// mutation.
pub struct LanOrg {
    pub code: String,
    pub broadcaster: Option<mdns_sd::ServiceDaemon>,
    pub discovered: Vec<DiscoveredPeer>,
    /// Set when an mDNS browse-event handler successfully resolved a peer
    /// — surfaced via `lan_org_status`. `None` if we've been broadcasting
    /// but haven't seen a peer yet.
    pub last_discovery_ts: Option<u64>,
    /// Background JoinHandle for the browse-event drain task. Kept so
    /// `lan_org_leave` can cancel it; otherwise the task would keep
    /// holding a Receiver into a torn-down ServiceDaemon.
    pub browse_task: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct LanOrgDisk {
    #[serde(default)]
    code: String,
    #[serde(default)]
    discovered: Vec<DiscoveredPeer>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LanOrgStatus {
    pub active: bool,
    pub code: Option<String>,
    pub peer_count: u32,
    pub last_discovery_ts: Option<String>,
}

fn lan_org_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(LAN_ORG_FILE))
}

fn load_lan_org(path: &std::path::Path) -> Option<LanOrgDisk> {
    let raw = fs::read(path).ok()?;
    serde_json::from_slice::<LanOrgDisk>(&raw).ok()
}

fn save_lan_org_disk(path: &std::path::Path, disk: &LanOrgDisk) -> anyhow::Result<()> {
    fs::write(path, serde_json::to_vec_pretty(disk)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Generate a fresh 6-character org code formatted as `XXX-XXX`. CSPRNG
/// (`OsRng`) seeds every character independently; the alphabet excludes
/// ambiguous glyphs (see [`LAN_ORG_CODE_ALPHABET`]).
fn generate_org_code() -> String {
    use rand::rngs::OsRng;
    use rand::Rng;

    let mut rng = OsRng;
    let mut chars = String::with_capacity(7);
    for i in 0..6 {
        let idx = rng.gen_range(0..LAN_ORG_CODE_ALPHABET.len());
        chars.push(LAN_ORG_CODE_ALPHABET[idx] as char);
        if i == 2 {
            chars.push('-');
        }
    }
    chars
}

/// Current wall-clock seconds since the unix epoch. Returns `0` on the
/// (impossible) clock-pre-1970 case rather than panicking — `lan_org.json`
/// is purely a UI surface, never a security invariant.
fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build TXT records for our outgoing mDNS service announcement. Order is
/// stable so the wire bytes are diff-friendly across sessions.
fn build_lan_txt_records(
    code: &str,
    label: &str,
    address: &str,
    sigpub_hex: &str,
) -> std::collections::HashMap<String, String> {
    let mut props = std::collections::HashMap::new();
    props.insert("org".to_string(), code.to_string());
    props.insert("label".to_string(), label.to_string());
    props.insert("addr".to_string(), address.to_string());
    props.insert("sigpub".to_string(), sigpub_hex.to_string());
    props
}

/// Resolve our own `(label, address, signing_pub_hex)` triple for the
/// outgoing TXT records. Falls back to the first 8 chars of the signing
/// pubkey hex when the user hasn't set a display name yet — same pattern
/// as `resolve_self_label` for MLS Welcomes.
fn resolve_self_lan_advertisement(
    app: &AppHandle,
) -> Result<(String, String, String), String> {
    let keys_path = keys_path(app).map_err(|e| e.to_string())?;
    if !keys_path.exists() {
        return Err("no identity yet — call generate_identity first".into());
    }
    let address = address_from_keyfile(&keys_path).map_err(|e| e.to_string())?;
    let raw = fs::read(&keys_path).map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    let sigpub = json["signing_public"]
        .as_str()
        .ok_or_else(|| "missing signing_public in keys.json".to_string())?
        .to_string();
    let me = load_me(app);
    let label = if me.label.trim().is_empty() {
        sigpub.chars().take(8).collect()
    } else {
        me.label.trim().to_string()
    };
    Ok((label, address, sigpub))
}

/// Spawn a `ServiceDaemon`, register our outgoing service, and (if
/// `with_browse` is true) start a continuous browse for the same service
/// type filtering by the org-code TXT match. Returns the daemon + an
/// optional JoinHandle for the browse-drain task.
///
/// The daemon binds to UDP 5353 on every available interface — both IPv4
/// and IPv6 — so the office mesh works regardless of whether the network
/// is dual-stack or v4-only. mdns-sd silently skips interfaces it can't
/// bind to (e.g. a Docker bridge with no multicast group), so a partially
/// firewalled host still announces on the interfaces it CAN reach.
fn start_lan_daemon(
    app: &AppHandle,
    code: &str,
    with_browse: bool,
) -> Result<(mdns_sd::ServiceDaemon, Option<tokio::task::JoinHandle<()>>), String> {
    use mdns_sd::{ServiceDaemon, ServiceInfo};

    let (label, address, sigpub_hex) = resolve_self_lan_advertisement(app)?;
    let txt = build_lan_txt_records(code, &label, &address, &sigpub_hex);

    let daemon = ServiceDaemon::new()
        .map_err(|e| format!("ServiceDaemon::new: {e}"))?;

    // Instance name. We use the first 16 chars of our signing-pub hex so
    // every member of the org broadcasts a stable, identity-derived name
    // that survives restarts (mDNS drops duplicate names on the wire, so
    // a stable name avoids name-flap if two laptops on the same org code
    // happen to share a prefix). The full sigpub is in the TXT record.
    let instance_name = format!("phantomchat-{}", &sigpub_hex.chars().take(16).collect::<String>());
    let host_name = format!("{}.local.", instance_name);

    // Port 0 — we don't expose a TCP service yet. mDNS still propagates
    // the TXT record without a usable port; the txt records are all the
    // discovery layer needs. (Future: a real port for direct LAN sync.)
    let info = ServiceInfo::new(
        LAN_ORG_SERVICE_TYPE,
        &instance_name,
        &host_name,
        "",
        0u16,
        Some(txt),
    )
    .map_err(|e| format!("ServiceInfo::new: {e}"))?
    // `enable_addr_auto` lets mdns-sd auto-detect host IPs from the
    // active interfaces — without it, the empty `host_ip` would emit
    // an unresolvable A/AAAA record.
    .enable_addr_auto();

    daemon
        .register(info)
        .map_err(|e| format!("daemon.register: {e}"))?;

    let browse_task = if with_browse {
        let receiver = daemon
            .browse(LAN_ORG_SERVICE_TYPE)
            .map_err(|e| format!("daemon.browse: {e}"))?;
        let app_for_browse = app.clone();
        let code_for_browse = code.to_string();
        let task = tokio::spawn(async move {
            // Drain the synchronous channel on a blocking-friendly task.
            // mdns-sd's `Receiver` is a flume channel which exposes a
            // non-async `recv` — so we hop into a blocking helper to
            // avoid stalling the tokio runtime.
            loop {
                let event = match tokio::task::spawn_blocking({
                    let r = receiver.clone();
                    move || r.recv()
                })
                .await
                {
                    Ok(Ok(ev)) => ev,
                    // `recv` Err means daemon was shut down; graceful exit.
                    Ok(Err(_)) => break,
                    Err(_) => break,
                };
                match event {
                    mdns_sd::ServiceEvent::ServiceResolved(info) => {
                        let props_map: std::collections::HashMap<String, String> = info
                            .get_properties()
                            .iter()
                            .map(|p| (p.key().to_string(), p.val_str().to_string()))
                            .collect();
                        let other_org = match props_map.get("org") {
                            Some(s) => s.clone(),
                            None => continue,
                        };
                        if other_org != code_for_browse {
                            continue;
                        }
                        let other_sigpub = match props_map.get("sigpub") {
                            Some(s) => s.clone(),
                            None => continue,
                        };
                        // Skip our own broadcast — no point adding ourselves.
                        let self_sigpub = match resolve_self_lan_advertisement(&app_for_browse) {
                            Ok((_, _, s)) => s,
                            Err(_) => String::new(),
                        };
                        if other_sigpub.eq_ignore_ascii_case(&self_sigpub) {
                            continue;
                        }
                        let other_label = props_map
                            .get("label")
                            .cloned()
                            .unwrap_or_else(|| other_sigpub.chars().take(8).collect());
                        let other_addr = match props_map.get("addr") {
                            Some(s) => s.clone(),
                            None => continue,
                        };
                        if PhantomAddress::parse(&other_addr).is_none() {
                            // Skip malformed addresses silently — could be
                            // a stale/buggy peer; no need to surface as an
                            // error to the user.
                            continue;
                        }
                        ingest_lan_peer(
                            &app_for_browse,
                            DiscoveredPeer {
                                label: other_label,
                                address: other_addr,
                                signing_pub_hex: other_sigpub,
                                last_seen: unix_now_secs(),
                            },
                        )
                        .await;
                    }
                    mdns_sd::ServiceEvent::SearchStopped(_) => break,
                    _ => {}
                }
            }
        });
        Some(task)
    } else {
        None
    };

    Ok((daemon, browse_task))
}

/// Idempotent ingest path for a freshly-resolved LAN peer. Dedupes by
/// `signing_pub_hex` — refreshes the `last_seen` timestamp on a re-hit,
/// appends + persists + auto-adds-as-contact on a first-hit. Emits the
/// `lan_peer_discovered` event so the React UI can refresh without
/// polling. Auto-binds the contact's `signing_pub` so the very first
/// incoming sealed-sender envelope from the peer resolves to its label.
async fn ingest_lan_peer(app: &AppHandle, peer: DiscoveredPeer) {
    let state = match app.try_state::<AppState>() {
        Some(s) => s,
        None => return,
    };
    let mut guard = state.lan_org.lock().await;
    let lan = match guard.as_mut() {
        Some(l) => l,
        None => return,
    };
    let pub_hex_lower = peer.signing_pub_hex.to_lowercase();
    let mut is_new = false;
    if let Some(existing) = lan
        .discovered
        .iter_mut()
        .find(|p| p.signing_pub_hex.eq_ignore_ascii_case(&pub_hex_lower))
    {
        existing.last_seen = peer.last_seen;
        // Tolerate label / address drift (a peer may have updated them
        // between sessions). Last writer wins.
        existing.label = peer.label.clone();
        existing.address = peer.address.clone();
    } else {
        lan.discovered.push(peer.clone());
        is_new = true;
    }
    lan.last_discovery_ts = Some(peer.last_seen);
    let code = lan.code.clone();
    let snapshot = lan.discovered.clone();
    drop(guard);

    // Persist BEFORE doing any contact-side mutation so a crash mid-add
    // doesn't lose the discovery itself.
    if let Ok(path) = lan_org_path(app) {
        let _ = save_lan_org_disk(
            &path,
            &LanOrgDisk {
                code,
                discovered: snapshot,
            },
        );
    }

    if is_new {
        // Auto-add as a 1:1 contact via the same on-disk path that
        // `add_contact` writes. Idempotent — if the label is already taken
        // we skip silently (the user already has them under a different
        // label). We DO bind the signing_pub on insert so the first
        // incoming sealed-sender envelope renders with the human label
        // instead of `?<8hex>`.
        if let Ok(contacts_path) = contacts_path(app) {
            let mut book = load_contacts(&contacts_path);
            // Skip if any existing contact already has this signing_pub
            // (don't double-add under a different label) OR has the same
            // label (label collision — leave the user-set one alone).
            let already = book.contacts.iter().any(|c| {
                c.label == peer.label
                    || c.signing_pub
                        .as_deref()
                        .map(|h| h.eq_ignore_ascii_case(&peer.signing_pub_hex))
                        .unwrap_or(false)
            });
            if !already {
                book.contacts.push(Contact {
                    label: peer.label.clone(),
                    address: peer.address.clone(),
                    signing_pub: Some(peer.signing_pub_hex.clone()),
                });
                if save_contacts(&contacts_path, &book).is_ok() {
                    audit(
                        app,
                        "lan",
                        "peer_added",
                        serde_json::json!({ "label": peer.label }),
                    );
                }
            }
        }
    }
    let (boot_path, raw) = match (found_path, raw_bytes) {
        (Some(p), Some(r)) => (p, r),
        _ => return Ok(false),
    };

    let parsed: BootstrapFile = serde_json::from_slice(&raw)
        .map_err(|e| format!("bootstrap parse {}: {}", boot_path.display(), e))?;
    if parsed.schema_version != BOOTSTRAP_SCHEMA_VERSION {
        return Err(format!(
            "bootstrap {} has schema_version={} — this build only supports version {}",
            boot_path.display(),
            parsed.schema_version,
            BOOTSTRAP_SCHEMA_VERSION
        ));
    }

    // 1. Generate fresh identity (same write_identity helper the wizard's
    //    generate_identity command uses, kept additive so the wizard path
    //    stays untouched).
    let identity = write_identity(&keyfile)
        .map_err(|e| format!("bootstrap: identity gen failed: {}", e))?;
    // `signing_pub_short` is the 16-hex-char prefix of the freshly-
    // generated identity. We use it as the self-exclusion key against
    // `directory[].signing_pub_hex` (full 64-char hex) — `starts_with`
    // gives us a 64-bit collision domain inside a single org directory,
    // which is astronomically safe.
    let self_signing_pub_short = identity.signing_pub_short.clone();

    // 2. Pre-populate contacts.json from directory[] (excluding self).
    let contacts_p = contacts_path(&app).map_err(|e| e.to_string())?;
    let mut book = ContactBook::default();
    let mut contacts_added = 0usize;
    let mut contacts_skipped_self = 0usize;
    for entry in &parsed.directory {
        // Self-exclusion: a directory entry whose signing_pub_hex starts
        // with our own short signing-pub prefix is "us" (probability of
        // collision in a same-org directory is astronomical for a 64-bit
        // prefix). The directory is the org-wide list, so the install
        // doesn't a-priori know which entry is itself — we mark all
        // others as contacts and rely on (a) Wave 7A mDNS auto-discovery
        // for onsite + (b) other org members adding this fresh identity
        // through their own contact-add flow for remote.
        if !entry.signing_pub_hex.is_empty()
            && entry.signing_pub_hex.starts_with(&self_signing_pub_short)
        {
            contacts_skipped_self += 1;
            continue;
        }
        book.contacts.push(Contact {
            label: entry.label.clone(),
            address: entry.address.clone(),
            signing_pub: if entry.signing_pub_hex.is_empty() {
                None
            } else {
                Some(entry.signing_pub_hex.clone())
            },
        });
        contacts_added += 1;
    }
    save_contacts(&contacts_p, &book)
        .map_err(|e| format!("bootstrap: write contacts.json: {}", e))?;

    // 3. Pre-populate relays.json from default_relays.
    if !parsed.default_relays.is_empty() {
        if let Err(e) = save_relays(&app, &parsed.default_relays) {
            eprintln!("bootstrap: write relays.json failed: {}", e);
        }
    }

    // 4. Persist branding + window title (best-effort).
    if let Some(display_name) = parsed
        .branding
        .display_name
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        // Stamp into me.json so MLS Welcome metadata shows the branded
        // label as the inviter.
        let _ = save_me(
            &app,
            &MeDisk {
                label: display_name.clone(),
            },
        );
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.set_title(&display_name);
        }
    }

    // 5. Persist the org bootstrap blob into a separate sidecar so a
    //    follow-up wave can read `org_secret` / `org_id` without re-parsing
    //    the original bootstrap.json (which the admin may delete after
    //    apply). Best-effort.
    if let Ok(dir) = app_data(&app) {
        let sidecar = dir.join("org_bootstrap.json");
        let blob = serde_json::json!({
            "schema_version": parsed.schema_version,
            "org_name":   parsed.org_name,
            "org_id":     parsed.org_id,
            "org_secret": parsed.org_secret,
            "branding": {
                "display_name":  parsed.branding.display_name,
                "primary_color": parsed.branding.primary_color,
            },
        });
        if let Ok(buf) = serde_json::to_vec_pretty(&blob) {
            let _ = fs::write(&sidecar, buf);
        }
    }

    // 6. Best-effort LAN auto-join. Wave 7A's `lan_org_join` is on a
    //    separate branch (`feat/wave7-mdns-lan-discovery`) and not yet
    //    merged into this worktree's base. When that PR lands, replace
    //    this stub with `let _ = lan_org_join(app.clone(), code.clone());`
    //    — the bootstrap.json field is wire-compatible already.
    if let Some(code) = parsed.auto_join_lan_org_code.as_ref() {
        eprintln!(
            "bootstrap: auto_join_lan_org_code {:?} present — Wave 7A `lan_org_join` not yet merged into this branch; skipping (best-effort)",
            code
        );
    }

    // 7. Audit + auto-mark onboarded so the wizard never shows.
    audit(
        &app,
        "bootstrap",
        "applied",
        serde_json::json!({
            "org_id": parsed.org_id,
            "org_name_len": parsed.org_name.len(),
            "directory_entries": parsed.directory.len(),
            "contacts_added": contacts_added,
            "contacts_skipped_self": contacts_skipped_self,
            "relay_count": parsed.default_relays.len(),
            "lan_code_present": parsed.auto_join_lan_org_code.is_some(),
            "source_path": boot_path.display().to_string(),
        }),
    );
    if let Err(e) = mark_onboarded(app.clone()) {
        eprintln!("bootstrap: mark_onboarded failed: {} — continuing", e);
    }
    Ok(true)
        let _ = app.emit("lan_peer_discovered", peer);
    }
}

/// Best-effort daemon shutdown. mdns-sd's `shutdown` is a graceful op —
/// peers see a goodbye packet so they can age our entry out instead of
/// waiting for the TTL to expire.
fn stop_lan_daemon(daemon: &mdns_sd::ServiceDaemon) {
    let _ = daemon.shutdown();
}

#[tauri::command]
async fn lan_org_create(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let mut guard = state.lan_org.lock().await;
    if guard.is_some() {
        return Err("already in a LAN org — call lan_org_leave first".into());
    }
    let code = generate_org_code();
    let (daemon, browse_task) = start_lan_daemon(&app, &code, true)?;
    *guard = Some(LanOrg {
        code: code.clone(),
        broadcaster: Some(daemon),
        discovered: Vec::new(),
        last_discovery_ts: None,
        browse_task,
    });
    drop(guard);
    if let Ok(path) = lan_org_path(&app) {
        let _ = save_lan_org_disk(
            &path,
            &LanOrgDisk {
                code: code.clone(),
                discovered: Vec::new(),
            },
        );
    }
    audit(
        &app,
        "lan",
        "created",
        serde_json::json!({ "code": code }),
    );
    Ok(code)
}

#[tauri::command]
async fn lan_org_join(
    app: AppHandle,
    state: State<'_, AppState>,
    code: String,
) -> Result<(), String> {
    // Light validation. We accept the canonical `XXX-XXX` form OR a
    // hyphen-less `XXXXXX` (some users will paste without the hyphen).
    let cleaned: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if cleaned.len() != 6 {
        return Err(format!(
            "invalid org code — expected 6 alphanumeric chars (got {})",
            cleaned.len()
        ));
    }
    if !cleaned
        .as_bytes()
        .iter()
        .all(|b| LAN_ORG_CODE_ALPHABET.contains(b))
    {
        return Err("invalid org code — contains ambiguous chars (0/O/1/I/L)".into());
    }
    let canonical = format!("{}-{}", &cleaned[..3], &cleaned[3..]);

    let mut guard = state.lan_org.lock().await;
    if guard.is_some() {
        return Err("already in a LAN org — call lan_org_leave first".into());
    }
    let (daemon, browse_task) = start_lan_daemon(&app, &canonical, true)?;
    // Rehydrate any prior discovered peers if `lan_org.json` carried them
    // and the codes match (a re-join with the same code preserves history).
    let prior = lan_org_path(&app)
        .ok()
        .and_then(|p| load_lan_org(&p))
        .filter(|d| d.code == canonical)
        .map(|d| d.discovered)
        .unwrap_or_default();
    *guard = Some(LanOrg {
        code: canonical.clone(),
        broadcaster: Some(daemon),
        discovered: prior.clone(),
        last_discovery_ts: None,
        browse_task,
    });
    drop(guard);
    if let Ok(path) = lan_org_path(&app) {
        let _ = save_lan_org_disk(
            &path,
            &LanOrgDisk {
                code: canonical.clone(),
                discovered: prior,
            },
        );
    }
    audit(
        &app,
        "lan",
        "joined",
        serde_json::json!({ "code": canonical }),
    );
    Ok(())
}

#[tauri::command]
async fn lan_org_status(state: State<'_, AppState>) -> Result<LanOrgStatus, String> {
    let guard = state.lan_org.lock().await;
    Ok(match guard.as_ref() {
        None => LanOrgStatus {
            active: false,
            code: None,
            peer_count: 0,
            last_discovery_ts: None,
        },
        Some(lan) => LanOrgStatus {
            active: lan.broadcaster.is_some(),
            code: Some(lan.code.clone()),
            peer_count: lan.discovered.len() as u32,
            last_discovery_ts: lan.last_discovery_ts.map(|ts| ts.to_string()),
        },
    })
}

#[tauri::command]
async fn lan_org_leave(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut guard = state.lan_org.lock().await;
    let lan = match guard.take() {
        Some(l) => l,
        None => return Err("not in a LAN org".into()),
    };
    drop(guard);
    if let Some(d) = lan.broadcaster.as_ref() {
        stop_lan_daemon(d);
    }
    if let Some(t) = lan.browse_task {
        t.abort();
    }
    if let Ok(path) = lan_org_path(&app) {
        // Best-effort: unlink the persisted file so the next launch
        // doesn't auto-rehydrate a torn-down org.
        let _ = fs::remove_file(&path);
    }
    audit(
        &app,
        "lan",
        "left",
        serde_json::json!({ "code": lan.code }),
    );
    Ok(())
}

/// Setup-time helper: re-arm the broadcaster + browser if `lan_org.json`
/// already exists from a previous session. Best-effort — a daemon spawn
/// failure (e.g. UDP 5353 blocked by a firewall) is logged via stderr but
/// MUST NOT prevent the app from booting; the user can manually retry
/// from the Settings panel.
async fn rehydrate_lan_org_on_startup(app: AppHandle) {
    let path = match lan_org_path(&app) {
        Ok(p) => p,
        Err(_) => return,
    };
    let disk = match load_lan_org(&path) {
        Some(d) if !d.code.is_empty() => d,
        _ => return,
    };
    let state = match app.try_state::<AppState>() {
        Some(s) => s,
        None => return,
    };
    let (daemon, browse_task) = match start_lan_daemon(&app, &disk.code, true) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("lan_org rehydrate skipped: {e}");
            return;
        }
    };
    let mut guard = state.lan_org.lock().await;
    *guard = Some(LanOrg {
        code: disk.code,
        broadcaster: Some(daemon),
        discovered: disk.discovered,
        last_discovery_ts: None,
        browse_task,
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            generate_identity,
            get_address,
            address_qr_svg,
            list_contacts,
            add_contact,
            send_message,
            start_listener,
            bind_last_unbound_sender,
            mls_init,
            mls_publish_key_package,
            mls_create_group,
            mls_add_member,
            mls_join_via_welcome,
            mls_encrypt,
            mls_decrypt,
            mls_status,
            set_my_label,
            get_my_label,
            save_history,
            load_history,
            get_connection_status,
            list_relays,
            set_relays,
            export_keyfile,
            import_keyfile,
            wipe_all_data,
            get_app_version,
            is_onboarded,
            mark_onboarded,
            send_file,
            open_downloads_folder,
            mls_list_members,
            mls_leave_group,
            mls_remove_member,
            get_privacy_config,
            set_privacy_config,
            restart_listener,
            mark_read,
            typing_ping,
            search_messages,
            read_audit_log,
            export_audit_log,
            check_for_updates,
            install_update,
            send_reply,
            send_reaction,
            set_disappearing_ttl,
            get_disappearing_ttl,
            outgoing_expires_at,
            list_crash_reports,
            clear_crash_reports,
            dispatch_crash_report,
            get_crash_reporting_opt_in,
            set_crash_reporting_opt_in,
            export_backup,
            import_backup,
            verify_backup,
            lan_org_create,
            lan_org_join,
            lan_org_status,
            lan_org_leave,
            reset_window_state,
        ])
        .setup(|app| {
            // Pre-create the data dir on first launch so command handlers
            // never have to defensively `mkdir -p` on the hot path.
            let handle = app.handle().clone();
            let _ = app_data(&handle);

            // Kick off the disappearing-messages auto-purge timer. Idempotent
            // via `AppState.purge_started`. The first sweep fires after one
            // full PURGE_INTERVAL_SECS so we don't trample the React-side
            // hydrate-then-debounced-save path on cold start.
            spawn_purge_task(&handle);
            // ── Wave 7C: enterprise pre-seeded bootstrap ─────────────────
            // If a `bootstrap.json` is present (admin-pushed via Group
            // Policy / Intune file deploy or bundled inside the MSI),
            // apply it BEFORE the wizard's `is_onboarded` check fires on
            // the frontend. `try_apply_bootstrap` is idempotent + a no-op
            // on already-onboarded installs, so re-launches are safe.
            // Failures are logged but never fatal — the wizard fallback
            // path stays available for the user.
            match try_apply_bootstrap(handle.clone()) {
                Ok(true) => {
                    eprintln!("bootstrap: applied — onboarding wizard will be skipped");
                }
                Ok(false) => {
                    // Normal startup — no bootstrap or already-onboarded.
                }
                Err(e) => {
                    eprintln!("bootstrap: apply failed: {} — falling back to wizard", e);
                }
            }
            // Install the panic hook so any subsequent panic appends a
            // structured row to `crashes.jsonl` BEFORE the runtime
            // aborts. Must happen after `app_data()` so the hook's
            // cached path resolves to a real, mkdir'd directory.
            set_panic_hook(handle.clone());
            // ── LAN org rehydrate ───────────────────────────────────────
            // If `lan_org.json` exists from a previous session, re-arm
            // the mDNS broadcast + browse so the office mesh keeps
            // converging without user intervention. Best-effort — a
            // failure (e.g. UDP 5353 firewalled) is logged via stderr but
            // does NOT block the app from launching. Spawned on the
            // tokio runtime so the synchronous setup hook returns
            // promptly.
            let lan_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                rehydrate_lan_org_on_startup(lan_handle).await;
            });

            // ── System tray ──────────────────────────────────────────────
            // Build a minimal menu: Show/Hide toggles main window, Status
            // surfaces the latest connection_status, Quit terminates the
            // app. Single-click on the tray icon also toggles visibility.
            let status_text = handle
                .try_state::<AppState>()
                .and_then(|s| s.connection_status.lock().ok().map(|g| g.clone()))
                .filter(|s| !s.is_empty())
                .map(|s| format!("Status: {}", s))
                .unwrap_or_else(|| "Status: -".to_string());

            let show_hide = MenuItemBuilder::with_id("toggle", "Show / Hide").build(app)?;
            let status = MenuItemBuilder::with_id("status", &status_text)
                .enabled(false)
                .build(app)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

            let menu = MenuBuilder::new(app)
                .items(&[&show_hide, &status, &sep, &quit])
                .build()?;

            // The bundle icon (`icons/icon.png` per tauri.conf.json) is
            // exposed as the default window icon at runtime — reuse it
            // for the tray so we don't have to ship a second image.
            let tray_builder = TrayIconBuilder::with_id("phantomchat-tray")
                .tooltip("PhantomChat");
            let tray_builder = match app.default_window_icon().cloned() {
                Some(img) => tray_builder.icon(img),
                None => tray_builder,
            };
            let _tray = tray_builder
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "toggle" => toggle_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // ── Window close → hide instead of exit ──────────────────────
            // ── Window-state restore + persistent geometry ───────────────
            //
            // Restore happens BEFORE we wire up the listener so the
            // initial set_position/set_size we apply doesn't trigger an
            // immediate echo-write back to disk. The listener is then
            // installed with a debounce so that drag/resize gestures
            // collapse to one write per WINDOW_STATE_DEBOUNCE_MS.
            if let Some(win) = app.get_webview_window("main") {
                // 1. Apply persisted geometry if a matching monitor is
                //    still attached and the rect fits on it.
                if let Some(label) = restore_window_state(&handle, &win) {
                    audit(
                        &handle,
                        "display",
                        "window_restored",
                        serde_json::json!({ "monitor_label": label }),
                    );
                }

                // 2. Combined window-event listener:
                //    - CloseRequested → hide (existing behavior)
                //    - Resized / Moved → debounced save_window_state
                //    Maximize/unmaximize is reported as a Resized event
                //    on every desktop platform Tauri supports, so we
                //    re-read `is_maximized()` inside that branch.
                let win_for_event = win.clone();
                let app_for_event = handle.clone();
                let last_save = Arc::new(StdMutex::new(
                    std::time::Instant::now()
                        - std::time::Duration::from_millis(WINDOW_STATE_DEBOUNCE_MS),
                ));
                win.on_window_event(move |e| match e {
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        let _ = win_for_event.hide();
                    }
                    WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
                        // Debounce: only persist if the previous save
                        // happened more than WINDOW_STATE_DEBOUNCE_MS ago.
                        // We deliberately accept losing the very last
                        // gesture if the user rage-quits within 500 ms —
                        // worst case is we restore a position that's off
                        // by a few pixels from where they let go of the
                        // mouse, which the off-screen rescue catches if
                        // it ever matters.
                        let now = std::time::Instant::now();
                        let mut guard = match last_save.lock() {
                            Ok(g) => g,
                            Err(_) => return,
                        };
                        if now.duration_since(*guard)
                            < std::time::Duration::from_millis(WINDOW_STATE_DEBOUNCE_MS)
                        {
                            return;
                        }
                        *guard = now;
                        drop(guard);
                        if let Ok(state) = capture_window_state(&win_for_event) {
                            let _ = save_window_state(&app_for_event, state);
                        }
                    }
                    _ => {}
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Toggle the main window visibility from a tray menu / icon click. Called
/// from both the menu's "Show / Hide" item and the left-click event so
/// behavior stays consistent regardless of entry point.
fn toggle_main_window(app: &AppHandle) {
    let win = match app.get_webview_window("main") {
        Some(w) => w,
        None => return,
    };
    let visible = win.is_visible().unwrap_or(false);
    if visible {
        let _ = win.hide();
    } else {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

// ── 1:1 encrypted file transfer (single-envelope, max 5 MiB) ────────────────
//
// Wire format on the sealed-sender plaintext:
//
//     "FILE1:01" || ULEB128(meta_len) || meta_json || raw_bytes
//
// `meta_json` is a [`FileManifest`] (filename, size, mime, sha256_hex). The
// filename is ALWAYS basename-only on the sender; the receiver re-strips on
// its side (defense in depth against `../` traversal). Receiver verifies the
// sha256 of the raw bytes matches the manifest before flagging `sha256_ok`.
//
// MVP: single-envelope only. Files > MAX_FILE_BYTES (5 MiB) are rejected on
// the sender. Larger payloads will need chunking — deliberately deferred.

const FILE_PREFIX_V1: &[u8; 8] = b"FILE1:01";
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileManifest {
    filename: String,
    size: u64,
    mime: String,
    sha256_hex: String,
}

/// Result returned by [`send_file`] so the frontend can immediately echo a
/// "📎 sent <filename>" row into the message stream without waiting for the
/// next listener tick.
#[derive(Clone, Debug, Serialize)]
pub struct FileSendResult {
    pub filename: String,
    pub size: u64,
    pub sha256_hex: String,
}

/// Payload for the `file_received` event. Mirrored 1:1 by TS
/// `FileReceivedEvent` in `desktop/src/types.ts`.
#[derive(Clone, Debug, Serialize)]
pub struct FileReceivedEvent {
    pub from_label: String,
    pub filename: String,
    pub size: u64,
    pub saved_path: String,
    pub sha256_ok: bool,
    pub sha256_hex: String,
    pub ts: String,
    /// Mirrors the 1:1 text path's `sender_pub_hex` so the frontend can
    /// surface the same "bind unknown sender" affordance for files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_pub_hex: Option<String>,
}

/// 1024-base human size, mirrors the CLI's `cli/src/file_cmd.rs::human_bytes`.
/// Stand-alone copy (not a shared helper) per the task spec.
fn human_size(n: u64) -> String {
    const K: f64 = 1024.0;
    let n = n as f64;
    if n < K {
        return format!("{n:.0} B");
    }
    if n < K * K {
        return format!("{:.1} KiB", n / K);
    }
    if n < K * K * K {
        return format!("{:.1} MiB", n / (K * K));
    }
    format!("{:.1} GiB", n / (K * K * K))
}

/// Strip every path-separator-bearing component, leaving only the final
/// basename. We ALSO reject anything containing `..` or NUL — defense
/// against `../` directory traversal AND filesystem-path-injection attacks
/// where an attacker tries to write outside the Downloads folder.
///
/// Returns `Err` for an empty/invalid name so callers can refuse the
/// payload entirely instead of saving as `<empty>` or `unknown`.
fn sanitize_filename(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("filename is empty".into());
    }
    if name.contains('\0') {
        return Err("filename contains NUL byte".into());
    }
    // `..` segment ban. We check the raw input AND the post-basename to
    // catch `subdir/..` (becomes "..") and bare ".." alike.
    let trimmed = name.trim();
    if trimmed == "." || trimmed == ".." {
        return Err(format!("invalid filename '{trimmed}'"));
    }
    // Strip every `/` and `\` segment via std::path::Path::file_name. This
    // works even on Linux for backslash-bearing names because we ALSO do a
    // manual rsplit on `\` first — `Path::file_name` on Linux treats `\` as
    // a regular character, which is the wrong behavior for a Windows-
    // origin filename arriving over the wire.
    let after_bs = trimmed.rsplit('\\').next().unwrap_or(trimmed);
    let path_view = std::path::Path::new(after_bs);
    let basename = path_view
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("filename '{name}' has no basename"))?;
    if basename.contains("..") {
        // E.g. `..hidden` is fine but `foo..bar` we let through; reject the
        // sneaky `..` *segment* representations here defensively.
        if basename == ".." {
            return Err("filename '..' rejected".into());
        }
    }
    if basename.is_empty() {
        return Err("filename basename is empty after sanitization".into());
    }
    Ok(basename.to_string())
}

/// Resolve the on-disk Downloads/PhantomChat directory, creating it on
/// demand. Falls back to `$HOME` if the platform Downloads dir is
/// unresolvable (e.g. a barebones CI environment).
fn downloads_dir() -> Result<PathBuf, String> {
    let base = dirs::download_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| "could not resolve Downloads or home directory".to_string())?;
    let target = base.join("PhantomChat");
    fs::create_dir_all(&target)
        .map_err(|e| format!("mkdir -p {}: {}", target.display(), e))?;
    Ok(target)
}

/// Pick a non-colliding output path under `dir`. Strategy: if `name` is
/// free, use it; otherwise try `name (2)`, `name (3)`, … up to a cap of
/// 999. Splits on the FIRST `.` after the basename so a name like
/// `report.tar.gz` becomes `report (2).tar.gz` (matches OS file-manager
/// convention closer than splitting on the last `.`).
fn next_available_path(dir: &std::path::Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    // Split into stem + ext so the suffix lands BEFORE the extension.
    let (stem, ext) = match name.find('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 2..=999 {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // Pathological fallback — at >999 collisions just timestamp it and
    // accept the ugliness rather than silently overwriting.
    dir.join(format!(
        "{stem}-{}{ext}",
        chrono::Local::now().format("%Y%m%d%H%M%S")
    ))
}

/// Build the wire payload for an outgoing file. Validates the size cap
/// before it's encoded so we don't waste cycles on a doomed envelope.
fn build_file_payload(filename: &str, bytes: &[u8]) -> Result<(Vec<u8>, FileSendResult), String> {
    use sha2::{Digest, Sha256};

    let size = bytes.len() as u64;
    if size > MAX_FILE_BYTES {
        return Err(format!(
            "file too large: {} (max {})",
            human_size(size),
            human_size(MAX_FILE_BYTES)
        ));
    }
    let basename = sanitize_filename(filename)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let sha256_hex = hex::encode(hasher.finalize());
    // Cheap MIME guess from extension only — full content sniffing would
    // add a dep we don't need for an MVP wire-display field. The receiver
    // doesn't gate on `mime`; it's purely UI hint metadata.
    let mime = guess_mime(&basename);
    let manifest = FileManifest {
        filename: basename.clone(),
        size,
        mime,
        sha256_hex: sha256_hex.clone(),
    };
    let meta_json = serde_json::to_vec(&manifest)
        .map_err(|e| format!("serialize manifest: {e}"))?;
    let mut payload =
        Vec::with_capacity(FILE_PREFIX_V1.len() + 5 + meta_json.len() + bytes.len());
    payload.extend_from_slice(FILE_PREFIX_V1);
    write_uleb128(&mut payload, meta_json.len() as u64);
    payload.extend_from_slice(&meta_json);
    payload.extend_from_slice(bytes);
    Ok((
        payload,
        FileSendResult {
            filename: basename,
            size,
            sha256_hex,
        },
    ))
}

/// Coarse MIME guess from filename extension — covers the handful of
/// office/media types most likely to flow through a B2B internal
/// messenger. Anything unknown falls back to `application/octet-stream`,
/// which any OS file manager will route to "open with…".
fn guess_mime(filename: &str) -> String {
    let lower = filename.to_ascii_lowercase();
    let ext = std::path::Path::new(&lower)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    match ext {
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" | "tgz" => "application/gzip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Receiver-side handler for a `FILE1:01` payload. Decodes the manifest,
/// verifies size + sha256, re-strips the filename to basename, writes the
/// file under `Downloads/PhantomChat/<filename>` (collision-safe), then
/// emits a `file_received` event AND fires a native notification.
fn handle_incoming_file_v1(
    app: &AppHandle,
    body: &[u8],
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
    contacts_path: &std::path::Path,
) {
    use sha2::{Digest, Sha256};

    // 1. Parse meta_len varint + manifest JSON.
    let (meta_len, consumed) = match read_uleb128(body) {
        Some(t) => t,
        None => {
            let _ = app.emit("error", "FILE1:01 truncated meta length".to_string());
            return;
        }
    };
    let meta_end = match consumed.checked_add(meta_len as usize) {
        Some(v) => v,
        None => {
            let _ = app.emit("error", "FILE1:01 meta_len overflow".to_string());
            return;
        }
    };
    if body.len() < meta_end {
        let _ = app.emit(
            "error",
            format!(
                "FILE1:01 meta_len {} exceeds body of {} bytes",
                meta_len,
                body.len()
            ),
        );
        return;
    }
    let manifest: FileManifest = match serde_json::from_slice(&body[consumed..meta_end]) {
        Ok(m) => m,
        Err(e) => {
            let _ = app.emit("error", format!("FILE1:01 manifest JSON: {e}"));
            return;
        }
    };
    let raw = &body[meta_end..];

    // 2. Size sanity. Reject anything claiming or carrying more than the cap
    // so a malicious sender can't burn our disk + RAM with a bogus payload.
    let actual_size = raw.len() as u64;
    if actual_size > MAX_FILE_BYTES || manifest.size > MAX_FILE_BYTES {
        let _ = app.emit(
            "error",
            format!(
                "FILE1:01 oversized: claimed {} actual {} (cap {})",
                manifest.size,
                actual_size,
                MAX_FILE_BYTES
            ),
        );
        return;
    }
    if actual_size != manifest.size {
        let _ = app.emit(
            "error",
            format!(
                "FILE1:01 size mismatch: manifest {} vs actual {}",
                manifest.size, actual_size
            ),
        );
        return;
    }

    // 3. Re-strip filename on receiver (defense in depth).
    let safe_name = match sanitize_filename(&manifest.filename) {
        Ok(n) => n,
        Err(e) => {
            let _ = app.emit("error", format!("FILE1:01 unsafe filename: {e}"));
            return;
        }
    };

    // 4. Verify hash. We DON'T abort on mismatch — we still save the file
    // so the user can decide what to do, but we flip `sha256_ok` so the UI
    // renders a red ⚠ tint and a "DO NOT OPEN" tooltip.
    let mut hasher = Sha256::new();
    hasher.update(raw);
    let computed_hex = hex::encode(hasher.finalize());
    let sha256_ok = computed_hex.eq_ignore_ascii_case(&manifest.sha256_hex);

    // 5. Resolve sender attribution against the contact list — same vocab
    // as the 1:1 text path so the UI's row renderer can reuse its
    // contact / INBOX / "?<8hex>" branches.
    let (from_label, sender_pub_hex) = match sender_pub {
        None => ("INBOX".to_string(), None),
        Some(_) if !sig_ok => ("INBOX!".to_string(), None),
        Some(pub_bytes) => {
            let pub_hex = hex::encode(pub_bytes);
            let book = load_contacts(contacts_path);
            let matched = book
                .contacts
                .iter()
                .find(|c| {
                    c.signing_pub
                        .as_deref()
                        .map(|h| h.eq_ignore_ascii_case(&pub_hex))
                        .unwrap_or(false)
                })
                .map(|c| c.label.clone());
            match matched {
                Some(lbl) => (lbl, Some(pub_hex)),
                None => {
                    if let Some(state) = app.try_state::<AppState>() {
                        if let Ok(mut slot) = state.last_unbound_sender.lock() {
                            *slot = Some(pub_bytes);
                        }
                    }
                    let label = format!("?{}", &pub_hex[..8]);
                    (label, Some(pub_hex))
                }
            }
        }
    };

    // 6. Choose a non-colliding output path under Downloads/PhantomChat/.
    let dir = match downloads_dir() {
        Ok(d) => d,
        Err(e) => {
            let _ = app.emit("error", format!("FILE1:01 downloads dir: {e}"));
            return;
        }
    };
    let out_path = next_available_path(&dir, &safe_name);
    if let Err(e) = fs::write(&out_path, raw) {
        let _ = app.emit(
            "error",
            format!("FILE1:01 write {}: {}", out_path.display(), e),
        );
        return;
    }

    let saved_path_str = out_path.to_string_lossy().to_string();

    // 7. Native notification (skipped if window is focused — `maybe_notify`
    // handles that gating).
    maybe_notify(
        app,
        &format!("\u{1F4CE} File from {from_label}"),
        &format!("{} ({})", safe_name, human_size(manifest.size)),
    );

    // 8. Emit the structured event for the React side.
    let event = FileReceivedEvent {
        from_label,
        filename: safe_name,
        size: manifest.size,
        saved_path: saved_path_str,
        sha256_ok,
        sha256_hex: manifest.sha256_hex,
        ts: chrono::Local::now().format("%H:%M:%S").to_string(),
        sender_pub_hex,
    };
    let _ = app.emit("file_received", event);
}

#[tauri::command]
async fn send_file(
    app: AppHandle,
    contact_label: String,
    file_path: String,
) -> Result<FileSendResult, String> {
    // 1. Resolve contact. Same lookup logic as `send_message_inner` — we
    // duplicate rather than reach into that helper so the file path keeps
    // its own clear error semantics.
    let contacts_path = contacts_path(&app).map_err(|e| e.to_string())?;
    let book = load_contacts(&contacts_path);
    let contact = book
        .contacts
        .iter()
        .find(|c| c.label == contact_label)
        .ok_or_else(|| format!("unknown contact '{contact_label}'"))?
        .clone();

    // 2. Read + size-cap + manifest-build.
    let raw_path = std::path::Path::new(&file_path);
    let metadata = fs::metadata(raw_path)
        .map_err(|e| format!("stat {}: {}", raw_path.display(), e))?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(format!(
            "file too large: {} (max {})",
            human_size(metadata.len()),
            human_size(MAX_FILE_BYTES)
        ));
    }
    let bytes = fs::read(raw_path)
        .map_err(|e| format!("read {}: {}", raw_path.display(), e))?;

    // Sender-side basename strip (defense in depth — receiver re-strips).
    let basename = raw_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("file_path '{file_path}' has no basename"))?;

    let (payload, result) = build_file_payload(basename, &bytes)?;

    // 3. Ship via the same sealed-sender path as text messages. Reuses
    // multi-relay fan-out + retry from `send_sealed_to_address`.
    send_sealed_to_address(&app, &contact.address, &payload)
        .await
        .map_err(|e| format!("relay send: {e}"))?;

    Ok(result)
}

#[tauri::command]
fn open_downloads_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let dir = downloads_dir()?;
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| format!("open downloads dir: {e}"))?;
    Ok(())
}
// ── Read receipts + typing indicators (RCPT-1: / TYPN-1:) ───────────────────
//
// Both prefixes piggyback on the existing sealed-sender 1:1 transport so
// the relay can't see who is acknowledging whose messages or who is
// typing to whom. The wire shape mirrors the FILE1:01 / MLS-WLC2 pattern:
//
//     <prefix> || ULEB128(meta_len) || meta_json
//
// `RCPT-1:` payload (`ReceiptMetaV1`):
//     { "msg_id": "<16-hex>", "kind": "delivered" | "read" }
//
// `TYPN-1:` payload (`TypingMetaV1`):
//     { "contact_label": "<sender's label for the recipient>",
//       "ttl_secs": 5 }
//
// `contact_label` in the typing meta is the SENDER's local label for the
// recipient; the receiver ignores it and uses the resolved sealed-sender
// pubkey -> contact mapping to decide which conversation the "typing" pill
// belongs to. We carry it for forward compatibility / debug telemetry.

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReceiptMetaV1 {
    msg_id: String,
    /// "delivered" | "read".
    kind: String,
}

/// Inline metadata carried by a REPL-1: envelope.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReplyMetaV1 {
    in_reply_to_msg_id: String,
    quoted_preview: String,
}

/// Inline metadata carried by a RACT-1: envelope. `action` is "add" or
/// "remove" — the receiver mutates the target row's `reactions` array
/// accordingly and re-emits the merged state to the frontend.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReactionMetaV1 {
    target_msg_id: String,
    emoji: String,
    /// "add" or "remove".
    action: String,
}

/// Inline metadata carried by a DISA-1: envelope. `ttl_secs == None`
/// disables the timer for the conversation. The peer applies the same
/// TTL to its local copy on receive.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct DisappearingMetaV1 {
    contact_label: String,
    ttl_secs: Option<u32>,
}

/// On-disk shape of `disappearing.json` — one entry per contact label
/// whose conversation has an active TTL. Missing key = no auto-purge.
/// Stored as a flat object so a user can hand-edit it to inspect /
/// remove entries without going through the UI.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DisappearingDisk {
    /// Map<contact_label, ttl_secs>.
    #[serde(flatten)]
    entries: std::collections::HashMap<String, u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TypingMetaV1 {
    contact_label: String,
    ttl_secs: u32,
}

/// Frontend-facing event emitted when a peer's `RCPT-1:` envelope decodes.
/// React's reducer escalates the matching outgoing row's `delivery_state`
/// monotonically -- see `App.tsx` listener.
#[derive(Clone, Debug, Serialize)]
pub struct ReceiptEvent {
    pub from_label: String,
    pub msg_id: String,
    pub kind: String,
}

/// Frontend-facing event emitted when a peer's `TYPN-1:` envelope decodes.
/// React maintains a `Map<from_label, expiry_ms>` and renders the
/// "<label> is typing..." pill above the input bar until the TTL elapses.
#[derive(Clone, Debug, Serialize)]
pub struct TypingEvent {
    pub from_label: String,
    pub ttl_secs: u32,
}

/// Compute the stable per-message identifier used to thread receipts back
/// to the correct outgoing row.
///
/// Recipe: `sha256("v1|" || hex(plaintext))`, truncated to the first 16
/// hex chars (= 64 bits of entropy). The `ts` arg is accepted but kept
/// out of the hash on purpose: sender + receiver each derive their own
/// local `HH:MM:SS` and would disagree across the second boundary,
/// breaking the receipt-to-row matchup. Hashing only the plaintext is
/// enough collision protection inside a single conversation (birthday
/// threshold ~2^32 distinct messages — several orders of magnitude
/// beyond any plausible usage), and it's bit-exactly reproducible on
/// both endpoints regardless of clock skew.
fn compute_msg_id(_ts: &str, plaintext: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"v1|");
    hasher.update(hex::encode(plaintext).as_bytes());
    let digest = hasher.finalize();
    let full = hex::encode(digest);
    full[..16].to_string()
}

/// Wrap a `ReceiptMetaV1` into the `RCPT-1:` wire shape.
fn build_receipt_payload(msg_id: &str, kind: &str) -> Result<Vec<u8>, String> {
    let meta = ReceiptMetaV1 {
        msg_id: msg_id.to_string(),
        kind: kind.to_string(),
    };
    let meta_json = serde_json::to_vec(&meta)
        .map_err(|e| format!("serialize receipt meta: {e}"))?;
    let mut out = Vec::with_capacity(RCPT_PREFIX_V1.len() + 5 + meta_json.len());
    out.extend_from_slice(RCPT_PREFIX_V1);
    write_uleb128(&mut out, meta_json.len() as u64);
    out.extend_from_slice(&meta_json);
    Ok(out)
}

/// Wrap a `TypingMetaV1` into the `TYPN-1:` wire shape.
fn build_typing_payload(contact_label: &str, ttl_secs: u32) -> Result<Vec<u8>, String> {
    let meta = TypingMetaV1 {
        contact_label: contact_label.to_string(),
        ttl_secs,
    };
    let meta_json = serde_json::to_vec(&meta)
        .map_err(|e| format!("serialize typing meta: {e}"))?;
    let mut out = Vec::with_capacity(TYPN_PREFIX_V1.len() + 5 + meta_json.len());
    out.extend_from_slice(TYPN_PREFIX_V1);
    write_uleb128(&mut out, meta_json.len() as u64);
    out.extend_from_slice(&meta_json);
    Ok(out)
}

/// Resolve the sender of an inbound RCPT/TYPN envelope to a human label
/// using the same convention as the 1:1 text path. Returns `None` for
/// the no-attribution / forged-signature cases -- receipts + typing pings
/// are USELESS without a known sender (we'd have no row to update / no
/// pill to render), so we drop them quietly.
fn resolve_meta_sender_label(
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
    contacts_path: &std::path::Path,
) -> Option<String> {
    let pub_bytes = sender_pub?;
    if !sig_ok {
        return None;
    }
    let pub_hex = hex::encode(pub_bytes);
    let book = load_contacts(contacts_path);
    book.contacts
        .iter()
        .find(|c| {
            c.signing_pub
                .as_deref()
                .map(|h| h.eq_ignore_ascii_case(&pub_hex))
                .unwrap_or(false)
        })
        .map(|c| c.label.clone())
}

/// Receiver-side handler for a `RCPT-1:` envelope. Parses meta, resolves
/// sender -> contact label, and emits a `receipt` event so the frontend can
/// escalate the matching outgoing row's delivery state.
fn handle_incoming_receipt_v1(
    app: &AppHandle,
    body: &[u8],
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
    contacts_path: &std::path::Path,
) {
    let (meta_len, consumed) = match read_uleb128(body) {
        Some(t) => t,
        None => {
            let _ = app.emit("error", "RCPT-1: truncated meta length".to_string());
            return;
        }
    };
    let meta_end = match consumed.checked_add(meta_len as usize) {
        Some(v) => v,
        None => {
            let _ = app.emit("error", "RCPT-1: meta_len overflow".to_string());
            return;
        }
    };
    if body.len() < meta_end {
        let _ = app.emit(
            "error",
            format!("RCPT-1: meta_len {} exceeds body of {} bytes", meta_len, body.len()),
        );
        return;
    }
    let meta: ReceiptMetaV1 = match serde_json::from_slice(&body[consumed..meta_end]) {
        Ok(m) => m,
        Err(e) => {
            let _ = app.emit("error", format!("RCPT-1: meta JSON: {e}"));
            return;
        }
    };
    if meta.kind != "delivered" && meta.kind != "read" {
        let _ = app.emit(
            "error",
            format!("RCPT-1: unknown receipt kind '{}'", meta.kind),
        );
        return;
    }
    let from_label = match resolve_meta_sender_label(sender_pub, sig_ok, contacts_path) {
        Some(l) => l,
        None => return,
    };
    let _ = app.emit(
        "receipt",
        ReceiptEvent {
            from_label,
            msg_id: meta.msg_id,
            kind: meta.kind,
        },
    );
}

/// Receiver-side handler for a `TYPN-1:` envelope. Same parse + resolve
/// path as `RCPT-1:`. Emits a `typing` event; the React layer holds the
/// per-contact expiry timer and renders the "is typing..." pill.
fn handle_incoming_typing_v1(
    app: &AppHandle,
    body: &[u8],
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
    contacts_path: &std::path::Path,
) {
    let (meta_len, consumed) = match read_uleb128(body) {
        Some(t) => t,
        None => {
            let _ = app.emit("error", "TYPN-1: truncated meta length".to_string());
            return;
        }
    };
    let meta_end = match consumed.checked_add(meta_len as usize) {
        Some(v) => v,
        None => {
            let _ = app.emit("error", "TYPN-1: meta_len overflow".to_string());
            return;
        }
    };
    if body.len() < meta_end {
        let _ = app.emit(
            "error",
            format!("TYPN-1: meta_len {} exceeds body of {} bytes", meta_len, body.len()),
        );
        return;
    }
    let meta: TypingMetaV1 = match serde_json::from_slice(&body[consumed..meta_end]) {
        Ok(m) => m,
        Err(e) => {
            let _ = app.emit("error", format!("TYPN-1: meta JSON: {e}"));
            return;
        }
    };
    let from_label = match resolve_meta_sender_label(sender_pub, sig_ok, contacts_path) {
        Some(l) => l,
        None => return,
    };
    let ttl = meta.ttl_secs.clamp(1, 30);
    let _ = app.emit(
        "typing",
        TypingEvent {
            from_label,
            ttl_secs: ttl,
        },
    );
}

/// Build + send a sealed-sender RCPT-1: envelope to the named contact.
/// Centralised so both the auto-`delivered` path inside the listener and
/// the user-driven `mark_read` command go through the same wire builder.
async fn send_receipt(
    app: &AppHandle,
    contact_label: &str,
    msg_id: &str,
    kind: &str,
) -> Result<(), String> {
    let contacts_path = contacts_path(app).map_err(|e| e.to_string())?;
    let book = load_contacts(&contacts_path);
    let contact = book
        .contacts
        .iter()
        .find(|c| c.label == contact_label)
        .ok_or_else(|| format!("unknown contact '{contact_label}'"))?
        .clone();
    let payload = build_receipt_payload(msg_id, kind)?;
    send_sealed_to_address(app, &contact.address, &payload)
        .await
        .map_err(|e| format!("relay send: {e}"))
}

/// Frontend command: called by MessageStream's IntersectionObserver when
/// an INCOMING row scrolls into view AND the window is focused. Fires a
/// sealed-sender `RCPT-1:` envelope with `kind: "read"` to the original
/// sender so they can escalate the row's delivery_state to read.
#[tauri::command]
async fn mark_read(
    app: AppHandle,
    msg_id: String,
    contact_label: String,
) -> Result<(), String> {
    send_receipt(&app, &contact_label, &msg_id, "read").await
}

/// Frontend command: called by InputBar on the leading edge of every
/// 1.5s typing burst. Builds + ships a sealed-sender `TYPN-1:` envelope
/// to the active conversation peer carrying the default `TYPING_TTL_SECS`.
#[tauri::command]
async fn typing_ping(app: AppHandle, contact_label: String) -> Result<(), String> {
    let contacts_path = contacts_path(&app).map_err(|e| e.to_string())?;
    let book = load_contacts(&contacts_path);
    let contact = book
        .contacts
        .iter()
        .find(|c| c.label == contact_label)
        .ok_or_else(|| format!("unknown contact '{contact_label}'"))?
        .clone();
    let payload = build_typing_payload(&contact_label, TYPING_TTL_SECS)?;
    send_sealed_to_address(&app, &contact.address, &payload)
        .await
        .map_err(|e| format!("relay send: {e}"))
}

// ── Reply / reactions / disappearing messages ──────────────────────────────
//
// Three additive sealed-sender envelopes:
//
//   REPL-1: || ULEB128(meta_len) || { in_reply_to_msg_id, quoted_preview } || body
//   RACT-1: || ULEB128(meta_len) || { target_msg_id, emoji, action: "add"|"remove" }
//   DISA-1: || ULEB128(meta_len) || { contact_label, ttl_secs: u32 | null }
//
// Reply payloads decode into a normal `IncomingMessage` row whose
// `reply_to` is populated. Reactions accumulate client-side onto the
// target message's `reactions` array (we mutate `messages.json` in place).
// Disappearing-TTL events update the per-contact `disappearing.json`
// file and emit `disappearing_ttl_changed` so the UI can re-render.
//
// Auto-purge: a periodic 60s tokio task scans `messages.json`, drops
// rows whose `expires_at <= now()`, and emits `messages_purged` with
// the dropped msg_ids so the React state can prune in lockstep.

fn disappearing_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(DISAPPEARING_FILE))
}

fn load_disappearing(app: &AppHandle) -> DisappearingDisk {
    disappearing_path(app)
        .ok()
        .and_then(|p| fs::read(&p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_disappearing(app: &AppHandle, disk: &DisappearingDisk) -> anyhow::Result<()> {
    let path = disappearing_path(app)?;
    fs::write(&path, serde_json::to_vec_pretty(disk)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Truncate a body to <=80 characters (UTF-8-safe via char iteration) so
/// the on-wire `quoted_preview` never balloons the envelope.
fn truncate_preview(body: &str) -> String {
    let mut out = String::new();
    for (i, ch) in body.chars().enumerate() {
        if i >= 80 {
            out.push('\u{2026}');
            break;
        }
        out.push(ch);
    }
    out
}

/// Wrap a `ReplyMetaV1` + body into the `REPL-1:` wire shape.
fn build_reply_payload(
    in_reply_to_msg_id: &str,
    quoted_preview: &str,
    body: &[u8],
) -> Result<Vec<u8>, String> {
    let meta = ReplyMetaV1 {
        in_reply_to_msg_id: in_reply_to_msg_id.to_string(),
        quoted_preview: quoted_preview.to_string(),
    };
    let meta_json = serde_json::to_vec(&meta)
        .map_err(|e| format!("serialize reply meta: {e}"))?;
    let mut out = Vec::with_capacity(REPL_PREFIX_V1.len() + 5 + meta_json.len() + body.len());
    out.extend_from_slice(REPL_PREFIX_V1);
    write_uleb128(&mut out, meta_json.len() as u64);
    out.extend_from_slice(&meta_json);
    out.extend_from_slice(body);
    Ok(out)
}

/// Wrap a `ReactionMetaV1` into the `RACT-1:` wire shape.
fn build_reaction_payload(
    target_msg_id: &str,
    emoji: &str,
    action: &str,
) -> Result<Vec<u8>, String> {
    let meta = ReactionMetaV1 {
        target_msg_id: target_msg_id.to_string(),
        emoji: emoji.to_string(),
        action: action.to_string(),
    };
    let meta_json = serde_json::to_vec(&meta)
        .map_err(|e| format!("serialize reaction meta: {e}"))?;
    let mut out = Vec::with_capacity(RACT_PREFIX_V1.len() + 5 + meta_json.len());
    out.extend_from_slice(RACT_PREFIX_V1);
    write_uleb128(&mut out, meta_json.len() as u64);
    out.extend_from_slice(&meta_json);
    Ok(out)
}

/// Wrap a `DisappearingMetaV1` into the `DISA-1:` wire shape.
fn build_disappearing_payload(
    contact_label: &str,
    ttl_secs: Option<u32>,
) -> Result<Vec<u8>, String> {
    let meta = DisappearingMetaV1 {
        contact_label: contact_label.to_string(),
        ttl_secs,
    };
    let meta_json = serde_json::to_vec(&meta)
        .map_err(|e| format!("serialize disappearing meta: {e}"))?;
    let mut out = Vec::with_capacity(DISA_PREFIX_V1.len() + 5 + meta_json.len());
    out.extend_from_slice(DISA_PREFIX_V1);
    write_uleb128(&mut out, meta_json.len() as u64);
    out.extend_from_slice(&meta_json);
    Ok(out)
}

/// Current Unix time in seconds. Wraps `chrono::Utc::now().timestamp()`
/// to a `u64` for storage in `expires_at`.
fn now_unix_secs() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

/// Receiver-side handler for a `REPL-1:` envelope. Parses meta + body,
/// builds an `IncomingMessage` row whose `reply_to` is populated, and
/// emits the standard `message` event so the existing React reducer +
/// notification + auto-`delivered`-receipt path all fire unchanged.
fn handle_incoming_reply_v1(
    app: &AppHandle,
    body: &[u8],
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
    contacts_path: &std::path::Path,
) {
    let (meta_len, consumed) = match read_uleb128(body) {
        Some(t) => t,
        None => {
            let _ = app.emit("error", "REPL-1: truncated meta length".to_string());
            return;
        }
    };
    let meta_end = match consumed.checked_add(meta_len as usize) {
        Some(v) => v,
        None => {
            let _ = app.emit("error", "REPL-1: meta_len overflow".to_string());
            return;
        }
    };
    if body.len() < meta_end {
        let _ = app.emit(
            "error",
            format!("REPL-1: meta_len {} exceeds body of {} bytes", meta_len, body.len()),
        );
        return;
    }
    let meta: ReplyMetaV1 = match serde_json::from_slice(&body[consumed..meta_end]) {
        Ok(m) => m,
        Err(e) => {
            let _ = app.emit("error", format!("REPL-1: meta JSON: {e}"));
            return;
        }
    };
    let plaintext_bytes = &body[meta_end..];

    // Resolve sender attribution mirror of the 1:1 text path.
    let (sender_label, sender_pub_hex) = match sender_pub {
        None => ("INBOX".to_string(), None),
        Some(_) if !sig_ok => ("INBOX!".to_string(), None),
        Some(pub_bytes) => {
            let pub_hex = hex::encode(pub_bytes);
            let book = load_contacts(contacts_path);
            let matched = book
                .contacts
                .iter()
                .find(|c| {
                    c.signing_pub
                        .as_deref()
                        .map(|h| h.eq_ignore_ascii_case(&pub_hex))
                        .unwrap_or(false)
                })
                .map(|c| c.label.clone());
            match matched {
                Some(lbl) => (lbl, Some(pub_hex)),
                None => {
                    if let Some(state) = app.try_state::<AppState>() {
                        if let Ok(mut slot) = state.last_unbound_sender.lock() {
                            *slot = Some(pub_bytes);
                        }
                    }
                    let label = format!("?{}", &pub_hex[..8]);
                    (label, Some(pub_hex))
                }
            }
        }
    };

    let ts = chrono::Local::now().format("%H:%M:%S").to_string();
    let msg_id = compute_msg_id(&ts, plaintext_bytes);
    let plaintext = String::from_utf8_lossy(plaintext_bytes).to_string();

    // Apply per-contact disappearing TTL on incoming reply rows so the
    // peer's auto-disappear clock starts the moment we receive it.
    let expires_at = match sender_label.as_str() {
        "INBOX" | "INBOX!" => None,
        l if l.starts_with('?') => None,
        l => {
            let disk = load_disappearing(app);
            disk.entries.get(l).map(|secs| now_unix_secs() + (*secs as u64))
        }
    };

    let payload = IncomingMessage {
        plaintext: plaintext.clone(),
        timestamp: ts.clone(),
        sender_label: sender_label.clone(),
        sig_ok,
        sender_pub_hex,
        direction: "incoming".to_string(),
        kind: None,
        file_meta: None,
        msg_id: Some(msg_id.clone()),
        delivery_state: None,
        reply_to: Some(ReplyMeta {
            in_reply_to_msg_id: meta.in_reply_to_msg_id,
            quoted_preview: meta.quoted_preview,
        }),
        reactions: Vec::new(),
        expires_at,
    };
    maybe_notify(
        app,
        &format!("From {}", payload.sender_label),
        &payload.plaintext,
    );
    let _ = app.emit("message", payload);

    // Auto-`delivered` receipt — same condition as the 1:1 text path.
    if !sender_label.starts_with('?')
        && sender_label != "INBOX"
        && sender_label != "INBOX!"
    {
        let app_for_rcpt = app.clone();
        let label_for_rcpt = sender_label.clone();
        let id_for_rcpt = msg_id;
        tokio::spawn(async move {
            if let Err(e) =
                send_receipt(&app_for_rcpt, &label_for_rcpt, &id_for_rcpt, "delivered").await
            {
                let _ = app_for_rcpt
                    .emit("error", format!("send delivered receipt (reply): {e}"));
            }
        });
    }
}

/// Receiver-side handler for a `RACT-1:` envelope. Loads `messages.json`,
/// finds the row whose `msg_id` matches `target_msg_id`, mutates its
/// `reactions` array (add or remove the matching `(sender_label, emoji)`
/// entry), persists the file, and emits `reaction_updated` with the
/// merged reactions so the React side can patch its in-memory state.
fn handle_incoming_reaction_v1(
    app: &AppHandle,
    body: &[u8],
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
    contacts_path: &std::path::Path,
) {
    let (meta_len, consumed) = match read_uleb128(body) {
        Some(t) => t,
        None => {
            let _ = app.emit("error", "RACT-1: truncated meta length".to_string());
            return;
        }
    };
    let meta_end = match consumed.checked_add(meta_len as usize) {
        Some(v) => v,
        None => {
            let _ = app.emit("error", "RACT-1: meta_len overflow".to_string());
            return;
        }
    };
    if body.len() < meta_end {
        let _ = app.emit(
            "error",
            format!("RACT-1: meta_len {} exceeds body of {} bytes", meta_len, body.len()),
        );
        return;
    }
    let meta: ReactionMetaV1 = match serde_json::from_slice(&body[consumed..meta_end]) {
        Ok(m) => m,
        Err(e) => {
            let _ = app.emit("error", format!("RACT-1: meta JSON: {e}"));
            return;
        }
    };
    if meta.action != "add" && meta.action != "remove" {
        let _ = app.emit(
            "error",
            format!("RACT-1: unknown action '{}'", meta.action),
        );
        return;
    }
    let from_label = match resolve_meta_sender_label(sender_pub, sig_ok, contacts_path) {
        Some(l) => l,
        None => return,
    };

    // Mutate the on-disk history under the history lock so a concurrent
    // `save_history` from the frontend can't trample our update.
    let state = match app.try_state::<AppState>() {
        Some(s) => s,
        None => return,
    };
    let _guard = match state.history_lock.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let path = match messages_path(app) {
        Ok(p) => p,
        Err(e) => {
            let _ = app.emit("error", format!("RACT-1: messages_path: {e}"));
            return;
        }
    };
    let mut history: Vec<IncomingMessage> = if path.exists() {
        let raw = match fs::read(&path) {
            Ok(r) => r,
            Err(e) => {
                let _ = app.emit("error", format!("RACT-1: read history: {e}"));
                return;
            }
        };
        serde_json::from_slice(&raw).unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut updated_reactions: Option<Vec<ReactionEntry>> = None;
    for row in history.iter_mut() {
        if row.msg_id.as_deref() == Some(meta.target_msg_id.as_str()) {
            let entry = ReactionEntry {
                sender_label: from_label.clone(),
                emoji: meta.emoji.clone(),
            };
            match meta.action.as_str() {
                "add" => {
                    if !row.reactions.contains(&entry) {
                        row.reactions.push(entry);
                    }
                }
                "remove" => {
                    row.reactions.retain(|e| e != &entry);
                }
                _ => unreachable!("validated above"),
            }
            updated_reactions = Some(row.reactions.clone());
            break;
        }
    }

    if let Some(ref reactions) = updated_reactions {
        if let Ok(buf) = serde_json::to_vec_pretty(&history) {
            if let Err(e) = fs::write(&path, buf) {
                let _ = app.emit("error", format!("RACT-1: write history: {e}"));
                return;
            }
        }
        let _ = app.emit(
            "reaction_updated",
            serde_json::json!({
                "target_msg_id": meta.target_msg_id,
                "reactions": reactions,
            }),
        );
    }
    // No matching row -> silently drop. The reaction may have arrived
    // before the history hydrated on a fresh launch; the wire stream is
    // append-only so a future re-send would re-apply.
}

/// Receiver-side handler for a `DISA-1:` envelope. Mutates the local
/// `disappearing.json` file with the peer's chosen TTL and emits
/// `disappearing_ttl_changed` so the React UI can show a system message.
fn handle_incoming_disappearing_v1(
    app: &AppHandle,
    body: &[u8],
    sender_pub: Option<[u8; 32]>,
    sig_ok: bool,
    contacts_path: &std::path::Path,
) {
    let (meta_len, consumed) = match read_uleb128(body) {
        Some(t) => t,
        None => {
            let _ = app.emit("error", "DISA-1: truncated meta length".to_string());
            return;
        }
    };
    let meta_end = match consumed.checked_add(meta_len as usize) {
        Some(v) => v,
        None => {
            let _ = app.emit("error", "DISA-1: meta_len overflow".to_string());
            return;
        }
    };
    if body.len() < meta_end {
        let _ = app.emit(
            "error",
            format!("DISA-1: meta_len {} exceeds body of {} bytes", meta_len, body.len()),
        );
        return;
    }
    let meta: DisappearingMetaV1 = match serde_json::from_slice(&body[consumed..meta_end]) {
        Ok(m) => m,
        Err(e) => {
            let _ = app.emit("error", format!("DISA-1: meta JSON: {e}"));
            return;
        }
    };
    let from_label = match resolve_meta_sender_label(sender_pub, sig_ok, contacts_path) {
        Some(l) => l,
        None => return,
    };

    // The receiver applies the TTL to ITS local label for the sender,
    // not whatever the sender called itself. `meta.contact_label` is
    // carried for forward-compat / debug telemetry only.
    let _ = meta.contact_label;
    let mut disk = load_disappearing(app);
    match meta.ttl_secs {
        Some(secs) => {
            disk.entries.insert(from_label.clone(), secs);
        }
        None => {
            disk.entries.remove(&from_label);
        }
    }
    if let Err(e) = save_disappearing(app, &disk) {
        let _ = app.emit("error", format!("DISA-1: save: {e}"));
        return;
    }
    let _ = app.emit(
        "disappearing_ttl_changed",
        serde_json::json!({
            "contact_label": from_label,
            "ttl_secs": meta.ttl_secs,
        }),
    );
}

/// Frontend command: send a sealed-sender REPL-1: envelope quoting the
/// row identified by `in_reply_to_msg_id`. Returns the new outgoing
/// row's `msg_id` so the React side can stamp it on the optimistic echo.
#[tauri::command]
async fn send_reply(
    app: AppHandle,
    contact_label: String,
    body: String,
    in_reply_to_msg_id: String,
    quoted_preview: String,
) -> Result<String, String> {
    let contacts_path = contacts_path(&app).map_err(|e| e.to_string())?;
    let book = load_contacts(&contacts_path);
    let contact = book
        .contacts
        .iter()
        .find(|c| c.label == contact_label)
        .ok_or_else(|| format!("unknown contact '{contact_label}'"))?
        .clone();

    let preview = truncate_preview(&quoted_preview);
    let payload = build_reply_payload(&in_reply_to_msg_id, &preview, body.as_bytes())?;
    let msg_id = compute_msg_id("", body.as_bytes());
    send_sealed_to_address(&app, &contact.address, &payload)
        .await
        .map_err(|e| format!("relay send: {e}"))?;
    Ok(msg_id)
}

/// Frontend command: send a sealed-sender RACT-1: envelope. `action`
/// is "add" or "remove". The peer's listener mutates its on-disk
/// history row's `reactions` array.
#[tauri::command]
async fn send_reaction(
    app: AppHandle,
    contact_label: String,
    target_msg_id: String,
    emoji: String,
    action: String,
) -> Result<(), String> {
    if action != "add" && action != "remove" {
        return Err(format!("invalid reaction action '{action}' (want add|remove)"));
    }
    let contacts_path = contacts_path(&app).map_err(|e| e.to_string())?;
    let book = load_contacts(&contacts_path);
    let contact = book
        .contacts
        .iter()
        .find(|c| c.label == contact_label)
        .ok_or_else(|| format!("unknown contact '{contact_label}'"))?
        .clone();
    let payload = build_reaction_payload(&target_msg_id, &emoji, &action)?;
    send_sealed_to_address(&app, &contact.address, &payload)
        .await
        .map_err(|e| format!("relay send: {e}"))
}

/// Frontend command: persist a per-contact disappearing-messages TTL
/// AND notify the peer so they apply the same setting to their own
/// copy. `ttl_secs == None` clears the timer for that conversation.
#[tauri::command]
async fn set_disappearing_ttl(
    app: AppHandle,
    contact_label: String,
    ttl_secs: Option<u32>,
) -> Result<(), String> {
    let contacts_path = contacts_path(&app).map_err(|e| e.to_string())?;
    let book = load_contacts(&contacts_path);
    let contact = book
        .contacts
        .iter()
        .find(|c| c.label == contact_label)
        .ok_or_else(|| format!("unknown contact '{contact_label}'"))?
        .clone();

    // Persist locally first so a relay failure doesn't roll back the
    // user-visible "TTL is on" indicator (the peer will simply not have
    // matching settings until a retry).
    {
        let mut disk = load_disappearing(&app);
        match ttl_secs {
            Some(secs) => {
                disk.entries.insert(contact_label.clone(), secs);
            }
            None => {
                disk.entries.remove(&contact_label);
            }
        }
        save_disappearing(&app, &disk).map_err(|e| e.to_string())?;
    }

    let payload = build_disappearing_payload(&contact_label, ttl_secs)?;
    send_sealed_to_address(&app, &contact.address, &payload)
        .await
        .map_err(|e| format!("relay send: {e}"))?;

    let _ = app.emit(
        "disappearing_ttl_changed",
        serde_json::json!({
            "contact_label": contact_label,
            "ttl_secs": ttl_secs,
        }),
    );
    Ok(())
}

/// Frontend command: read the current TTL (in seconds) for a given
/// contact. `None` means no auto-disappear is configured.
#[tauri::command]
fn get_disappearing_ttl(
    app: AppHandle,
    contact_label: String,
) -> Result<Option<u32>, String> {
    let disk = load_disappearing(&app);
    Ok(disk.entries.get(&contact_label).copied())
}

/// Frontend command called by `App.tsx` immediately before
/// `send_message` so the optimistic outgoing row gets the same
/// `expires_at` the peer will compute on receive. Returns the
/// configured TTL for the contact (or `None` if disappearing is off).
/// This is a thin alias of `get_disappearing_ttl` for naming symmetry
/// with the send path — both share the same on-disk source of truth.
#[tauri::command]
fn outgoing_expires_at(
    app: AppHandle,
    contact_label: String,
) -> Result<Option<u64>, String> {
    let disk = load_disappearing(&app);
    Ok(disk
        .entries
        .get(&contact_label)
        .map(|secs| now_unix_secs() + (*secs as u64)))
}

/// Background sweep: every 60s, walk `messages.json` and drop rows
/// whose `expires_at <= now()`. Emits `messages_purged` with the
/// dropped msg_ids so the React reducer can prune in lockstep. Held
/// under the same `history_lock` `save_history` uses so we never
/// race a concurrent save.
fn run_purge_sweep(app: &AppHandle) {
    let state = match app.try_state::<AppState>() {
        Some(s) => s,
        None => return,
    };
    let _guard = match state.history_lock.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let path = match messages_path(app) {
        Ok(p) => p,
        Err(_) => return,
    };
    if !path.exists() {
        return;
    }
    let raw = match fs::read(&path) {
        Ok(r) => r,
        Err(_) => return,
    };
    let history: Vec<IncomingMessage> = match serde_json::from_slice(&raw) {
        Ok(h) => h,
        Err(_) => return,
    };
    let now = now_unix_secs();
    let mut purged_ids: Vec<String> = Vec::new();
    let kept: Vec<IncomingMessage> = history
        .into_iter()
        .filter_map(|row| {
            if let Some(deadline) = row.expires_at {
                if deadline <= now {
                    if let Some(id) = row.msg_id.clone() {
                        purged_ids.push(id);
                    }
                    return None;
                }
            }
            Some(row)
        })
        .collect();
    if purged_ids.is_empty() {
        return;
    }
    if let Ok(buf) = serde_json::to_vec_pretty(&kept) {
        let _ = fs::write(&path, buf);
    }
    let _ = app.emit(
        "messages_purged",
        serde_json::json!({ "msg_ids": purged_ids }),
    );
}

/// Spawn the periodic auto-purge tokio task. Idempotent — guarded by
/// `AppState.purge_started` so a hot-reload double-mount doesn't end up
/// with two timers writing the file in parallel. Called from `setup()`.
fn spawn_purge_task(app: &AppHandle) {
    let state = match app.try_state::<AppState>() {
        Some(s) => s,
        None => return,
    };
    {
        let mut started = match state.purge_started.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if *started {
            return;
        }
        *started = true;
    }
    let app_for_task = app.clone();
    tokio::spawn(async move {
        // Sleep first so a cold-start purge doesn't fire before the
        // history file even exists (the React side hydrates async on
        // launch). Subsequent ticks fire on the configured interval.
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(PURGE_INTERVAL_SECS));
        // Skip the first tick (which fires immediately) so the very
        // first sweep happens after one full interval. This prevents
        // a startup race where the history file is mid-rewrite by the
        // React side (it does a debounced save 500ms after hydrate).
        interval.tick().await;
        loop {
            interval.tick().await;
            run_purge_sweep(&app_for_task);
        }
    });
}
// ── Encrypted backup / restore (Wave 8c, compliance Aufbewahrungspflicht) ────
//
// Steuerberater + Anwälte are legally required to retain client communication
// for 10 years (German GoBD / §147 AO). Without an export path, a stolen
// laptop = permanent data loss = unsellable for the compliance-bound segment.
//
// Format — single `.pcbackup` file (a regular ZIP with a renamed extension):
//
//   backup-meta.json          UNENCRYPTED. { version: 1, created_at,
//                             item_count, host_label }. Lets `verify_backup`
//                             show provenance BEFORE the user types the
//                             passphrase.
//   wrapped-key.json          UNENCRYPTED wrapper:
//                             { version, kdf: "argon2id",
//                               kdf_params: { m: 65536, t: 3, p: 1 },
//                               salt: <b64 16B>,
//                               wrapped_data_key_nonce: <b64 24B>,
//                               wrapped_data_key_ct:    <b64> }
//   <filename>.nonce          24 bytes raw (XChaCha20-Poly1305 nonce).
//   <filename>.ct             ciphertext + 16-byte Poly1305 tag.
//
// Crypto recipe:
//   - 32-byte data-key from `OsRng`.
//   - Argon2id derives a 32-byte KEK from passphrase. Parameters pinned to
//     OWASP 2023 (m = 64 MiB, t = 3, p = 1). 16-byte salt from `OsRng`.
//   - Each per-file payload encrypted with XChaCha20-Poly1305 + a fresh
//     24-byte nonce (so we never repeat (key, nonce) even if the same
//     filename is re-encrypted on a future backup).
//   - The data-key itself is wrapped with the KEK using XChaCha20-Poly1305
//     + its own fresh nonce so brute-forcing the passphrase is the only
//     attack path.
//   - `Zeroize` wipes both keys after each backup or restore.
//
// Restore is two-phase: decrypt every file into a TEMP dir under app_data,
// then atomically rename onto the live paths after a clean listener
// shutdown. If decryption of any file fails the temp dir is wiped and the
// live data is untouched.

use chacha20poly1305::{
    aead::{rand_core::RngCore as _, Aead, AeadCore, KeyInit, OsRng as AeadOsRng},
    XChaCha20Poly1305, XNonce,
};
use zeroize::Zeroize;

const BACKUP_FORMAT_VERSION: u8 = 1;
const BACKUP_META_FILENAME: &str = "backup-meta.json";
const BACKUP_WRAPPED_KEY_FILENAME: &str = "wrapped-key.json";
const BACKUP_KDF_M_KIB: u32 = 64 * 1024; // 64 MiB
const BACKUP_KDF_T: u32 = 3;
const BACKUP_KDF_P: u32 = 1;
const BACKUP_SALT_LEN: usize = 16;
const BACKUP_KEY_LEN: usize = 32;
const BACKUP_XNONCE_LEN: usize = 24;

/// Files at the root of `app_data_dir/` we attempt to back up. Missing
/// entries are skipped quietly — a fresh install may not have a privacy.json
/// or audit.log yet, and that's a valid backup. The list intentionally
/// includes every file the spec calls out plus the optional `lan_org.json`
/// / `window_state.json` / `disappearing.json` slots so a future feature
/// addition picks them up automatically.
const BACKUP_ROOT_FILES: &[&str] = &[
    KEYS_FILE,
    CONTACTS_FILE,
    SESSIONS_FILE,
    MESSAGES_FILE,
    MLS_DIRECTORY_FILE,
    ME_FILE,
    RELAYS_FILE,
    PRIVACY_FILE,
    AUDIT_LOG_FILE,
    "disappearing.json",
    "lan_org.json",
    "window_state.json",
];

/// Files inside the `mls_state/` subdirectory. Walked by name (rather than
/// dir-listed) so the backup payload is deterministic across runs and
/// future additions to the MLS storage backend show up explicitly here.
const BACKUP_MLS_FILES: &[&str] = &["mls_state.bin", "mls_meta.json"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupMeta {
    pub version: u8,
    pub created_at: String,
    pub item_count: u32,
    pub host_label: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BackupResult {
    pub path: String,
    pub size_bytes: u64,
    pub sha256_hex: String,
    pub item_count: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct RestoreResult {
    pub items_restored: u32,
    pub identity_replaced: bool,
    pub requires_restart: bool,
}

/// On-disk layout of the unencrypted `wrapped-key.json` blob. Carries
/// every parameter `verify_backup` / `import_backup` need to recreate the
/// KEK and unwrap the data-key.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct WrappedKeyBlob {
    version: u8,
    kdf: String,
    kdf_params: WrappedKeyKdfParams,
    salt: String,
    wrapped_data_key_nonce: String,
    wrapped_data_key_ct: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WrappedKeyKdfParams {
    m: u32,
    t: u32,
    p: u32,
}

/// OS hostname best-effort. Used purely as a `host_label` UX hint inside
/// `backup-meta.json` so the user can tell "alice-laptop" backups apart
/// from "alice-desktop" ones in the file picker. Falls back to the
/// platform default if the env var isn't set — never errors.
fn detect_host_label() -> String {
    if let Ok(h) = std::env::var("HOSTNAME") {
        if !h.trim().is_empty() {
            return h;
        }
    }
    if let Ok(h) = std::env::var("COMPUTERNAME") {
        if !h.trim().is_empty() {
            return h;
        }
    }
    if let Ok(h) = std::env::var("HOST") {
        if !h.trim().is_empty() {
            return h;
        }
    }
    "phantomchat-host".to_string()
}

/// Derive a 32-byte KEK from passphrase + salt via Argon2id with the
/// pinned OWASP 2023 params. Wraps the byte array in `Zeroize` semantics
/// at the call site (caller is responsible for `key.zeroize()` after use).
fn derive_kek(passphrase: &str, salt: &[u8]) -> Result<[u8; BACKUP_KEY_LEN], String> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(BACKUP_KDF_M_KIB, BACKUP_KDF_T, BACKUP_KDF_P, Some(BACKUP_KEY_LEN))
        .map_err(|e| format!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; BACKUP_KEY_LEN];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())?;
    Ok(out)
}

/// Random fixed-length byte buffer via the AEAD-re-exported `OsRng`.
fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    AeadOsRng.fill_bytes(&mut buf);
    buf
}

/// Encrypt `plaintext` with `key` + a fresh random nonce. Returns
/// `(nonce_bytes, ciphertext_with_tag)`. The nonce is 24 bytes (XChaCha20)
/// so collision probability across a single-user backup history is
/// negligible (birthday bound ~2^96).
fn aead_encrypt(key: &[u8; BACKUP_KEY_LEN], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut AeadOsRng);
    let ct = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| format!("aead encrypt: {e}"))?;
    Ok((nonce.to_vec(), ct))
}

/// Inverse of [`aead_encrypt`]. Returns the German error string on tag
/// mismatch so the user sees the same passphrase-or-corruption hint
/// regardless of which ciphertext fails first.
fn aead_decrypt(key: &[u8; BACKUP_KEY_LEN], nonce: &[u8], ct: &[u8]) -> Result<Vec<u8>, String> {
    if nonce.len() != BACKUP_XNONCE_LEN {
        return Err(format!(
            "nonce length mismatch: expected {} got {}",
            BACKUP_XNONCE_LEN,
            nonce.len()
        ));
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XNonce::from_slice(nonce);
    cipher
        .decrypt(nonce, ct)
        .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())
}

/// Read every backup-eligible file from `app_data_dir/` into an in-memory
/// `(name, bytes)` list. The order is stable (root files first, then
/// `mls_state/` entries) so two consecutive backups of an unchanged
/// install round-trip to the same SHA256.
fn collect_backup_payload(
    data_dir: &std::path::Path,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    for name in BACKUP_ROOT_FILES {
        let p = data_dir.join(name);
        if let Ok(bytes) = fs::read(&p) {
            out.push(((*name).to_string(), bytes));
        }
    }
    let mls_dir = data_dir.join(MLS_STATE_DIR);
    if mls_dir.is_dir() {
        for name in BACKUP_MLS_FILES {
            let p = mls_dir.join(name);
            if let Ok(bytes) = fs::read(&p) {
                out.push((format!("{}/{}", MLS_STATE_DIR, name), bytes));
            }
        }
    }
    if out.is_empty() {
        return Err(
            "Keine Daten zum Sichern gefunden — bitte zuerst eine Identität erzeugen".into(),
        );
    }
    Ok(out)
}

/// Build the `.pcbackup` archive in-memory and write it to `output_path`.
/// Audit-log entry omits BOTH the passphrase and any key material — only
/// the destination path + item count + sha256 hex are recorded. Any error
/// returned is safe to surface verbatim to the React layer.
fn write_backup_archive(
    output_path: &std::path::Path,
    payload: Vec<(String, Vec<u8>)>,
    passphrase: &str,
    host_label: String,
) -> Result<BackupResult, String> {
    use std::io::Write as IoWrite;
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    if passphrase.chars().count() < 12 {
        return Err("Passphrase muss mindestens 12 Zeichen haben".into());
    }

    let item_count = payload.len() as u32;

    // ── Generate keys + meta ────────────────────────────────────────────
    let mut data_key = [0u8; BACKUP_KEY_LEN];
    AeadOsRng.fill_bytes(&mut data_key);
    let salt = random_bytes(BACKUP_SALT_LEN);
    let mut kek = derive_kek(passphrase, &salt)?;

    let (wrap_nonce, wrap_ct) = match aead_encrypt(&kek, &data_key) {
        Ok(v) => v,
        Err(e) => {
            data_key.zeroize();
            kek.zeroize();
            return Err(e);
        }
    };

    let wrapped = WrappedKeyBlob {
        version: BACKUP_FORMAT_VERSION,
        kdf: "argon2id".to_string(),
        kdf_params: WrappedKeyKdfParams {
            m: BACKUP_KDF_M_KIB,
            t: BACKUP_KDF_T,
            p: BACKUP_KDF_P,
        },
        salt: B64.encode(&salt),
        wrapped_data_key_nonce: B64.encode(&wrap_nonce),
        wrapped_data_key_ct: B64.encode(&wrap_ct),
    };

    let meta = BackupMeta {
        version: BACKUP_FORMAT_VERSION,
        created_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        item_count,
        host_label,
    };

    // ── Encrypt each payload entry into a per-file (nonce, ct) pair ─────
    let mut encrypted: Vec<(String, Vec<u8>, Vec<u8>)> = Vec::with_capacity(payload.len());
    for (name, bytes) in payload {
        let result = aead_encrypt(&data_key, &bytes);
        // Drop plaintext bytes ASAP — they may contain identity material.
        drop(bytes);
        match result {
            Ok((nonce, ct)) => encrypted.push((name, nonce, ct)),
            Err(e) => {
                data_key.zeroize();
                kek.zeroize();
                return Err(e);
            }
        }
    }
    // Plaintext data-key + KEK are no longer needed — wipe before any
    // further allocations / I/O so a panic during ZIP write can't leak.
    data_key.zeroize();
    kek.zeroize();

    // ── Assemble ZIP ────────────────────────────────────────────────────
    let mut zip_bytes: Vec<u8> = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut zip_bytes);
        let mut zw = ZipWriter::new(cursor);
        // Stored (no compression) — XChaCha ciphertext is incompressible
        // and skipping deflate is markedly faster.
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        zw.start_file(BACKUP_META_FILENAME, opts)
            .map_err(|e| format!("zip meta: {e}"))?;
        zw.write_all(
            &serde_json::to_vec_pretty(&meta).map_err(|e| format!("meta ser: {e}"))?,
        )
        .map_err(|e| format!("zip meta write: {e}"))?;

        zw.start_file(BACKUP_WRAPPED_KEY_FILENAME, opts)
            .map_err(|e| format!("zip wrapped-key: {e}"))?;
        zw.write_all(
            &serde_json::to_vec_pretty(&wrapped).map_err(|e| format!("wrapped ser: {e}"))?,
        )
        .map_err(|e| format!("zip wrapped-key write: {e}"))?;

        for (name, nonce, ct) in &encrypted {
            zw.start_file(format!("{}.nonce", name), opts)
                .map_err(|e| format!("zip {name}.nonce: {e}"))?;
            zw.write_all(nonce)
                .map_err(|e| format!("zip {name}.nonce write: {e}"))?;
            zw.start_file(format!("{}.ct", name), opts)
                .map_err(|e| format!("zip {name}.ct: {e}"))?;
            zw.write_all(ct)
                .map_err(|e| format!("zip {name}.ct write: {e}"))?;
        }
        zw.finish().map_err(|e| format!("zip finish: {e}"))?;
    }

    // ── Persist + checksum ──────────────────────────────────────────────
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
        }
    }
    fs::write(output_path, &zip_bytes)
        .map_err(|e| format!("write {}: {}", output_path.display(), e))?;

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&zip_bytes);
    let sha256_hex = hex::encode(hasher.finalize());

    Ok(BackupResult {
        path: output_path.to_string_lossy().to_string(),
        size_bytes: zip_bytes.len() as u64,
        sha256_hex,
        item_count,
    })
}

/// Open a `.pcbackup` archive and decode the unencrypted meta + wrapped-key
/// blobs. Helper shared by `verify_backup` (no decryption) and
/// `import_backup` (full decrypt path).
fn open_backup_archive(
    input_path: &std::path::Path,
) -> Result<(BackupMeta, WrappedKeyBlob, zip::ZipArchive<std::io::Cursor<Vec<u8>>>), String> {
    let bytes = fs::read(input_path)
        .map_err(|e| format!("read {}: {}", input_path.display(), e))?;
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|_| {
        "Falsche Passphrase oder beschädigtes Backup".to_string()
    })?;

    let meta: BackupMeta = {
        let mut entry = zip
            .by_name(BACKUP_META_FILENAME)
            .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf)
            .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())?;
        serde_json::from_slice(&buf)
            .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())?
    };
    if meta.version != BACKUP_FORMAT_VERSION {
        return Err(format!(
            "Backup-Version {} wird nicht unterstützt (erwartet {})",
            meta.version, BACKUP_FORMAT_VERSION
        ));
    }

    let wrapped: WrappedKeyBlob = {
        let mut entry = zip
            .by_name(BACKUP_WRAPPED_KEY_FILENAME)
            .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf)
            .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())?;
        serde_json::from_slice(&buf)
            .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())?
    };
    if wrapped.version != BACKUP_FORMAT_VERSION || wrapped.kdf != "argon2id" {
        return Err("Falsche Passphrase oder beschädigtes Backup".into());
    }

    Ok((meta, wrapped, zip))
}

/// Argon2id-derive the KEK from `passphrase` + the wrapped blob's salt,
/// then unwrap the data-key. Returns the 32-byte data-key on success;
/// caller MUST `zeroize()` it after use.
fn unwrap_data_key(
    passphrase: &str,
    wrapped: &WrappedKeyBlob,
) -> Result<[u8; BACKUP_KEY_LEN], String> {
    let salt = B64
        .decode(&wrapped.salt)
        .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())?;
    let nonce = B64
        .decode(&wrapped.wrapped_data_key_nonce)
        .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())?;
    let ct = B64
        .decode(&wrapped.wrapped_data_key_ct)
        .map_err(|_| "Falsche Passphrase oder beschädigtes Backup".to_string())?;
    if salt.len() != BACKUP_SALT_LEN {
        return Err("Falsche Passphrase oder beschädigtes Backup".into());
    }
    let mut kek = derive_kek(passphrase, &salt)?;
    let data_key_vec = match aead_decrypt(&kek, &nonce, &ct) {
        Ok(v) => v,
        Err(e) => {
            kek.zeroize();
            return Err(e);
        }
    };
    kek.zeroize();
    if data_key_vec.len() != BACKUP_KEY_LEN {
        return Err("Falsche Passphrase oder beschädigtes Backup".into());
    }
    let mut out = [0u8; BACKUP_KEY_LEN];
    out.copy_from_slice(&data_key_vec);
    // `data_key_vec` is the in-vector copy; wipe it before drop.
    let mut dk = data_key_vec;
    dk.zeroize();
    Ok(out)
}

/// Tauri command: build a `.pcbackup` at `output_path`. The audit-log
/// entry intentionally omits the passphrase + key material — only path,
/// item count, and sha256 are recorded.
#[tauri::command]
async fn export_backup(
    app: AppHandle,
    output_path: String,
    passphrase: String,
) -> Result<BackupResult, String> {
    let data_dir = app_data(&app).map_err(|e| e.to_string())?;
    let payload = collect_backup_payload(&data_dir)?;
    let host_label = detect_host_label();
    let out_path = std::path::PathBuf::from(&output_path);

    // Move the heavy Argon2 derive + AEAD work to a blocking pool so the
    // UI thread doesn't hang for the full ~1s KDF on the IPC await.
    let result = tokio::task::spawn_blocking(move || {
        write_backup_archive(&out_path, payload, &passphrase, host_label)
    })
    .await
    .map_err(|e| format!("export task join: {e}"))??;

    audit(
        &app,
        "data",
        "backup_exported",
        serde_json::json!({
            "path": result.path,
            "size_bytes": result.size_bytes,
            "sha256": result.sha256_hex,
            "item_count": result.item_count,
        }),
    );
    Ok(result)
}

/// Tauri command: open a `.pcbackup`, validate ZIP integrity + meta schema
/// + KEK derivation. Does NOT decrypt the per-file payload — used by the
/// restore UI to surface "Erstellt am X von Host Y, Z Einträge" before
/// the user commits to overwriting their live state.
#[tauri::command]
async fn verify_backup(input_path: String, passphrase: String) -> Result<BackupMeta, String> {
    let path = std::path::PathBuf::from(input_path);
    tokio::task::spawn_blocking(move || {
        let (meta, wrapped, _zip) = open_backup_archive(&path)?;
        // Derive the KEK + unwrap the data-key as proof of passphrase.
        // We immediately wipe the unwrapped key — verify is a metadata
        // call only.
        let mut data_key = unwrap_data_key(&passphrase, &wrapped)?;
        data_key.zeroize();
        Ok(meta)
    })
    .await
    .map_err(|e| format!("verify task join: {e}"))?
}

/// Tauri command: decrypt every entry in `input_path` into a temp staging
/// dir under `app_data`, then atomically swap them onto the live paths.
/// Stops + restarts the relay listener around the swap so an in-flight
/// `messages.json` write can't race the rename.
#[tauri::command]
async fn import_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    input_path: String,
    passphrase: String,
) -> Result<RestoreResult, String> {
    let data_dir = app_data(&app).map_err(|e| e.to_string())?;
    let in_path = std::path::PathBuf::from(&input_path);

    // ── Phase 1: decrypt into a temp staging dir ────────────────────────
    let stage_dir = data_dir.join(format!(
        ".pcbackup-restore-{}",
        chrono::Utc::now().timestamp_millis()
    ));
    let stage_dir_clone = stage_dir.clone();
    let decrypt_result = tokio::task::spawn_blocking(move || -> Result<u32, String> {
        let (meta, wrapped, mut zip) = open_backup_archive(&in_path)?;
        if meta.version != BACKUP_FORMAT_VERSION {
            return Err(format!(
                "Backup-Version {} wird nicht unterstützt",
                meta.version
            ));
        }
        let mut data_key = unwrap_data_key(&passphrase, &wrapped)?;

        fs::create_dir_all(&stage_dir_clone)
            .map_err(|e| format!("mkdir {}: {}", stage_dir_clone.display(), e))?;

        // Walk the archive once, pairing `<name>.nonce` + `<name>.ct`
        // entries via a HashMap so we don't depend on ZIP entry order.
        let mut nonces: std::collections::HashMap<String, Vec<u8>> =
            std::collections::HashMap::new();
        let mut cts: std::collections::HashMap<String, Vec<u8>> =
            std::collections::HashMap::new();
        for i in 0..zip.len() {
            let mut entry = match zip.by_index(i) {
                Ok(e) => e,
                Err(e) => {
                    data_key.zeroize();
                    return Err(format!("zip entry {i}: {e}"));
                }
            };
            let name = entry.name().to_string();
            if name == BACKUP_META_FILENAME || name == BACKUP_WRAPPED_KEY_FILENAME {
                continue;
            }
            let mut buf = Vec::new();
            if let Err(e) = std::io::Read::read_to_end(&mut entry, &mut buf) {
                data_key.zeroize();
                return Err(format!("read entry {name}: {e}"));
            }
            if let Some(stem) = name.strip_suffix(".nonce") {
                nonces.insert(stem.to_string(), buf);
            } else if let Some(stem) = name.strip_suffix(".ct") {
                cts.insert(stem.to_string(), buf);
            } else {
                // Unknown entry — ignore (forward-compat for v2+ adds).
            }
        }

        let mut count: u32 = 0;
        for (stem, ct) in &cts {
            let nonce = match nonces.get(stem) {
                Some(n) => n,
                None => {
                    data_key.zeroize();
                    return Err(format!("Backup beschädigt: nonce für '{stem}' fehlt"));
                }
            };
            let plaintext = match aead_decrypt(&data_key, nonce, ct) {
                Ok(p) => p,
                Err(e) => {
                    data_key.zeroize();
                    return Err(e);
                }
            };
            // Reject any path traversal — backup must address files
            // *under* app_data only.
            if stem.contains("..") || stem.starts_with('/') || stem.contains('\\') {
                data_key.zeroize();
                return Err(format!("Backup beschädigt: ungültiger Pfad '{stem}'"));
            }
            let dest = stage_dir_clone.join(stem);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
            }
            fs::write(&dest, &plaintext)
                .map_err(|e| format!("write {}: {}", dest.display(), e))?;
            count += 1;
        }

        data_key.zeroize();
        Ok(count)
    })
    .await
    .map_err(|e| format!("import task join: {e}"))?;

    let items_restored = match decrypt_result {
        Ok(n) => n,
        Err(e) => {
            // Decryption failed — wipe the half-built stage dir before
            // returning so we don't leave plaintext on disk.
            let _ = fs::remove_dir_all(&stage_dir);
            return Err(e);
        }
    };

    // ── Phase 2: stop the listener for the swap ─────────────────────────
    // Take the prior subscriber handle out of the AppState slot, signal
    // shutdown, await with a 3s timeout (mirrors `restart_listener`).
    let prev = {
        let mut slot = state.subscriber.lock().await;
        slot.take()
    };
    if let Some(ListenerControl { mut handle, shutdown_tx }) = prev {
        let _ = shutdown_tx.send(());
        match tokio::time::timeout(std::time::Duration::from_secs(3), &mut handle).await {
            Ok(_) => {}
            Err(_) => {
                handle.abort();
            }
        }
    }
    // Drop the in-memory MLS bundle so the next `mls_init` rehydrates
    // from the freshly restored `mls_state/` files.
    {
        if let Ok(mut slot) = state.mls.lock() {
            *slot = None;
        }
    }

    // ── Phase 3: atomic swap ────────────────────────────────────────────
    // Walk the stage dir, copy each file onto its live path. We use
    // `fs::rename` first (atomic on the same filesystem) and fall back
    // to copy + remove_file if rename fails (cross-device).
    fn swap_into(src_root: &std::path::Path, dst_root: &std::path::Path) -> Result<(), String> {
        let entries = fs::read_dir(src_root)
            .map_err(|e| format!("read_dir {}: {}", src_root.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("read_dir entry: {e}"))?;
            let name = entry.file_name();
            let src = entry.path();
            let dst = dst_root.join(&name);
            if src.is_dir() {
                fs::create_dir_all(&dst)
                    .map_err(|e| format!("mkdir {}: {}", dst.display(), e))?;
                swap_into(&src, &dst)?;
                let _ = fs::remove_dir(&src);
            } else {
                // Best-effort atomic rename. `fs::rename` overwrites on
                // POSIX; on Windows it returns an error if `dst` exists,
                // so we remove first.
                let _ = fs::remove_file(&dst);
                if let Err(_e) = fs::rename(&src, &dst) {
                    fs::copy(&src, &dst)
                        .map_err(|e| format!("copy {} -> {}: {}", src.display(), dst.display(), e))?;
                    let _ = fs::remove_file(&src);
                }
            }
        }
        Ok(())
    }
    let swap_result = swap_into(&stage_dir, &data_dir);
    let _ = fs::remove_dir_all(&stage_dir);
    swap_result?;

    // ── Phase 4: restart the listener with the restored config ──────────
    // Mark started=true so a parallel `start_listener` becomes a no-op,
    // then spawn the task with the freshly-restored `relays.json` /
    // `privacy.json`.
    {
        if let Ok(mut started) = state.listener_started.lock() {
            *started = true;
        }
    }
    let _ = spawn_listener_task(app.clone(), &state, None).await;

    // Tell the React layer to drop and reload everything from disk.
    let _ = app.emit("app_data_replaced", ());

    audit(
        &app,
        "data",
        "backup_imported",
        serde_json::json!({
            "items_restored": items_restored,
        }),
    );

    Ok(RestoreResult {
        items_restored,
        identity_replaced: true,
        requires_restart: false,
    })
/// Frontend command: surfaced in Settings → "Erscheinungsbild" as
/// "Fenster zurücksetzen". Deletes `window_state.json` so the next
/// launch falls back to the default 1100×720 window centered on the
/// primary monitor — the rescue button for users who somehow ended up
/// with persisted geometry that crashes them off-screen.
///
/// No-op (and Ok) if the file doesn't exist; we don't want a missing
/// file to surface as an error toast in the UI.
#[tauri::command]
async fn reset_window_state(app: AppHandle) -> Result<(), String> {
    let path = window_state_path(&app).map_err(|e| e.to_string())?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("remove {}: {}", path.display(), e))?;
    }
    audit(&app, "display", "window_state_reset", serde_json::json!({}));
    Ok(())
}
