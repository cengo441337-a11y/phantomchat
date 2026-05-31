import 'dart:async';
import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

/// Persistent record of the last successful update-manifest check.
///
/// Lives in `SharedPreferences` so it survives app restarts. Used by:
/// - the persistent "update available" banner on the home screen,
/// - the always-visible Update-Status block in Settings,
/// - the direct-download fallback when the live manifest fetch fails.
///
/// We persist ENOUGH information that the UI can still render a useful
/// "tap to install" link even when the device is currently offline.
class UpdateStateStore {
  static const _kKey = 'argos.update.state.v1';

  static Future<UpdateState> load() async {
    try {
      final p = await SharedPreferences.getInstance();
      final raw = p.getString(_kKey);
      if (raw == null) return UpdateState.empty();
      final json = jsonDecode(raw);
      if (json is! Map<String, dynamic>) return UpdateState.empty();
      return UpdateState.fromJson(json);
    } catch (_) {
      return UpdateState.empty();
    }
  }

  static Future<void> save(UpdateState state) async {
    try {
      final p = await SharedPreferences.getInstance();
      await p.setString(_kKey, jsonEncode(state.toJson()));
    } catch (_) {
      // Persistence is best-effort. A failed write just means next launch
      // sees an older snapshot — the in-memory state is still valid.
    }
  }

  /// Wipe the persisted snapshot — used when the user opts out of update
  /// checks or when we want to force a fresh discovery cycle on next start.
  static Future<void> clear() async {
    try {
      final p = await SharedPreferences.getInstance();
      await p.remove(_kKey);
    } catch (_) {}
  }
}

/// Snapshot of the last update-manifest check.
class UpdateState {
  /// Unix milliseconds when the last check completed, regardless of outcome.
  /// 0 means "never checked".
  final int lastCheckedAtMs;

  /// Free-text outcome label — `"latest"`, `"updateAvailable"`,
  /// `"unreachable"`, etc. Mirrors `UpdateCheckOutcome.name` so the UI can
  /// switch on it without re-importing the service enum.
  final String lastOutcome;

  /// Latest manifest version we ever saw on this device. Surface this in
  /// the UI even when the most recent fetch failed, so the user still has
  /// a believable "v1.2.X is out there" reference.
  final String? lastKnownManifestVersion;

  /// Latest manifest version_code we ever saw. Same idea as the version
  /// string but bump-protected.
  final int? lastKnownManifestCode;

  /// Latest known direct-download URL for this device's ABI. Saved so the
  /// "manual install" fallback button can still link to a real APK even
  /// when the manifest endpoint is currently down.
  final String? lastKnownDownloadUrl;

  /// Free-text reason string when `lastOutcome == "unreachable"`. Helps
  /// surface "connection timed out", "HTTP 502", etc. in Settings.
  final String? lastErrorReason;

  const UpdateState({
    required this.lastCheckedAtMs,
    required this.lastOutcome,
    this.lastKnownManifestVersion,
    this.lastKnownManifestCode,
    this.lastKnownDownloadUrl,
    this.lastErrorReason,
  });

  factory UpdateState.empty() => const UpdateState(
        lastCheckedAtMs: 0,
        lastOutcome: 'never',
      );

  factory UpdateState.fromJson(Map<String, dynamic> json) => UpdateState(
        lastCheckedAtMs: (json['lastCheckedAtMs'] as num?)?.toInt() ?? 0,
        lastOutcome: (json['lastOutcome'] as String?) ?? 'never',
        lastKnownManifestVersion:
            json['lastKnownManifestVersion'] as String?,
        lastKnownManifestCode:
            (json['lastKnownManifestCode'] as num?)?.toInt(),
        lastKnownDownloadUrl: json['lastKnownDownloadUrl'] as String?,
        lastErrorReason: json['lastErrorReason'] as String?,
      );

  Map<String, dynamic> toJson() => {
        'lastCheckedAtMs': lastCheckedAtMs,
        'lastOutcome': lastOutcome,
        if (lastKnownManifestVersion != null)
          'lastKnownManifestVersion': lastKnownManifestVersion,
        if (lastKnownManifestCode != null)
          'lastKnownManifestCode': lastKnownManifestCode,
        if (lastKnownDownloadUrl != null)
          'lastKnownDownloadUrl': lastKnownDownloadUrl,
        if (lastErrorReason != null) 'lastErrorReason': lastErrorReason,
      };

  bool get isNeverChecked => lastCheckedAtMs == 0;

  /// True when the most recent check actually reached the server, regardless
  /// of whether an update was available. Lets the UI decide whether the
  /// "last known" data is fresh (≤ 24 h) vs stale.
  bool get lastCheckSucceeded =>
      lastOutcome == 'latest' || lastOutcome == 'updateAvailable';

  /// Human-readable "checked X ago" for the Settings status block.
  String formatChecked() {
    if (isNeverChecked) return 'noch nie';
    final nowMs = DateTime.now().millisecondsSinceEpoch;
    final diffMs = (nowMs - lastCheckedAtMs).abs();
    final mins = diffMs ~/ 60000;
    if (mins < 1) return 'vor wenigen Sekunden';
    if (mins < 60) return 'vor $mins min';
    final hours = mins ~/ 60;
    if (hours < 24) return 'vor ${hours}h';
    final days = hours ~/ 24;
    return 'vor ${days}d';
  }
}
