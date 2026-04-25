// V3 contact directory — companion to the existing PhantomContact /
// StorageService stack. Adds the `signing_pub_hex` field needed for
// sealed-sender attribution (matches the Tauri Desktop's
// `Contact.signing_pub` JSON field). Stored in `SharedPreferences` under
// a separate key so it can evolve without touching the v2 contact list.

import 'dart:convert';
import 'package:shared_preferences/shared_preferences.dart';

class V3Contact {
  final String label;
  final String address;
  String? signingPubHex;

  V3Contact({
    required this.label,
    required this.address,
    this.signingPubHex,
  });

  Map<String, dynamic> toJson() => {
        'label': label,
        'address': address,
        if (signingPubHex != null) 'signing_pub': signingPubHex,
      };

  factory V3Contact.fromJson(Map<String, dynamic> j) => V3Contact(
        label: j['label'] as String,
        address: j['address'] as String,
        signingPubHex: j['signing_pub'] as String?,
      );
}

/// Persistent contact book used by the v3 sealed-sender attribution path.
/// Mirrors the desktop `contacts.json` on-disk shape so a future shared-
/// storage path can read either.
class ContactDirectory {
  static const _kKey = 'phantom_v3_contacts';

  /// Mirrors the Tauri AppState.last_unbound_sender slot. Set by the relay
  /// listener whenever an envelope arrives with a sealed-sender pubkey
  /// that doesn't match any contact's `signing_pub_hex`. Consumed by
  /// [bindLastUnboundSender] when the user picks a contact to attach it
  /// to. In-process, NOT persisted (matches Desktop's RAM-only handling).
  static String? lastUnboundSenderPubHex;

  static Future<List<V3Contact>> load() async {
    final prefs = await SharedPreferences.getInstance();
    final raw = prefs.getString(_kKey);
    if (raw == null) return <V3Contact>[];
    final list = jsonDecode(raw) as List<dynamic>;
    return list
        .map((j) => V3Contact.fromJson(j as Map<String, dynamic>))
        .toList();
  }

  static Future<void> save(List<V3Contact> contacts) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_kKey,
        jsonEncode(contacts.map((c) => c.toJson()).toList()));
  }

  /// Add a contact. Returns the post-write list. Idempotent on `label`.
  static Future<List<V3Contact>> upsert(V3Contact c) async {
    final list = await load();
    final idx = list.indexWhere((x) => x.label == c.label);
    if (idx >= 0) {
      list[idx] = c;
    } else {
      list.add(c);
    }
    await save(list);
    return list;
  }

  /// Bind the most-recently-seen unbound sealed-sender pubkey to
  /// `contactLabel`. Mirrors Desktop's `bind_last_unbound_sender` Tauri
  /// command — the slot is consumed on success so a double-tap can't
  /// bind the same key to two contacts.
  static Future<({bool ok, String? error})> bindLastUnboundSender(
      String contactLabel) async {
    final hex = lastUnboundSenderPubHex;
    if (hex == null) {
      return (
        ok: false,
        error:
            'no unbound sender pending — wait for an incoming sealed message tagged ?<hex>',
      );
    }
    final list = await load();
    final idx = list.indexWhere((c) => c.label == contactLabel);
    if (idx < 0) {
      return (ok: false, error: "unknown contact '$contactLabel'");
    }
    list[idx].signingPubHex = hex;
    await save(list);
    lastUnboundSenderPubHex = null;
    return (ok: true, error: null);
  }
}
