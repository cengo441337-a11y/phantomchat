import 'dart:convert';

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

  String get phantomId {
    final data = jsonEncode({
      'vk': publicViewKey,
      'sk': publicSpendKey,
      'n': nickname,
    });
    return 'phantom:${base64Url.encode(utf8.encode(data))}';
  }

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
