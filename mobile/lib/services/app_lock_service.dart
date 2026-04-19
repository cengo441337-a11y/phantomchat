import 'dart:convert';
import 'dart:typed_data';
import 'package:cryptography/cryptography.dart';
import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:local_auth/local_auth.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'security_service.dart';
import 'storage_service.dart';

/// PhantomChat app-lock service.
///
/// Combines:
/// - **PIN** — PBKDF2-HMAC-SHA256(100 000 iters, 16-byte salt), stored in
///   `FlutterSecureStorage` (Android Keystore / iOS Keychain).
/// - **Biometrie** — optional quick-unlock via `local_auth`
///   (fingerprint / face / device credentials).
/// - **Auto-lock** — re-locks the UI after a configurable timeout once the
///   app has been backgrounded.
/// - **Panic-Wipe** — after [maxFailedAttempts] consecutive wrong PINs, the
///   entire on-device state (identity, contacts, messages, DB password,
///   preferences) is securely erased.
///
/// This class is stateless beyond the two in-memory fields it tracks for the
/// lifetime of the process: `_unlockedAt` and `_failedAttempts`. Everything
/// else is persisted through the backing stores.
class AppLockService {
  static const _storage = FlutterSecureStorage();

  // Secure-storage keys
  static const _kPinHash   = 'phantom_pin_hash';
  static const _kPinSalt   = 'phantom_pin_salt';
  static const _kBioEnable = 'phantom_biometric_enabled';
  static const _kAutoLock  = 'phantom_autolock_seconds';
  static const _kFailCount = 'phantom_pin_fail_count';

  /// After this many consecutive wrong PINs the device self-wipes.
  static const int maxFailedAttempts = 10;

  /// Default inactivity timeout in seconds. User-configurable at runtime.
  static const int defaultAutoLockSeconds = 60;

  static final LocalAuthentication _auth = LocalAuthentication();

  // Process-local state
  static DateTime? _unlockedAt;

  // ── PIN setup / verification ───────────────────────────────────────────────

  /// True once a PIN has been configured on this device.
  static Future<bool> hasPin() async =>
      (await _storage.read(key: _kPinHash)) != null;

  /// Configure a new PIN. Overwrites any existing PIN.
  /// Minimum length is 4 digits; caller is responsible for UI-level validation.
  static Future<void> setPin(String pin) async {
    if (pin.length < 4) {
      throw ArgumentError('PIN must be at least 4 digits');
    }
    final salt = _randomBytes(16);
    final hash = await _derivePinHash(pin, salt);
    await _storage.write(key: _kPinSalt, value: base64Encode(salt));
    await _storage.write(key: _kPinHash, value: base64Encode(hash));
    await _storage.write(key: _kFailCount, value: '0');
    _unlockedAt = DateTime.now();
  }

  /// Verify a candidate PIN. On success: returns `true`, resets the failed-
  /// attempt counter, records the unlock timestamp. On failure: returns
  /// `false`, increments the counter, and triggers [panicWipe] once
  /// [maxFailedAttempts] is reached.
  static Future<bool> verifyPin(String candidate) async {
    final saltB64 = await _storage.read(key: _kPinSalt);
    final hashB64 = await _storage.read(key: _kPinHash);
    if (saltB64 == null || hashB64 == null) return false;

    final salt   = base64Decode(saltB64);
    final stored = base64Decode(hashB64);
    final fresh  = await _derivePinHash(candidate, salt);

    if (_constantTimeEq(fresh, stored)) {
      await _storage.write(key: _kFailCount, value: '0');
      _unlockedAt = DateTime.now();
      return true;
    }

    final failed = (int.tryParse(await _storage.read(key: _kFailCount) ?? '0') ?? 0) + 1;
    await _storage.write(key: _kFailCount, value: '$failed');

    if (failed >= maxFailedAttempts) {
      await panicWipe();
    }
    return false;
  }

  /// Remaining wrong-PIN attempts before a panic wipe triggers.
  static Future<int> remainingAttempts() async {
    final raw = await _storage.read(key: _kFailCount);
    final used = int.tryParse(raw ?? '0') ?? 0;
    final left = maxFailedAttempts - used;
    return left < 0 ? 0 : left;
  }

  // ── Biometrics ─────────────────────────────────────────────────────────────

