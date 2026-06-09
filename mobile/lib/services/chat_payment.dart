import 'dart:convert';

import 'wallet_service.dart';

/// In-chat crypto payments — encoded as a marker-prefixed plaintext message
/// so they ride the EXISTING sealed-sender E2E pipeline with zero changes to
/// the wire format or the crypto core. A normal text message never starts
/// with the zero-width marker, so detection is unambiguous and invisible.
///
/// Two payload kinds:
/// - `payreq`: "please pay me X of ASSET on CHAIN to ADDR" — the requester's
///   own receive address travels inside the encrypted message, so no wallet
///   directory / address-resolution service is needed (fully E2E).
/// - `paid`: "I sent you X of ASSET — here's the tx signature" — a receipt
///   the other side renders as a confirmation card with an explorer link.
class ChatPayment {
  /// Zero-width-space sentinel + version tag. Invisible in any plain-text
  /// renderer, so a client that doesn't understand payments just shows an
  /// empty-ish bubble rather than garbage (older clients).
  static const marker = '​ARGOSPAY1​';

  final String kind; // 'payreq' | 'paid'
  final String chain; // ArgosChain.backendId
  final String asset; // display symbol, e.g. USDC / SOL / ETH
  final String amount; // human decimal string, e.g. "5" or "0.25"
  final String? address; // payreq: requester receive address
  final String? signature; // paid: tx signature / hash

  const ChatPayment({
    required this.kind,
    required this.chain,
    required this.asset,
    required this.amount,
    this.address,
    this.signature,
  });

  /// True if [text] is a payment payload (vs a normal chat message).
  static bool isPayment(String text) => text.startsWith(marker);

  /// Encode to a marker-prefixed wire string.
  String encode() {
    final m = <String, dynamic>{
      't': kind,
      'c': chain,
      'a': asset,
      'm': amount,
      if (address != null) 'addr': address,
      if (signature != null) 'sig': signature,
    };
    return '$marker${jsonEncode(m)}';
  }

  /// Decode a marker-prefixed wire string. Returns null on any malformed
  /// payload so the chat can fall back to rendering it as plain text.
  static ChatPayment? decode(String text) {
    if (!isPayment(text)) return null;
    try {
      final json = jsonDecode(text.substring(marker.length));
      if (json is! Map) return null;
      final kind = json['t'] as String?;
      if (kind != 'payreq' && kind != 'paid') return null;
      return ChatPayment(
        kind: kind!,
        chain: (json['c'] as String?) ?? 'mainnet-beta',
        asset: (json['a'] as String?) ?? 'SOL',
        amount: (json['m'] as String?) ?? '0',
        address: json['addr'] as String?,
        signature: json['sig'] as String?,
      );
    } catch (_) {
      return null;
    }
  }

  /// Friendly one-line summary for the contact-list "last message" preview.
  String get summary => kind == 'payreq'
      ? '\u{1F4B8} Zahlung angefordert · $amount $asset'
      : '✅ $amount $asset gesendet';

  ArgosChain? get parsedChain {
    for (final c in ArgosChain.values) {
      if (c.backendId == chain) return c;
    }
    return null;
  }

  /// Explorer URL for a `paid` receipt.
  String? explorerUrl() {
    final c = parsedChain;
    if (c == null || signature == null) return null;
    switch (c) {
      case ArgosChain.solanaMainnet:
        return 'https://solscan.io/tx/$signature';
      case ArgosChain.solanaDevnet:
        return 'https://solscan.io/tx/$signature?cluster=devnet';
      case ArgosChain.ethereum:
        return 'https://etherscan.io/tx/$signature';
      case ArgosChain.base:
        return 'https://basescan.org/tx/$signature';
      case ArgosChain.polygon:
        return 'https://polygonscan.com/tx/$signature';
    }
  }
}
