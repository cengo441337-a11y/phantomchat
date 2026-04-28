//! OS-level secure key-storage abstraction.
//!
//! Replaces "plaintext base64 in `keys.json`" with one of the platform-native
//! credential vaults:
//!
//! | OS         | Backend                                | `name()`              |
//! | ---------- | -------------------------------------- | --------------------- |
//! | Windows    | DPAPI via `keyring` v3.6                | `"dpapi"`             |
//! | macOS      | Keychain via `keyring` v3.6             | `"keychain"`          |
//! | Linux      | libsecret / D-Bus via `keyring` v3.6    | `"libsecret"`         |
//! | Android    | (deferred — falls back to plaintext)    | `"fallback-plaintext"`|
//! | wasm/other | In-process plaintext map                | `"fallback-plaintext"`|
//!
//! The `keyring` crate is the primary implementation everywhere a real
//! secure store exists — battle-tested cross-platform abstraction over
//! DPAPI / Keychain / libsecret with no `windows-rs` / `Foundation`
//! transitive bloat. Hand-rolling a separate DPAPI binding via
//! `windows-sys` was considered and rejected: the same `Entry` API works
//! on all three desktop OSes, the maintenance surface is tiny, and a
//! per-OS detect path would only have value if we needed direct access
//! to a feature `keyring` does not expose (we don't).
//!
//! The Android backend (Java `KeyStore` "AndroidKeyStore" provider) is
//! intentionally deferred — wiring it requires JNI access to the Android
//! `Context`, which the Flutter bridge does not currently surface to Rust.
//! Until that lands, Android falls back to the in-process plaintext store
//! and an audit-log warning is emitted at app start.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Errors surfaced by [`SecureStorage`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum SecureStorageError {
    /// The backend reported "no entry under that key_id". Mostly returned by
    /// `load` / `delete` after a fresh install.
    #[error("no entry for key_id '{0}'")]
    NotFound(String),
    /// Anything the backend itself raised (DPAPI handle failure, Keychain
    /// access denial by the user, libsecret D-Bus error, …). The string is
    /// the backend's verbatim message, prefixed with the backend name so
    /// upstream telemetry can differentiate.
    #[error("backend error: {0}")]
    Backend(String),
}

/// Trait every platform-specific store implements. Keys are addressed by an
/// opaque ASCII `key_id` (the desktop chooses
/// `"phantomchat:" + sha256(file_path)[..16]`).
///
/// Implementations are `Send + Sync` so the desktop can hand a single
/// `Box<dyn SecureStorage>` to the Tauri command runtime.
pub trait SecureStorage: Send + Sync {
    /// Persist `secret` under `key_id`. Overwrites any existing entry.
    fn store(&self, key_id: &str, secret: &[u8]) -> Result<(), SecureStorageError>;
    /// Load the bytes previously stored under `key_id`, or
    /// [`SecureStorageError::NotFound`] if no such entry exists.
    fn load(&self, key_id: &str) -> Result<Vec<u8>, SecureStorageError>;
    /// Remove the entry. A subsequent `load` for the same `key_id` must
    /// return `NotFound`. Idempotent — deleting a missing key returns Ok.
    fn delete(&self, key_id: &str) -> Result<(), SecureStorageError>;
    /// Stable identifier for telemetry / audit logs:
    /// `"dpapi" | "keychain" | "libsecret" | "android-keystore" | "fallback-plaintext"`.
    fn name(&self) -> &'static str;
}

/// Pick the strongest backend available on this host.
///
/// On desktop OSes this is always the platform vault (`keyring`-backed).
/// On Android and wasm32 it is currently the in-process plaintext store —
/// callers SHOULD inspect `.name() == "fallback-plaintext"` and emit a WARN
/// to the audit log so operators see they are not getting hardware-backed
/// protection.
pub fn detect_best_storage() -> Box<dyn SecureStorage> {
    #[cfg(all(
        any(target_os = "linux", target_os = "macos", target_os = "windows"),
        not(target_arch = "wasm32"),
    ))]
    {
        return Box::new(KeyringStorage::new());
    }
    #[allow(unreachable_code)]
    {
        Box::new(FallbackPlaintext::new_in_memory())
    }
}

