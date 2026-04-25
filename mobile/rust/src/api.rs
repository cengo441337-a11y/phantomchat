//! flutter_rust_bridge API surface for the PhantomChat mobile app.
//!
//! This wrapper crate re-implements the slim subset of the desktop
//! `core/src/api.rs` surface that the existing Flutter screens actually
//! call AND adds the v3 entry points needed for prefix-dispatch interop
//! with v3.0.0 Desktop:
//!
//! - v2 (carried over so existing screens compile):
//!   `generate_phantom_id`, `load_local_identity`, `send_secure_message`,
//!   `scan_incoming_envelope`, `join_group`, `send_group_message`,
//!   `update_avatar_cid`, `set_privacy_mode`, `get_privacy_mode`,
//!   `init_secure_storage` (stub — secure storage gated to ffi feature
//!   in core, never hit on mobile), `perform_panic_wipe` (stub).
//!
//! - v3 (new):
//!   - `send_sealed_v3` / `receive_full_v3` — sealed-sender 1:1 with
//!     attribution surfaced as [`ReceivedFullV3`].
//!   - `mls_init` / `mls_publish_key_package` / `mls_create_group` /
//!     `mls_add_member` / `mls_join_via_welcome` / `mls_directory_insert` /
//!     `mls_encrypt` / `mls_decrypt` / `mls_member_count` / `mls_in_group`
//!     / `mls_self_label` / `mls_self_signing_pub_hex` / `mls_list_members`
//!     / `mls_directory` — RFC 9420 group-chat ops (single-group MVP).
//!
//! The wrapper does NOT pull `phantomchat_core`'s `ffi-mobile` feature —
//! doing so would link a second copy of `flutter_rust_bridge` runtime
//! symbols into the cdylib (`frb_dart_fn_deliver_output` collision). The
//! `start_network_node` libp2p path is intentionally omitted from the
//! mobile build for the same reason; mobile transport is layered on top
//! of the relays crate (Wave 7B-followup) rather than direct libp2p.

use flutter_rust_bridge::frb;
use once_cell::sync::OnceCell;
#[cfg(feature = "mls")]
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use phantomchat_core::address::PhantomAddress;
use phantomchat_core::envelope::Envelope;
use phantomchat_core::keys::{PhantomSigningKey, SpendKey, ViewKey};
use phantomchat_core::session::SessionStore;
use rand_core::{OsRng, RngCore};
use x25519_dalek::{PublicKey, StaticSecret};

#[cfg(feature = "mls")]
use openmls::group::GroupId;
#[cfg(feature = "mls")]
use phantomchat_core::mls::{self as core_mls, PhantomMlsGroup, PhantomMlsMember};

extern crate rand_core;

// ── Process-wide state (mirrors core::api's OnceLock pattern) ───────────────

/// Loaded local view keypair. Used by both the legacy
/// `scan_incoming_envelope` path and the new `receive_full_v3` path so a
/// single `load_local_identity[_v3]` call seeds both.
static LOCAL_VIEW: OnceCell<RwLock<Option<ViewKey>>> = OnceCell::new();
static LOCAL_SPEND: OnceCell<RwLock<Option<SpendKey>>> = OnceCell::new();
/// `Some` iff `load_local_identity_v3` was called with a third (signing)
/// hex argument. `send_sealed_v3` errors out clearly when this slot is
/// empty so the caller knows to upgrade their identity load path.
static LOCAL_SIGN: OnceCell<RwLock<Option<PhantomSigningKey>>> = OnceCell::new();
/// Per-peer Double-Ratchet sessions, shared across the v2 and v3 receive
/// paths so a session opened by one survives use by the other.
static SESSIONS: OnceCell<Mutex<SessionStore>> = OnceCell::new();

fn sessions() -> &'static Mutex<SessionStore> {
    SESSIONS.get_or_init(|| Mutex::new(SessionStore::new()))
}

// ── v2 entry points (called by the existing Flutter screens) ────────────────

#[frb(sync)]
pub fn generate_phantom_id() -> String {
    let mut id_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut id_bytes);
    format!("PH-{}", base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, id_bytes))
}

