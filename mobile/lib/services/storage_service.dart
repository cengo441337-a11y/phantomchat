import 'dart:convert';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:shared_preferences/shared_preferences.dart';
import '../models/identity.dart';
import '../models/contact.dart';
import '../models/message.dart';

class StorageService {
  static const _storage = FlutterSecureStorage();
  static const _identityKey = 'phantom_identity';
  static const _contactsKey = 'phantom_contacts';
  static const _messagesPrefix = 'msgs_';

  // Identity
  static Future<void> saveIdentity(PhantomIdentity identity) async {
    await _storage.write(
      key: _identityKey,
      value: jsonEncode(identity.toJson()),
    );
  }

  static Future<PhantomIdentity?> loadIdentity() async {
    final raw = await _storage.read(key: _identityKey);
    if (raw == null) return null;
    return PhantomIdentity.fromJson(jsonDecode(raw));
  }

  static Future<bool> hasIdentity() async {
    final val = await _storage.read(key: _identityKey);
    return val != null;
  }

  // Contacts
  static Future<void> saveContacts(List<PhantomContact> contacts) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(
      _contactsKey,
      jsonEncode(contacts.map((c) => c.toJson()).toList()),
    );
  }

  static Future<List<PhantomContact>> loadContacts() async {
    final prefs = await SharedPreferences.getInstance();
    final raw = prefs.getString(_contactsKey);
    if (raw == null) return [];
    final list = jsonDecode(raw) as List;
    return list.map((j) => PhantomContact.fromJson(j as Map<String, dynamic>)).toList();
  }

  // Messages
  static Future<void> saveMessages(
    String contactId,
    List<PhantomMessage> messages,
  ) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(
      '$_messagesPrefix$contactId',
      jsonEncode(messages.map((m) => m.toJson()).toList()),
    );
  }

  static Future<List<PhantomMessage>> loadMessages(String contactId) async {
    final prefs = await SharedPreferences.getInstance();
    final raw = prefs.getString('$_messagesPrefix$contactId');
    if (raw == null) return [];
    final list = jsonDecode(raw) as List;
    return list
        .map((j) => PhantomMessage.fromJson(j as Map<String, dynamic>))
        .toList();
  }

  static Future<void> addMessage(PhantomMessage message) async {
    final messages = await loadMessages(message.contactId);
    messages.add(message);
    await saveMessages(message.contactId, messages);
  }

  /// Erase every piece of PhantomChat state from the device. Called by the
  /// panic-wipe path in [AppLockService] after too many failed PINs and by
  /// the manual "Wipe device" button in the security settings.
  ///
  /// Scrubs, in order:
  /// 1. The identity blob from `FlutterSecureStorage`.
  /// 2. Contacts + all per-contact message histories from `SharedPreferences`.
  static Future<void> wipe() async {
    await _storage.delete(key: _identityKey);

    final prefs = await SharedPreferences.getInstance();
    await prefs.remove(_contactsKey);
    final msgKeys = prefs.getKeys().where((k) => k.startsWith(_messagesPrefix));
    for (final k in msgKeys) {
      await prefs.remove(k);
    }
  }
}
