import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';

import '../services/app_lock_service.dart';
import '../theme.dart';
import '../widgets/cyber_card.dart';

/// Full-screen PIN / biometric unlock screen.
///
/// Rendered by [AppLockGate] whenever [AppLockService.isCurrentlyLocked]
/// returns true. Calls [onUnlocked] exactly once on successful auth.
class LockScreen extends StatefulWidget {
  /// Called once the user has successfully unlocked via PIN or biometrics.
  final VoidCallback onUnlocked;

  /// When true the screen offers a "Set up PIN" flow instead of unlocking —
  /// used at the end of onboarding before the user enters the main app.
  final bool setupMode;

  const LockScreen({
    super.key,
    required this.onUnlocked,
    this.setupMode = false,
  });

  @override
  State<LockScreen> createState() => _LockScreenState();
}

class _LockScreenState extends State<LockScreen> {
  String _buffer = '';
  String? _pendingSetupPin;         // first entry during setup-confirm flow
  String? _statusLine;              // transient inline status
  int? _remainingAttempts;
  bool _biometricAvailable = false;
  bool _biometricEnabled   = false;

  @override
  void initState() {
    super.initState();
    _loadState();
    // Offer biometrics right away on re-unlock (not during initial setup).
    if (!widget.setupMode) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _tryBiometric(silent: true));
    }
  }

  Future<void> _loadState() async {
    final avail   = await AppLockService.biometricAvailable();
    final enabled = await AppLockService.biometricEnabled();
    final left    = await AppLockService.remainingAttempts();
    if (!mounted) return;
    setState(() {
      _biometricAvailable = avail;
      _biometricEnabled   = enabled;
      _remainingAttempts  = left;
    });
  }

  // ── PIN input ──────────────────────────────────────────────────────────────

  void _append(String d) {
    if (_buffer.length >= 8) return;
    HapticFeedback.selectionClick();
    setState(() {
      _buffer += d;
      _statusLine = null;
    });
    if (_buffer.length >= 4) {
      // Auto-submit once we have a sensible minimum length, but only if the
      // user stopped typing for a beat. A simple length check is enough.
      // We defer actual verification to an explicit CONFIRM tap to avoid
      // prematurely locking out users who type 5- or 6-digit PINs.
    }
  }

  void _backspace() {
    if (_buffer.isEmpty) return;
    HapticFeedback.selectionClick();
    setState(() {
      _buffer = _buffer.substring(0, _buffer.length - 1);
      _statusLine = null;
    });
  }

  Future<void> _submit() async {
    if (_buffer.length < 4) {
      setState(() => _statusLine = 'PIN must be at least 4 digits');
      return;
    }

    if (widget.setupMode) {
      await _handleSetupSubmit();
    } else {
      await _handleUnlockSubmit();
    }
  }

  Future<void> _handleSetupSubmit() async {
    if (_pendingSetupPin == null) {
      setState(() {
        _pendingSetupPin = _buffer;
        _buffer = '';
        _statusLine = 'Repeat PIN to confirm';
      });
      return;
    }
    if (_buffer != _pendingSetupPin) {
      setState(() {
        _statusLine = 'PINs did not match — try again';
        _pendingSetupPin = null;
        _buffer = '';
      });
      return;
    }
    await AppLockService.setPin(_buffer);
    HapticFeedback.heavyImpact();
    widget.onUnlocked();
  }

  Future<void> _handleUnlockSubmit() async {
    final ok = await AppLockService.verifyPin(_buffer);
    if (!mounted) return;

    if (ok) {
      HapticFeedback.heavyImpact();
      widget.onUnlocked();
      return;
    }

    HapticFeedback.vibrate();
    final left = await AppLockService.remainingAttempts();
    if (!mounted) return;
    setState(() {
      _buffer = '';
      _remainingAttempts = left;
      _statusLine = left == 0
          ? 'DEVICE WIPED — restart to begin again'
          : 'Wrong PIN — $left attempts left before wipe';
    });
  }

  // ── Biometrics ─────────────────────────────────────────────────────────────

  Future<void> _tryBiometric({bool silent = false}) async {
    if (!_biometricAvailable || !_biometricEnabled) {
      if (!silent) {
        setState(() => _statusLine = 'Biometrics not configured');
      }
      return;
    }
    final ok = await AppLockService.authenticateBiometric();
    if (ok && mounted) widget.onUnlocked();
  }

  // ── UI ─────────────────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    final title = widget.setupMode
        ? (_pendingSetupPin == null ? '> SET PIN' : '> CONFIRM PIN')
        : '> UNLOCK';

    return Scaffold(
      backgroundColor: kBg,
      body: GridBackground(
        child: SafeArea(
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 24),
            child: Column(
              children: [
                const SizedBox(height: 24),

                // Title
                Text(
                  title,
                  style: GoogleFonts.orbitron(
                    fontSize: 32,
                    fontWeight: FontWeight.w900,
                    color: kWhite,
                    letterSpacing: 3,
                    shadows: [Shadow(color: kCyan.withOpacity(0.6), blurRadius: 14)],
                  ),
                ),
                const SizedBox(height: 8),
                Text(
                  widget.setupMode
                      ? '// 4–8 DIGITS · PANIC-WIPE AFTER ${AppLockService.maxFailedAttempts} WRONG ENTRIES'
                      : '// BIOMETRIC OR PIN',
                  style: GoogleFonts.spaceMono(
                    fontSize: 10,
                    color: kGrayText,
                    letterSpacing: 1.5,
                  ),
                ),

                const SizedBox(height: 36),

                // PIN dots
                _PinDots(length: _buffer.length, max: _buffer.length < 4 ? 4 : _buffer.length),

                const SizedBox(height: 18),

                // Status line (error / hint)
                SizedBox(
                  height: 18,
                  child: _statusLine == null
                      ? const SizedBox.shrink()
                      : Text(
                          _statusLine!,
                          style: GoogleFonts.spaceMono(
                            fontSize: 11,
                            color: kMagenta,
                            letterSpacing: 0.5,
                          ),
                        ),
                ),

                if (!widget.setupMode && _remainingAttempts != null && _remainingAttempts! < 4)
                  Padding(
                    padding: const EdgeInsets.only(top: 4),
                    child: Text(
                      '! $_remainingAttempts attempts before panic-wipe',
                      style: GoogleFonts.spaceMono(
                        fontSize: 10,
                        color: kMagenta,
                        letterSpacing: 1,
                      ),
                    ),
                  ),

                const Spacer(),

                // Numeric pad
                _PinPad(
                  onDigit: _append,
                  onBackspace: _backspace,
                  onConfirm: _submit,
                  onBiometric: (!widget.setupMode && _biometricAvailable && _biometricEnabled)
                      ? _tryBiometric
                      : null,
                ),

                const SizedBox(height: 24),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

// ── Dots indicator ───────────────────────────────────────────────────────────

class _PinDots extends StatelessWidget {
  final int length;
  final int max;
  const _PinDots({required this.length, required this.max});

  @override
  Widget build(BuildContext context) {
    final total = max < 4 ? 4 : max;
    return Row(
      mainAxisAlignment: MainAxisAlignment.center,
      children: List.generate(total, (i) {
        final filled = i < length;
        return Container(
          margin: const EdgeInsets.symmetric(horizontal: 6),
          width: 14,
          height: 14,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: filled ? kCyan : Colors.transparent,
            border: Border.all(color: kCyan.withOpacity(filled ? 1 : 0.35), width: 1.5),
            boxShadow: filled
                ? [BoxShadow(color: kCyan.withOpacity(0.6), blurRadius: 10)]
                : null,
          ),
        );
      }),
    );
  }
}

// ── Numeric pad ──────────────────────────────────────────────────────────────

class _PinPad extends StatelessWidget {
  final void Function(String) onDigit;
  final VoidCallback onBackspace;
  final VoidCallback onConfirm;
  final VoidCallback? onBiometric;

  const _PinPad({
    required this.onDigit,
    required this.onBackspace,
    required this.onConfirm,
    required this.onBiometric,
  });

  @override
  Widget build(BuildContext context) {
    Widget digit(String d) => _PadKey(
          label: d,
          onTap: () => onDigit(d),
        );

    return Column(
      children: [
        Row(mainAxisAlignment: MainAxisAlignment.center, children: [digit('1'), digit('2'), digit('3')]),
        const SizedBox(height: 10),
        Row(mainAxisAlignment: MainAxisAlignment.center, children: [digit('4'), digit('5'), digit('6')]),
        const SizedBox(height: 10),
        Row(mainAxisAlignment: MainAxisAlignment.center, children: [digit('7'), digit('8'), digit('9')]),
        const SizedBox(height: 10),
        Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            _PadKey(
              icon: onBiometric != null ? Icons.fingerprint : Icons.radio_button_unchecked,
              onTap: onBiometric ?? () {},
              color: onBiometric != null ? kCyan : kGray,
            ),
            digit('0'),
            _PadKey(
              icon: Icons.backspace_outlined,
              onTap: onBackspace,
              color: kMagenta,
            ),
          ],
        ),
        const SizedBox(height: 16),
        GestureDetector(
          onTap: onConfirm,
          child: CyberCard(
            borderColor: kCyan,
            glow: true,
            padding: const EdgeInsets.symmetric(vertical: 14, horizontal: 24),
            child: Text(
              'CONFIRM',
              style: GoogleFonts.orbitron(
                fontSize: 13,
                fontWeight: FontWeight.w700,
                color: kCyan,
                letterSpacing: 3,
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class _PadKey extends StatelessWidget {
  final String? label;
  final IconData? icon;
  final VoidCallback onTap;
  final Color color;

  const _PadKey({
    this.label,
    this.icon,
    required this.onTap,
    this.color = kCyan,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 8),
      child: GestureDetector(
        onTap: onTap,
        child: Container(
          width: 72,
          height: 64,
          decoration: BoxDecoration(
            border: Border.all(color: color.withOpacity(0.5)),
            color: color.withOpacity(0.05),
          ),
          child: Center(
            child: label != null
                ? Text(
                    label!,
                    style: GoogleFonts.orbitron(
                      fontSize: 22,
                      fontWeight: FontWeight.w700,
                      color: kWhite,
                      letterSpacing: 1,
                    ),
                  )
                : Icon(icon, color: color, size: 22),
          ),
        ),
      ),
    );
  }
}
