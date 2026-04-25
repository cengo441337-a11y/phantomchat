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
    sync::{Arc, Mutex as StdMutex},
};

use anyhow::{anyhow, Context};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use tauri_plugin_updater::UpdaterExt;
use phantomchat_core::{
    address::PhantomAddress,
    keys::{IdentityKey, PhantomSigningKey, SpendKey, ViewKey},
    mls::{self, GroupId, PhantomMlsGroup, PhantomMlsMember},
    privacy::{PrivacyConfig, PrivacyMode, ProxyConfig, ProxyKind},
    session::SessionStore,
};
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
    /// Per-message "pinned" affordance. Persisted on the row itself so a
    /// reload preserves user intent. `false` (default) is omitted from
    /// the on-disk JSON so legacy rows round-trip cleanly.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// Per-message "starred" / favourite affordance. Same persistence
    /// + back-compat strategy as `pinned`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub starred: bool,
}

fn default_direction() -> String {
    "incoming".to_string()
}

fn is_false(b: &bool) -> bool {
    !*b
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
    /// MIME guess from the file's extension (matches the value stored in
    /// the wire `FileManifest.mime`). Used by the React side to branch
    /// between inline-image rendering and the generic 📎 link. Optional +
    /// skip-if-none keeps legacy persisted file rows round-tripping
    /// cleanly without a migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
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

/// Load view + spend + signing keys from the on-disk keyfile.
///
/// Backwards-compat: if `signing_private` is absent (pre-attribution
/// keyfiles), a fresh [`PhantomSigningKey`] is generated and the keyfile
/// is rewritten in-place with the two new fields. The fourth tuple element
/// signals whether such an upgrade happened so the caller can emit a
/// one-time `status` event to the frontend.
fn load_identity(
    path: &std::path::Path,
) -> anyhow::Result<(ViewKey, SpendKey, PhantomSigningKey, bool)> {
    let raw = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut json: serde_json::Value = serde_json::from_slice(&raw)?;

    let view_bytes = B64.decode(
        json["view_private"]
            .as_str()
            .context("missing view_private")?,
    )?;
    let view_secret = StaticSecret::from(
        <[u8; 32]>::try_from(view_bytes.as_slice()).map_err(|_| anyhow!("bad view key"))?,
    );
    let view_key = ViewKey {
        public: PublicKey::from(&view_secret),
        secret: view_secret,
    };

    let spend_bytes = B64.decode(
        json["spend_private"]
            .as_str()
            .context("missing spend_private")?,
    )?;
    let spend_secret = StaticSecret::from(
        <[u8; 32]>::try_from(spend_bytes.as_slice()).map_err(|_| anyhow!("bad spend key"))?,
    );
    let spend_key = SpendKey {
        public: PublicKey::from(&spend_secret),
        secret: spend_secret,
    };

    let (signing, upgraded) = match json["signing_private"].as_str() {
        Some(s) => {
            let bytes = B64.decode(s)?;
            let arr: [u8; 32] = <[u8; 32]>::try_from(bytes.as_slice())
                .map_err(|_| anyhow!("bad signing key"))?;
            (PhantomSigningKey::from_bytes(arr), false)
        }
        None => {
            let sk = PhantomSigningKey::generate();
            // Persist so subsequent runs reuse the same identity for
            // attribution. Best-effort: a write failure is non-fatal — the
            // in-memory key still works for this session.
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "signing_private".to_string(),
                    serde_json::Value::String(B64.encode(sk.to_bytes())),
                );
                obj.insert(
                    "signing_public".to_string(),
                    serde_json::Value::String(hex::encode(sk.public_bytes())),
                );
                let _ = serde_json::to_vec_pretty(&json)
                    .ok()
                    .and_then(|b| fs::write(path, b).ok());
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

    // Field names + encodings match `cli/src/main.rs::cmd_keygen` exactly so
    // the same keyfile is interchangeable between CLI and Desktop.
    let json = serde_json::json!({
        "identity_private": B64.encode(id.private),
        "identity_public":  B64.encode(id.public),
        "view_private":     B64.encode(view.secret.to_bytes()),
        "view_public":      hex::encode(view.public.as_bytes()),
        "spend_private":    B64.encode(spend.secret.to_bytes()),
        "spend_public":     hex::encode(spend.public.as_bytes()),
        "signing_private":  B64.encode(signing.to_bytes()),
        "signing_public":   hex::encode(signing.public_bytes()),
    });

    fs::write(path, serde_json::to_vec_pretty(&json)?)
        .with_context(|| format!("writing {}", path.display()))?;

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

    let (_view, _spend, signing_key, _upgraded) = load_identity(&keys_path)?;

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
        load_identity(&keys_path).map_err(|e| e.to_string())?;
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
                        pinned: false,
                        starred: false,
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
    fs::write(&path, serde_json::to_vec_pretty(&json).map_err(|e| e.to_string())?)
        .map_err(|e| format!("write {}: {}", path.display(), e))?;
    audit(&app, "identity", "restored", serde_json::json!({}));
    Ok(())
}

/// Wipe the entire app-data directory (keys, contacts, sessions, history,
/// MLS directory, relays.json, onboarding marker) then exit. Caller is
/// expected to confirm the destructive intent before invoking.
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
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {}", dir.display(), e))?;
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
            // ── Wave 8G: pin / star / archive ───────────────────────
            pin_message,
            unpin_message,
            star_message,
            unstar_message,
            archive_conversation,
            unarchive_conversation,
            pin_conversation,
            unpin_conversation,
            get_conversation_state,
            list_pinned_messages,
            list_starred_messages,
            list_archived_conversations,
        ])
        .setup(|app| {
            // Pre-create the data dir on first launch so command handlers
            // never have to defensively `mkdir -p` on the hot path.
            let handle = app.handle().clone();
            let _ = app_data(&handle);

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
            if let Some(win) = app.get_webview_window("main") {
                let win_for_event = win.clone();
                win.on_window_event(move |e| {
                    if let WindowEvent::CloseRequested { api, .. } = e {
                        api.prevent_close();
                        let _ = win_for_event.hide();
                    }
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
    /// MIME guess from the file's extension, mirrored from the wire
    /// manifest. Empty for legacy callers — `serde` skips the field on
    /// emit when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
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
    /// MIME hint copied from the wire manifest. Lets the React side decide
    /// between inline-image render vs the generic 📎 affordance without
    /// having to re-derive it from the extension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
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
        mime: mime.clone(),
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
            mime: Some(mime),
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

    // 8. Emit the structured event for the React side. We pass the wire
    // manifest's MIME along so the renderer can branch on `image/*` vs
    // anything else without having to re-derive it from the extension on
    // the JS side.
    let event = FileReceivedEvent {
        from_label,
        filename: safe_name,
        size: manifest.size,
        saved_path: saved_path_str,
        sha256_ok,
        sha256_hex: manifest.sha256_hex,
        ts: chrono::Local::now().format("%H:%M:%S").to_string(),
        sender_pub_hex,
        mime: Some(manifest.mime),
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

// ── Wave 8G: Pin / Star (per-message) + Archive (per-conversation) ─────────
//
// Per-message pin/star bits live inline on `IncomingMessage`, persisted
// alongside the rest of the row in `messages.json`. Per-conversation state
// lives in a separate `conversation_state.json` map keyed by contact label
// so a conversation can be archived/pinned/muted without touching message
// history. Schema:
//
//   {
//     "alice": { "archived": false, "pinned": true,  "muted": false },
//     "bob":   { "archived": true,  "pinned": false, "muted": false }
//   }
//
// All mutations re-use `state.history_lock` so a concurrent `save_history`
// burst can't half-write the underlying files.

const CONVERSATION_STATE_FILE: &str = "conversation_state.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConversationState {
    #[serde(default, skip_serializing_if = "is_false")]
    pub archived: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
}

fn conversation_state_path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app_data(app)?.join(CONVERSATION_STATE_FILE))
}

/// Load `conversation_state.json` into an in-memory map. Lenient: a
/// corrupt or missing file resolves to an empty map so the UI can keep
/// rendering instead of bricking on a malformed JSON edit.
fn load_conversation_state(
    app: &AppHandle,
) -> std::collections::BTreeMap<String, ConversationState> {
    let path = match conversation_state_path(app) {
        Ok(p) => p,
        Err(_) => return Default::default(),
    };
    if !path.exists() {
        return Default::default();
    }
    let raw = match fs::read(&path) {
        Ok(b) => b,
        Err(_) => return Default::default(),
    };
    serde_json::from_slice(&raw).unwrap_or_default()
}

fn save_conversation_state(
    app: &AppHandle,
    map: &std::collections::BTreeMap<String, ConversationState>,
) -> Result<(), String> {
    let path = conversation_state_path(app).map_err(|e| e.to_string())?;
    let buf = serde_json::to_vec_pretty(map).map_err(|e| e.to_string())?;
    fs::write(&path, buf).map_err(|e| format!("write {}: {}", path.display(), e))
}

/// Per-message-state-change event payload. Mirrors TS
/// `MessageStateChangedEvent`. Fired by `pin_message` / `unpin_message` /
/// `star_message` / `unstar_message` so the React reducer can update the
/// in-memory `messages` array without reloading the whole history.
#[derive(Clone, Debug, Serialize)]
pub struct MessageStateChangedEvent {
    pub msg_id: String,
    pub pinned: bool,
    pub starred: bool,
}

/// Per-conversation-state-change event payload. Mirrors TS
/// `ConversationStateChangedEvent`.
#[derive(Clone, Debug, Serialize)]
pub struct ConversationStateChangedEvent {
    pub contact_label: String,
    pub state: ConversationState,
}

/// Mutate the persisted message row matching `msg_id` via `mutator`,
/// re-write `messages.json`, and emit the matching `message_state_changed`
/// event. Returns `Err` if the row isn't found so the React side can show
/// a clear "row no longer exists" message rather than failing silently.
fn mutate_message_state<F>(
    app: &AppHandle,
    state: &AppState,
    msg_id: &str,
    mutator: F,
) -> Result<(), String>
where
    F: Fn(&mut IncomingMessage),
{
    let _guard = state.history_lock.lock().map_err(|e| e.to_string())?;
    let path = messages_path(app).map_err(|e| e.to_string())?;
    let mut history: Vec<IncomingMessage> = if path.exists() {
        let raw = fs::read(&path).map_err(|e| format!("read {}: {}", path.display(), e))?;
        serde_json::from_slice(&raw).unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut hit = None;
    for m in history.iter_mut() {
        if m.msg_id.as_deref() == Some(msg_id) {
            mutator(m);
            hit = Some((m.pinned, m.starred));
            break;
        }
    }
    let (pinned, starred) =
        hit.ok_or_else(|| format!("msg_id '{msg_id}' not found in history"))?;
    let buf = serde_json::to_vec_pretty(&history).map_err(|e| e.to_string())?;
    fs::write(&path, buf).map_err(|e| format!("write {}: {}", path.display(), e))?;
    drop(_guard);
    let _ = app.emit(
        "message_state_changed",
        MessageStateChangedEvent {
            msg_id: msg_id.to_string(),
            pinned,
            starred,
        },
    );
    Ok(())
}

/// Mutate `conversation_state.json` for the named contact via `mutator`,
/// persist, and emit `conversation_state_changed`. Inserts a default
/// `ConversationState` if the contact has no entry yet so a first-time
/// archive/pin/mute lands cleanly.
fn mutate_conversation_state<F>(
    app: &AppHandle,
    state: &AppState,
    contact_label: &str,
    mutator: F,
) -> Result<(), String>
where
    F: Fn(&mut ConversationState),
{
    let _guard = state.history_lock.lock().map_err(|e| e.to_string())?;
    let mut map = load_conversation_state(app);
    let entry = map.entry(contact_label.to_string()).or_default();
    mutator(entry);
    let snapshot = entry.clone();
    save_conversation_state(app, &map)?;
    drop(_guard);
    let _ = app.emit(
        "conversation_state_changed",
        ConversationStateChangedEvent {
            contact_label: contact_label.to_string(),
            state: snapshot,
        },
    );
    Ok(())
}

#[tauri::command]
async fn pin_message(
    app: AppHandle,
    state: State<'_, AppState>,
    msg_id: String,
) -> Result<(), String> {
    mutate_message_state(&app, &state, &msg_id, |m| m.pinned = true)
}

#[tauri::command]
async fn unpin_message(
    app: AppHandle,
    state: State<'_, AppState>,
    msg_id: String,
) -> Result<(), String> {
    mutate_message_state(&app, &state, &msg_id, |m| m.pinned = false)
}

#[tauri::command]
async fn star_message(
    app: AppHandle,
    state: State<'_, AppState>,
    msg_id: String,
) -> Result<(), String> {
    mutate_message_state(&app, &state, &msg_id, |m| m.starred = true)
}

#[tauri::command]
async fn unstar_message(
    app: AppHandle,
    state: State<'_, AppState>,
    msg_id: String,
) -> Result<(), String> {
    mutate_message_state(&app, &state, &msg_id, |m| m.starred = false)
}

#[tauri::command]
async fn archive_conversation(
    app: AppHandle,
    state: State<'_, AppState>,
    contact_label: String,
) -> Result<(), String> {
    mutate_conversation_state(&app, &state, &contact_label, |s| s.archived = true)
}

#[tauri::command]
async fn unarchive_conversation(
    app: AppHandle,
    state: State<'_, AppState>,
    contact_label: String,
) -> Result<(), String> {
    mutate_conversation_state(&app, &state, &contact_label, |s| s.archived = false)
}

#[tauri::command]
async fn pin_conversation(
    app: AppHandle,
    state: State<'_, AppState>,
    contact_label: String,
) -> Result<(), String> {
    mutate_conversation_state(&app, &state, &contact_label, |s| s.pinned = true)
}

#[tauri::command]
async fn unpin_conversation(
    app: AppHandle,
    state: State<'_, AppState>,
    contact_label: String,
) -> Result<(), String> {
    mutate_conversation_state(&app, &state, &contact_label, |s| s.pinned = false)
}

/// Snapshot of the entire `conversation_state.json` map. The frontend
/// hydrates the `Map<label, ConversationState>` from this on cold start
/// and then patches in-place via `conversation_state_changed` events.
#[tauri::command]
fn get_conversation_state(
    app: AppHandle,
) -> Result<std::collections::BTreeMap<String, ConversationState>, String> {
    Ok(load_conversation_state(&app))
}

/// Return all pinned messages from the persisted history. Optional
/// `contact_label` restricts to a single conversation — used by the
/// MessageStream header's "📌 Pinned (N)" drawer. Order: oldest-first
/// (history is already chronological), so pins read top-to-bottom in the
/// drawer and the click-to-jump scroll lands on the right row.
#[tauri::command]
async fn list_pinned_messages(
    app: AppHandle,
    contact_label: Option<String>,
) -> Result<Vec<IncomingMessage>, String> {
    let history = load_history(app)?;
    Ok(history
        .into_iter()
        .filter(|m| m.pinned)
        .filter(|m| {
            contact_label
                .as_deref()
                .map(|lbl| m.sender_label == lbl)
                .unwrap_or(true)
        })
        .collect())
}

/// All starred messages across every conversation — backs the global
/// "⭐ Starred" drawer.
#[tauri::command]
async fn list_starred_messages(app: AppHandle) -> Result<Vec<IncomingMessage>, String> {
    let history = load_history(app)?;
    Ok(history.into_iter().filter(|m| m.starred).collect())
}

/// All currently-archived contact labels. Used by the Settings panel's
/// "Archiv" section + by ContactsPane to split archived contacts into
/// their own collapsible group.
#[tauri::command]
async fn list_archived_conversations(app: AppHandle) -> Result<Vec<String>, String> {
    let map = load_conversation_state(&app);
    Ok(map
        .into_iter()
        .filter(|(_, s)| s.archived)
        .map(|(k, _)| k)
        .collect())
}
