import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:qr_flutter/qr_flutter.dart';

import '../services/wallet_service.dart';
import '../services/risk_check_service.dart';
import '../src/rust/api/wallet.dart' as rust;
import '../theme.dart';

/// Argos Wallet — single-screen state machine.
///
/// Lifecycle (all transitions internal — no Navigator pushes for the inner
/// onboarding states so the back-button never lands the user mid-flow):
///
///   none          → no wallet on disk → CreateOrRestore
///   creating      → PIN entry, then backup-reveal, then main
///   restoring     → paste mnemonic + PIN, then main
///   locked        → wallet exists, enter PIN
///   unlocked      → main wallet view (balance + actions)
/// Parse a user-entered decimal amount string into raw base units using
/// EXACT BigInt math. `double` has a 53-bit mantissa (~15-16 significant
/// digits) and silently loses precision / overflows for high-decimal tokens
/// (USDC 6, SOL 9, ETH 18). Going through `double` could send a slightly
/// different amount than the user typed. This stays integer the whole way.
///
/// Truncates any digits beyond `decimals` (never rounds up) so the user can
/// never accidentally send MORE than they entered. Returns null on invalid
/// input or a non-positive amount.
BigInt? decimalToBaseUnits(String input, int decimals) {
  var t = input.trim().replaceAll(',', '.');
  if (t.isEmpty || t == '.') return null;
  if (!RegExp(r'^\d*\.?\d*$').hasMatch(t)) return null;
  final dot = t.indexOf('.');
  String wholePart;
  String fracPart;
  if (dot < 0) {
    wholePart = t;
    fracPart = '';
  } else {
    wholePart = t.substring(0, dot);
    fracPart = t.substring(dot + 1);
  }
  if (wholePart.isEmpty) wholePart = '0';
  if (fracPart.length > decimals) {
    fracPart = fracPart.substring(0, decimals);
  } else {
    fracPart = fracPart.padRight(decimals, '0');
  }
  final combined = wholePart + fracPart;
  final v = BigInt.tryParse(combined.isEmpty ? '0' : combined);
  if (v == null || v <= BigInt.zero) return null;
  return v;
}

class ArgosWalletScreen extends StatefulWidget {
  const ArgosWalletScreen({super.key});

  @override
  State<ArgosWalletScreen> createState() => _ArgosWalletScreenState();
}

enum _Stage { loading, none, createPin, backupReveal, restore, locked, main }

class _ArgosWalletScreenState extends State<ArgosWalletScreen>
    with WidgetsBindingObserver {
  final _svc = ArgosWalletService.instance;
  _Stage _stage = _Stage.loading;
  String? _justRevealedMnemonic;
  String? _network;

  // Main-view state.
  ArgosChain _activeChain = ArgosChain.solanaMainnet;
  BigInt _solLamports = BigInt.zero;
  final Map<String, BigInt> _tokenBalances = {};
  String _ethAddress = '';
  String _ethBalanceWei = '0';
  final Map<String, String> _evmTokenBalances = {}; // key = token address, decimal string
  bool _refreshing = false;
  String? _refreshError;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _bootstrap();
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // Auto-lock the wallet when the app goes to background. Defense
    // against a thief who picks up an unlocked-but-screen-off phone.
    // Skip during the mnemonic-reveal flow so the user can switch to
    // a notes app to write the words down without losing the screen.
    if (state == AppLifecycleState.paused ||
        state == AppLifecycleState.detached) {
      if (_stage == _Stage.main && _svc.isUnlocked) {
        unawaited(_svc.lock());
        if (mounted) setState(() => _stage = _Stage.locked);
      }
    }
  }

  Future<void> _bootstrap() async {
    final has = await _svc.hasWallet();
    if (!mounted) return;
    if (!has) {
      setState(() => _stage = _Stage.none);
      return;
    }
    if (_svc.isUnlocked) {
      setState(() => _stage = _Stage.main);
      unawaited(_refresh());
    } else {
      setState(() => _stage = _Stage.locked);
    }
  }

  Future<void> _refresh() async {
    if (!_svc.isUnlocked) return;
    setState(() {
      _refreshing = true;
      _refreshError = null;
    });
    try {
      if (_activeChain.isSolana) {
        final sol = await _svc.balanceSol();
        final tokens = <String, BigInt>{};
        for (final t in argosKnownTokens) {
          try {
            tokens[t.mint] = await _svc.balanceToken(t.mint);
          } catch (_) {
            tokens[t.mint] = BigInt.zero;
          }
        }
        if (!mounted) return;
        setState(() {
          _solLamports = sol;
          _tokenBalances
            ..clear()
            ..addAll(tokens);
          _refreshing = false;
        });
      } else {
        final net = _activeChain.backendId;
        final addr = await _svc.ethAddress(net);
        final wei = await _svc.ethBalanceWei(net);
        final tokens = <String, String>{};
        for (final t
            in argosEvmKnownTokens.where((t) => t.chain == _activeChain)) {
          try {
            tokens[t.address] = await _svc.ethErc20Balance(net, t.address);
          } catch (_) {
            tokens[t.address] = '0';
          }
        }
        if (!mounted) return;
        setState(() {
          _ethAddress = addr;
          _ethBalanceWei = wei;
          _evmTokenBalances
            ..clear()
            ..addAll(tokens);
          _refreshing = false;
        });
      }
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _refreshing = false;
        _refreshError = e.toString();
      });
    }
  }

  void _switchChain(ArgosChain next) {
    if (next == _activeChain) return;
    setState(() {
      _activeChain = next;
      _refreshError = null;
    });
    unawaited(_refresh());
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      appBar: AppBar(
        title: Text(
          'ARGOS · WALLET',
          style: GoogleFonts.orbitron(
            color: kCyan,
            fontSize: 14,
            letterSpacing: 3,
            fontWeight: FontWeight.w700,
          ),
        ),
        backgroundColor: kBg,
        elevation: 0,
        actions: [
          if (_stage == _Stage.main)
            IconButton(
              tooltip: 'Lock',
              icon: const Icon(Icons.lock_outline, color: kCyan),
              onPressed: () async {
                await _svc.lock();
                if (!mounted) return;
                setState(() => _stage = _Stage.locked);
              },
            ),
        ],
      ),
      body: SafeArea(
        child: switch (_stage) {
          _Stage.loading => const Center(
              child: CircularProgressIndicator(color: kCyan, strokeWidth: 2),
            ),
          _Stage.none => _OnboardingChoice(
              onCreate: () => setState(() => _stage = _Stage.createPin),
              onRestore: () => setState(() => _stage = _Stage.restore),
              onNetwork: (n) => setState(() => _network = n),
              network: _network ?? 'mainnet-beta',
            ),
          _Stage.createPin => _CreatePinPanel(
              defaultNetwork: _network ?? 'mainnet-beta',
              onCreated: (info) {
                setState(() {
                  _justRevealedMnemonic = info.mnemonic;
                  _stage = _Stage.backupReveal;
                });
              },
              onCancel: () => setState(() => _stage = _Stage.none),
            ),
          _Stage.backupReveal => _BackupRevealPanel(
              mnemonic: _justRevealedMnemonic!,
              onConfirmed: () {
                _justRevealedMnemonic = null;
                setState(() => _stage = _Stage.main);
                unawaited(_refresh());
              },
            ),
          _Stage.restore => _RestorePanel(
              defaultNetwork: _network ?? 'mainnet-beta',
              onRestored: () {
                setState(() => _stage = _Stage.main);
                unawaited(_refresh());
              },
              onCancel: () => setState(() => _stage = _Stage.none),
            ),
          _Stage.locked => _UnlockPanel(
              onUnlocked: () {
                setState(() => _stage = _Stage.main);
                unawaited(_refresh());
              },
              onWipe: () async {
                final wipe = await _confirmWipe();
                if (!wipe) return;
                await _svc.wipe();
                if (!mounted) return;
                setState(() => _stage = _Stage.none);
              },
            ),
          _Stage.main => _MainPanel(
              pubkey: _svc.pubkey ?? '?',
              network: _svc.network ?? 'mainnet-beta',
              activeChain: _activeChain,
              onChainChange: _switchChain,
              solLamports: _solLamports,
              tokenBalances: _tokenBalances,
              ethAddress: _ethAddress,
              ethBalanceWei: _ethBalanceWei,
              evmTokenBalances: _evmTokenBalances,
              refreshing: _refreshing,
              error: _refreshError,
              onRefresh: _refresh,
            ),
        },
      ),
    );
  }

  Future<bool> _confirmWipe() async {
    return await showDialog<bool>(
          context: context,
          builder: (ctx) => AlertDialog(
            backgroundColor: kBgCard,
            title: Text('Wallet löschen?',
                style: GoogleFonts.orbitron(color: kMagenta, fontSize: 14)),
            content: Text(
              'Dies löscht deinen verschlüsselten Wallet-Blob unwiderruflich. '
              'Ohne deine 24-Wort-Recovery-Phrase ist dein Guthaben verloren. '
              'Sicher?',
              style: GoogleFonts.spaceMono(color: kWhite, fontSize: 12),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(ctx, false),
                child: Text('Abbrechen',
                    style: GoogleFonts.orbitron(color: kCyan, fontSize: 11)),
              ),
              TextButton(
                onPressed: () => Navigator.pop(ctx, true),
                child: Text('LÖSCHEN',
                    style: GoogleFonts.orbitron(
                        color: kMagenta,
                        fontSize: 11,
                        fontWeight: FontWeight.w700)),
              ),
            ],
          ),
        ) ??
        false;
  }
}

