import 'dart:convert';

class PhantomContact {
  final String id;
  final String nickname;
  final String publicViewKey;
  final String publicSpendKey;
  final DateTime addedAt;
  String? lastMessage;
  DateTime? lastMessageAt;

  PhantomContact({
    required this.id,
    required this.nickname,
    required this.publicViewKey,
    required this.publicSpendKey,
    required this.addedAt,
    this.lastMessage,
    this.lastMessageAt,
  });

  static PhantomContact? fromPhantomId(String phantomId, String nickname) {
    try {
      final encoded = phantomId.replaceFirst('phantom:', '');
      final decoded = utf8.decode(base64Url.decode(encoded));
      final json = jsonDecode(decoded) as Map<String, dynamic>;
      return PhantomContact(
        id: json['sk'] as String,
        nickname: nickname,
        publicViewKey: json['vk'] as String,
        publicSpendKey: json['sk'] as String,
        addedAt: DateTime.now(),
      );
    } catch (_) {
      return null;
    }
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'nickname': nickname,
    'publicViewKey': publicViewKey,
    'publicSpendKey': publicSpendKey,
    'addedAt': addedAt.toIso8601String(),
    'lastMessage': lastMessage,
    'lastMessageAt': lastMessageAt?.toIso8601String(),
  };

  factory PhantomContact.fromJson(Map<String, dynamic> j) => PhantomContact(
    id: j['id'],
    nickname: j['nickname'],
    publicViewKey: j['publicViewKey'],
    publicSpendKey: j['publicSpendKey'],
    addedAt: DateTime.parse(j['addedAt']),
    lastMessage: j['lastMessage'],
    lastMessageAt: j['lastMessageAt'] != null ? DateTime.parse(j['lastMessageAt']) : null,
  );
}
