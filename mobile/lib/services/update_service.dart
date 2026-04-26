// Wave 11G — in-app APK auto-update.
//
// Sideloaded PhantomChat builds (we publish APKs at
// https://updates.dc-infosec.de/download/) have no Play-Store auto-update,
// so customers stay pinned to whatever they originally installed. This
// service implements a manifest-driven self-update flow:
//
//   1. On startup the home screen calls `checkForUpdate()`.
//      We GET https://updates.dc-infosec.de/phantomchat/android/latest.json
//      with a short timeout. Any fetch / parse failure is swallowed and
//      returns `null` — the banner just doesn't appear, the app keeps
//      working offline. NEVER throw past this method into the boot path.
//
//   2. If the manifest's `version` is strictly greater than the installed
//      version (read via `package_info_plus` from pubspec.yaml at compile
//      time), we return an `UpdateInfo` describing the matching ABI's
//      download URL, sha256 and size. The home screen surfaces a banner.
//
//   3. User taps Download → `downloadApk()` streams the APK to the app's
//      external cache dir, hashing as it goes. Mismatched SHA256 throws
//      and the partial file is deleted — this is the MITM mitigation:
//      the manifest is the trust root (it lives behind HTTPS on the same
//      host), the APK download URL is verified against it.
//
//   4. `installApk()` hands the verified file to `open_filex`, which wraps
//      it in a content:// URI via our FileProvider and fires an
//      ACTION_VIEW Intent with mime application/vnd.android.package-archive.
//      Android shows the package-installer; if "Install unknown apps" is
//      not yet granted for our package, the system handles that prompt.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:device_info_plus/device_info_plus.dart';
import 'package:open_filex/open_filex.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:path_provider/path_provider.dart';

/// URL of the manifest JSON. Kept as a top-level const so a future build
/// flag (e.g. staging vs production) can swap it without code changes
/// inside the service.
const String kUpdateManifestUrl =
    'https://updates.dc-infosec.de/phantomchat/android/latest.json';

/// Origin pin: every variant.url returned in the manifest MUST start with
/// this prefix. Any deviation (different host, plain http://, other path)
/// causes [UpdateService.checkForUpdate] to drop the manifest. This is the
/// last line of defence if the manifest host is somehow MITMed before TLS
/// validation kicks in — even with a tampered manifest the attacker can't
/// redirect the APK fetch to their own bucket.
const String kUpdateOriginPrefix = 'https://updates.dc-infosec.de/';

/// How long to wait for the manifest before we give up. Short, because
/// this runs in the boot path and we never want to delay app start.
const Duration kManifestTimeout = Duration(seconds: 6);

/// Callback fired during APK download — receives `(bytesReceived, totalBytes)`.
/// `totalBytes` may be `-1` if the server didn't send Content-Length, but
/// the manifest's `size_bytes` is always authoritative.
typedef ProgressCallback = void Function(int received, int total);

/// One ABI variant inside the manifest's `abis` map.
///
/// `versionCode` is the Android build-number (matches `versionCode` in
/// AndroidManifest.xml and `PackageInfo.buildNumber` from
/// `package_info_plus`). It is checked alongside the semver `version`
/// string to defend against bump-protection attacks where an attacker
/// re-publishes an OLD signed APK under a NEW version label.
///
/// `signature` is reserved for a future minisign-style signature over the
/// manifest entry. Currently only logged — full verification (Wave TBD)
/// will use the `ed25519` Dart package and a baked-in pubkey.
class AbiVariant {
  final String url;
  final String sha256;
  final int sizeBytes;
  final int versionCode;
  final String? signature;

  const AbiVariant({
    required this.url,
    required this.sha256,
    required this.sizeBytes,
    required this.versionCode,
    this.signature,
  });

  factory AbiVariant.fromJson(Map<String, dynamic> json) {
    return AbiVariant(
      url: json['url'] as String,
      sha256: (json['sha256'] as String).toLowerCase(),
      sizeBytes: json['size_bytes'] as int,
      versionCode: json['version_code'] as int,
      signature: json['signature'] as String?,
    );
  }
}

/// Parsed `latest.json` plus the ABI we picked for *this* device. Returned
/// to the UI only when the manifest's version is newer than the installed
/// version AND a matching ABI variant exists.
class UpdateInfo {
  final String currentVersion;
  final String newVersion;
  final String releasedAt;
  final String notes;
  final String abi;
  final AbiVariant variant;

  const UpdateInfo({
    required this.currentVersion,
    required this.newVersion,
    required this.releasedAt,
    required this.notes,
    required this.abi,
    required this.variant,
  });
}