// ── Onboarding choice ────────────────────────────────────────────────────

class _OnboardingChoice extends StatelessWidget {
  final VoidCallback onCreate;
  final VoidCallback onRestore;
  final ValueChanged<String> onNetwork;
  final String network;
  const _OnboardingChoice({
    required this.onCreate,
    required this.onRestore,
    required this.onNetwork,
    required this.network,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const SizedBox(height: 32),
          // Logo / hero
          Center(
            child: Container(
              width: 96,
              height: 96,
              decoration: BoxDecoration(
                border: Border.all(color: kCyan, width: 2),
                boxShadow: neonGlow(kCyan, radius: 24),
              ),
              child: Center(
                child: Text(
                  'A',
                  style: GoogleFonts.orbitron(
                    color: kCyan,
                    fontSize: 56,
                    fontWeight: FontWeight.w900,
                  ),
                ),
              ),
            ),
          ),
          const SizedBox(height: 24),
          Text(
            'ARGOS WALLET',
            textAlign: TextAlign.center,
            style: GoogleFonts.orbitron(
              color: kWhite,
              fontSize: 20,
              letterSpacing: 4,
              fontWeight: FontWeight.w700,
            ),
          ),
          const SizedBox(height: 8),
          Text(
            'Non-custodial Solana Wallet · BIP39 + Argon2id\n'
            'Auto-Swap-on-Send · 0,5 % Plattform-Gebühr',
            textAlign: TextAlign.center,
            style: GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 11),
          ),
          const SizedBox(height: 40),
          _NetworkToggle(network: network, onChanged: onNetwork),
          const SizedBox(height: 32),
          OutlinedButton(
            onPressed: onCreate,
            style: OutlinedButton.styleFrom(
              padding: const EdgeInsets.symmetric(vertical: 18),
              side: const BorderSide(color: kCyan, width: 2),
              shape: const RoundedRectangleBorder(
                  borderRadius: BorderRadius.zero),
            ),
            child: Text(
              'NEUE WALLET ERSTELLEN',
              style: GoogleFonts.orbitron(
                color: kCyan,
                fontSize: 13,
                letterSpacing: 3,
                fontWeight: FontWeight.w700,
              ),
            ),
          ),
          const SizedBox(height: 12),
          TextButton(
            onPressed: onRestore,
            child: Text(
              'BESTEHENDE WALLET WIEDERHERSTELLEN',
              style: GoogleFonts.orbitron(
                color: kWhiteDim,
                fontSize: 11,
                letterSpacing: 2,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          const Spacer(),
          Text(
            'Schlüssel verlassen NIE dieses Gerät.\n'
            'Cloud-Sync · Telemetrie · Custodian = 0',
            textAlign: TextAlign.center,
            style: GoogleFonts.spaceMono(color: kGrayText, fontSize: 10),
          ),
          const SizedBox(height: 12),
        ],
      ),
    );
  }
}

class _NetworkToggle extends StatelessWidget {
  final String network;
  final ValueChanged<String> onChanged;
  const _NetworkToggle({required this.network, required this.onChanged});

  @override
  Widget build(BuildContext context) {
    Widget chip(String label, String value) {
      final active = network == value;
      return Expanded(
        child: GestureDetector(
          onTap: () => onChanged(value),
          child: Container(
            padding: const EdgeInsets.symmetric(vertical: 10),
            decoration: BoxDecoration(
              color: active ? kCyanDim : Colors.transparent,
              border: Border.all(color: active ? kCyan : kGray),
            ),
            child: Center(
              child: Text(
                label,
                style: GoogleFonts.orbitron(
                  color: active ? kCyan : kGrayText,
                  fontSize: 11,
                  letterSpacing: 2,
                  fontWeight: FontWeight.w700,
                ),
              ),
            ),
          ),
        ),
      );
    }

    return Row(
      children: [
        chip('MAINNET', 'mainnet-beta'),
        const SizedBox(width: 8),
        chip('DEVNET', 'devnet'),
      ],
    );
  }
}

// ── PIN create panel ─────────────────────────────────────────────────────

class _CreatePinPanel extends StatefulWidget {
  final String defaultNetwork;
  final ValueChanged<rust.ArgosWalletInfo> onCreated;
  final VoidCallback onCancel;
  const _CreatePinPanel({
    required this.defaultNetwork,
    required this.onCreated,
    required this.onCancel,
  });

  @override
  State<_CreatePinPanel> createState() => _CreatePinPanelState();
}

class _CreatePinPanelState extends State<_CreatePinPanel> {
  final _pin1 = TextEditingController();
  final _pin2 = TextEditingController();
  String? _error;
  bool _busy = false;
  late String _network = widget.defaultNetwork;

  @override
  void dispose() {
    _pin1.dispose();
    _pin2.dispose();
    super.dispose();
  }

