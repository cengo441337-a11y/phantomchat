import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
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
  const RustBootState({required this.initialised, this.phantomId, this.error});
}

Future<RustBootState> _bootRust() async {
  try {
    await RustLib.init();
    final id = rust_api.generatePhantomId();
    return RustBootState(initialised: true, phantomId: id);
  } catch (e) {
    return RustBootState(initialised: false, error: e.toString());
  }
}

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
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
        ? 'rust core ACTIVE · ${rust.phantomId ?? "(no id)"}'
        : 'rust core FAILED · ${(rust.error ?? "").split("\n").first}';
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