// ── Keyring-crate backend (desktop OSes) ────────────────────────────────────

/// Thin wrapper over the `keyring` crate. Maps every supported desktop OS
/// onto its native credential vault; the constant `BACKEND_NAME` reflects
/// which one at compile time.
#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(target_arch = "wasm32"),
))]
pub struct KeyringStorage;

#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(target_arch = "wasm32"),
))]
impl Default for KeyringStorage {
    fn default() -> Self {
        Self::new()
    }
}

// Re-gate after the `impl Default` insert — clippy's `--fix` pass put the
// Default impl between the original cfg attribute and `impl KeyringStorage`,
// so the cfg ended up on Default and this block lost its guard. Without
// the cfg the Android build can't resolve `KeyringStorage` (the type
// itself IS gated, line 93-97).
#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(target_arch = "wasm32"),
))]
impl KeyringStorage {
    /// Service identifier passed to `keyring::Entry::new(SERVICE, key_id)`.
    /// Lets the user audit our entries with `secret-tool search service
    /// phantomchat` (Linux) or `cmdkey /list:phantomchat:*` (Windows).
    const SERVICE: &'static str = "phantomchat";

    #[cfg(target_os = "windows")]
    const BACKEND_NAME: &'static str = "dpapi";
    #[cfg(target_os = "macos")]
    const BACKEND_NAME: &'static str = "keychain";
    #[cfg(target_os = "linux")]
    const BACKEND_NAME: &'static str = "libsecret";

    pub fn new() -> Self {
        Self
    }

    fn entry(&self, key_id: &str) -> Result<keyring::Entry, SecureStorageError> {
        keyring::Entry::new(Self::SERVICE, key_id).map_err(|e| {
            SecureStorageError::Backend(format!("{}: open entry: {}", Self::BACKEND_NAME, e))
        })
    }
}

#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    not(target_arch = "wasm32"),
))]
impl SecureStorage for KeyringStorage {
    fn store(&self, key_id: &str, secret: &[u8]) -> Result<(), SecureStorageError> {
        let entry = self.entry(key_id)?;
        entry.set_secret(secret).map_err(|e| {
            SecureStorageError::Backend(format!("{}: set_secret: {}", Self::BACKEND_NAME, e))
        })
    }

    fn load(&self, key_id: &str) -> Result<Vec<u8>, SecureStorageError> {
        let entry = self.entry(key_id)?;
        match entry.get_secret() {
            Ok(bytes) => Ok(bytes),
            Err(keyring::Error::NoEntry) => Err(SecureStorageError::NotFound(key_id.to_string())),
            Err(e) => Err(SecureStorageError::Backend(format!(
                "{}: get_secret: {}",
                Self::BACKEND_NAME,
                e
            ))),
        }
    }

    fn delete(&self, key_id: &str) -> Result<(), SecureStorageError> {
        let entry = self.entry(key_id)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            // Idempotent: deleting an absent entry is success.
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecureStorageError::Backend(format!(
                "{}: delete: {}",
                Self::BACKEND_NAME,
                e
            ))),
        }
    }

    fn name(&self) -> &'static str {
        Self::BACKEND_NAME
    }
}

// ── Fallback: in-process plaintext map ──────────────────────────────────────

/// Last-resort store used in environments where no OS keyring exists:
///
/// - WASM (no native crypto API for arbitrary apps)
/// - Android (until the JNI bridge to `AndroidKeyStore` lands)
/// - Headless CI / scripted CLI runs where libsecret-daemon is not running
///
/// Bytes live in process memory only; nothing hits disk. Callers that want
/// disk persistence with `0600` perms should bolt that on at the desktop
/// layer (we keep this module crypto-clean and FFI-free).
///
/// The desktop wrapper around this falls back to the legacy plaintext
/// `keys.json` path if even this in-memory store cannot be promoted to disk.
pub struct FallbackPlaintext {
    inner: &'static Mutex<HashMap<String, Vec<u8>>>,
}