  Future<void> _go() async {
    final p1 = _pin1.text.trim();
    final p2 = _pin2.text.trim();
    if (p1.length < 6) {
      setState(() => _error = 'PIN muss mindestens 6 Stellen haben.');
      return;
    }
    if (p1 != p2) {
      setState(() => _error = 'PINs stimmen nicht überein.');
      return;
    }
    setState(() {
      _error = null;
      _busy = true;
    });
    try {
      final info = await ArgosWalletService.instance.create(
        network: _network,
        pin: p1,
      );
      widget.onCreated(info);
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = '$e';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 16),
      child: ListView(
        children: [
          Text('PIN FESTLEGEN',
              style: GoogleFonts.orbitron(
                color: kCyan,
                fontSize: 18,
                letterSpacing: 3,
                fontWeight: FontWeight.w700,
              )),
          const SizedBox(height: 8),
          Text(
            'Die PIN verschlüsselt deine Recovery-Phrase mit Argon2id auf '
            'dem Gerät. Bei 10 Fehleingaben hintereinander wipet die App '
            'den Wallet.',
            style: GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 11),
          ),
          const SizedBox(height: 24),
          _NetworkToggle(
              network: _network, onChanged: (n) => setState(() => _network = n)),
          const SizedBox(height: 24),
          TextField(
            controller: _pin1,
            obscureText: true,
            keyboardType: TextInputType.number,
            inputFormatters: [FilteringTextInputFormatter.digitsOnly],
            decoration: const InputDecoration(labelText: 'NEUE PIN (≥ 6)'),
            style: GoogleFonts.spaceMono(color: kCyan, letterSpacing: 4),
          ),
          const SizedBox(height: 12),
          TextField(
            controller: _pin2,
            obscureText: true,
            keyboardType: TextInputType.number,
            inputFormatters: [FilteringTextInputFormatter.digitsOnly],
            decoration: const InputDecoration(labelText: 'PIN BESTÄTIGEN'),
            style: GoogleFonts.spaceMono(color: kCyan, letterSpacing: 4),
          ),
          if (_error != null) ...[
            const SizedBox(height: 12),
            Text(_error!,
                style: GoogleFonts.spaceMono(color: kMagenta, fontSize: 11)),
          ],
          const SizedBox(height: 24),
          ElevatedButton(
            onPressed: _busy ? null : _go,
            child: _busy
                ? const SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(
                        color: kCyan, strokeWidth: 2),
                  )
                : const Text('WALLET ERSTELLEN'),
          ),
          const SizedBox(height: 8),
          TextButton(
            onPressed: widget.onCancel,
            child: Text('ABBRECHEN',
                style: GoogleFonts.orbitron(
                    color: kWhiteDim, fontSize: 11, letterSpacing: 2)),
          ),
        ],
      ),
    );
  }
}

// ── Mnemonic backup reveal ───────────────────────────────────────────────

class _BackupRevealPanel extends StatefulWidget {
  final String mnemonic;
  final VoidCallback onConfirmed;
  const _BackupRevealPanel({
    required this.mnemonic,
    required this.onConfirmed,
  });

  @override
  State<_BackupRevealPanel> createState() => _BackupRevealPanelState();
}

class _BackupRevealPanelState extends State<_BackupRevealPanel> {
  bool _revealed = false;
  bool _wroteDown = false;

  @override
  Widget build(BuildContext context) {
    final words = widget.mnemonic.trim().split(RegExp(r'\s+'));
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
      child: ListView(
        children: [
          Text('RECOVERY-PHRASE',
              style: GoogleFonts.orbitron(
                  color: kYellow,
                  fontSize: 18,
                  letterSpacing: 3,
                  fontWeight: FontWeight.w700)),
          const SizedBox(height: 8),
          Text(
            'Diese 24 Wörter sind der EINZIGE Weg, deine Wallet '
            'wiederherzustellen. Schreibe sie auf Papier. NIE in Cloud / '
            'Foto / Screenshot speichern.',
            style: GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 11),
          ),
          const SizedBox(height: 16),
          Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              color: kBgInput,
              border: Border.all(color: _revealed ? kCyan : kGray),
            ),
            child: _revealed
                ? GridView.builder(
                    shrinkWrap: true,
                    physics: const NeverScrollableScrollPhysics(),
                    gridDelegate:
                        const SliverGridDelegateWithFixedCrossAxisCount(
                      crossAxisCount: 3,
                      childAspectRatio: 2.6,
                      crossAxisSpacing: 6,
                      mainAxisSpacing: 6,
                    ),
                    itemCount: words.length,
                    itemBuilder: (_, i) {
                      final n = (i + 1).toString().padLeft(2, '0');
                      return Container(
                        padding: const EdgeInsets.symmetric(
                            horizontal: 8, vertical: 6),
                        decoration: BoxDecoration(
                          color: kBgCard,
                          border: Border.all(color: kCyanDim),
                        ),
                        child: RichText(
                          text: TextSpan(
                            children: [
                              TextSpan(
                                  text: '$n ',
                                  style: GoogleFonts.spaceMono(
                                      color: kGrayText, fontSize: 11)),
                              TextSpan(
                                  text: words[i],
                                  style: GoogleFonts.spaceMono(
                                      color: kCyan,
                                      fontSize: 12,
                                      fontWeight: FontWeight.w700)),
                            ],
                          ),
                        ),
                      );
                    },
                  )
                : GestureDetector(
                    onTap: () => setState(() => _revealed = true),
                    child: Container(
                      height: 240,
                      alignment: Alignment.center,
                      child: Column(
                        mainAxisAlignment: MainAxisAlignment.center,
                        children: [
                          const Icon(Icons.visibility_off,
                              color: kYellow, size: 40),
                          const SizedBox(height: 12),
                          Text('TAPPEN ZUM AUFDECKEN',
                              style: GoogleFonts.orbitron(
                                  color: kYellow,
                                  fontSize: 12,
                                  letterSpacing: 3)),
                          const SizedBox(height: 6),
                          Text('Schau dich um — niemand sollte zusehen.',
                              style: GoogleFonts.spaceMono(
                                  color: kWhiteDim, fontSize: 10)),
                        ],
                      ),
                    ),
                  ),
          ),
          if (_revealed) ...[
            const SizedBox(height: 12),
            CheckboxListTile(
              value: _wroteDown,
              onChanged: (v) => setState(() => _wroteDown = v ?? false),
              activeColor: kCyan,
              checkColor: kBg,
              title: Text(
                'Ich habe alle 24 Wörter aufgeschrieben.',
                style: GoogleFonts.spaceMono(color: kWhite, fontSize: 12),
              ),
              dense: true,
              contentPadding: EdgeInsets.zero,
            ),
            const SizedBox(height: 12),
            ElevatedButton(
              onPressed: _wroteDown ? widget.onConfirmed : null,
              child: const Text('WEITER ZUR WALLET'),
            ),
          ],
        ],
      ),
    );
  }
}

// ── Restore panel ────────────────────────────────────────────────────────

class _RestorePanel extends StatefulWidget {
  final String defaultNetwork;
  final VoidCallback onRestored;
  final VoidCallback onCancel;
  const _RestorePanel({
    required this.defaultNetwork,
    required this.onRestored,
    required this.onCancel,
  });

  @override
  State<_RestorePanel> createState() => _RestorePanelState();
}

class _RestorePanelState extends State<_RestorePanel> {
  final _mn = TextEditingController();
  final _pin1 = TextEditingController();
  final _pin2 = TextEditingController();
  String? _error;
  bool _busy = false;
  late String _network = widget.defaultNetwork;

  @override
  void dispose() {
    _mn.dispose();
    _pin1.dispose();
    _pin2.dispose();
    super.dispose();
  }