/// Stub — mobile builds do not link rusqlite/SQLCipher (no OpenSSL on
/// plain Android NDK). Identity material lives in `flutter_secure_storage`
/// on the Dart side. Returns a clear error so a caller that still wires
/// it gets a debuggable message instead of a silent no-op.
pub async fn init_secure_storage(_db_path: String, _password: String) -> String {
    "ERROR: secure storage not available on mobile (no SQLCipher in this build); \
     identity is held in flutter_secure_storage instead"
        .into()
}

/// Stub — see [`init_secure_storage`].
pub async fn perform_panic_wipe(_db_path: String) {}

/// Bind the local view + spend keys for the receive pipeline. Hex-only
/// signature kept so the existing Dart `loadLocalIdentity({viewSecretHex,
/// spendSecretHex})` call site keeps working unchanged.
pub fn load_local_identity(view_secret_hex: String, spend_secret_hex: String) -> String {
    let parse = |s: &str| -> Option<StaticSecret> {
        let bytes: [u8; 32] = hex::decode(s).ok()?.try_into().ok()?;
        Some(StaticSecret::from(bytes))
    };
    let v_secret = match parse(&view_secret_hex) {
        Some(s) => s,
        None => return "ERROR: view_secret parse".into(),
    };
    let s_secret = match parse(&spend_secret_hex) {
        Some(s) => s,
        None => return "ERROR: spend_secret parse".into(),
    };
    let view = ViewKey { public: PublicKey::from(&v_secret), secret: v_secret };
    let spend = SpendKey { public: PublicKey::from(&s_secret), secret: s_secret };
    let _ = LOCAL_VIEW.get_or_init(|| RwLock::new(None)).write().map(|mut g| *g = Some(view));
    let _ = LOCAL_SPEND.get_or_init(|| RwLock::new(None)).write().map(|mut g| *g = Some(spend));
    "OK: identity loaded".into()
}