class UpdateService {
  /// Fetches the manifest, compares versions, returns an `UpdateInfo` if
  /// an upgrade is available for the current device's ABI. Returns `null`
  /// if up-to-date, the device's ABI is missing from the manifest, or the
  /// fetch / parse failed for any reason.
  static Future<UpdateInfo?> checkForUpdate() async {
    try {
      final pkg = await PackageInfo.fromPlatform();
      final installed = pkg.version;
      // `buildNumber` is the Android `versionCode` as a string. Empty on
      // platforms where the concept doesn't exist; treat that as 0 so the
      // bump-protection check still works (any positive manifest value
      // wins).
      final installedCode = int.tryParse(pkg.buildNumber) ?? 0;

      final manifest = await _fetchManifest();
      if (manifest == null) return null;

      final newVersion = manifest['version'] as String?;
      if (newVersion == null) return null;
      if (!_isNewer(newVersion, installed)) return null;

      final abis = manifest['abis'] as Map<String, dynamic>?;
      if (abis == null) return null;

      final abi = await primaryAbi();
      final raw = abis[abi];
      if (raw is! Map<String, dynamic>) return null;
      final AbiVariant variant;
      try {
        variant = AbiVariant.fromJson(raw);
      } catch (_) {
        // Missing fields (e.g. version_code) → drop the manifest entirely
        // rather than show a half-validated banner.
        return null;
      }

      // Origin pin: refuse any download URL pointing outside our update
      // host. Defends against a tampered manifest that redirects the APK
      // fetch to an attacker-controlled bucket.
      if (!isOriginAllowed(variant.url)) {
        return null;
      }

      // Bump-protection: the manifest's `version_code` MUST exceed the
      // installed APK's buildNumber. Without this, an attacker who can
      // serve a tampered manifest could re-advertise an OLDER (but
      // legitimately signed) APK with a known vulnerability under a
      // higher semver string.
      if (variant.versionCode <= installedCode) {
        return null;
      }

      // Wave TBD — proper minisign-style signature verification using the
      // `ed25519` Dart package + a baked-in pubkey. For now we only flag
      // the presence of the field so we can start populating it on the
      // server side without breaking older clients.
      // TODO(wave-tbd): verify variant.signature against manifest digest.
      if (variant.signature != null) {
        // ignore: avoid_print
        print('UpdateService: signature field present but verification '
            'not yet implemented — value=${variant.signature}');
      }

      return UpdateInfo(
        currentVersion: installed,
        newVersion: newVersion,
        releasedAt: manifest['released_at'] as String? ?? '',
        notes: manifest['notes'] as String? ?? '',
        abi: abi,
        variant: variant,
      );
    } catch (_) {
      // Swallow EVERYTHING — update check must never crash the boot path.
      return null;
    }
  }

  /// True iff [url] points at our pinned update host. Public so tests can
  /// exercise the origin-pin matrix without spinning up a manifest server.
  static bool isOriginAllowed(String url) =>
      url.startsWith(kUpdateOriginPrefix);

  static Future<Map<String, dynamic>?> _fetchManifest() async {
    final client = HttpClient()..connectionTimeout = kManifestTimeout;
    try {
      final req = await client
          .getUrl(Uri.parse(kUpdateManifestUrl))
          .timeout(kManifestTimeout);
      final res = await req.close().timeout(kManifestTimeout);
      if (res.statusCode != 200) return null;
      final body = await res
          .transform(utf8.decoder)
          .join()
          .timeout(kManifestTimeout);
      final decoded = jsonDecode(body);
      if (decoded is! Map<String, dynamic>) return null;
      return decoded;
    } catch (_) {
      return null;
    } finally {
      client.close(force: true);
    }
  }

  /// Test-visible alias for the private semver comparator. Lives in the
  /// same library so production paths can keep using `_isNewer`, while
  /// `update_service_test.dart` can drive the comparator without a fragile
  /// `@visibleForTesting` reflection dance.
  static bool isNewer(String a, String b) => _isNewer(a, b);

  /// Strict numeric semver comparison: returns true if [a] > [b].
  /// Handles `1.0.0`, `1.0.0+1` (drops build metadata after `+`).
  /// On parse failure returns `false` — i.e. we err on the side of
  /// "no update" rather than nagging the user with a bogus banner.
  static bool _isNewer(String a, String b) {
    final pa = _parseVersion(a);
    final pb = _parseVersion(b);
    if (pa == null || pb == null) return false;
    for (var i = 0; i < 3; i++) {
      if (pa[i] > pb[i]) return true;
      if (pa[i] < pb[i]) return false;
    }
    return false;
  }

  static List<int>? _parseVersion(String v) {
    final core = v.split('+').first.split('-').first;
    final parts = core.split('.');
    if (parts.length != 3) return null;
    try {
      return parts.map(int.parse).toList(growable: false);
    } catch (_) {
      return null;
    }
  }