  Future<void> _go() async {
    final words = _mn.text.trim();
    if (words.split(RegExp(r'\s+')).length < 12) {
      setState(() => _error = 'Mindestens 12 Wörter erforderlich.');
      return;
    }
    final p1 = _pin1.text.trim();
    if (p1.length < 6) {
      setState(() => _error = 'PIN muss mindestens 6 Stellen haben.');
      return;
    }
    if (p1 != _pin2.text.trim()) {
      setState(() => _error = 'PINs stimmen nicht überein.');
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await ArgosWalletService.instance
          .restore(mnemonic: words, network: _network, pin: p1);
      widget.onRestored();
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = '$e';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
      child: ListView(
        children: [
          Text('WALLET WIEDERHERSTELLEN',
              style: GoogleFonts.orbitron(
                  color: kCyan,
                  fontSize: 16,
                  letterSpacing: 2,
                  fontWeight: FontWeight.w700)),
          const SizedBox(height: 12),
          _NetworkToggle(
              network: _network, onChanged: (n) => setState(() => _network = n)),
          const SizedBox(height: 12),
          TextField(
            controller: _mn,
            maxLines: 4,
            decoration: const InputDecoration(
              labelText: '12 ODER 24 WÖRTER',
              hintText: 'maple gravity ...',
            ),
            style: GoogleFonts.spaceMono(color: kCyan, fontSize: 13),
          ),
          const SizedBox(height: 12),
          TextField(
            controller: _pin1,
            obscureText: true,
            keyboardType: TextInputType.number,
            inputFormatters: [FilteringTextInputFormatter.digitsOnly],
            decoration: const InputDecoration(labelText: 'NEUE PIN (≥ 6)'),
            style: GoogleFonts.spaceMono(color: kCyan, letterSpacing: 4),
          ),
          const SizedBox(height: 12),
          TextField(
            controller: _pin2,
            obscureText: true,
            keyboardType: TextInputType.number,
            inputFormatters: [FilteringTextInputFormatter.digitsOnly],
            decoration: const InputDecoration(labelText: 'PIN BESTÄTIGEN'),
            style: GoogleFonts.spaceMono(color: kCyan, letterSpacing: 4),
          ),
          if (_error != null) ...[
            const SizedBox(height: 12),
            Text(_error!,
                style: GoogleFonts.spaceMono(color: kMagenta, fontSize: 11)),
          ],
          const SizedBox(height: 20),
          ElevatedButton(
            onPressed: _busy ? null : _go,
            child: _busy
                ? const SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(
                        color: kCyan, strokeWidth: 2),
                  )
                : const Text('WIEDERHERSTELLEN'),
          ),
          const SizedBox(height: 8),
          TextButton(
            onPressed: widget.onCancel,
            child: Text('ABBRECHEN',
                style: GoogleFonts.orbitron(
                    color: kWhiteDim, fontSize: 11, letterSpacing: 2)),
          ),
        ],
      ),
    );
  }
}

// ── Unlock panel ─────────────────────────────────────────────────────────

class _UnlockPanel extends StatefulWidget {
  final VoidCallback onUnlocked;
  final VoidCallback onWipe;
  const _UnlockPanel({required this.onUnlocked, required this.onWipe});

  @override
  State<_UnlockPanel> createState() => _UnlockPanelState();
}

class _UnlockPanelState extends State<_UnlockPanel> {
  static const _maxAttempts = 10;
  final _pin = TextEditingController();
  String? _error;
  bool _busy = false;

  @override
  void dispose() {
    _pin.dispose();
    super.dispose();
  }

  Future<void> _go() async {
    final p = _pin.text.trim();
    if (p.isEmpty) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await ArgosWalletService.instance.unlock(p);
      // Reset the PERSISTED counter on success.
      await ArgosWalletService.instance.resetFailedAttempts();
      widget.onUnlocked();
    } catch (e) {
      if (!mounted) return;
      _pin.clear();
      // Increment the PERSISTED counter — survives app restart / background,
      // so an attacker cannot dodge the 10-try wipe by force-stopping the
      // app between attempt batches (the old in-memory _attempts reset to 0
      // on every rebuild).
      final attempts =
          await ArgosWalletService.instance.incrementFailedAttempts();
      if (attempts >= _maxAttempts) {
        // Panic-wipe: defense against an attacker brute-forcing the PIN
        // on a stolen device. The encrypted blob + mnemonic sidecar are
        // destroyed; only the recovery phrase can rebuild the wallet.
        await ArgosWalletService.instance.wipe();
        widget.onWipe();
        return;
      }
      final remaining = _maxAttempts - attempts;
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = (remaining <= 3)
            ? 'Falsche PIN. Noch $remaining Versuche — danach wird die Wallet gelöscht.'
            : 'Falsche PIN. $remaining Versuche übrig.';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const SizedBox(height: 40),
          const Icon(Icons.lock_outline, color: kCyan, size: 48),
          const SizedBox(height: 16),
          Text('WALLET ENTSPERREN',
              textAlign: TextAlign.center,
              style: GoogleFonts.orbitron(
                  color: kCyan,
                  fontSize: 16,
                  letterSpacing: 3,
                  fontWeight: FontWeight.w700)),
          const SizedBox(height: 32),
          TextField(
            controller: _pin,
            obscureText: true,
            keyboardType: TextInputType.number,
            inputFormatters: [FilteringTextInputFormatter.digitsOnly],
            decoration: const InputDecoration(labelText: 'PIN'),
            style: GoogleFonts.spaceMono(
                color: kCyan, letterSpacing: 8, fontSize: 18),
            onSubmitted: (_) => _go(),
          ),
          if (_error != null) ...[
            const SizedBox(height: 12),
            Text(_error!,
                style: GoogleFonts.spaceMono(color: kMagenta, fontSize: 11)),
          ],
          const SizedBox(height: 24),
          ElevatedButton(
            onPressed: _busy ? null : _go,
            child: _busy
                ? const SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(
                        color: kCyan, strokeWidth: 2),
                  )
                : const Text('ENTSPERREN'),
          ),
          const Spacer(),
          TextButton(
            onPressed: widget.onWipe,
            child: Text('WALLET LÖSCHEN (Recovery erforderlich)',
                style: GoogleFonts.spaceMono(
                    color: kMagenta, fontSize: 11)),
          ),
        ],
      ),
    );
  }
}

// ── Main wallet view ────────────────────────────────────────────────────

class _MainPanel extends StatelessWidget {
  final String pubkey;
  final String network;
  final ArgosChain activeChain;
  final ValueChanged<ArgosChain> onChainChange;
  final BigInt solLamports;
  final Map<String, BigInt> tokenBalances;
  final String ethAddress;
  final String ethBalanceWei;
  final Map<String, String> evmTokenBalances;
  final bool refreshing;
  final String? error;
  final Future<void> Function() onRefresh;

  const _MainPanel({
    required this.pubkey,
    required this.network,
    required this.activeChain,
    required this.onChainChange,
    required this.solLamports,
    required this.tokenBalances,
    required this.ethAddress,
    required this.ethBalanceWei,
    required this.evmTokenBalances,
    required this.refreshing,
    required this.error,
    required this.onRefresh,
  });

  String _short(String pk) =>
      pk.length > 12 ? '${pk.substring(0, 5)}…${pk.substring(pk.length - 5)}' : pk;

  String _solHuman(BigInt lam) {
    final whole = lam ~/ BigInt.from(1000000000);
    final frac = lam % BigInt.from(1000000000);
    final fracStr =
        frac.toString().padLeft(9, '0').substring(0, 4); // 4 decimals on UI
    return '$whole.$fracStr';
  }

  String _ethHumanFromDecStr(String wei) {
    // wei is a decimal-string; we render 4 fractional digits without
    // pulling a BigDecimal lib. Trim leading zeros, then place a decimal
    // point 18 chars from the right.
    final clean = wei.replaceAll(RegExp(r'[^0-9]'), '');
    if (clean.isEmpty || clean == '0') return '0.0000';
    final padded = clean.padLeft(19, '0');
    final whole = padded.substring(0, padded.length - 18);
    final frac = padded.substring(padded.length - 18, padded.length - 14);
    final wholeTrim = whole.replaceFirst(RegExp(r'^0+'), '');
    return '${wholeTrim.isEmpty ? '0' : wholeTrim}.$frac';
  }

  String _tokenHuman(BigInt raw, int decimals) {
    if (decimals == 0) return raw.toString();
    final pow = BigInt.from(10).pow(decimals);
    final whole = raw ~/ pow;
    final frac = raw % pow;
    final fracStr =
        frac.toString().padLeft(decimals, '0').substring(0, decimals < 4 ? decimals : 4);
    return '$whole.$fracStr';
  }

