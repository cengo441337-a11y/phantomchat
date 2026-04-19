import 'package:flutter/material.dart';

import '../screens/lock_screen.dart';
import '../services/app_lock_service.dart';
import '../services/storage_service.dart';

/// Wraps the main app and intercepts rendering whenever [AppLockService]
/// reports a locked state.
///
/// Behaviour:
/// - On mount → ask the service whether we're currently locked.
/// - On `AppLifecycleState.paused` → record the paused timestamp so the
///   auto-lock timeout can run even while the process is in the background.
/// - On `AppLifecycleState.resumed` → re-check the lock state; if the timeout
///   has passed, render [LockScreen] in front of the wrapped [child].
///
/// If the user has no PIN configured yet, the gate is transparent — the main
/// app renders directly and the onboarding flow is responsible for calling
/// [AppLockService.setPin] before finishing.
class AppLockGate extends StatefulWidget {
  final Widget child;
  const AppLockGate({super.key, required this.child});

  @override
  State<AppLockGate> createState() => _AppLockGateState();
}

class _AppLockGateState extends State<AppLockGate> with WidgetsBindingObserver {
  bool _locked = false;
  bool _needsSetup = false;
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
        // the moment the app was actually put into the background.
        AppLockService.touch();
        break;
      case AppLifecycleState.resumed:
        _refreshLockState();
        break;
      case AppLifecycleState.detached:
        break;
    }
  }

  Future<void> _refreshLockState() async {
    final hasId   = await StorageService.hasIdentity();
    final hasPin  = await AppLockService.hasPin();
    final locked  = await AppLockService.isCurrentlyLocked();

    // An existing identity without a PIN is a gap — force setup on the next
    // resume. A freshly installed device with no identity yet goes straight
    // to onboarding, which is responsible for invoking the setup flow.
    final needsSetup = hasId && !hasPin;

    if (!mounted) return;
    setState(() {
      _locked = locked;
      _needsSetup = needsSetup;
      _initialised = true;
    });
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
    return widget.child;
  }
}
