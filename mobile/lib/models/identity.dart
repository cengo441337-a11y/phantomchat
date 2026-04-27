class PhantomIdentity {
  final String id;
  final String nickname;
  final String privateViewKey;
  final String publicViewKey;
  final String privateSpendKey;
  final String publicSpendKey;
  /// Ed25519 signing seed (32-byte hex) used by the sealed-sender-v3
  /// send path. `null` for legacy installs created before 1.0.7 — those
  /// upgrade-migrate at boot (see `main.dart`'s identity-load step): a
  /// fresh seed is generated and the identity record is rewritten so
  /// subsequent launches behave like a clean install. Without this the
  /// Rust core's `LOCAL_SIGN` slot stays empty and every `sendSealedV3`
  /// fails with "signing key not loaded — call load_local_identity_v3
  /// first".
  final String? privateSigningKey;
  final DateTime createdAt;

  const PhantomIdentity({
    required this.id,
    required this.nickname,
    required this.privateViewKey,
    required this.publicViewKey,
    required this.privateSpendKey,
    required this.publicSpendKey,
    required this.createdAt,
    this.privateSigningKey,
  });

  /// Returns a copy with the supplied fields overridden. Used by the
  /// boot-time migration in `main.dart` to fill in the missing signing
  /// seed for pre-1.0.7 identities.
  PhantomIdentity copyWith({String? privateSigningKey}) => PhantomIdentity(
    id: id,
    nickname: nickname,
    privateViewKey: privateViewKey,
    publicViewKey: publicViewKey,
    privateSpendKey: privateSpendKey,
    publicSpendKey: publicSpendKey,
    createdAt: createdAt,
    privateSigningKey: privateSigningKey ?? this.privateSigningKey,
  );

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
    'privateSigningKey': privateSigningKey,
    'createdAt': createdAt.toIso8601String(),
  };

  factory PhantomIdentity.fromJson(Map<String, dynamic> j) => PhantomIdentity(
    id: j['id'],
    nickname: j['nickname'],
    privateViewKey: j['privateViewKey'],
    publicViewKey: j['publicViewKey'],
    privateSpendKey: j['privateSpendKey'],
    publicSpendKey: j['publicSpendKey'],
    privateSigningKey: j['privateSigningKey'] as String?,
    createdAt: DateTime.parse(j['createdAt']),
  );
}