  @override
  Widget build(BuildContext context) {
    final displayAddr =
        activeChain.isSolana ? pubkey : (ethAddress.isEmpty ? '…' : ethAddress);
    return RefreshIndicator(
      color: kCyan,
      backgroundColor: kBgCard,
      onRefresh: onRefresh,
      child: ListView(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        children: [
          _ChainSwitcher(
            active: activeChain,
            onChange: onChainChange,
          ),
          const SizedBox(height: 12),
          // Address card
          Container(
            padding: const EdgeInsets.all(14),
            decoration: BoxDecoration(
              color: kBgCard,
              border: Border.all(color: kCyanDim),
            ),
            child: Row(
              children: [
                Container(
                  width: 40,
                  height: 40,
                  decoration: BoxDecoration(
                    border: Border.all(color: kCyan, width: 1.5),
                  ),
                  child: Center(
                    child: Text('A',
                        style: GoogleFonts.orbitron(
                            color: kCyan,
                            fontSize: 20,
                            fontWeight: FontWeight.w900)),
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(_short(displayAddr),
                          style: GoogleFonts.spaceMono(
                              color: kCyan,
                              fontSize: 14,
                              fontWeight: FontWeight.w700,
                              letterSpacing: 1)),
                      Text(activeChain.label.toUpperCase(),
                          style: GoogleFonts.orbitron(
                              color: kWhiteDim,
                              fontSize: 9,
                              letterSpacing: 2)),
                    ],
                  ),
                ),
                IconButton(
                  tooltip: 'Adresse kopieren',
                  icon: const Icon(Icons.copy_all, color: kWhite, size: 18),
                  onPressed: () {
                    Clipboard.setData(ClipboardData(text: displayAddr));
                    ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                      content: Text('Adresse kopiert',
                          style: GoogleFonts.spaceMono(color: kCyan)),
                      backgroundColor: kBgCard,
                      duration: const Duration(seconds: 2),
                    ));
                  },
                ),
              ],
            ),
          ),
          const SizedBox(height: 16),
          // Balance card
          Container(
            padding: const EdgeInsets.all(20),
            decoration: BoxDecoration(
              color: kBgCard,
              border: Border.all(color: kCyan),
              boxShadow: neonGlow(kCyan, radius: 16),
            ),
            child: Column(
              children: [
                Text('GUTHABEN',
                    style: GoogleFonts.orbitron(
                        color: kWhiteDim,
                        fontSize: 10,
                        letterSpacing: 3,
                        fontWeight: FontWeight.w700)),
                const SizedBox(height: 6),
                Row(
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.baseline,
                  textBaseline: TextBaseline.alphabetic,
                  children: [
                    Text(
                      activeChain.isSolana
                          ? _solHuman(solLamports)
                          : _ethHumanFromDecStr(ethBalanceWei),
                      style: GoogleFonts.orbitron(
                          color: kCyan,
                          fontSize: 36,
                          fontWeight: FontWeight.w700),
                    ),
                    const SizedBox(width: 8),
                    Text(activeChain.isSolana
                            ? 'SOL'
                            : (activeChain == ArgosChain.polygon
                                ? 'MATIC'
                                : 'ETH'),
                        style: GoogleFonts.orbitron(
                            color: kWhiteDim,
                            fontSize: 14,
                            letterSpacing: 2)),
                  ],
                ),
                if (refreshing)
                  const Padding(
                    padding: EdgeInsets.only(top: 6),
                    child: SizedBox(
                      width: 14,
                      height: 14,
                      child: CircularProgressIndicator(
                          color: kCyan, strokeWidth: 1.5),
                    ),
                  ),
                if (error != null)
                  Padding(
                    padding: const EdgeInsets.only(top: 8),
                    child: Text(error!,
                        textAlign: TextAlign.center,
                        style: GoogleFonts.spaceMono(
                            color: kMagenta, fontSize: 10)),
                  ),
              ],
            ),
          ),
          const SizedBox(height: 16),
          // Action bar
          Row(
            children: [
              Expanded(
                child: _ActionButton(
                  icon: Icons.arrow_upward,
                  label: 'SENDEN',
                  onTap: () => _openSendSheet(context),
                ),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: _ActionButton(
                  icon: Icons.arrow_downward,
                  label: 'EMPFANGEN',
                  onTap: () => _openReceiveSheet(context, pubkey),
                ),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: _ActionButton(
                  icon: Icons.swap_horiz,
                  label: 'SWAP',
                  onTap: () => _openSwapSheet(context),
                ),
              ),
            ],
          ),
          const SizedBox(height: 24),
          Text('TOKENS',
              style: GoogleFonts.orbitron(
                  color: kWhiteDim,
                  fontSize: 11,
                  letterSpacing: 3,
                  fontWeight: FontWeight.w700)),
          const SizedBox(height: 12),
          // Token list
          ...argosKnownTokens.map((t) {
            final bal = tokenBalances[t.mint] ?? BigInt.zero;
            return Container(
              margin: const EdgeInsets.only(bottom: 8),
              padding:
                  const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
              decoration: BoxDecoration(
                color: kBgCard,
                border: Border.all(color: kCyanDim),
              ),
              child: Row(
                children: [
                  Container(
                    width: 34,
                    height: 34,
                    decoration: BoxDecoration(
                      border: Border.all(color: kCyan),
                    ),
                    child: Center(
                      child: Text(
                        t.symbol.substring(0, 1),
                        style: GoogleFonts.orbitron(
                            color: kCyan, fontWeight: FontWeight.w700),
                      ),
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(t.symbol,
                            style: GoogleFonts.orbitron(
                                color: kWhite,
                                fontSize: 13,
                                letterSpacing: 2,
                                fontWeight: FontWeight.w700)),
                        Text(t.name,
                            style: GoogleFonts.spaceMono(
                                color: kWhiteDim, fontSize: 10)),
                      ],
                    ),
                  ),
                  Text(_tokenHuman(bal, t.decimals),
                      style: GoogleFonts.spaceMono(
                          color: bal > BigInt.zero ? kCyan : kGrayText,
                          fontSize: 14,
                          fontWeight: FontWeight.w700)),
                ],
              ),
            );
          }),
          if (network == 'devnet') ...[
            const SizedBox(height: 16),
            OutlinedButton.icon(
              icon: const Icon(Icons.water_drop_outlined,
                  color: kYellow, size: 16),
              label: Text('DEVNET AIRDROP · 1 SOL',
                  style: GoogleFonts.orbitron(
                      color: kYellow,
                      fontSize: 11,
                      letterSpacing: 2,
                      fontWeight: FontWeight.w700)),
              style: OutlinedButton.styleFrom(
                side: const BorderSide(color: kYellow),
                padding: const EdgeInsets.symmetric(vertical: 12),
              ),
              onPressed: () async {
                try {
                  final sig = await ArgosWalletService.instance
                      .devnetAirdropOneSol();
                  if (!context.mounted) return;
                  ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                    content: Text('Airdrop: ${sig.substring(0, 12)}…',
                        style: GoogleFonts.spaceMono(color: kCyan)),
                    backgroundColor: kBgCard,
                  ));
                  await onRefresh();
                } catch (e) {
                  if (!context.mounted) return;
                  ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                    content: Text('Airdrop fehlgeschlagen: $e',
                        style: GoogleFonts.spaceMono(color: kMagenta)),
                    backgroundColor: kBgCard,
                  ));
                }
              },
            ),
          ],
        ],
      ),
    );
  }

  Future<void> _openSendSheet(BuildContext context) async {
    final res = await showModalBottomSheet<bool>(
      context: context,
      backgroundColor: kBgCard,
      isScrollControlled: true,
      shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(8))),
      builder: (_) => Padding(
        padding: EdgeInsets.only(
          bottom: MediaQuery.of(context).viewInsets.bottom,
        ),
        child: const _SendSheet(),
      ),
    );
    if (res == true) await onRefresh();
  }

  Future<void> _openReceiveSheet(BuildContext context, String pk) async {
    await showModalBottomSheet<void>(
      context: context,
      backgroundColor: kBgCard,
      isScrollControlled: true,
      shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(8))),
      builder: (_) => _ReceiveSheet(pubkey: pk),
    );
  }

  Future<void> _openSwapSheet(BuildContext context) async {
    final res = await showModalBottomSheet<bool>(
      context: context,
      backgroundColor: kBgCard,
      isScrollControlled: true,
      shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(8))),
      builder: (_) => Padding(
        padding: EdgeInsets.only(
          bottom: MediaQuery.of(context).viewInsets.bottom,
        ),
        child: const _SwapSheet(),
      ),
    );
    if (res == true) await onRefresh();
  }
}

