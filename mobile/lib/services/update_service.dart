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
import 'package:cryptography/cryptography.dart';
import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter/foundation.dart';
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

/// Audit 2026-04-30 (mobile-H5) — overall APK-download cap. The manifest
/// fetch already had a tight 6 s budget; the APK fetch had **none**. A
/// slow-loris server (or a captured router holding the connection open)
/// could keep the download dialog spinning indefinitely. 10 minutes is
/// generous enough for a 30 MB APK on a marginal cellular link, tight
/// enough that a stalled download surfaces as an actionable error rather
/// than an open-ended hang. Per-chunk progress already has natural socket
/// timeouts via Dart's `HttpClient` defaults; this is the wall-clock cap.
const Duration kApkDownloadTimeout = Duration(minutes: 10);
const Duration kApkConnectionTimeout = Duration(seconds: 30);

/// Audit 2026-04-30 (mobile-H5) — Minisign-style Ed25519 verification of
/// the update manifest.
///
/// **Status: infrastructure shipped, key not yet generated.** The const
/// below is `null` until the project's release-engineer generates a
/// dedicated update-signing keypair (separate from the desktop Tauri
/// minisign key, separate from the disclosure PGP key) and configures
/// `publish-android-update-manifest.sh` to sign the canonical JSON of
/// every manifest before upload. While `null`, the verifier short-circuits
/// to "no enforcement" — the existing HTTPS + sha256 + version_code
/// trust chain is unchanged. Once the key is set, every manifest with a
/// `"signature"` field is verified and a verification failure REJECTS
/// the manifest (returns `null` from `checkForUpdate`).
///
/// Format: 32-byte Ed25519 public key, base64-encoded. The signed payload
/// is the manifest object with the `"signature"` key removed, then
/// re-serialised via `jsonEncode` (Dart's deterministic JSON encoder is
/// stable enough for a payload of this shape — flat top level, fixed
/// keys). Server-side signing must do the same: load JSON, drop
/// `signature`, jsonEncode in the same canonicalisation, sign with
/// the private key, base64-encode, write back into the JSON.
const String? kManifestPubkeyB64 = null;


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

      // The current threat model is defended by HTTPS (TLS to
      // dc-infosec.de), origin-pin (URL must match `isOriginAllowed`),
      // sha256 (APK content), version_code bump-protection (can't
      // downgrade), and Android's keystore-signature match (cannot
      // install an APK signed by a different keystore than the one
      // currently installed). Audit 2026-04-30 (mobile-H5) adds the
      // Ed25519-on-manifest verifier below. While `kManifestPubkeyB64`
      // is `null` (key not yet generated by release-eng) the verifier
      // skips to soft-warn; once the key is set, signature mismatch
      // hard-rejects the manifest.
      if (!await _verifyManifestSignature(manifest)) {
        return null;
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

  /// Audit 2026-04-30 (mobile-H5) — verify the manifest's Ed25519
  /// signature against [kManifestPubkeyB64].
  ///
  /// Returns `true` when:
  ///   - `kManifestPubkeyB64` is `null` (no pubkey configured → no
  ///     enforcement; legacy trust chain stands), OR
  ///   - the manifest carries a valid `"signature"` field that verifies
  ///     against the pubkey for the canonical-JSON of `manifest \
  ///     {signature}`.
  ///
  /// Returns `false` when the pubkey IS configured but the signature
  /// is absent / wrong shape / fails verification — refuses the manifest
  /// entirely so a downgrade or tamper attempt can't fall through.
  ///
  /// Visible for tests via [debugVerifyManifestSignature].
  static Future<bool> _verifyManifestSignature(
    Map<String, dynamic> manifest,
  ) async {
    final pubkeyB64 = kManifestPubkeyB64;
    if (pubkeyB64 == null) {
      // No pubkey configured. Log loudly so an operator notices that the
      // verifier is in passthrough mode, but don't refuse the update —
      // existing pilots ship without the key and their users would be
      // stranded otherwise.
      debugPrint(
        '[update_service] manifest verify SKIPPED — kManifestPubkeyB64 '
        'is null. Configure a project Ed25519 update-signing key and '
        'set the const to enable enforcement.',
      );
      return true;
    }

    final sigB64 = manifest['signature'];
    if (sigB64 is! String || sigB64.isEmpty) {
      debugPrint(
        '[update_service] manifest verify FAILED — pubkey is configured '
        'but the manifest carries no signature.',
      );
      return false;
    }

    try {
      final pubkey = base64Decode(pubkeyB64);
      if (pubkey.length != 32) {
        debugPrint(
          '[update_service] manifest verify FAILED — pubkey is '
          '${pubkey.length} bytes, expected 32 (Ed25519).',
        );
        return false;
      }
      final sig = base64Decode(sigB64);
      if (sig.length != 64) {
        debugPrint(
          '[update_service] manifest verify FAILED — signature is '
          '${sig.length} bytes, expected 64 (Ed25519).',
        );
        return false;
      }

      // Build the signed payload: manifest with `signature` removed,
      // re-encoded. Server-side signer MUST mirror this exactly.
      final unsigned = Map<String, dynamic>.from(manifest)..remove('signature');
      final payload = utf8.encode(jsonEncode(unsigned));

      final algo = Ed25519();
      final pub = SimplePublicKey(pubkey, type: KeyPairType.ed25519);
      final ok = await algo.verify(
        payload,
        signature: Signature(sig, publicKey: pub),
      );
      if (!ok) {
        debugPrint(
          '[update_service] manifest verify FAILED — Ed25519 signature '
          'did not verify against pubkey ${pubkeyB64.substring(0, 8)}…',
        );
      }
      return ok;
    } catch (e) {
      debugPrint('[update_service] manifest verify ERROR: $e');
      return false;
    }
  }

  /// Test-visible wrapper for [_verifyManifestSignature] so unit tests can
  /// drive the verify path without poking at private symbols.
  @visibleForTesting
  static Future<bool> debugVerifyManifestSignature(
    Map<String, dynamic> manifest,
  ) =>
      _verifyManifestSignature(manifest);

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

    final client = HttpClient()
      ..connectionTimeout = kApkConnectionTimeout
      ..idleTimeout = const Duration(seconds: 30);
    try {
      final req = await client
          .getUrl(Uri.parse(info.variant.url))
          .timeout(kApkConnectionTimeout);
      final res = await req.close().timeout(kApkConnectionTimeout);
      if (res.statusCode != 200) {
        throw StateError('apk download HTTP ${res.statusCode}');
      }

      final total = info.variant.sizeBytes;
      var received = 0;
      final sink = partFile.openWrite();
      final hasher = AccumulatingSha256();
      try {
        // Audit 2026-04-30 (mobile-H5) — wall-clock cap on the entire
        // download stream. A slow-loris server can't keep the download
        // dialog spinning indefinitely; if 10 min isn't enough for the
        // user's link, an explicit timeout error surfaces rather than a
        // silent hang.
        await for (final chunk in res.timeout(kApkDownloadTimeout)) {
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