  /// Whether the device supports biometric authentication (configured
  /// fingerprint, face, or device credentials).
  static Future<bool> biometricAvailable() async {
    try {
      final supported = await _auth.isDeviceSupported();
      final canCheck  = await _auth.canCheckBiometrics;
      return supported && canCheck;
    } on PlatformException {
      return false;
    }
  }

  /// Whether the user has opted into biometric quick-unlock.
  static Future<bool> biometricEnabled() async =>
      (await _storage.read(key: _kBioEnable)) == '1';

  static Future<void> setBiometricEnabled(bool enabled) async {
    await _storage.write(key: _kBioEnable, value: enabled ? '1' : '0');
  }

  /// Prompt the OS for biometric authentication. Returns `true` on success.
  /// Does **not** reset the failed-PIN counter — biometric skips the PIN
  /// path entirely and counts only as a successful unlock.
  static Future<bool> authenticateBiometric({String? reason}) async {
    try {
      final ok = await _auth.authenticate(
        localizedReason: reason ?? 'Unlock PhantomChat',
        options: const AuthenticationOptions(
          biometricOnly: false,   // allow device PIN fallback at the OS level
          stickyAuth: true,
          useErrorDialogs: true,
        ),
      );
      if (ok) _unlockedAt = DateTime.now();
      return ok;
    } on PlatformException {
      return false;
    }
  }

  // ── Auto-lock ──────────────────────────────────────────────────────────────

  static Future<int> autoLockSeconds() async {
    final raw = await _storage.read(key: _kAutoLock);
    return int.tryParse(raw ?? '') ?? defaultAutoLockSeconds;
  }

  static Future<void> setAutoLockSeconds(int seconds) async {
    await _storage.write(key: _kAutoLock, value: '$seconds');
  }

  /// True if the app should be considered locked right now. A freshly
  /// installed device (no PIN) is never locked — the onboarding flow is
  /// expected to set one up.
  static Future<bool> isCurrentlyLocked() async {
    if (!await hasPin()) return false;
    if (_unlockedAt == null) return true;
    final seconds = await autoLockSeconds();
    return DateTime.now().difference(_unlockedAt!).inSeconds >= seconds;
  }

  /// Explicit lock (called from the UI — e.g. a "Lock now" button or on
  /// app-backgrounded lifecycle events).
  static void lock() {
    _unlockedAt = null;
  }

  /// Bump the last-activity timestamp so an in-use session does not auto-lock.
  static void touch() {
    if (_unlockedAt != null) _unlockedAt = DateTime.now();
  }

  // ── Panic wipe ─────────────────────────────────────────────────────────────

  /// Wipes everything. Called automatically on too many wrong PINs, and can
  /// also be invoked manually from the security settings.
  ///
  /// Clears, in order:
  /// 1. Identity and DB-password material from `FlutterSecureStorage`
  /// 2. Contacts, messages, and app preferences from `SharedPreferences`
  /// 3. Any in-memory unlock state
  static Future<void> panicWipe() async {
    await SecurityService.clearAll();
    await StorageService.wipe();
    final prefs = await SharedPreferences.getInstance();
    await prefs.clear();
    _unlockedAt = null;
  }

  // ── Crypto primitives ──────────────────────────────────────────────────────

  /// PBKDF2-HMAC-SHA256, 100 000 iterations, 32-byte output.
  static Future<Uint8List> _derivePinHash(String pin, List<int> salt) async {
    final pbkdf2 = Pbkdf2(
      macAlgorithm: Hmac.sha256(),
      iterations: 100000,
      bits: 256,
    );
    final key = await pbkdf2.deriveKey(
      secretKey: SecretKey(utf8.encode(pin)),
      nonce: salt,
    );
    return Uint8List.fromList(await key.extractBytes());
  }

  /// Constant-time byte comparison (mitigates timing side-channels on the
  /// PIN-verify path).
  static bool _constantTimeEq(List<int> a, List<int> b) {
    if (a.length != b.length) return false;
    var diff = 0;
    for (var i = 0; i < a.length; i++) {
      diff |= a[i] ^ b[i];
    }
    return diff == 0;
  }

  /// CSPRNG bytes via the platform's `FlutterSecureStorage`-backed secure
  /// random, falling back to `SecretKeyData.random` if unavailable.
  static List<int> _randomBytes(int n) {
    final key = SecretKeyData.random(length: n);
    return key.bytes;
  }
}