/// 1:1 send (v2-compatible). Returns the wire bytes of the freshly-sealed
/// envelope so the Flutter side can hand them to its transport (relays
/// websocket / future libp2p direct). The middle `_local_phantom_id`
/// argument is preserved for backwards-compat with the existing Dart call
/// sites and is unused internally.
pub async fn send_secure_message(
    recipient_address: String,
    _local_phantom_id: String,
    plaintext: String,
) -> String {
    let recipient = match PhantomAddress::parse(&recipient_address) {
        Some(a) => a,
        None => return "ERROR: invalid recipient address".into(),
    };
    let envelope = {
        let mut guard = match sessions().lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.send(&recipient, plaintext.as_bytes(), 16)
    };
    let wire = envelope.to_bytes();
    // Caller (Dart RelayService) is responsible for shipping these bytes.
    format!("OK:wire_b64:{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, wire))
}

/// Try to decrypt an inbound envelope using the loaded local identity.
/// Returns the plaintext bytes on a hit, `None` on someone-else's
/// envelope (the silent-drop path).
pub fn scan_incoming_envelope(wire_bytes: Vec<u8>) -> Option<Vec<u8>> {
    let envelope = Envelope::from_bytes(&wire_bytes)?;
    let view_guard = LOCAL_VIEW.get()?.read().ok()?;
    let spend_guard = LOCAL_SPEND.get()?.read().ok()?;
    let view = view_guard.as_ref()?;
    let spend = spend_guard.as_ref()?;
    let mut guard = sessions().lock().ok()?;
    guard.receive(&envelope, view, spend).ok().flatten()
}

/// No-op stubs — group transport is the relays crate's job, and the mobile
/// build doesn't pull libp2p. Kept as cheap async returns so the existing
/// `joinGroup` / `sendGroupMessage` Dart call sites stay shape-compatible.
pub async fn join_group(_group_id: String) {}
pub async fn send_group_message(_group_id: String, _message: String) {}
pub async fn update_avatar_cid(_cid: String) {}

#[frb(sync)]
pub fn set_privacy_mode(_mode_str: String, _proxy_addr: Option<String>, _use_nym: bool) -> String {
    // Privacy modes ride on the libp2p stack we've intentionally left out
    // of the mobile build. Stub returns OK so the existing settings panel
    // doesn't render an error every time it boots.
    "OK: privacy mode (mobile-stub) accepted".into()
}

#[frb(sync)]
pub fn get_privacy_mode() -> String {
    "DailyUse".into()
}

// ── v3 sealed-sender + receive-full ─────────────────────────────────────────

/// Outcome of [`receive_full_v3`] — plaintext + sealed-sender attribution
/// flattened into a wire-friendly struct the FRB layer can mirror in Dart.
#[derive(Clone, Debug)]
pub struct ReceivedFullV3 {
    /// Decrypted plaintext bytes.
    pub plaintext: Vec<u8>,
    /// Hex-encoded sender Ed25519 verifying key, when the envelope carried
    /// a Sealed-Sender attribution. `None` for unauthenticated envelopes.
    pub sender_pub_hex: Option<String>,
    /// `true` when the attached signature verified against the wire bytes.
    /// Vacuously `true` for the no-attribution case so the Dart side can
    /// branch on `(sender_pub_hex.is_some() && !sig_ok)` to flag tampered
    /// envelopes (== Desktop's `INBOX!` rendering).
    pub sig_ok: bool,
}

/// Sealed-sender flavour of [`load_local_identity`]. Call this once at
/// startup with the hex encodings of the view, spend, AND signing
/// secrets so [`send_sealed_v3`] has the signing seed available.
pub fn load_local_identity_v3(
    view_secret_hex: String,
    spend_secret_hex: String,
    signing_secret_hex: String,
) -> String {
    let _ = load_local_identity(view_secret_hex, spend_secret_hex);
    let parse32 = |s: &str| -> Option<[u8; 32]> {
        let bytes = hex::decode(s).ok()?;
        bytes.try_into().ok()
    };
    let sg_bytes = match parse32(&signing_secret_hex) {
        Some(b) => b,
        None => return "ERROR: signing_secret parse".into(),
    };
    let signer = PhantomSigningKey::from_bytes(sg_bytes);
    let _ = LOCAL_SIGN
        .get_or_init(|| RwLock::new(None))
        .write()
        .map(|mut g| *g = Some(signer));
    "OK: v3 identity loaded".into()
}

/// Sealed-sender variant of [`send_secure_message`]. Returns the wire bytes
/// directly (no base64 wrap) since the v3 caller knows it's the sealed
/// path and ships the bytes itself.
pub fn send_sealed_v3(
    recipient_address: String,
    plaintext: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let recipient = PhantomAddress::parse(&recipient_address)
        .ok_or_else(|| "invalid recipient address".to_string())?;
    let sign_guard = LOCAL_SIGN
        .get()
        .ok_or_else(|| "signing key not loaded — call load_local_identity_v3 first".to_string())?
        .read()
        .map_err(|e| format!("signing key lock: {e}"))?;
    let signer = sign_guard
        .as_ref()
        .ok_or_else(|| "signing key slot empty — call load_local_identity_v3 first".to_string())?;
    let envelope = {
        let mut guard = sessions().lock().map_err(|e| format!("session lock: {e}"))?;
        guard.send_sealed(&recipient, &plaintext, signer, 16)
    };
    Ok(envelope.to_bytes())
}

/// v3 receive that surfaces sealed-sender attribution. `Ok(None)` for
/// envelopes that don't pass our view-tag check (silent-drop path on a
/// public relay). Errors on a malformed wire blob or AEAD failure on
/// a tag-passing envelope.
pub fn receive_full_v3(wire_bytes: Vec<u8>) -> Result<Option<ReceivedFullV3>, String> {
    let envelope = Envelope::from_bytes(&wire_bytes)
        .ok_or_else(|| "envelope deserialise failed".to_string())?;
    let v_guard = LOCAL_VIEW
        .get()
        .ok_or_else(|| "view key not loaded".to_string())?
        .read()
        .map_err(|e| format!("view lock: {e}"))?;
    let s_guard = LOCAL_SPEND
        .get()
        .ok_or_else(|| "spend key not loaded".to_string())?
        .read()
        .map_err(|e| format!("spend lock: {e}"))?;
    let view = v_guard.as_ref().ok_or("view slot empty")?;
    let spend = s_guard.as_ref().ok_or("spend slot empty")?;
    let mut sessions = sessions().lock().map_err(|e| format!("session lock: {e}"))?;
    let outcome = sessions
        .receive_full(&envelope, view, spend, None)
        .map_err(|e| format!("receive_full: {e}"))?;
    Ok(outcome.map(|m| ReceivedFullV3 {
        plaintext: m.plaintext,
        sender_pub_hex: m
            .sender
            .as_ref()
            .map(|(attr, _)| hex::encode(attr.sender_pub)),
        sig_ok: m.sender.as_ref().map(|(_, ok)| *ok).unwrap_or(true),
    }))
}

// ── MLS bundle (RFC 9420) ──────────────────────────────────────────────────

#[cfg(feature = "mls")]
pub struct MlsBundle {
    member: PhantomMlsMember,
    /// `Some(id_bytes)` while in a group. Mirrors Desktop's MlsBundle.
    group_id: Option<Vec<u8>>,
    /// Refreshed after every successful op so `mls_member_count` doesn't
    /// have to reload + walk the tree on every call.
    member_count: u32,
    identity_label: String,
    /// Auto-transport directory: per-MLS-member pointers (label / address /
    /// signing-pub-hex). Same shape as Desktop's so welcome-v2 metadata
    /// round-trips byte-identically.
    member_addresses: Vec<MlsMemberRefV3>,
}

#[cfg(feature = "mls")]
#[derive(Clone, Debug)]
pub struct MlsMemberRefV3 {
    pub label: String,
    pub address: String,
    pub signing_pub_hex: String,
}

#[cfg(feature = "mls")]
#[derive(Clone, Debug)]
pub struct MlsMemberInfoV3 {
    pub credential_label: String,
    pub signature_pub_hex: String,
    pub is_self: bool,
    pub mapped_contact_label: Option<String>,
}

#[cfg(feature = "mls")]
static MLS_BUNDLE: OnceCell<Mutex<Option<MlsBundle>>> = OnceCell::new();

#[cfg(feature = "mls")]
fn mls_slot() -> &'static Mutex<Option<MlsBundle>> {
    MLS_BUNDLE.get_or_init(|| Mutex::new(None))
}