class _ActionButton extends StatelessWidget {
  final IconData icon;
  final String label;
  final VoidCallback onTap;
  const _ActionButton({
    required this.icon,
    required this.label,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(vertical: 14),
        decoration: BoxDecoration(
          color: kBgCard,
          border: Border.all(color: kCyan, width: 1.5),
        ),
        child: Column(
          children: [
            Icon(icon, color: kCyan, size: 22),
            const SizedBox(height: 4),
            Text(label,
                style: GoogleFonts.orbitron(
                    color: kCyan,
                    fontSize: 10,
                    letterSpacing: 2,
                    fontWeight: FontWeight.w700)),
          ],
        ),
      ),
    );
  }
}

// ── Receive sheet ────────────────────────────────────────────────────────

class _ReceiveSheet extends StatelessWidget {
  final String pubkey;
  const _ReceiveSheet({required this.pubkey});

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('EMPFANGEN',
                style: GoogleFonts.orbitron(
                    color: kCyan,
                    fontSize: 14,
                    letterSpacing: 3,
                    fontWeight: FontWeight.w700)),
            const SizedBox(height: 16),
            Container(
              padding: const EdgeInsets.all(16),
              decoration: BoxDecoration(
                color: Colors.white,
                border: Border.all(color: kCyan, width: 2),
              ),
              child: QrImageView(
                data: pubkey,
                version: QrVersions.auto,
                size: 220,
                backgroundColor: Colors.white,
                eyeStyle:
                    const QrEyeStyle(eyeShape: QrEyeShape.square, color: kBg),
                dataModuleStyle: const QrDataModuleStyle(
                    dataModuleShape: QrDataModuleShape.square, color: kBg),
              ),
            ),
            const SizedBox(height: 16),
            SelectableText(
              pubkey,
              textAlign: TextAlign.center,
              style: GoogleFonts.spaceMono(
                  color: kCyan,
                  fontSize: 11,
                  letterSpacing: 0.5,
                  fontWeight: FontWeight.w600),
            ),
            const SizedBox(height: 12),
            Row(
              children: [
                Expanded(
                  child: OutlinedButton.icon(
                    icon: const Icon(Icons.copy, color: kCyan, size: 16),
                    label: Text('KOPIEREN',
                        style: GoogleFonts.orbitron(
                            color: kCyan,
                            fontSize: 11,
                            letterSpacing: 2)),
                    onPressed: () {
                      Clipboard.setData(ClipboardData(text: pubkey));
                      ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                        content: Text('Adresse kopiert',
                            style: GoogleFonts.spaceMono(color: kCyan)),
                        backgroundColor: kBgCard,
                      ));
                    },
                  ),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: ElevatedButton(
                    onPressed: () => Navigator.pop(context),
                    child: const Text('FERTIG'),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

// ── Send sheet (SOL + SPL token) ─────────────────────────────────────────

class _SendSheet extends StatefulWidget {
  const _SendSheet();

  @override
  State<_SendSheet> createState() => _SendSheetState();
}

class _SendSheetState extends State<_SendSheet> {
  String _asset = 'SOL';
  final _recipient = TextEditingController();
  final _amount = TextEditingController();
  String? _error;
  bool _busy = false;
  String? _confirmedSig;

  @override
  void dispose() {
    _recipient.dispose();
    _amount.dispose();
    super.dispose();
  }

  Future<bool?> _showRiskDialog(RiskVerdict v) async {
    final headlineColor = switch (v.severity) {
      'red' => kMagenta,
      'amber' => kYellow,
      _ => kCyan,
    };
    return showDialog<bool>(
      context: context,
      barrierDismissible: false,
      builder: (ctx) => AlertDialog(
        backgroundColor: kBgCard,
        title: Row(
          children: [
            Icon(
              v.severity == 'red'
                  ? Icons.warning_amber_rounded
                  : Icons.info_outline,
              color: headlineColor,
            ),
            const SizedBox(width: 10),
            Text(
              v.severity == 'red'
                  ? 'Risiko erkannt'
                  : 'Risk-Check Hinweis',
              style: GoogleFonts.orbitron(
                color: headlineColor,
                fontSize: 14,
                fontWeight: FontWeight.w700,
                letterSpacing: 2,
              ),
            ),
          ],
        ),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              v.summary,
              style: GoogleFonts.spaceMono(color: kWhite, fontSize: 12),
            ),
            const SizedBox(height: 12),
            Text(
              'Quelle: argos.dc-infosec.de/api/risk',
              style: GoogleFonts.spaceMono(color: kGrayText, fontSize: 10),
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx, false),
            child: Text(
              'Abbrechen',
              style: GoogleFonts.orbitron(
                color: kCyan,
                fontSize: 11,
                letterSpacing: 2,
              ),
            ),
          ),
          TextButton(
            onPressed: () => Navigator.pop(ctx, true),
            child: Text(
              v.severity == 'red' ? 'TROTZDEM SENDEN' : 'Senden',
              style: GoogleFonts.orbitron(
                color: headlineColor,
                fontSize: 11,
                fontWeight: FontWeight.w700,
                letterSpacing: 2,
              ),
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _go() async {
    final raw = _recipient.text.trim();
    if (raw.isEmpty) {
      setState(() => _error = 'Empfänger fehlt.');
      return;
    }
    final sendDecimals = _asset == 'SOL'
        ? 9
        : argosKnownTokens.firstWhere((t) => t.symbol == _asset).decimals;
    final baseUnits = decimalToBaseUnits(_amount.text, sendDecimals);
    if (baseUnits == null) {
      setState(() => _error = 'Betrag ungültig.');
      return;
    }
    String validated;
    try {
      validated = ArgosWalletService.instance.validateAddress(raw);
    } catch (e) {
      setState(() => _error = '$e');
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    // Argos Pre-Send Risk-Check (v1.2.2). Calls argos.dc-infosec.de/api/risk
    // for the recipient + the token mint about to be transferred. The
    // backend currently returns hardcoded clean-for-USDC/USDT/wSOL and
    // amber for everything unknown; the real Pylonyx scoring engine will
    // replace it without a wire-format change.
    final mintForCheck = _asset == 'SOL'
        ? argosWsolMint
        : argosKnownTokens.firstWhere((t) => t.symbol == _asset).mint;
    final verdict = await RiskCheckService.check(
      recipient: validated,
      mint: mintForCheck,
    );
    if (verdict.shouldWarn && mounted) {
      final proceed = await _showRiskDialog(verdict);
      if (proceed != true) {
        setState(() => _busy = false);
        return;
      }
    }
    try {
      String sig;
      if (_asset == 'SOL') {
        sig = await ArgosWalletService.instance
            .sendSol(recipient: validated, lamports: baseUnits);
      } else {
        final tok = argosKnownTokens.firstWhere((t) => t.symbol == _asset);
        sig = await ArgosWalletService.instance
            .sendToken(mint: tok.mint, recipient: validated, amount: baseUnits);
      }
      if (!mounted) return;
      setState(() {
        _busy = false;
        _confirmedSig = sig;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = '$e';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_confirmedSig != null) {
      return SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(20),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Icon(Icons.check_circle_outline,
                  color: kGreen, size: 56),
              const SizedBox(height: 12),
              Text('GESENDET',
                  style: GoogleFonts.orbitron(
                      color: kGreen,
                      fontSize: 16,
                      letterSpacing: 3,
                      fontWeight: FontWeight.w700)),
              const SizedBox(height: 12),
              SelectableText('${_confirmedSig!.substring(0, 16)}…',
                  style: GoogleFonts.spaceMono(color: kCyan, fontSize: 12)),
              const SizedBox(height: 18),
              ElevatedButton(
                onPressed: () => Navigator.pop(context, true),
                child: const Text('FERTIG'),
              ),
            ],
          ),
        ),
      );
    }
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('SENDEN',
                style: GoogleFonts.orbitron(
                    color: kCyan,
                    fontSize: 14,
                    letterSpacing: 3,
                    fontWeight: FontWeight.w700)),
            const SizedBox(height: 16),
            _AssetChips(
              current: _asset,
              onChanged: (a) => setState(() => _asset = a),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _recipient,
              decoration: const InputDecoration(
                labelText: 'EMPFÄNGER · Solana-Adresse',
                hintText: '9xKL…',
              ),
              style: GoogleFonts.spaceMono(color: kCyan, fontSize: 12),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _amount,
              keyboardType:
                  const TextInputType.numberWithOptions(decimal: true),
              decoration: InputDecoration(
                labelText: 'BETRAG · $_asset',
              ),
              style: GoogleFonts.spaceMono(
                  color: kCyan, fontSize: 18, letterSpacing: 2),
            ),
            if (_error != null) ...[
              const SizedBox(height: 10),
              Text(_error!,
                  style: GoogleFonts.spaceMono(color: kMagenta, fontSize: 11)),
            ],
            const SizedBox(height: 18),
            ElevatedButton(
              onPressed: _busy ? null : _go,
              child: _busy
                  ? const SizedBox(
                      width: 18,
                      height: 18,
                      child: CircularProgressIndicator(
                          color: kCyan, strokeWidth: 2),
                    )
                  : Text('$_asset SENDEN'),
            ),
            const SizedBox(height: 6),
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: Text('ABBRECHEN',
                  style: GoogleFonts.orbitron(
                      color: kWhiteDim,
                      fontSize: 11,
                      letterSpacing: 2)),
            ),
          ],
        ),
      ),
    );
  }
}

