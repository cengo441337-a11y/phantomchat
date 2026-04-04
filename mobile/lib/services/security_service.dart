import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'dart:math';
import 'dart:convert';

class SecurityService {
  static const _storage = FlutterSecureStorage();
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