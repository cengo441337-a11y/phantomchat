import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'dart:math';
import 'dart:convert';

// Audit 2026-04-30: explicit secure-storage options. Defaults under
// flutter_secure_storage 9.x don't enable AndroidEncryptedSharedPreferences
// (so the on-disk blob lives in plain SharedPreferences with cipher-wrapping
// via a non-Keystore-backed master key), and on iOS the default accessibility
// is `unlocked` — meaning the keychain item rides along into iCloud/iTunes
// backups. The combination breaks the "keys live only on this device"
// promise the onboarding screen makes. Switching to encryptedSharedPreferences
// + unlocked_this_device makes the blob KeyStore-backed on Android and
// device-bound on iOS.
const _aOptions = AndroidOptions(encryptedSharedPreferences: true);
const _iOptions =
    IOSOptions(accessibility: KeychainAccessibility.unlocked_this_device);

class SecurityService {
  static const _storage = FlutterSecureStorage(
    aOptions: _aOptions,
    iOptions: _iOptions,
  );
  static const _keyDbPass = "phantom_db_password";

  static Future<String> getDatabasePassword() async {
    String? pass = await _storage.read(key: _keyDbPass);
    if (pass == null) {
      pass = _generateRandomPassword();
      await _storage.write(key: _keyDbPass, value: pass);
    }
    return pass;
  }

  static Future<void> clearAll() async {
    await _storage.deleteAll();
  }

  static String _generateRandomPassword() {
    final random = Random.secure();
    final values = List<int>.generate(32, (i) => random.nextInt(256));
    return base64Url.encode(values);
  }
}