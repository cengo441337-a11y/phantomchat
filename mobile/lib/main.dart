import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:cryptography_flutter/cryptography_flutter.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'services/background_service_config.dart';
import 'services/crypto_service.dart';
import 'services/log_service.dart';
import 'services/relay_service.dart';
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
    // Load any existing identity into the Rust core so the v3 sealed-
    // sender send path has the signing seed available. Without this
    // step, every `sendSealedV3` errors with "signing key not loaded —
    // call load_local_identity_v3 first" and no message ever leaves the
    // device. (This was the regression in 1.0.x: the FRB binding was
    // generated, the Rust API existed, but the Dart side never invoked
    // it.)
    //
    // Pre-1.0.7 identities don't carry a signing seed — generate one
    // and rewrite the stored record so the migration is one-shot.
    var identity = await StorageService.loadIdentity();
    if (identity != null) {
      if (identity.privateSigningKey == null) {
        final seed = await CryptoService.generateSigningSeedHex();
        identity = identity.copyWith(privateSigningKey: seed);
        await StorageService.saveIdentity(identity);
      }
      try {
        await rust_api.loadLocalIdentityV3(
          viewSecretHex: identity.privateViewKey,
          spendSecretHex: identity.privateSpendKey,
          signingSecretHex: identity.privateSigningKey!,
        );
      } catch (_) { /* not fatal — first-launch users hit this before
                       onboarding completes. The home flow re-loads
                       after identity creation. */ }
    }
    final id = rust_api.generatePhantomId();
    return RustBootState(initialised: true, phantomId: id, version: version);
  } catch (e) {
    return RustBootState(initialised: false, error: e.toString(), version: version);
  }
}

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  // Install the in-memory log capture FIRST so every subsequent
  // `debugPrint` (incl. the relay-error subscriber a few lines down,
  // setPin's timing logs, FRB binding messages, etc.) lands in the
  // ring buffer the Diagnostik screen surfaces.
  LogService().install();
  // Force-install the native PBKDF2 / HKDF / AES-GCM impls. The
  // package's own deprecation message claims plugin auto-registration
  // makes this unnecessary, but real-device reports of "PIN-confirm
  // freezes for 10+ s" suggest the auto-registration isn't reliably
  // firing on every Android build — the calls fall back to pure-Dart
  // PBKDF2 which on emulator-class CPUs takes seconds for 50k iters.
  // Calling enable() explicitly costs nothing on devices where the
  // plugin already registered, and rescues us on the ones where it
  // didn't. The deprecation lint is suppressed below.
  // ignore: deprecated_member_use
  FlutterCryptography.enable();
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

  // Kick the relay listener as soon as we have an identity loaded into
  // the Rust core. Pre-1.1.3 this was only fired from chat.dart +
  // channels.dart, so a user who opened the app and stayed on the home
  // contact list received nothing — the WebSockets never connected.
  // Idempotent guard inside `RelayService.connect` makes this safe to
  // call multiple times.
  if (hasIdentity && rust.initialised) {
    unawaited(RelayService.instance.connect());
    // Surface receive-path errors that `feedEnvelope` emits to its
    // event stream. Pre-1.1.3 these went into a controller no global
    // subscriber listened to — "view key not loaded" / "envelope
    // decode failed" / etc. dropped silently. We log via debugPrint
    // so logcat picks them up; per-screen UIs can still subscribe for
    // visual surfaces.
    RelayService.instance.events.listen((evt) {
      if (evt.kind == 'error') {
        debugPrint('[relay] error: ${evt.payload}');
      }
    });
  }

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
