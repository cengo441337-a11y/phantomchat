enum MessageStatus { sending, sent, delivered, failed }

class PhantomMessage {
  final String id;
  final String contactId;
  final bool outgoing;
  final String plaintext;      // only after decryption / before encryption
  final String ciphertext;     // hex-encoded encrypted payload
  final String ephemeralKey;   // sender's ephemeral public key (hex)
  final String nonce;          // hex
  final DateTime timestamp;
  MessageStatus status;

  PhantomMessage({
    required this.id,
    required this.contactId,
    required this.outgoing,
    required this.plaintext,
    required this.ciphertext,
    required this.ephemeralKey,
    required this.nonce,
    required this.timestamp,
    this.status = MessageStatus.sent,
  });

  Map<String, dynamic> toJson() => {
    'id': id,
    'contactId': contactId,
    'outgoing': outgoing,
    'plaintext': plaintext,
    'ciphertext': ciphertext,
    'ephemeralKey': ephemeralKey,
    'nonce': nonce,
    'timestamp': timestamp.toIso8601String(),
    'status': status.name,
  };

  factory PhantomMessage.fromJson(Map<String, dynamic> j) => PhantomMessage(
    id: j['id'],
    contactId: j['contactId'],
    outgoing: j['outgoing'],
    plaintext: j['plaintext'] ?? '',
    ciphertext: j['ciphertext'] ?? '',
    ephemeralKey: j['ephemeralKey'] ?? '',
    nonce: j['nonce'] ?? '',
    timestamp: DateTime.parse(j['timestamp']),
    status: MessageStatus.values.firstWhere(
      (s) => s.name == j['status'],
      orElse: () => MessageStatus.sent,
    ),
  );
}
