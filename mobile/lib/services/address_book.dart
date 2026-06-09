import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Named recipients ("address book"). Stored locally in secure storage —
/// addresses are not secret, but keeping them off plaintext prefs avoids a
/// trivial backup-scrape and matches where the wallet keeps its other
/// metadata. Entries are chain-tagged so the send sheet can offer only the
/// addresses valid for the active chain.
class AddressBook {
  static const _secure = FlutterSecureStorage(
    aOptions: AndroidOptions(encryptedSharedPreferences: true),
  );
  static const _key = 'argos.addressbook.v1';

  static List<AddressEntry>? _cache;

  static Future<List<AddressEntry>> all() async {
    if (_cache != null) return _cache!;
    try {
      final raw = await _secure.read(key: _key);
      if (raw == null) {
        _cache = [];
        return _cache!;
      }
      final list = jsonDecode(raw);
      _cache = (list as List)
          .map((e) => AddressEntry.fromJson(e as Map<String, dynamic>))
          .toList();
    } catch (_) {
      _cache = [];
    }
    return _cache!;
  }

  /// Entries usable on [chainBackendId]: chain-specific ones plus any tagged
  /// "any". Solana and EVM address formats differ, so we filter by family.
  static Future<List<AddressEntry>> forChain(String chainBackendId) async {
    final isSolana = chainBackendId == 'mainnet-beta' || chainBackendId == 'devnet';
    final entries = await all();
    return entries.where((e) {
      if (e.chain == 'any') return true;
      final eSolana = e.chain == 'mainnet-beta' || e.chain == 'devnet';
      // Same family (both Solana or both EVM) is enough — an EVM address
      // works across Ethereum/Base/Polygon.
      return eSolana == isSolana;
    }).toList();
  }

  static Future<void> add(AddressEntry entry) async {
    final entries = await all();
    // De-dupe by (address, chain).
    entries.removeWhere(
        (e) => e.address == entry.address && e.chain == entry.chain);
    entries.add(entry);
    await _persist(entries);
  }

  static Future<void> remove(AddressEntry entry) async {
    final entries = await all();
    entries.removeWhere(
        (e) => e.address == entry.address && e.chain == entry.chain);
    await _persist(entries);
  }

  /// Returns the saved name for an address, or null.
  static Future<String?> nameFor(String address) async {
    final entries = await all();
    for (final e in entries) {
      if (e.address == address) return e.name;
    }
    return null;
  }

  static Future<void> _persist(List<AddressEntry> entries) async {
    _cache = entries;
    await _secure.write(
      key: _key,
      value: jsonEncode(entries.map((e) => e.toJson()).toList()),
    );
  }
}

class AddressEntry {
  final String name;
  final String address;

  /// Chain backend id, or 'any' for an address valid across a family.
  final String chain;

  const AddressEntry({
    required this.name,
    required this.address,
    required this.chain,
  });

  factory AddressEntry.fromJson(Map<String, dynamic> j) => AddressEntry(
        name: j['name'] as String? ?? '',
        address: j['address'] as String? ?? '',
        chain: j['chain'] as String? ?? 'any',
      );

  Map<String, dynamic> toJson() => {
        'name': name,
        'address': address,
        'chain': chain,
      };
}