class _AssetChips extends StatelessWidget {
  final String current;
  final ValueChanged<String> onChanged;
  const _AssetChips({required this.current, required this.onChanged});

  @override
  Widget build(BuildContext context) {
    final all = ['SOL', ...argosKnownTokens.map((t) => t.symbol)];
    return Row(
      children: all
          .map((a) {
            final active = current == a;
            return Padding(
              padding: const EdgeInsets.only(right: 8),
              child: GestureDetector(
                onTap: () => onChanged(a),
                child: Container(
                  padding: const EdgeInsets.symmetric(
                      horizontal: 14, vertical: 8),
                  decoration: BoxDecoration(
                    color: active ? kCyanDim : Colors.transparent,
                    border: Border.all(color: active ? kCyan : kGray),
                  ),
                  child: Text(a,
                      style: GoogleFonts.orbitron(
                          color: active ? kCyan : kGrayText,
                          fontSize: 11,
                          letterSpacing: 2,
                          fontWeight: FontWeight.w700)),
                ),
              ),
            );
          })
          .toList(),
    );
  }
}

// ── Swap sheet (Jupiter v6) ──────────────────────────────────────────────

class _SwapSheet extends StatefulWidget {
  const _SwapSheet();

  @override
  State<_SwapSheet> createState() => _SwapSheetState();
}

class _SwapSheetState extends State<_SwapSheet> {
  String _input = 'SOL';
  String _output = 'USDC';
  final _amount = TextEditingController();
  int _slippageBps = 50;
  rust.ArgosSwapPreview? _preview;
  String? _confirmedSig;
  String? _error;
  bool _busy = false;
  bool _autoSwapSend = false;
  final _recipient = TextEditingController();

  @override
  void dispose() {
    _amount.dispose();
    _recipient.dispose();
    super.dispose();
  }

  String _mintFor(String symbol) {
    if (symbol == 'SOL') return argosWsolMint;
    return argosKnownTokens.firstWhere((t) => t.symbol == symbol).mint;
  }

  int _decimalsFor(String symbol) {
    if (symbol == 'SOL') return 9;
    return argosKnownTokens.firstWhere((t) => t.symbol == symbol).decimals;
  }

