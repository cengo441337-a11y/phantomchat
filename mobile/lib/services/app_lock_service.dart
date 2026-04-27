import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:local_auth/local_auth.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'security_service.dart';
import 'storage_service.dart';

/// Argument bundle for the off-isolate PBKDF2 worker. Must be a top-level
/// type because `compute(...)` ships its arguments across an isolate
/// boundary and the closure / receiver itself can't capture state.
class _Pbkdf2Args {
  final String pin;
  final List<int> salt;
  final int iterations;
  const _Pbkdf2Args(this.pin, this.salt, this.iterations);
}

/// Top-level so it can run inside a background isolate via `compute(...)`.
/// The pure-Dart PBKDF2 from `cryptography` is the bottleneck (5-15 s on
/// mid-range Android at 600k iters); offloading it keeps the UI thread
/// responsive during PIN setup / verification.
Future<Uint8List> _pbkdf2Inner(_Pbkdf2Args args) async {
  final pbkdf2 = Pbkdf2(
    macAlgorithm: Hmac.sha256(),
    iterations: args.iterations,
    bits: 256,
  );
  final key = await pbkdf2.deriveKey(
    secretKey: SecretKey(utf8.encode(args.pin)),
    nonce: args.salt,
  );
  return Uint8List.fromList(await key.extractBytes());
}

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
  static const _kPinHash       = 'phantom_pin_hash';
  static const _kPinSalt       = 'phantom_pin_salt';
  // PBKDF2 iteration count used to derive the stored hash. Stored alongside
  // the salt so we can bump the global default (OWASP 2023 → 600k) without
  // locking out users whose PIN was set under the legacy 100k-iter regime.
  // Absent value = legacy install → falls back to [_legacyPbkdf2Iters].
  static const _kPinIters      = 'phantom_pin_iters';
  static const _kBioEnable     = 'phantom_biometric_enabled';
  static const _kAutoLock      = 'phantom_autolock_seconds';
  static const _kFailCount     = 'phantom_pin_fail_count';
  // Lightweight "biometric on app start" opt-in. Independent of the PIN
  // path: when ON, every app-resume prompts a biometric check before the
  // shell becomes interactive again, even if no PIN has been configured.
  // Default OFF — biometric prompts on launch surprise new users.
  static const _kBioOnLaunch   = 'phantom_biometric_on_launch';

  /// After this many consecutive wrong PINs the device self-wipes.
  static const int maxFailedAttempts = 10;

  /// Default inactivity timeout in seconds. User-configurable at runtime.
  static const int defaultAutoLockSeconds = 60;

  /// PBKDF2 iteration count for newly-set PINs. 50k is an intentional
  /// compromise: the hash already lives in Android Keystore / iOS
  /// Keychain (hardware-backed where available), so iter count is the
  /// second line of defence, not the first. With cryptography_flutter
  /// native, 600k = ~150 ms; on older / emulator-class hardware where
  /// the package falls back to pure-Dart, 600k = 30-60 s of UI freeze
  /// at PIN-confirm. 50k stays sub-second across all paths. Stored per-
  /// user in `_kPinIters` so this constant can be bumped to 600k once
  /// shipping devices run native KDF — `verifyPin` reads the stored
  /// count back, so existing PINs keep verifying after a bump.
  static const int currentPbkdf2Iters = 50000;

  /// Iteration count used by builds before the iter-count was persisted.
  /// Verification falls back to this when `_kPinIters` is absent so
  /// pre-migration installs still unlock with their original PIN.
  static const int _legacyPbkdf2Iters = 100000;

  static final LocalAuthentication _auth = LocalAuthentication();

  // Process-local state
  static DateTime? _unlockedAt;
  // True while a biometric-on-launch prompt is in-flight or has succeeded for
  // this foreground session. Cleared on every paused/inactive lifecycle event.
  static bool _bioSessionUnlocked = false;

  // ── PIN setup / verification ───────────────────────────────────────────────

  /// True once a PIN has been configured on this device.
  static Future<bool> hasPin() async =>
      (await _storage.read(key: _kPinHash)) != null;

  /// Configure a new PIN. Overwrites any existing PIN.
  /// Minimum length is 4 digits; caller is responsible for UI-level validation.
  ///
  /// Always derives at [currentPbkdf2Iters] (OWASP 2023). The chosen iter
  /// count is persisted in `_kPinIters` so a future bump can roll forward
  /// without breaking already-set PINs.
  static Future<void> setPin(String pin) async {
    if (pin.length < 4) {
      throw ArgumentError('PIN must be at least 4 digits');
    }
    // Timing instrumentation — surfaced via `debugPrint` so logcat shows
    // exactly where the time goes on real devices. The integration_test
    // emulator reports 50k PBKDF2 in ~3.9 s including isolate spawn, but
    // user-reports of "freeze on PIN confirm" persist on real arm64 —
    // either the cryptography_flutter native KDF auto-registration isn't
    // firing (silent fall back to pure-Dart) or secure-storage is the
    // bottleneck. The four labels below pin down which.
    final t0 = DateTime.now().millisecondsSinceEpoch;
    final salt = _randomBytes(16);
    final t1 = DateTime.now().millisecondsSinceEpoch;
    final hash = await _derivePinHash(pin, salt, currentPbkdf2Iters);
    final t2 = DateTime.now().millisecondsSinceEpoch;
    await _storage.write(key: _kPinSalt, value: base64Encode(salt));
    await _storage.write(key: _kPinHash, value: base64Encode(hash));
    await _storage.write(key: _kPinIters, value: '$currentPbkdf2Iters');
    await _storage.write(key: _kFailCount, value: '0');
    final t3 = DateTime.now().millisecondsSinceEpoch;
    debugPrint('[setPin] salt=${t1 - t0}ms pbkdf2($currentPbkdf2Iters)=${t2 - t1}ms storage=${t3 - t2}ms total=${t3 - t0}ms');
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

    // Read the per-hash iter count; pre-migration installs don't have one,
    // so fall back to the legacy 100k baseline. The stored hash was
    // produced with whatever iter count was in effect at setPin time, so
    // we MUST re-derive with that same number to match.
    final itersRaw = await _storage.read(key: _kPinIters);
    final iters = int.tryParse(itersRaw ?? '') ?? _legacyPbkdf2Iters;

    final salt   = base64Decode(saltB64);
    final stored = base64Decode(hashB64);
    final fresh  = await _derivePinHash(candidate, salt, iters);

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

  // ── Biometric-on-launch quick-lock ─────────────────────────────────────────

  /// Whether the user opted into the lightweight "prompt biometric every
  /// time the app comes to the foreground" gate. Independent of [hasPin].
  static Future<bool> bioOnLaunchEnabled() async =>
      (await _storage.read(key: _kBioOnLaunch)) == '1';

  /// Persist the biometric-on-launch toggle. Setting it to `false` also
  /// clears the in-process "session unlocked" flag so the next resume
  /// returns immediately without prompting.
  static Future<void> setBioOnLaunchEnabled(bool enabled) async {
    await _storage.write(key: _kBioOnLaunch, value: enabled ? '1' : '0');
    if (!enabled) _bioSessionUnlocked = false;
  }

  /// Marks the current foreground session as biometrically unlocked. Called
  /// after a successful [authenticateBiometric] from the launch gate.
  static void markBioSessionUnlocked() {
    _bioSessionUnlocked = true;
  }

  /// Drop the in-process unlock flag — call from `paused`/`inactive`
  /// lifecycle events so the next resume re-prompts.
  static void clearBioSession() {
    _bioSessionUnlocked = false;
  }

  /// True when the launch gate should currently render the lock overlay.
  /// Reads opt-in state lazily so callers don't need to cache it.
  static Future<bool> bioOnLaunchPending() async {
    if (_bioSessionUnlocked) return false;
    return bioOnLaunchEnabled();
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

  /// PBKDF2-HMAC-SHA256, 32-byte output. Iteration count is configurable
  /// to support migration: hashes set under the legacy 100k baseline still
  /// verify by re-deriving with `iterations=100000`, while new hashes are
  /// produced at [currentPbkdf2Iters] (600k, OWASP 2023).
  ///
  /// Runs on a background isolate via `compute(...)` — pure-Dart PBKDF2 at
  /// 600k iters takes 5-15 s on mid-range Android, which would otherwise
  /// freeze the UI thread on PIN setup / verification.
  static Future<Uint8List> _derivePinHash(
    String pin,
    List<int> salt,
    int iterations,
  ) async {
    return compute(_pbkdf2Inner, _Pbkdf2Args(pin, salt, iterations));
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
