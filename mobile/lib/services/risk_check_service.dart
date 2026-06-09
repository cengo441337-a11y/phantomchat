import 'dart:convert';
import 'dart:io';

/// Argos Pre-Send Risk-Check.
///
/// Wraps the FastAPI stub at `https://argos.dc-infosec.de/api/risk` (real
/// Pylonyx-backed scoring will replace the stub later — same wire shape).
///
/// Usage from the send/swap UI:
///
/// ```dart
/// final v = await RiskCheckService.check(
///   recipient: '86Kt31...',
///   mint: 'EPjFWdd...',
/// );
/// if (v.shouldWarn) showRiskWarning(v);
/// ```
class RiskCheckService {
  static const _endpoint = 'https://argos.dc-infosec.de/api/risk';
  static const _timeout = Duration(seconds: 6);

  /// Combined check for a recipient wallet + the token mint they'll
  /// receive. Returns a [RiskVerdict] — never throws. On network error
  /// we return a "neutral" verdict so the user can still send (failing
  /// closed would block sends every time the backend hiccups).
  static Future<RiskVerdict> check({
    required String recipient,
    String? mint,
  }) async {
    final checks = <Map<String, dynamic>>[
      {'type': 'wallet', 'address': recipient.trim()},
      if (mint != null && mint.trim().isNotEmpty)
        {'type': 'token', 'mint': mint.trim()},
    ];
    try {
      final client = HttpClient()..connectionTimeout = _timeout;
      final req = await client
          .postUrl(Uri.parse(_endpoint))
          .timeout(_timeout);
      req.headers.contentType = ContentType.json;
      req.headers.set('Accept', 'application/json');
      req.add(utf8.encode(jsonEncode({'checks': checks})));
      final resp = await req.close().timeout(_timeout);
      if (resp.statusCode != 200) {
        return RiskVerdict.neutral(
          reason: 'API status ${resp.statusCode}',
        );
      }
      final body = await resp
          .transform(utf8.decoder)
          .join()
          .timeout(_timeout);
      client.close(force: true);
      final json = jsonDecode(body) as Map<String, dynamic>;
      return RiskVerdict.fromJson(json);
    } catch (e) {
      return RiskVerdict.neutral(reason: 'Backend nicht erreichbar: $e');
    }
  }
}

/// Aggregated risk summary across one or more checks.
class RiskVerdict {
  /// 0 = absolut sauber, 100 = bestätigtes Scam.
  final int maxScore;

  /// Aggregated warnings across all checks.
  final List<String> warnings;

  /// Best-effort human-readable label for the highest-risk item.
  final String? label;

  /// `true` if the API responded normally; `false` on network/timeout.
  final bool reachable;

  /// `true` if the recipient mint is in the hardcoded `KNOWN_CLEAN`
  /// allow-list (USDC/USDT/Wrapped-SOL). Surfaces as a green chip.
  final bool trusted;

  const RiskVerdict({
    required this.maxScore,
    required this.warnings,
    required this.label,
    required this.reachable,
    required this.trusted,
  });

  factory RiskVerdict.neutral({String? reason}) => RiskVerdict(
        maxScore: 0,
        warnings: reason == null ? const [] : [reason],
        label: null,
        reachable: false,
        trusted: false,
      );

  factory RiskVerdict.fromJson(Map<String, dynamic> json) {
    final results = (json['results'] as List?) ?? const [];
    int max = 0;
    final warnings = <String>[];
    String? label;
    bool anyTrusted = false;
    for (final r in results) {
      final m = r as Map<String, dynamic>;
      final score = (m['score'] as num?)?.toInt() ?? 0;
      if (score > max) {
        max = score;
        label = (m['metadata'] is Map)
            ? (m['metadata']['name']?.toString())
            : null;
      }
      final w = (m['warnings'] as List?) ?? const [];
      warnings.addAll(w.cast<String>());
      if (m['metadata'] is Map &&
          (m['metadata']['trusted'] == true)) {
        anyTrusted = true;
      }
    }
    return RiskVerdict(
      maxScore: max,
      warnings: warnings,
      label: label,
      reachable: true,
      trusted: anyTrusted,
    );
  }

  /// `true` if the UI should require an explicit "yes, I know what I'm
  /// doing" tap before signing. Anything ≥ 50 OR explicit warning.
  ///  if the UI must pop a confirmation before signing. Includes the
  /// OFFLINE case: if the backend was unreachable we could NOT verify the
  /// recipient, so the user must explicitly acknowledge an unchecked send
  /// rather than the old silent fail-open that looked identical to "clean".
  bool get shouldWarn =>
      !reachable || maxScore >= 50 || warnings.isNotEmpty;

  /// `true` for the green "verified clean" chip on USDC/USDT/wSOL.
  bool get isClean => reachable && trusted && maxScore < 20 && warnings.isEmpty;

  /// Color hint: 'green' (clean), 'amber' (caution), 'red' (warn).
  String get severity {
    if (!reachable) return 'amber'; // unverified != clean
    if (maxScore >= 70) return 'red';
    if (maxScore >= 30 || warnings.isNotEmpty) return 'amber';
    return 'green';
  }

  /// Human-readable one-liner the UI can render directly.
  String get summary {
    if (!reachable) {
      return 'Risk-Check offline — Send möglich, prüfe manuell.';
    }
    if (isClean) {
      return 'Verifizierter Token${label != null ? " · $label" : ""} · Score $maxScore/100.';
    }
    if (maxScore >= 70) {
      return 'WARNUNG · Score $maxScore/100. ${warnings.join(", ")}';
    }
    if (maxScore >= 30 || warnings.isNotEmpty) {
      return 'Vorsicht · Score $maxScore/100. ${warnings.isNotEmpty ? warnings.join(", ") : "Unbekannter Token."}';
    }
    return 'Score $maxScore/100 · unauffällig.';
  }
}