/// Initialise the MLS bundle backed by `storage_dir/mls_state.bin`. Re-runs
/// are cheap — if a bundle already exists in the slot, we leave it alone
/// and return its current state.
#[cfg(feature = "mls")]
pub fn mls_init(identity_label: String, storage_dir: String) -> Result<String, String> {
    let mut slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    if slot.is_some() {
        return Ok("OK: bundle already initialised".into());
    }
    let dir = PathBuf::from(&storage_dir);
    let member = PhantomMlsMember::new_with_storage_dir(identity_label.as_bytes().to_vec(), &dir)
        .map_err(|e| format!("PhantomMlsMember::new_with_storage_dir: {e}"))?;
    let group_id = member.active_group_id().map(|s| s.to_vec());
    let mut bundle = MlsBundle {
        member,
        group_id,
        member_count: 0,
        identity_label,
        member_addresses: Vec::new(),
    };
    if bundle.group_id.is_some() {
        if mls_with_group(&mut bundle, |_g| Ok::<(), String>(())).is_err() {
            bundle.group_id = None;
            let _ = bundle.member.clear_active_group_id();
        }
    }
    *slot = Some(bundle);
    Ok("OK: mls initialised".into())
}

#[cfg(feature = "mls")]
fn mls_with_group<R>(
    bundle: &mut MlsBundle,
    op: impl FnOnce(&mut PhantomMlsGroup<'_>) -> Result<R, String>,
) -> Result<R, String> {
    let id_bytes = bundle
        .group_id
        .as_ref()
        .ok_or_else(|| "not in a group — call mls_create_group / mls_join_via_welcome first".to_string())?
        .clone();
    let gid = GroupId::from_slice(&id_bytes);
    let group =
        core_mls::load_group(&bundle.member, &gid).map_err(|e| format!("load_group: {e}"))?;
    let mut wrapper = PhantomMlsGroup::from_parts(&mut bundle.member, group);
    let out = op(&mut wrapper)?;
    bundle.member_count = wrapper.member_count() as u32;
    Ok(out)
}

#[cfg(feature = "mls")]
pub fn mls_publish_key_package() -> Result<Vec<u8>, String> {
    let slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    let bundle = slot.as_ref().ok_or("call mls_init first")?;
    bundle
        .member
        .publish_key_package()
        .map_err(|e| format!("publish_key_package: {e}"))
}

#[cfg(feature = "mls")]
pub fn mls_create_group() -> Result<u32, String> {
    let mut slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    let bundle = slot.as_mut().ok_or("call mls_init first")?;
    if bundle.group_id.is_some() {
        return Err("already in a group (single-group MVP)".into());
    }
    let (id_bytes, count) = {
        let wrapper = bundle
            .member
            .create_group()
            .map_err(|e| format!("create_group: {e}"))?;
        (wrapper.group_id_bytes(), wrapper.member_count() as u32)
    };
    bundle.group_id = Some(id_bytes);
    bundle.member_count = count;
    Ok(count)
}

/// Add a peer's KeyPackage. Returns `(commit_bytes, welcome_bytes)` —
/// caller wraps the welcome in a `MLS-WLC2` envelope and ships it via
/// the existing sealed-sender pipe.
#[cfg(feature = "mls")]
pub fn mls_add_member(
    key_package_bytes: Vec<u8>,
    new_member_label: String,
    new_member_address: String,
    new_member_signing_pub_hex: String,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    let bundle = slot.as_mut().ok_or("call mls_init first")?;
    let pair = mls_with_group(bundle, |g| {
        g.add_member(&key_package_bytes)
            .map_err(|e| format!("add_member: {e}"))
    })?;
    // Insert the new member into the directory so subsequent app messages
    // resolve to a human label instead of `?<8hex>`.
    let pub_hex = new_member_signing_pub_hex.to_lowercase();
    if !bundle
        .member_addresses
        .iter()
        .any(|m| m.signing_pub_hex.eq_ignore_ascii_case(&pub_hex))
    {
        bundle.member_addresses.push(MlsMemberRefV3 {
            label: new_member_label,
            address: new_member_address,
            signing_pub_hex: pub_hex,
        });
    }
    Ok(pair)
}

/// Process an incoming Welcome. Caller MUST have already stripped the
/// `MLS-WLC2` prefix + ULEB128(meta_len) + meta_json header; what arrives
/// here is the raw openmls Welcome bytes.
#[cfg(feature = "mls")]
pub fn mls_join_via_welcome(welcome_bytes: Vec<u8>) -> Result<u32, String> {
    let mut slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    let bundle = slot.as_mut().ok_or("call mls_init first")?;
    if bundle.group_id.is_some() {
        return Err("already in a group (single-group MVP)".into());
    }
    let (id_bytes, count) = {
        let wrapper = bundle
            .member
            .join_via_welcome(&welcome_bytes)
            .map_err(|e| format!("join_via_welcome: {e}"))?;
        (wrapper.group_id_bytes(), wrapper.member_count() as u32)
    };
    bundle.group_id = Some(id_bytes);
    bundle.member_count = count;
    Ok(count)
}

/// Push an inviter into the directory before processing their Welcome.
/// Mirrors `handle_incoming_mls_welcome_v2` so the very first incoming app
/// message resolves to the human label instead of `?<8hex>`.
#[cfg(feature = "mls")]
pub fn mls_directory_insert(
    label: String,
    address: String,
    signing_pub_hex: String,
) -> Result<(), String> {
    let mut slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    let bundle = slot.as_mut().ok_or("call mls_init first")?;
    let pub_hex = signing_pub_hex.to_lowercase();
    if !bundle
        .member_addresses
        .iter()
        .any(|m| m.signing_pub_hex.eq_ignore_ascii_case(&pub_hex))
    {
        bundle.member_addresses.push(MlsMemberRefV3 {
            label,
            address,
            signing_pub_hex: pub_hex,
        });
    }
    Ok(())
}

#[cfg(feature = "mls")]
pub fn mls_encrypt(plaintext: Vec<u8>) -> Result<Vec<u8>, String> {
    let mut slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    let bundle = slot.as_mut().ok_or("call mls_init first")?;
    mls_with_group(bundle, |g| {
        g.encrypt(&plaintext).map_err(|e| format!("encrypt: {e}"))
    })
}

/// Decrypt a `MLS-APP1` payload (already stripped of the 8-byte prefix).
/// `Ok(None)` for control messages that advanced the epoch but carried
/// no application data.
#[cfg(feature = "mls")]
pub fn mls_decrypt(wire_bytes: Vec<u8>) -> Result<Option<Vec<u8>>, String> {
    let mut slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    let bundle = slot.as_mut().ok_or("call mls_init first")?;
    mls_with_group(bundle, |g| {
        g.decrypt(&wire_bytes).map_err(|e| format!("decrypt: {e}"))
    })
}

#[cfg(feature = "mls")]
#[frb(sync)]
pub fn mls_member_count() -> u32 {
    mls_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|b| b.member_count))
        .unwrap_or(0)
}