  Future<void> _quote() async {
    final raw = decimalToBaseUnits(_amount.text, _decimalsFor(_input));
    if (raw == null) {
      setState(() => _error = 'Betrag ungültig.');
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
      _preview = null;
    });
    try {
      final p = await ArgosWalletService.instance.quoteSwap(
        inputMint: _mintFor(_input),
        outputMint: _mintFor(_output),
        amountIn: raw,
        slippageBps: _slippageBps,
      );
      if (!mounted) return;
      setState(() {
        _busy = false;
        _preview = p;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = '$e';
      });
    }
  }

  Future<void> _execute() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      String sig;
      if (_autoSwapSend) {
        final raw = _recipient.text.trim();
        if (raw.isEmpty) throw 'Empfänger fehlt.';
        final outcome =
            await ArgosWalletService.instance.swapAndSend(raw);
        sig = outcome.signatureB58;
      } else {
        sig = await ArgosWalletService.instance.executeSwap();
      }
      if (!mounted) return;
      setState(() {
        _busy = false;
        _confirmedSig = sig;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = '$e';
      });
    }
  }

  String _humanOut(BigInt raw) {
    final d = _decimalsFor(_output);
    final pow = BigInt.from(10).pow(d);
    final whole = raw ~/ pow;
    final frac = raw % pow;
    final fracStr =
        frac.toString().padLeft(d, '0').substring(0, d < 4 ? d : 4);
    return '$whole.$fracStr $_output';
  }

  @override
  Widget build(BuildContext context) {
    if (_confirmedSig != null) {
      return SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(20),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Icon(Icons.check_circle_outline,
                  color: kGreen, size: 56),
              const SizedBox(height: 12),
              Text(_autoSwapSend ? 'GESCHWAPT & GESENDET' : 'SWAP AUSGEFÜHRT',
                  style: GoogleFonts.orbitron(
                      color: kGreen,
                      fontSize: 14,
                      letterSpacing: 2,
                      fontWeight: FontWeight.w700)),
              const SizedBox(height: 12),
              SelectableText('${_confirmedSig!.substring(0, 16)}…',
                  style: GoogleFonts.spaceMono(color: kCyan, fontSize: 12)),
              const SizedBox(height: 18),
              ElevatedButton(
                onPressed: () => Navigator.pop(context, true),
                child: const Text('FERTIG'),
              ),
            ],
          ),
        ),
      );
    }
    return SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(20),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('SWAP · JUPITER v6',
                style: GoogleFonts.orbitron(
                    color: kCyan,
                    fontSize: 14,
                    letterSpacing: 3,
                    fontWeight: FontWeight.w700)),
            const SizedBox(height: 4),
            Text('Gebühr: 0,5 % an Argos-Treasury',
                style: GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 10)),
            const SizedBox(height: 16),
            Row(
              children: [
                Expanded(
                  child: _MintDropdown(
                    label: 'VON',
                    current: _input,
                    onChanged: (v) => setState(() {
                      _input = v;
                      _preview = null;
                    }),
                  ),
                ),
                const SizedBox(width: 8),
                IconButton(
                  icon: const Icon(Icons.swap_horiz, color: kCyan),
                  onPressed: () => setState(() {
                    final a = _input;
                    _input = _output;
                    _output = a;
                    _preview = null;
                  }),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: _MintDropdown(
                    label: 'ZU',
                    current: _output,
                    onChanged: (v) => setState(() {
                      _output = v;
                      _preview = null;
                    }),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _amount,
              keyboardType:
                  const TextInputType.numberWithOptions(decimal: true),
              decoration: InputDecoration(
                  labelText: 'BETRAG · $_input',
                  hintText: '0.0'),
              style: GoogleFonts.spaceMono(
                  color: kCyan, fontSize: 18, letterSpacing: 2),
              onChanged: (_) => setState(() => _preview = null),
            ),
            const SizedBox(height: 8),
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text('Slippage',
                    style: GoogleFonts.spaceMono(
                        color: kWhiteDim, fontSize: 11)),
                Row(
                  children: [25, 50, 100, 300]
                      .map((bps) {
                        final active = _slippageBps == bps;
                        return Padding(
                          padding: const EdgeInsets.only(left: 6),
                          child: GestureDetector(
                            onTap: () => setState(() {
                              _slippageBps = bps;
                              _preview = null;
                            }),
                            child: Container(
                              padding: const EdgeInsets.symmetric(
                                  horizontal: 8, vertical: 4),
                              decoration: BoxDecoration(
                                color: active
                                    ? kCyanDim
                                    : Colors.transparent,
                                border: Border.all(
                                    color: active ? kCyan : kGray),
                              ),
                              child: Text('${bps / 100}%',
                                  style: GoogleFonts.spaceMono(
                                      color: active ? kCyan : kGrayText,
                                      fontSize: 11,
                                      fontWeight: FontWeight.w700)),
                            ),
                          ),
                        );
                      })
                      .toList(),
                ),
              ],
            ),
            const SizedBox(height: 8),
            SwitchListTile(
              value: _autoSwapSend,
              onChanged: (v) => setState(() => _autoSwapSend = v),
              activeThumbColor: kCyan,
              dense: true,
              contentPadding: EdgeInsets.zero,
              title: Text('Auto-Swap-on-Send',
                  style: GoogleFonts.orbitron(
                      color: kCyan,
                      fontSize: 11,
                      letterSpacing: 2,
                      fontWeight: FontWeight.w700)),
              subtitle: Text(
                  'Output geht direkt an Empfänger (1 Signatur, 1 Tx)',
                  style: GoogleFonts.spaceMono(
                      color: kWhiteDim, fontSize: 10)),
            ),
            if (_autoSwapSend) ...[
              TextField(
                controller: _recipient,
                decoration: const InputDecoration(
                    labelText: 'EMPFÄNGER · Solana-Adresse'),
                style: GoogleFonts.spaceMono(color: kCyan, fontSize: 12),
              ),
              const SizedBox(height: 8),
            ],
            if (_preview != null) ...[
              Container(
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: kBgInput,
                  border: Border.all(color: kCyanDim),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Text('Du erhältst',
                            style: GoogleFonts.spaceMono(
                                color: kWhiteDim, fontSize: 10)),
                        Text(_humanOut(_preview!.amountOutExpected),
                            style: GoogleFonts.orbitron(
                                color: kCyan,
                                fontSize: 14,
                                fontWeight: FontWeight.w700)),
                      ],
                    ),
                    const SizedBox(height: 4),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Text('Mindestens',
                            style: GoogleFonts.spaceMono(
                                color: kWhiteDim, fontSize: 10)),
                        Text(_humanOut(_preview!.amountOutMin),
                            style: GoogleFonts.spaceMono(
                                color: kWhite, fontSize: 11)),
                      ],
                    ),
                    const SizedBox(height: 4),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Text('Route',
                            style: GoogleFonts.spaceMono(
                                color: kWhiteDim, fontSize: 10)),
                        Flexible(
                          child: Text(_preview!.routeLabel,
                              textAlign: TextAlign.right,
                              style: GoogleFonts.spaceMono(
                                  color: kCyan, fontSize: 10)),
                        ),
                      ],
                    ),
                    const SizedBox(height: 4),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Text('Argos-Gebühr (0,5 %)',
                            style: GoogleFonts.spaceMono(
                                color: kWhiteDim, fontSize: 10)),
                        Text(_humanOut(_preview!.platformFeeOut),
                            style: GoogleFonts.spaceMono(
                                color: kYellow, fontSize: 10)),
                      ],
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 12),
            ],
            if (_error != null) ...[
              Text(_error!,
                  style: GoogleFonts.spaceMono(
                      color: kMagenta, fontSize: 11)),
              const SizedBox(height: 8),
            ],
            ElevatedButton(
              onPressed: _busy
                  ? null
                  : (_preview == null ? _quote : _execute),
              child: _busy
                  ? const SizedBox(
                      width: 18,
                      height: 18,
                      child: CircularProgressIndicator(
                          color: kCyan, strokeWidth: 2),
                    )
                  : Text(_preview == null
                      ? 'PREVIEW HOLEN'
                      : (_autoSwapSend ? 'SWAP & SENDEN' : 'SWAP AUSFÜHREN')),
            ),
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: Text('ABBRECHEN',
                  style: GoogleFonts.orbitron(
                      color: kWhiteDim,
                      fontSize: 11,
                      letterSpacing: 2)),
            ),
          ],
        ),
      ),
    );
  }
}

class _MintDropdown extends StatelessWidget {
  final String label;
  final String current;
  final ValueChanged<String> onChanged;
  const _MintDropdown({
    required this.label,
    required this.current,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final all = ['SOL', ...argosKnownTokens.map((t) => t.symbol)];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(label,
            style: GoogleFonts.orbitron(
                color: kWhiteDim,
                fontSize: 9,
                letterSpacing: 2,
                fontWeight: FontWeight.w700)),
        const SizedBox(height: 4),
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 10),
          decoration: BoxDecoration(
            color: kBgInput,
            border: Border.all(color: kGray),
          ),
          child: DropdownButtonHideUnderline(
            child: DropdownButton<String>(
              value: current,
              isExpanded: true,
              dropdownColor: kBgCard,
              icon: const Icon(Icons.arrow_drop_down, color: kCyan),
              items: all
                  .map((a) => DropdownMenuItem(
                        value: a,
                        child: Text(a,
                            style: GoogleFonts.orbitron(
                                color: kCyan,
                                fontSize: 13,
                                letterSpacing: 2,
                                fontWeight: FontWeight.w700)),
                      ))
                  .toList(),
              onChanged: (v) => v != null ? onChanged(v) : null,
            ),
          ),
        ),
      ],
    );
  }
}


class _ChainSwitcher extends StatelessWidget {
  final ArgosChain active;
  final ValueChanged<ArgosChain> onChange;
  const _ChainSwitcher({required this.active, required this.onChange});

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      child: Row(
        children: ArgosChain.values.map((c) {
          final selected = c == active;
          return Padding(
            padding: const EdgeInsets.only(right: 6),
            child: GestureDetector(
              onTap: () => onChange(c),
              child: Container(
                padding:
                    const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                decoration: BoxDecoration(
                  color: selected ? kCyanDim : Colors.transparent,
                  border: Border.all(color: selected ? kCyan : kGray),
                ),
                child: Text(
                  c.shortLabel,
                  style: GoogleFonts.orbitron(
                    color: selected ? kCyan : kGrayText,
                    fontSize: 10,
                    letterSpacing: 2,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ),
            ),
          );
        }).toList(),
      ),
    );
  }
}
