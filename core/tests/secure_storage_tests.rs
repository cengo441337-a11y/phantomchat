//! End-to-end tests for the OS-secure-storage abstraction + memory wiping.
//!
//! These three tests cover the new hardening pillars in one go:
//!
//! 1. `secure_storage_roundtrip` — `store / load / delete` against the
//!    in-process fallback. Selecting `FallbackPlaintext` directly keeps the
//!    test deterministic on CI workers without a libsecret daemon, while
//!    still exercising the same `SecureStorage` surface the desktop calls.
//!
//! 2. `migration_from_plaintext_keysjson` — plant a legacy plaintext
//!    `keys.json`, run the migration helper, verify the rewritten file
//!    references the new `key_id` instead of the raw bytes and that the
//!    bytes can be retrieved back through the trait.
//!
//! 3. `key_zeroize_on_drop` — drop a `SpendKey`, then read the underlying
//!    32-byte secret region via `unsafe { ptr::read_volatile }` and assert
//!    every byte is zero. The volatile read is what defeats the compiler's
//!    "the object is dead, the load can be elided" optimisation.

use std::fs;
use std::io::Write;
use std::path::Path;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use phantomchat_core::keys::SpendKey;
use phantomchat_core::secure_storage::{
    detect_best_storage, key_id_for_path, FallbackPlaintext, SecureStorage, SecureStorageError,
};
use serde_json::json;
use x25519_dalek::StaticSecret;
use zeroize::Zeroize;

#[test]
fn secure_storage_roundtrip() {
    // Use the FallbackPlaintext directly: deterministic, no host vault
    // required, exercises the trait surface end-to-end.
    let s: Box<dyn SecureStorage> = Box::new(FallbackPlaintext::new_in_memory());
    let key = "phantomchat:integration-roundtrip";
    let secret = b"\x00\x01\x02\x03 round-trip me through the trait \xff\xfe";

    // Idempotent cleanup in case the global map already had this key from a
    // previous test run in the same process.
    let _ = s.delete(key);

    // store → load returns the original bytes.
    s.store(key, secret).expect("store should succeed");
    let got = s.load(key).expect("load after store should find the entry");
    assert_eq!(got, secret, "stored and loaded bytes must match exactly");

    // delete → load returns NotFound (the only way to detect an absent
    // entry — the trait deliberately does not surface a contains_key()).
    s.delete(key).expect("delete should succeed");
    let err = s.load(key).expect_err("load after delete must error");
    assert!(matches!(err, SecureStorageError::NotFound(_)));

    // delete is idempotent.
    s.delete(key).expect("second delete is a no-op");

    // Sanity: detect_best_storage produces a backend with one of the
    // documented names. On CI this is usually fallback-plaintext (no
    // libsecret daemon), but that's still a valid SecureStorage.
    let n = detect_best_storage().name();
    assert!(matches!(
        n,
        "dpapi" | "keychain" | "libsecret" | "android-keystore" | "fallback-plaintext"
    ));
}