#[cfg(feature = "mls")]
#[frb(sync)]
pub fn mls_in_group() -> bool {
    mls_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|b| b.group_id.is_some()))
        .unwrap_or(false)
}

#[cfg(feature = "mls")]
#[frb(sync)]
pub fn mls_self_label() -> Option<String> {
    mls_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|b| b.identity_label.clone()))
}

/// Hex-encoded signing public key of the local MLS member. Used by the
/// inviter side to populate `inviter_signing_pub_hex` in the V2 Welcome
/// metadata header.
#[cfg(feature = "mls")]
pub fn mls_self_signing_pub_hex() -> Result<String, String> {
    let slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    let bundle = slot.as_ref().ok_or("call mls_init first")?;
    Ok(hex::encode(bundle.member.signer().to_public_vec()))
}

/// Walk the MLS group tree and emit one `MlsMemberInfoV3` per leaf.
/// Cross-references `member_addresses` to surface the human contact label
/// associated with each leaf's signing pubkey (for the "alice (you)" /
/// "bob → contact" UI rendering).
#[cfg(feature = "mls")]
pub fn mls_list_members() -> Result<Vec<MlsMemberInfoV3>, String> {
    let mut slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    let bundle = slot.as_mut().ok_or("call mls_init first")?;
    let self_pub_hex = hex::encode(bundle.member.signer().to_public_vec());
    let directory = bundle.member_addresses.clone();
    let id_bytes = bundle
        .group_id
        .as_ref()
        .ok_or_else(|| "not in a group".to_string())?
        .clone();
    let gid = GroupId::from_slice(&id_bytes);
    let group =
        core_mls::load_group(&bundle.member, &gid).map_err(|e| format!("load_group: {e}"))?;
    let rows: Vec<MlsMemberInfoV3> = group
        .members()
        .map(|m| {
            let sig_hex = hex::encode(&m.signature_key);
            let cred_label = std::str::from_utf8(m.credential.serialized_content())
                .map(|s| s.to_string())
                .unwrap_or_else(|_| format!("?{}", &sig_hex[..8.min(sig_hex.len())]));
            let is_self = sig_hex.eq_ignore_ascii_case(&self_pub_hex);
            let mapped = directory
                .iter()
                .find(|d| d.signing_pub_hex.eq_ignore_ascii_case(&sig_hex))
                .map(|d| d.label.clone());
            MlsMemberInfoV3 {
                credential_label: cred_label,
                signature_pub_hex: sig_hex,
                is_self,
                mapped_contact_label: mapped,
            }
        })
        .collect();
    Ok(rows)
}

/// Snapshot of the MLS auto-transport directory.
#[cfg(feature = "mls")]
pub fn mls_directory() -> Result<Vec<MlsMemberRefV3>, String> {
    let slot = mls_slot().lock().map_err(|e| format!("mls lock: {e}"))?;
    let bundle = slot.as_ref().ok_or("call mls_init first")?;
    Ok(bundle.member_addresses.clone())
}
