class PhantomIdentity {
  final String id;
  final String nickname;
  final String privateViewKey;
  final String publicViewKey;
  final String privateSpendKey;
  final String publicSpendKey;
  final DateTime createdAt;

  const PhantomIdentity({
    required this.id,
    required this.nickname,
    required this.privateViewKey,
    required this.publicViewKey,
    required this.privateSpendKey,
    required this.publicSpendKey,
    required this.createdAt,
  });

  /// Canonical phantom-address wire form. Matches the Rust
  /// `core::address::PhantomAddress::Display` on the Desktop side, so a
  /// scan of this QR by Desktop's "Add contact" flow imports cleanly.
  /// The legacy `phantom:<base64-JSON>` form is no longer emitted (was
  /// only ever consumed by mobile↔mobile QR exchanges, and the
  /// `fromPhantomId` parser still accepts it as a backwards-compat
  /// fallback so older shared QR codes keep working).
  String get phantomId => 'phantom:$publicViewKey:$publicSpendKey';

  Map<String, dynamic> toJson() => {
    'id': id,
    'nickname': nickname,
    'privateViewKey': privateViewKey,
    'publicViewKey': publicViewKey,
    'privateSpendKey': privateSpendKey,
    'publicSpendKey': publicSpendKey,
    'createdAt': createdAt.toIso8601String(),
  };

  factory PhantomIdentity.fromJson(Map<String, dynamic> j) => PhantomIdentity(
    id: j['id'],
    nickname: j['nickname'],
    privateViewKey: j['privateViewKey'],
    publicViewKey: j['publicViewKey'],
    privateSpendKey: j['privateSpendKey'],
    publicSpendKey: j['publicSpendKey'],
    createdAt: DateTime.parse(j['createdAt']),
  );
}
