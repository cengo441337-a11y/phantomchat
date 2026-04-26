// Tests for the APK auto-update guards.
//
// These exercise the static helpers we exposed for testing:
//   * UpdateService.isNewer — strict semver comparator (build metadata
//     after `+` is dropped).
//   * UpdateService.isOriginAllowed — origin pin: variant URLs MUST live
//     under https://updates.dc-infosec.de/. Anything else is dropped to
//     defend against a tampered manifest pointing at a foreign bucket.
//
// We deliberately do NOT spin up an HttpClient — checkForUpdate() touches
// PackageInfo / DeviceInfo platform channels, which `flutter test`
// doesn't bind. The pure-Dart helpers are where the security-critical
// logic lives, so we test those directly.

import 'package:flutter_test/flutter_test.dart';
import 'package:phantomchat/services/update_service.dart';

void main() {
  group('UpdateService.isNewer (semver matrix)', () {
    test('1.0.0 < 1.0.1', () {
      expect(UpdateService.isNewer('1.0.1', '1.0.0'), isTrue);
      expect(UpdateService.isNewer('1.0.0', '1.0.1'), isFalse);
    });

    test('1.0.0 == 1.0.0 → not newer', () {
      expect(UpdateService.isNewer('1.0.0', '1.0.0'), isFalse);
    });

    test('major / minor bumps win', () {
      expect(UpdateService.isNewer('2.0.0', '1.99.99'), isTrue);
      expect(UpdateService.isNewer('1.2.0', '1.1.99'), isTrue);
      expect(UpdateService.isNewer('1.1.99', '1.2.0'), isFalse);
    });

    test('build metadata after + is ignored', () {
      // 1.0.0+1 vs 1.0.0+2 → both parse to [1,0,0]; the build number is
      // not part of the semver ordering, so neither side is "newer".
      expect(UpdateService.isNewer('1.0.0+1', '1.0.0+2'), isFalse);
      expect(UpdateService.isNewer('1.0.0+2', '1.0.0+1'), isFalse);
    });

    test('pre-release suffix is stripped', () {
      // 1.0.0-rc1 parses to [1,0,0] for ordering purposes — same as the
      // build-meta case. The user shouldn't be nagged to "downgrade" from
      // a release to an rc string.
      expect(UpdateService.isNewer('1.0.0-rc1', '1.0.0'), isFalse);
    });

    test('garbage strings → false (no nag-banner on parse failure)', () {
      expect(UpdateService.isNewer('not-a-version', '1.0.0'), isFalse);
      expect(UpdateService.isNewer('1.0.0', 'not-a-version'), isFalse);
      expect(UpdateService.isNewer('1.0', '1.0.0'), isFalse);
      expect(UpdateService.isNewer('', ''), isFalse);
    });
  });

  group('UpdateService.isOriginAllowed (origin pin)', () {
    test('canonical update host is allowed', () {
      expect(
        UpdateService.isOriginAllowed(
          'https://updates.dc-infosec.de/download/phantomchat-1.2.0-arm64.apk',
        ),
        isTrue,
      );
    });

    test('different host is rejected', () {
      expect(
        UpdateService.isOriginAllowed(
          'https://attacker.example/download/phantomchat.apk',
        ),
        isFalse,
      );
    });

    test('plain http:// is rejected', () {
      expect(
        UpdateService.isOriginAllowed(
          'http://updates.dc-infosec.de/download/phantomchat.apk',
        ),
        isFalse,
      );
    });

    test('subdomain trick is rejected', () {
      // A naive .endsWith check would pass this; we use startsWith on the
      // full prefix, so the hostname can't be spoofed via a malicious
      // suffix.
      expect(
        UpdateService.isOriginAllowed(
          'https://updates.dc-infosec.de.attacker.example/x.apk',
        ),
        isFalse,
      );
    });

    test('empty / garbage URLs are rejected', () {
      expect(UpdateService.isOriginAllowed(''), isFalse);
      expect(UpdateService.isOriginAllowed('not a url'), isFalse);
    });
  });

  group('AbiVariant.fromJson', () {
    test('parses required + optional fields', () {
      final v = AbiVariant.fromJson({
        'url': 'https://updates.dc-infosec.de/x.apk',
        'sha256': 'ABCDEF',
        'size_bytes': 12345,
        'version_code': 42,
        'signature': 'sig-bytes',
      });
      expect(v.url, 'https://updates.dc-infosec.de/x.apk');
      expect(v.sha256, 'abcdef'); // lowercased
      expect(v.sizeBytes, 12345);
      expect(v.versionCode, 42);
      expect(v.signature, 'sig-bytes');
    });

    test('signature is optional', () {
      final v = AbiVariant.fromJson({
        'url': 'https://updates.dc-infosec.de/x.apk',
        'sha256': 'ab',
        'size_bytes': 1,
        'version_code': 1,
      });
      expect(v.signature, isNull);
    });

    test('throws on missing version_code (manifest is dropped upstream)', () {
      // checkForUpdate() catches this and returns null rather than show a
      // half-validated banner — see the try/catch around fromJson there.
      expect(
        () => AbiVariant.fromJson({
          'url': 'https://updates.dc-infosec.de/x.apk',
          'sha256': 'ab',
          'size_bytes': 1,
        }),
        throwsA(isA<TypeError>()),
      );
    });
  });
}
