import 'dart:convert';
import 'dart:io';

import 'wallet_service.dart';

/// Fetches EUR spot prices from CoinGecko (free, no key) and computes the
/// fiat value of a chain's holdings. Best-effort — returns null on any
/// network/parse error so the UI just hides the value instead of erroring.
class PriceService {
  static const _base =
      'https://api.coingecko.com/api/v3/simple/price';
  static const _timeout = Duration(seconds: 8);

  // CoinGecko ids per native coin.
  static const _nativeIds = {
    'SOL': 'solana',
    'ETH': 'ethereum',
    'MATIC': 'polygon-ecosystem-token',
  };

  // Stablecoin CoinGecko ids (so we don't hardcode "1 USD" and ignore the
  // EUR/USD rate — USDC is ~0.92 EUR, not 1).
  static const _stableIds = {
    'USDC': 'usd-coin',
    'USDT': 'tether',
  };

  static Map<String, double>? _cache;
  static DateTime? _cacheAt;

  /// EUR prices for all symbols we might display, cached 60 s. Keyed by the
  /// display symbol (SOL/ETH/MATIC/USDC/USDT).
  static Future<Map<String, double>?> eurPrices() async {
    final now = DateTime.now();
    if (_cache != null &&
        _cacheAt != null &&
        now.difference(_cacheAt!).inSeconds < 60) {
      return _cache;
    }
    final ids = {..._nativeIds, ..._stableIds};
    final idParam = ids.values.toSet().join(',');
    final url = '$_base?ids=$idParam&vs_currencies=eur';
    try {
      final client = HttpClient()..connectionTimeout = _timeout;
      final req = await client.getUrl(Uri.parse(url)).timeout(_timeout);
      final res = await req.close().timeout(_timeout);
      if (res.statusCode != 200) return _cache;
      final body =
          await res.transform(utf8.decoder).join().timeout(_timeout);
      client.close(force: true);
      final json = jsonDecode(body) as Map<String, dynamic>;
      final out = <String, double>{};
      ids.forEach((symbol, cgId) {
        final entry = json[cgId];
        if (entry is Map && entry['eur'] is num) {
          out[symbol] = (entry['eur'] as num).toDouble();
        }
      });
      _cache = out;
      _cacheAt = now;
      return out;
    } catch (_) {
      return _cache;
    }
  }

  /// EUR value of the active chain's holdings. `nativeHuman` is the native
  /// balance as a decimal value; `tokenHuman` maps display-symbol → decimal
  /// balance. Returns null if prices couldn't be fetched.
  static Future<double?> portfolioEur({
    required ArgosChain chain,
    required double nativeHuman,
    required Map<String, double> tokenHuman,
  }) async {
    final prices = await eurPrices();
    if (prices == null) return null;
    final nativeSym = switch (chain) {
      ArgosChain.solanaMainnet || ArgosChain.solanaDevnet => 'SOL',
      ArgosChain.ethereum || ArgosChain.base => 'ETH',
      ArgosChain.polygon => 'MATIC',
    };
    var total = (prices[nativeSym] ?? 0) * nativeHuman;
    tokenHuman.forEach((sym, amount) {
      total += (prices[sym] ?? 0) * amount;
    });
    return total;
  }
}