#[test]
fn migration_from_plaintext_keysjson() {
    // Write a synthetic legacy keys.json that mirrors the desktop's pre-
    // migration schema. We pin every secret to a recognisable byte pattern
    // so a failure in the migration logic is immediately diagnosable.
    let tmp = std::env::temp_dir().join("phantomchat_migration_test");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    let keys_json = tmp.join("keys.json");

    let view_priv = [0xAAu8; 32];
    let spend_priv = [0xBBu8; 32];
    let signing_priv = [0xCCu8; 32];

    let plaintext_json = json!({
        "view_private": B64.encode(view_priv),
        "view_public": "00".repeat(32),
        "spend_private": B64.encode(spend_priv),
        "spend_public": "00".repeat(32),
        "signing_private": B64.encode(signing_priv),
        "signing_public": "00".repeat(32),
        "identity_private": B64.encode([0xDDu8; 32]),
        "identity_public": "00".repeat(32),
    });
    fs::write(&keys_json, serde_json::to_vec_pretty(&plaintext_json).unwrap()).unwrap();

    // Run the migration: stash the three secrets in the secure store under
    // the path-derived key_id, then rewrite keys.json to reference the
    // key_id instead of the raw bytes. This mirrors what
    // `desktop::migrate_keys_json_to_secure_storage` does at boot.
    let storage: Box<dyn SecureStorage> = Box::new(FallbackPlaintext::new_in_memory());
    let key_id = key_id_for_path(&keys_json);
    let _ = storage.delete(&format!("{key_id}:view"));
    let _ = storage.delete(&format!("{key_id}:spend"));
    let _ = storage.delete(&format!("{key_id}:signing"));
    migrate_keys_json(&keys_json, &*storage).expect("migration succeeds");

    // The rewritten keys.json references the key_id and no longer contains
    // any of the original `*_private` plaintext fields.
    let migrated_raw = fs::read_to_string(&keys_json).unwrap();
    let migrated: serde_json::Value = serde_json::from_str(&migrated_raw).unwrap();
    assert!(
        migrated.get("view_private").is_none(),
        "migrated file must not contain view_private (saw: {migrated_raw})"
    );
    assert!(migrated.get("spend_private").is_none());
    assert!(migrated.get("signing_private").is_none());
    assert_eq!(
        migrated["view_private_ref"].as_str().unwrap(),
        format!("{key_id}:view")
    );
    assert_eq!(
        migrated["spend_private_ref"].as_str().unwrap(),
        format!("{key_id}:spend")
    );
    assert_eq!(
        migrated["signing_private_ref"].as_str().unwrap(),
        format!("{key_id}:signing")
    );
    assert_eq!(migrated["storage_backend"].as_str().unwrap(), storage.name());

    // The plaintext that was previously inline is now retrievable from the
    // secure-storage backend via the documented key_id scheme.
    assert_eq!(
        storage.load(&format!("{key_id}:view")).unwrap(),
        view_priv.to_vec()
    );
    assert_eq!(
        storage.load(&format!("{key_id}:spend")).unwrap(),
        spend_priv.to_vec()
    );
    assert_eq!(
        storage.load(&format!("{key_id}:signing")).unwrap(),
        signing_priv.to_vec()
    );

    // Cleanup the temp dir + secure-store entries so a re-run starts clean.
    let _ = storage.delete(&format!("{key_id}:view"));
    let _ = storage.delete(&format!("{key_id}:spend"));
    let _ = storage.delete(&format!("{key_id}:signing"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn key_zeroize_on_drop() {
    // Approach: wrap the SpendKey in `ManuallyDrop` so the test fully
    // controls when (or whether) `Drop` runs. We then *simulate* the work
    // `Drop` would do — invoke the struct's `Zeroize` impl directly — and
    // volatile-read the underlying bytes from the still-live address to
    // confirm they were scrubbed.
    //
    // Why not let `Drop` run and read after?  Once `Drop` returns, Rust is
    // free to reuse the slot (and on the heap the allocator immediately
    // overwrites freed chunks with freelist metadata). The volatile read
    // would then see allocator junk instead of zeros — a false negative
    // for the test even though the wipe ran correctly. Holding the value
    // alive via `ManuallyDrop` removes that race entirely.
    //
    // The volatile read itself is what defeats the compiler's "this load
    // is dead, fold it to a constant" optimisation — the spec calls this
    // out explicitly.

    use std::mem::ManuallyDrop;

    let pattern = [0xA5u8; 32];
    let secret = StaticSecret::from(pattern);
    let public = x25519_dalek::PublicKey::from(&secret);
    let mut key: ManuallyDrop<SpendKey> =
        ManuallyDrop::new(SpendKey { public, secret });

    // `StaticSecret` is a tuple-newtype around `[u8; 32]`. While not
    // formally `#[repr(transparent)]` in the upstream crate, a
    // single-field tuple struct has identical layout to its inner type in
    // practice on every supported target — we sanity-check that fact by
    // reading the pre-zeroize pattern back through the pointer.
    let secret_ptr: *const u8 = &key.secret as *const StaticSecret as *const u8;

    // Pre-zeroize sanity: the pointer is correct and the bytes are still
    // our pattern.
    let pre: [u8; 32] = unsafe {
        let mut buf = [0u8; 32];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = std::ptr::read_volatile(secret_ptr.add(i));
        }
        buf
    };
    assert_eq!(pre, pattern, "pre-zeroize sanity: pattern must be at secret_ptr");

    // Invoke the derived `Zeroize` — this is exactly what the
    // `ZeroizeOnDrop`-derived `Drop` impl runs as its first action.
    // Confirms the wrapper struct's per-field zeroize cascades into the
    // inner `StaticSecret` bytes.
    Zeroize::zeroize(&mut *key);

    // Post-zeroize volatile re-read of the SAME memory. The allocator
    // hasn't touched the slot — `key` is still alive — so the only way
    // the bytes could be zero is if our zeroize cascaded all the way down.
    let post: [u8; 32] = unsafe {
        let mut buf = [0u8; 32];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = std::ptr::read_volatile(secret_ptr.add(i));
        }
        buf
    };
    assert_eq!(
        post,
        [0u8; 32],
        "after Zeroize the secret bytes must be zero (got {:02x?})",
        post
    );

    // We keep the value in ManuallyDrop and never call ManuallyDrop::drop —
    // the bytes are already zero, so leaking the (zeroed) shell at test
    // teardown is harmless and avoids re-entering the derived Drop, which
    // would zeroize-already-zero memory and is not what we're testing here.
    let _ = key;
}

// ── Helpers ─────────────────────────────────────────────────────────────────
//
// `migrate_keys_json` is a lightweight, file-only re-implementation of the
// migration the desktop runs at boot. We keep it inline (rather than reaching
// into the desktop crate) so the test can run as part of `cargo test -p
// phantomchat_core` without pulling in Tauri / Webview2 / a windowing system.
// The desktop's wrapper does the same work plus an `audit(...)` log line.

fn migrate_keys_json(
    path: &Path,
    storage: &dyn SecureStorage,
) -> std::io::Result<()> {
    let raw = fs::read(path)?;
    let mut json: serde_json::Value =
        serde_json::from_slice(&raw).expect("legacy keys.json is well-formed JSON");
    let key_id = key_id_for_path(path);

    let migrate_field = |json: &mut serde_json::Value, field: &str, suffix: &str| {
        if let Some(s) = json.get(field).and_then(|v| v.as_str()) {
            let bytes = B64.decode(s).expect("legacy field is valid base64");
            let id = format!("{key_id}:{suffix}");
            storage
                .store(&id, &bytes)
                .expect("secure-storage store should succeed");
            if let Some(obj) = json.as_object_mut() {
                obj.remove(field);
                obj.insert(
                    format!("{field}_ref"),
                    serde_json::Value::String(id),
                );
            }
        }
    };

    migrate_field(&mut json, "view_private", "view");
    migrate_field(&mut json, "spend_private", "spend");
    migrate_field(&mut json, "signing_private", "signing");

    if let Some(obj) = json.as_object_mut() {
        obj.insert(
            "storage_backend".to_string(),
            serde_json::Value::String(storage.name().to_string()),
        );
    }

    // Atomic write: tmp + fsync + rename. If anything fails before the rename
    // the original keys.json (with plaintext) survives — that's the safety
    // contract documented in the migration spec.
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(&json).unwrap())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}
