import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

import '../screens/lock_screen.dart';
import '../services/app_lock_service.dart';
import '../services/storage_service.dart';
import '../theme.dart';
import 'cyber_card.dart';

/// Wraps the main app and intercepts rendering whenever [AppLockService]
/// reports a locked state.
///
/// Gates the shell behind any of three conditions, in priority order:
///
/// 1. **Biometric-on-launch** — opt-in lightweight quick-lock. When the user
///    enabled "Biometrie bei Start anfordern" in Settings, every cold start
///    or app-resume triggers an OS biometric prompt and the chat surface
///    stays hidden behind a black overlay until it succeeds. Independent of
///    PIN setup — perfect for users who want a quick-glance protection
///    without the panic-wipe ceremony.
///
/// 2. **PIN setup gap** — an existing identity without a PIN forces the
///    setup screen on next resume.
///
/// 3. **Idle auto-lock** — a PIN-protected device that has been backgrounded
///    long enough renders the regular [LockScreen].
///
/// If none of the above apply, the wrapped [child] renders directly.
class AppLockGate extends StatefulWidget {
  final Widget child;
  const AppLockGate({super.key, required this.child});

  @override
  State<AppLockGate> createState() => _AppLockGateState();
}

class _AppLockGateState extends State<AppLockGate> with WidgetsBindingObserver {
  bool _locked = false;
  bool _needsSetup = false;
  bool _bioPending = false;
  bool _initialised = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _refreshLockState();
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    switch (state) {
      case AppLifecycleState.paused:
      case AppLifecycleState.inactive:
      case AppLifecycleState.hidden:
        // Touch the activity marker so the idle window starts counting from
        // the moment the app was actually put into the background, and clear
        // the in-process biometric session so the next resume re-prompts.
        AppLockService.touch();
        AppLockService.clearBioSession();
        break;
      case AppLifecycleState.resumed:
        _refreshLockState();
        break;
      case AppLifecycleState.detached:
        break;
    }
  }

  Future<void> _refreshLockState() async {
    final hasId      = await StorageService.hasIdentity();
    final hasPin     = await AppLockService.hasPin();
    final locked     = await AppLockService.isCurrentlyLocked();
    final bioPending = await AppLockService.bioOnLaunchPending();

    // An existing identity without a PIN is a gap — force setup on the next
    // resume. A freshly installed device with no identity yet goes straight
    // to onboarding, which is responsible for invoking the setup flow.
    final needsSetup = hasId && !hasPin;

    if (!mounted) return;
    setState(() {
      _locked = locked;
      _needsSetup = needsSetup;
      _bioPending = bioPending;
      _initialised = true;
    });

    // Auto-fire the biometric prompt as soon as we know we need it. Holds
    // the black overlay in place until the OS dialog resolves.
    if (bioPending && !needsSetup && !locked) {
      _runBioGate();
    }
  }

  Future<void> _runBioGate() async {
    final ok = await AppLockService.authenticateBiometric(
      reason: 'Sperrbildschirm — entsperren mit Fingerabdruck oder PIN',
    );
    if (!mounted) return;
    if (ok) {
      AppLockService.markBioSessionUnlocked();
      setState(() => _bioPending = false);
    }
    // If the user cancels we leave _bioPending = true so they can retry via
    // the on-overlay "ENTSPERREN" button. We do not exit the app.
  }

  void _onUnlocked() {
    if (!mounted) return;
    setState(() {
      _locked = false;
      _needsSetup = false;
    });
  }

  @override
  Widget build(BuildContext context) {
    if (!_initialised) {
      return const SizedBox.shrink(); // brief splash while we read storage
    }
    if (_needsSetup) {
      return LockScreen(onUnlocked: _onUnlocked, setupMode: true);
    }
    if (_locked) {
      return LockScreen(onUnlocked: _onUnlocked);
    }
    if (_bioPending) {
      // Render the wrapped child underneath, but obscure it with the
      // biometric overlay — no app content is ever visible until the OS
      // prompt succeeds.
      return _BioLaunchOverlay(onRetry: _runBioGate);
    }
    return widget.child;
  }
}

/// Full-screen black-out shown while the biometric-on-launch prompt is in
/// flight or after the user dismissed it without authenticating. Tapping
/// "ENTSPERREN" re-triggers the OS prompt.
class _BioLaunchOverlay extends StatelessWidget {
  final VoidCallback onRetry;
  const _BioLaunchOverlay({required this.onRetry});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      body: SafeArea(
        child: Center(
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 32),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Icon(
                  Icons.lock_outline,
                  size: 56,
                  color: kCyan,
                  shadows: [Shadow(color: kCyan.withOpacity(0.6), blurRadius: 16)],
                ),
                const SizedBox(height: 24),
                Text(
                  'PHANTOMCHAT',
                  style: GoogleFonts.orbitron(
                    fontSize: 22,
                    fontWeight: FontWeight.w900,
                    color: kWhite,
                    letterSpacing: 4,
                    shadows: [Shadow(color: kCyan.withOpacity(0.5), blurRadius: 12)],
                  ),
                ),
                const SizedBox(height: 6),
                Text(
                  '// GESPERRT',
                  style: GoogleFonts.spaceMono(
                    fontSize: 11,
                    color: kGrayText,
                    letterSpacing: 2,
                  ),
                ),
                const SizedBox(height: 36),
                GestureDetector(
                  onTap: onRetry,
                  child: CyberCard(
                    borderColor: kCyan,
                    glow: true,
                    padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 14),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        const Icon(Icons.fingerprint, color: kCyan, size: 18),
                        const SizedBox(width: 12),
                        Text(
                          'ENTSPERREN',
                          style: GoogleFonts.orbitron(
                            fontSize: 13,
                            fontWeight: FontWeight.w700,
                            color: kCyan,
                            letterSpacing: 3,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
