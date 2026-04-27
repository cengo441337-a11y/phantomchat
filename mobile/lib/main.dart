import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
// ignore: unused_import
import 'package:cryptography_flutter/cryptography_flutter.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'services/background_service_config.dart';
import 'services/storage_service.dart';
import 'screens/onboarding.dart';
import 'screens/home.dart';
import 'theme.dart';
import 'widgets/app_lock_gate.dart';
import 'src/rust/api.dart' as rust_api;
import 'src/rust/frb_generated.dart';

/// Result of trying to bring the Rust core online at startup.
class RustBootState {
  final bool initialised;
  final String? phantomId;
  final String? error;
  // Installed-app version + build number, surfaced in the rust-core
  // banner so we can confirm at a glance which APK someone is running
  // (e.g. when debugging "I updated and the bug is still there" — was
  // the manifest pointing at the new APK or did they install the wrong
  // one?).
  final String version;
  const RustBootState({
    required this.initialised,
    required this.version,
    this.phantomId,
    this.error,
  });
}

Future<RustBootState> _bootRust() async {
  String version = '?';
  try {
    final pkg = await PackageInfo.fromPlatform();
    version = '${pkg.version}+${pkg.buildNumber}';
  } catch (_) { /* best-effort; the banner just shows '?' */ }
  try {
    await RustLib.init();
    final id = rust_api.generatePhantomId();
    return RustBootState(initialised: true, phantomId: id, version: version);
  } catch (e) {
    return RustBootState(initialised: false, error: e.toString(), version: version);
  }
}

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  // Importing cryptography_flutter is enough — Flutter's plugin
  // auto-registration installs FlutterCryptography as the default
  // Cryptography instance, so PBKDF2 / HKDF / AES-GCM all use JNI
  // (Android Keystore) or ObjC (iOS CommonCrypto) without an explicit
  // enable() call. The unused_import lint above is intentional: we
  // need the side-effect of plugin registration, not any symbol.
  SystemChrome.setPreferredOrientations([DeviceOrientation.portraitUp]);
  SystemChrome.setSystemUIOverlayStyle(
    const SystemUiOverlayStyle(
      statusBarColor: Colors.transparent,
      statusBarIconBrightness: Brightness.light,
      systemNavigationBarColor: kBg,
    ),
  );

  final hasIdentity = await StorageService.hasIdentity();
  final rust = await _bootRust();
  // Wave 8B — register the background relay-listener config (does not
  // start the service; that requires explicit opt-in in Settings).
  await PhantomBackgroundService.initialize();

  runApp(PhantomApp(hasIdentity: hasIdentity, rust: rust));
}

class PhantomApp extends StatelessWidget {
  final bool hasIdentity;
  final RustBootState rust;
  const PhantomApp({super.key, required this.hasIdentity, required this.rust});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'PhantomChat',
      theme: phantomTheme(),
      debugShowCheckedModeBanner: false,
      home: AppLockGate(
        child: Stack(
          children: [
            hasIdentity ? const HomeScreen() : const OnboardingScreen(),
            _RustCoreBanner(rust: rust),
          ],
        ),
      ),
    );
  }
}

class _RustCoreBanner extends StatelessWidget {
  final RustBootState rust;
  const _RustCoreBanner({required this.rust});

  @override
  Widget build(BuildContext context) {
    final ok = rust.initialised;
    final bg = ok ? const Color(0xCC003321) : const Color(0xCC3B0014);
    final fg = ok ? const Color(0xFF00F0A0) : const Color(0xFFFF5060);
    final label = ok
        ? 'v${rust.version} · rust core ACTIVE · ${rust.phantomId ?? "(no id)"}'
        : 'v${rust.version} · rust core FAILED · ${(rust.error ?? "").split("\n").first}';
    return SafeArea(
      child: Align(
        alignment: Alignment.bottomCenter,
        child: Container(
          margin: const EdgeInsets.only(bottom: 8, left: 12, right: 12),
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
          decoration: BoxDecoration(
            color: bg,
            borderRadius: BorderRadius.circular(4),
            border: Border.all(color: fg, width: 0.5),
          ),
          child: Text(
            label,
            style: TextStyle(
              color: fg,
              fontSize: 10,
              fontFamily: 'monospace',
              letterSpacing: 1,
            ),
            overflow: TextOverflow.ellipsis,
          ),
        ),
      ),
    );
  }
}