  /// Returns the device's primary supported ABI (first entry of
  /// Build.SUPPORTED_ABIS), e.g. `arm64-v8a`. Falls back to `arm64-v8a`
  /// if the platform query fails — modern Android devices are
  /// overwhelmingly arm64, so this default is the least-bad guess.
  static Future<String> primaryAbi() async {
    try {
      final info = await DeviceInfoPlugin().androidInfo;
      final abis = info.supportedAbis;
      if (abis.isNotEmpty) return abis.first;
    } catch (_) {}
    return 'arm64-v8a';
  }

  /// Streams the APK described by [info] to disk, hashing as we go.
  /// Calls [onProgress] for every chunk. Verifies SHA256 against
  /// `info.variant.sha256` before returning — on mismatch the partial
  /// file is deleted and a [StateError] is thrown.
  ///
  /// We stream into a `.part` file and only rename to the final name on
  /// successful hash verification, so a half-finished download from a
  /// previous attempt can never be mistaken for a verified APK.
  static Future<File> downloadApk(
    UpdateInfo info, {
    ProgressCallback? onProgress,
  }) async {
    final dir = await _updateCacheDir();
    final filename = 'phantomchat-${info.newVersion}-${info.abi}.apk';
    final finalFile = File('${dir.path}/$filename');
    final partFile = File('${finalFile.path}.part');

    if (await partFile.exists()) {
      await partFile.delete();
    }

    final client = HttpClient();
    try {
      final req = await client.getUrl(Uri.parse(info.variant.url));
      final res = await req.close();
      if (res.statusCode != 200) {
        throw StateError('apk download HTTP ${res.statusCode}');
      }

      final total = info.variant.sizeBytes;
      var received = 0;
      final sink = partFile.openWrite();
      final hasher = AccumulatingSha256();
      try {
        await for (final chunk in res) {
          sink.add(chunk);
          hasher.add(chunk);
          received += chunk.length;
          onProgress?.call(received, total);
        }
        await sink.flush();
      } finally {
        await sink.close();
      }

      final actual = hasher.hex();
      if (actual != info.variant.sha256) {
        if (await partFile.exists()) {
          await partFile.delete();
        }
        throw StateError(
          'apk sha256 mismatch: expected ${info.variant.sha256}, got $actual',
        );
      }

      if (await finalFile.exists()) {
        await finalFile.delete();
      }
      await partFile.rename(finalFile.path);
      return finalFile;
    } finally {
      client.close(force: true);
    }
  }

  /// Hands the verified APK file to the OS package-installer. We use
  /// `open_filex`, which wraps the file in a content:// URI through the
  /// FileProvider declared in AndroidManifest.xml and dispatches an
  /// ACTION_VIEW Intent with the APK mime type. The system then takes
  /// over (potentially asking for "Install unknown apps" the first time).
  static Future<bool> installApk(File apk) async {
    final result = await OpenFilex.open(
      apk.path,
      type: 'application/vnd.android.package-archive',
    );
    return result.type == ResultType.done;
  }

  /// Resolves a writable cache dir for the downloaded APK. Prefers the
  /// app's *external* cache (so the system installer process — which
  /// runs outside our UID — can read the file via FileProvider). Falls
  /// back to the internal cache dir if external storage is unavailable
  /// (e.g. a device with no /sdcard).
  ///
  /// Android 11+ scoped storage gotcha: writing to `<external-cache>/`
  /// (i.e. `/sdcard/Android/data/<pkg>/cache/`) does NOT require
  /// MANAGE_EXTERNAL_STORAGE — it's app-private even though it lives on
  /// shared storage. That's why we don't need any storage permission
  /// here.
  static Future<Directory> _updateCacheDir() async {
    Directory? d;
    try {
      d = await getExternalStorageDirectory();
    } catch (_) {
      d = null;
    }
    d ??= await getApplicationCacheDirectory();
    final subdir = Directory('${d.path}/updates');
    if (!await subdir.exists()) {
      await subdir.create(recursive: true);
    }
    return subdir;
  }
}

/// Helper that accumulates SHA256 across streamed chunks. We can't reuse
/// `crypto.sha256.convert(...)` because that requires a single bytes
/// list — which would defeat streaming and force the whole APK (~30 MB)
/// into RAM. Instead we use `crypto`'s incremental Sink API.
class AccumulatingSha256 {
  final List<Digest> _out = [];
  late final ByteConversionSink _sink;

  AccumulatingSha256() {
    _sink = sha256.startChunkedConversion(
      _DigestSink(_out),
    );
  }

  void add(List<int> data) => _sink.add(data);

  String hex() {
    _sink.close();
    return _out.single.toString();
  }
}

class _DigestSink implements Sink<Digest> {
  final List<Digest> _out;
  _DigestSink(this._out);

  @override
  void add(Digest d) => _out.add(d);

  @override
  void close() {}
}
