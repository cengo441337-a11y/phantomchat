import 'dart:convert';
import 'dart:io';

/// Argos Pro status — checks the Pylonyx backend for an active 4 EUR/Mo
/// subscription keyed by the wallet's Solana address. Cached 5 min so the
/// swap sheet can read it synchronously after one async refresh.
class ProService {
  static const _base = 'https://pylonyx-dev.dc-infosec.de';
  static const _timeout = Duration(seconds: 8);

  /// Reduced swap fee in basis points for Pro users (matches Rust PRO_FEE_BPS).
  static const proFeeBps = 25;

  /// Standard swap fee in basis points (matches Rust PLATFORM_FEE_BPS).
  static const standardFeeBps = 50;

  static bool _isPro = false;
  static DateTime? _checkedAt;

  /// Last-known Pro flag (synchronous). Refresh via [refresh].
  static bool get isPro => _isPro;

  /// The swap fee bps to use for the current Pro state.
  static int get swapFeeBps => _isPro ? proFeeBps : standardFeeBps;

  /// Re-query Pro status for [address]. Returns the new flag. Best-effort:
  /// on network error keeps the last-known value.
  static Future<bool> refresh(String address) async {
    if (_checkedAt != null &&
        DateTime.now().difference(_checkedAt!).inMinutes < 5) {
      return _isPro;
    }
    try {
      final client = HttpClient()..connectionTimeout = _timeout;
      final req = await client
          .getUrl(Uri.parse(
              '$_base/api/argos/pro/status?addr=${Uri.encodeComponent(address)}'))
          .timeout(_timeout);
      final res = await req.close().timeout(_timeout);
      if (res.statusCode == 200) {
        final body = await res.transform(utf8.decoder).join();
        client.close(force: true);
        final json = jsonDecode(body) as Map<String, dynamic>;
        _isPro = json['pro'] == true;
        _checkedAt = DateTime.now();
      }
    } catch (_) {
      // keep last-known
    }
    return _isPro;
  }

  /// The Stripe-checkout URL to subscribe the given address.
  static String checkoutUrl() => '$_base/argos/pro';

  /// POST the address to start a checkout; returns the Stripe URL or null.
  static Future<String?> startCheckout(String address) async {
    try {
      final client = HttpClient()..connectionTimeout = _timeout;
      final req = await client
          .postUrl(Uri.parse('$_base/api/argos/pro/checkout'))
          .timeout(_timeout);
      req.headers.contentType = ContentType.json;
      req.add(utf8.encode(jsonEncode({'address': address})));
      final res = await req.close().timeout(_timeout);
      if (res.statusCode != 200) return null;
      final body = await res.transform(utf8.decoder).join();
      client.close(force: true);
      final json = jsonDecode(body) as Map<String, dynamic>;
      return json['url'] as String?;
    } catch (_) {
      return null;
    }
  }
}