static FALLBACK_STORE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();

impl FallbackPlaintext {
    /// Returns a handle onto the process-global in-memory store. Multiple
    /// `FallbackPlaintext::new_in_memory()` calls share the same backing
    /// map so two callers with the same `key_id` see each other's writes.
    pub fn new_in_memory() -> Self {
        Self {
            inner: FALLBACK_STORE.get_or_init(|| Mutex::new(HashMap::new())),
        }
    }
}

impl SecureStorage for FallbackPlaintext {
    fn store(&self, key_id: &str, secret: &[u8]) -> Result<(), SecureStorageError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|e| SecureStorageError::Backend(format!("fallback lock poisoned: {e}")))?;
        g.insert(key_id.to_string(), secret.to_vec());
        Ok(())
    }

    fn load(&self, key_id: &str) -> Result<Vec<u8>, SecureStorageError> {
        let g = self
            .inner
            .lock()
            .map_err(|e| SecureStorageError::Backend(format!("fallback lock poisoned: {e}")))?;
        g.get(key_id)
            .cloned()
            .ok_or_else(|| SecureStorageError::NotFound(key_id.to_string()))
    }

    fn delete(&self, key_id: &str) -> Result<(), SecureStorageError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|e| SecureStorageError::Backend(format!("fallback lock poisoned: {e}")))?;
        g.remove(key_id);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "fallback-plaintext"
    }
}

/// Stable `key_id` derived from an absolute file path.
///
/// `"phantomchat:" + sha256(path_bytes)[..16]` (32 hex chars). Stable across
/// runs, distinct per identity-file, and never carries any user-controlled
/// substring into the OS-level service registry (so a hostile `KEYS_FILE`
/// env override cannot collide with another app's keychain entry).
pub fn key_id_for_path(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    format!("phantomchat:{}", hex::encode(&digest[..8]))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_roundtrip_store_load_delete() {
        let s = FallbackPlaintext::new_in_memory();
        let key = "phantomchat:roundtrip-unit-1";
        let secret = b"hunter2-but-32-bytes-of-data!!!!";

        // Cleanup from any prior test re-run sharing the global map.
        let _ = s.delete(key);

        s.store(key, secret).expect("store");
        let got = s.load(key).expect("load");
        assert_eq!(got, secret);

        s.delete(key).expect("delete");
        match s.load(key) {
            Err(SecureStorageError::NotFound(_)) => {}
            other => panic!("expected NotFound after delete, got {other:?}"),
        }
    }

    #[test]
    fn fallback_overwrites_on_repeated_store() {
        let s = FallbackPlaintext::new_in_memory();
        let key = "phantomchat:overwrite-unit-2";
        let _ = s.delete(key);

        s.store(key, b"v1").unwrap();
        s.store(key, b"v2-longer-payload").unwrap();
        assert_eq!(s.load(key).unwrap(), b"v2-longer-payload");
        s.delete(key).unwrap();
    }

    #[test]
    fn key_id_is_deterministic_and_path_dependent() {
        use std::path::PathBuf;
        let a = key_id_for_path(&PathBuf::from("/tmp/a/keys.json"));
        let b = key_id_for_path(&PathBuf::from("/tmp/a/keys.json"));
        let c = key_id_for_path(&PathBuf::from("/tmp/b/keys.json"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("phantomchat:"));
        // 12 hex chars after the prefix (sha256[..8] = 8 bytes = 16 hex … wait,
        // we sliced [..8] which is 8 BYTES, encoded as 16 hex chars).
        assert_eq!(a.len(), "phantomchat:".len() + 16);
    }

    #[test]
    fn name_reflects_platform_or_fallback() {
        let s = detect_best_storage();
        let n = s.name();
        assert!(matches!(
            n,
            "dpapi" | "keychain" | "libsecret" | "android-keystore" | "fallback-plaintext"
        ));
    }
}
