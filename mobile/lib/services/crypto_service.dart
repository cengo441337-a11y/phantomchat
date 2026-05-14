// ⚠️  DEPRECATED — only the two `generateKeyPair` / `generateSigningSeedHex`
// helpers below remain to satisfy the onboarding boot path. The full
// envelope+ratchet pipeline lives in `phantomchat_core` (Rust) and is
// exposed to Flutter via `flutter_rust_bridge`. New code MUST NOT add
// crypto operations to this file — every wire-format primitive belongs
// in the Rust core.
//
// Audit 2026-04-30 (mobile audit-M7): the previous version of this file
// also exported `encrypt(plaintext, recipientSpendKeyHex)` and
// `decrypt(...)` based on `Chacha20.poly1305Aead()` (96-bit nonce). The
// production wire format uses **XChaCha20-Poly1305** (192-bit nonce) on
// the Rust side, so any envelope produced or consumed via that Dart path
// was wire-incompatible with itself. No remaining caller used either
// method (grep confirmed only `// Legacy CryptoService.encrypt`-shaped
// comments survived); the methods were dead ammunition. Deleted, plus
// the unused `ecdh` helper and the static `_chacha` field they shared.

import 'dart:typed_data';
import 'package:cryptography/cryptography.dart';

@Deprecated(
  'Use phantomchat_core via flutter_rust_bridge — see lib/src/rust/api.dart.'
  ' Only the two key-generation helpers below remain; the encrypt/decrypt'
  ' wire path was deleted in audit-M7 (mobile, 2026-04-30) because it used'
  ' a 96-bit-nonce ChaCha20 that did not match the Rust core wire format.',
)
class CryptoService {
  static final _x25519 = X25519();
  static final _ed25519 = Ed25519();

  /// Generate a new X25519 key pair, returns `{private: hex, public: hex}`.
  /// Used by the onboarding flow on first launch — eventually this should
  /// move to `phantomchat_core` (already there as `ViewKey::generate` /
  /// `SpendKey::generate`); the Dart caller exists only because the
  /// onboarding-screen widget tree was built before the FRB bridge was
  /// stable.
  static Future<Map<String, String>> generateKeyPair() async {
    final pair = await _x25519.newKeyPair();
    final privBytes = await pair.extractPrivateKeyBytes();
    final pubKey = await pair.extractPublicKey();
    return {
      'private': _hex(Uint8List.fromList(privBytes)),
      'public': _hex(Uint8List.fromList(pubKey.bytes)),
    };
  }

  /// Generate a fresh Ed25519 signing seed (32 random bytes). Returned
  /// hex-encoded so it slots straight into [PhantomIdentity.privateSigningKey]
  /// and the FRB `loadLocalIdentityV3` arg. The Rust core's
  /// `PhantomSigningKey::from_bytes` rehydrates the `ed25519_dalek::SigningKey`
  /// on demand, so we only need to ship the 32-byte seed across the bridge —
  /// not a full SigningKey serialisation.
  ///
  /// Note: `Ed25519().newKeyPair()` returns a 64-byte expanded private key in
  /// some `cryptography` builds; we always extract the FIRST 32 bytes which
  /// is the seed. This matches what `PhantomSigningKey::generate` produces
  /// on the Rust side (raw OsRng bytes).
  static Future<String> generateSigningSeedHex() async {
    final pair = await _ed25519.newKeyPair();
    final priv = await pair.extractPrivateKeyBytes();
    final seed = priv.length >= 32
        ? Uint8List.fromList(priv.sublist(0, 32))
        : Uint8List.fromList(priv);
    return _hex(seed);
  }

  // Hex encode
  static String _hex(Uint8List bytes) =>
      bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
}
