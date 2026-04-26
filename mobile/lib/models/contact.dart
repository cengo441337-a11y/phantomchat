import 'dart:convert';

class PhantomContact {
  final String id;
  final String nickname;
  final String publicViewKey;
  final String publicSpendKey;
  final DateTime addedAt;
  /// Optional ML-KEM (Kyber-768) public key, base64. Present when the
  /// contact was imported from a `phantomx:` (PQXDH-hybrid) address; null
  /// for legacy classic-only `phantom:` contacts. Persisted in toJson /
  /// fromJson so the mobile send-path can reconstruct the hybrid address
  /// for `sendSealedV3` and avoid silently downgrading to X25519-only.
  String? mlkemPubB64;
  String? lastMessage;
  DateTime? lastMessageAt;

  PhantomContact({
    required this.id,
    required this.nickname,
    required this.publicViewKey,
    required this.publicSpendKey,
    required this.addedAt,
    this.mlkemPubB64,
    this.lastMessage,
    this.lastMessageAt,
  });

  /// Parse a phantom address string. Accepts both the canonical Desktop
  /// wire format (since v3) and the legacy mobile-only base64-JSON form
  /// emitted by older mobile builds — that way an existing mobile↔mobile
  /// QR code keeps working AND mobile can finally add a Desktop-generated
  /// address.
  ///
  ///   New: `phantom:<view_hex>:<spend_hex>`
  ///        (matches `core::address::PhantomAddress::Display` on the Rust
  ///        side; what `get_address` Tauri command returns)
  ///   Hybrid: `phantomx:<view_hex>:<spend_hex>:<mlkem_b64>`
  ///        (PQXDH path; we store view+spend, drop the mlkem since the
  ///        mobile send-path doesn't yet emit hybrid envelopes)
  ///   Legacy: `phantom:<base64url-of-JSON-{vk,sk,n}>`
  static PhantomContact? fromPhantomId(String phantomId, String nickname) {
    final trimmed = phantomId.trim();

    // Hybrid `phantomx:` — strip prefix, treat first two colon-separated
    // parts as view/spend hex. The optional 3rd segment is the ML-KEM
    // (Kyber-768) public key in base64 — keep it so the send-path can
    // build a `phantomx:` recipient address for PQXDH-hybrid sealed-
    // sender. Without this we silently downgrade to classic X25519.
    if (trimmed.startsWith('phantomx:')) {
      final rest = trimmed.substring('phantomx:'.length);
      final parts = rest.split(':');
      if (parts.length >= 2 &&
          _isHex(parts[0], 64) &&
          _isHex(parts[1], 64)) {
        String? mlkem;
        if (parts.length >= 3 && parts[2].isNotEmpty) {
          mlkem = parts[2];
        }
        return PhantomContact(
          id: parts[1],
          nickname: nickname,
          publicViewKey: parts[0],
          publicSpendKey: parts[1],
          addedAt: DateTime.now(),
          mlkemPubB64: mlkem,
        );
      }
      return null;
    }

    final body = trimmed.startsWith('phantom:')
        ? trimmed.substring('phantom:'.length)
        : trimmed;

    // New colon-hex format: phantom:<view_hex>:<spend_hex>
    final colonParts = body.split(':');
    if (colonParts.length == 2 &&
        _isHex(colonParts[0], 64) &&
        _isHex(colonParts[1], 64)) {
      return PhantomContact(
        id: colonParts[1],
        nickname: nickname,
        publicViewKey: colonParts[0],
        publicSpendKey: colonParts[1],
        addedAt: DateTime.now(),
      );
    }

    // Legacy base64-JSON fallback so existing mobile↔mobile QR codes still
    // import. New mobile builds emit the colon-hex format above.
    try {
      final decoded = utf8.decode(base64Url.decode(body));
      final json = jsonDecode(decoded) as Map<String, dynamic>;
      final vk = json['vk'] as String?;
      final sk = json['sk'] as String?;
      if (vk == null || sk == null) return null;
      return PhantomContact(
        id: sk,
        nickname: nickname,
        publicViewKey: vk,
        publicSpendKey: sk,
        addedAt: DateTime.now(),
      );
    } catch (_) {
      return null;
    }
  }

  static bool _isHex(String s, int expectedLen) {
    if (s.length != expectedLen) return false;
    for (final c in s.codeUnits) {
      final isDigit = c >= 0x30 && c <= 0x39;
      final isLowAF = c >= 0x61 && c <= 0x66;
      final isUpAF = c >= 0x41 && c <= 0x46;
      if (!isDigit && !isLowAF && !isUpAF) return false;
    }
    return true;
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'nickname': nickname,
    'publicViewKey': publicViewKey,
    'publicSpendKey': publicSpendKey,
    'addedAt': addedAt.toIso8601String(),
    'mlkemPubB64': mlkemPubB64,
    'lastMessage': lastMessage,
    'lastMessageAt': lastMessageAt?.toIso8601String(),
  };

  factory PhantomContact.fromJson(Map<String, dynamic> j) => PhantomContact(
    id: j['id'],
    nickname: j['nickname'],
    publicViewKey: j['publicViewKey'],
    publicSpendKey: j['publicSpendKey'],
    addedAt: DateTime.parse(j['addedAt']),
    mlkemPubB64: j['mlkemPubB64'] as String?,
    lastMessage: j['lastMessage'],
    lastMessageAt: j['lastMessageAt'] != null ? DateTime.parse(j['lastMessageAt']) : null,
  );
}
