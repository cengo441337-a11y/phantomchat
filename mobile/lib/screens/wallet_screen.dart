import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:qr_flutter/qr_flutter.dart';
import 'package:url_launcher/url_launcher.dart';

import '../services/price_service.dart';
import '../services/pro_service.dart';
import '../services/address_book.dart';
import '../services/app_lock_service.dart';
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
  double? _portfolioEur;

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
        unawaited(_updatePortfolio());
        final pk = _svc.pubkey;
        if (pk != null) unawaited(ProService.refresh(pk));
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
        unawaited(_updatePortfolio());
      }
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _refreshing = false;
        _refreshError = e.toString();
      });
    }
  }

  double _rawToHuman(BigInt raw, int decimals) {
    // Display-only conversion; double is fine for a portfolio estimate
    // (exact base-unit math is reserved for the send path).
    return raw.toDouble() / _pow10d(decimals);
  }

  double _pow10d(int n) {
    var r = 1.0;
    for (var i = 0; i < n; i++) {
      r *= 10;
    }
    return r;
  }

  Future<void> _updatePortfolio() async {
    double nativeHuman;
    final tokenHuman = <String, double>{};
    if (_activeChain.isSolana) {
      nativeHuman = _solLamports.toDouble() / 1e9;
      for (final t in argosKnownTokens) {
        final bal = _tokenBalances[t.mint];
        if (bal != null && bal > BigInt.zero) {
          tokenHuman[t.symbol] = _rawToHuman(bal, t.decimals);
        }
      }
    } else {
      nativeHuman =
          (BigInt.tryParse(_ethBalanceWei) ?? BigInt.zero).toDouble() / 1e18;
      for (final t in argosEvmKnownTokens.where((t) => t.chain == _activeChain)) {
        final raw = _evmTokenBalances[t.address];
        final bal = raw == null ? BigInt.zero : (BigInt.tryParse(raw) ?? BigInt.zero);
        if (bal > BigInt.zero) {
          tokenHuman[t.symbol] = _rawToHuman(bal, t.decimals);
        }
      }
    }
    final eur = await PriceService.portfolioEur(
      chain: _activeChain,
      nativeHuman: nativeHuman,
      tokenHuman: tokenHuman,
    );
    if (!mounted) return;
    setState(() => _portfolioEur = eur);
  }

  void _switchChain(ArgosChain next) {
    if (next == _activeChain) return;
    setState(() {
      _activeChain = next;
      _refreshError = null;
      _portfolioEur = null;
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
              portfolioEur: _portfolioEur,
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
  bool _bioEnabled = false;
  bool _bioAvailable = false;

  @override
  void initState() {
    super.initState();
    _loadBio();
  }

  Future<void> _loadBio() async {
    final enabled = await ArgosWalletService.instance.biometricUnlockEnabled();
    final avail = await AppLockService.biometricAvailable();
    if (!mounted) return;
    setState(() {
      _bioEnabled = enabled;
      _bioAvailable = avail;
    });
    // Auto-prompt biometric on open if it's set up (fast path).
    if (enabled && avail) _bioUnlock();
  }

  Future<void> _bioUnlock() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await ArgosWalletService.instance.unlockWithBiometric();
      widget.onUnlocked();
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = '$e';
      });
    }
  }

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
      // Offer to enable biometric unlock once, if available + not yet on.
      if (_bioAvailable && !_bioEnabled && mounted) {
        final enable = await showDialog<bool>(
          context: context,
          builder: (ctx) => AlertDialog(
            backgroundColor: kBgCard,
            title: Text('Biometrik aktivieren?',
                style: GoogleFonts.orbitron(color: kCyan, fontSize: 13)),
            content: Text(
              'Künftig per Fingerabdruck / FaceID entsperren statt PIN tippen. '
              'Die PIN bleibt als Fallback gültig.',
              style: GoogleFonts.spaceMono(color: kWhite, fontSize: 12),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(ctx, false),
                child: Text('Später',
                    style:
                        GoogleFonts.orbitron(color: kWhiteDim, fontSize: 11)),
              ),
              TextButton(
                onPressed: () => Navigator.pop(ctx, true),
                child: Text('Aktivieren',
                    style: GoogleFonts.orbitron(color: kCyan, fontSize: 11)),
              ),
            ],
          ),
        );
        if (enable == true) {
          await ArgosWalletService.instance.enableBiometricUnlock(p);
        }
      }
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
          if (_bioEnabled && _bioAvailable) ...[
            const SizedBox(height: 12),
            OutlinedButton.icon(
              onPressed: _busy ? null : _bioUnlock,
              icon: const Icon(Icons.fingerprint, color: kCyan),
              label: Text('Per Biometrik entsperren',
                  style: GoogleFonts.orbitron(
                      color: kCyan, fontSize: 11, letterSpacing: 1)),
            ),
          ],
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
  final double? portfolioEur;
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
    this.portfolioEur,
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
                  tooltip: 'Verlauf',
                  icon: const Icon(Icons.history, color: kWhite, size: 18),
                  onPressed: () =>
                      _openHistorySheet(context, activeChain, displayAddr),
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
                if (portfolioEur != null) ...[
                  const SizedBox(height: 4),
                  Text('≈ ${portfolioEur!.toStringAsFixed(2)} €',
                      style: GoogleFonts.spaceMono(
                          color: kGreen, fontSize: 13, letterSpacing: 1)),
                ],
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
                  onTap: () => _openReceiveSheet(context, displayAddr),
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
          if (activeChain.isSolana) ...[
            const SizedBox(height: 12),
            OutlinedButton.icon(
              icon: const Icon(Icons.image_outlined, color: kCyan, size: 16),
              label: Text('NFTs ansehen',
                  style: GoogleFonts.orbitron(
                      color: kCyan,
                      fontSize: 11,
                      letterSpacing: 2,
                      fontWeight: FontWeight.w700)),
              style: OutlinedButton.styleFrom(
                side: const BorderSide(color: kCyan),
                padding: const EdgeInsets.symmetric(vertical: 12),
              ),
              onPressed: () => _openNftSheet(context, pubkey),
            ),
            const SizedBox(height: 12),
            _ProCard(pubkey: pubkey),
            const SizedBox(height: 12),
            GestureDetector(
              onTap: () => _openStakingSheet(context),
              child: Container(
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: kBgCard,
                  border: Border.all(color: kCyanDim),
                ),
                child: Row(
                  children: [
                    const Icon(Icons.savings_outlined, color: kCyan, size: 22),
                    const SizedBox(width: 12),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text('SOL STAKEN',
                              style: GoogleFonts.orbitron(
                                  color: kCyan,
                                  fontSize: 12,
                                  letterSpacing: 2,
                                  fontWeight: FontWeight.w700)),
                          const SizedBox(height: 2),
                          Text('~7-8 % p.a. · liquid via jitoSOL',
                              style: GoogleFonts.spaceMono(
                                  color: kWhiteDim, fontSize: 10)),
                        ],
                      ),
                    ),
                    const Icon(Icons.chevron_right, color: kCyan, size: 20),
                  ],
                ),
              ),
            ),
          ],
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
        child: _SendSheet(chain: activeChain),
      ),
    );
    if (res == true) await onRefresh();
  }

  Future<void> _openStakingSheet(BuildContext context) async {
    final res = await showModalBottomSheet<bool>(
      context: context,
      backgroundColor: kBgCard,
      isScrollControlled: true,
      shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(8))),
      builder: (_) => Padding(
        padding: EdgeInsets.only(
            bottom: MediaQuery.of(context).viewInsets.bottom),
        child: const _StakingSheet(),
      ),
    );
    if (res == true) await onRefresh();
  }

  Future<void> _openNftSheet(BuildContext context, String owner) async {
    await showModalBottomSheet<void>(
      context: context,
      backgroundColor: kBgCard,
      isScrollControlled: true,
      shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(8))),
      builder: (_) => _NftSheet(owner: owner),
    );
  }

  Future<void> _openHistorySheet(
      BuildContext context, ArgosChain chain, String addr) async {
    await showModalBottomSheet<void>(
      context: context,
      backgroundColor: kBgCard,
      isScrollControlled: true,
      shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(8))),
      builder: (_) => _HistorySheet(chain: chain, address: addr),
    );
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
            SizedBox(
              width: double.infinity,
              child: OutlinedButton.icon(
                icon: const Icon(Icons.qr_code_2, color: kYellow, size: 16),
                label: Text('ZAHLUNG ANFORDERN (QR)',
                    style: GoogleFonts.orbitron(
                        color: kYellow, fontSize: 11, letterSpacing: 1)),
                style: OutlinedButton.styleFrom(
                    side: const BorderSide(color: kYellow)),
                onPressed: () {
                  showModalBottomSheet<void>(
                    context: context,
                    backgroundColor: kBgCard,
                    isScrollControlled: true,
                    shape: const RoundedRectangleBorder(
                        borderRadius:
                            BorderRadius.vertical(top: Radius.circular(8))),
                    builder: (_) => Padding(
                      padding: EdgeInsets.only(
                          bottom: MediaQuery.of(context).viewInsets.bottom),
                      child: _MerchantSheet(pubkey: pubkey),
                    ),
                  );
                },
              ),
            ),
            const SizedBox(height: 10),
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
  final ArgosChain chain;
  const _SendSheet({required this.chain});

  @override
  State<_SendSheet> createState() => _SendSheetState();
}

class _SendSheetState extends State<_SendSheet> {
  late String _asset;
  final _recipient = TextEditingController();
  final _amount = TextEditingController();
  String? _error;
  bool _busy = false;
  String? _confirmedSig;

  ArgosChain get _chain => widget.chain;

  /// Native-coin symbol for the active chain.
  String get _nativeSymbol => switch (_chain) {
        ArgosChain.solanaMainnet || ArgosChain.solanaDevnet => 'SOL',
        ArgosChain.ethereum || ArgosChain.base => 'ETH',
        ArgosChain.polygon => 'MATIC',
      };

  /// Selectable assets: native + the chain's known tokens.
  List<String> get _assetSymbols => _chain.isSolana
      ? ['SOL', ...argosKnownTokens.map((t) => t.symbol)]
      : [
          _nativeSymbol,
          ...argosEvmKnownTokens
              .where((t) => t.chain == _chain)
              .map((t) => t.symbol),
        ];

  /// Decimals for the currently-selected asset.
  int get _selectedDecimals {
    if (_asset == _nativeSymbol) return _chain.isSolana ? 9 : 18;
    if (_chain.isSolana) {
      return argosKnownTokens.firstWhere((t) => t.symbol == _asset).decimals;
    }
    return argosEvmKnownTokens
        .firstWhere((t) => t.chain == _chain && t.symbol == _asset)
        .decimals;
  }

  @override
  void initState() {
    super.initState();
    _asset = _nativeSymbol;
  }

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

  Future<void> _pickFromBook() async {
    final entries = await AddressBook.forChain(_chain.backendId);
    if (!mounted) return;
    if (entries.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(
        content: Text('Adressbuch leer — nach dem Senden kannst du speichern.',
            style: GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 11)),
        backgroundColor: kBgCard,
      ));
      return;
    }
    final picked = await showModalBottomSheet<AddressEntry>(
      context: context,
      backgroundColor: kBgCard,
      builder: (_) => SafeArea(
        child: ListView(
          shrinkWrap: true,
          padding: const EdgeInsets.all(16),
          children: [
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text('ADRESSBUCH',
                  style: GoogleFonts.orbitron(
                      color: kCyan, fontSize: 12, letterSpacing: 3)),
            ),
            ...entries.map((e) => ListTile(
                  leading: const Icon(Icons.person_outline, color: kCyan),
                  title: Text(e.name,
                      style: GoogleFonts.spaceGrotesk(color: kWhite)),
                  subtitle: Text(
                    e.address.length > 16
                        ? '${e.address.substring(0, 8)}…${e.address.substring(e.address.length - 6)}'
                        : e.address,
                    style:
                        GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 10),
                  ),
                  trailing: IconButton(
                    icon: const Icon(Icons.delete_outline,
                        color: kMagenta, size: 18),
                    onPressed: () async {
                      final nav = Navigator.of(context);
                      await AddressBook.remove(e);
                      nav.pop();
                    },
                  ),
                  onTap: () => Navigator.pop(context, e),
                )),
          ],
        ),
      ),
    );
    if (picked != null) {
      setState(() => _recipient.text = picked.address);
    }
  }

  Future<void> _maybeOfferSave(String address) async {
    final existing = await AddressBook.nameFor(address);
    if (existing != null || !mounted) return;
    final nameCtrl = TextEditingController();
    final save = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: kBgCard,
        title: Text('Empfänger speichern?',
            style: GoogleFonts.orbitron(color: kCyan, fontSize: 13)),
        content: TextField(
          controller: nameCtrl,
          decoration: const InputDecoration(labelText: 'Name'),
          style: GoogleFonts.spaceMono(color: kCyan),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx, false),
            child: Text('Nein',
                style: GoogleFonts.orbitron(color: kWhiteDim, fontSize: 11)),
          ),
          TextButton(
            onPressed: () => Navigator.pop(ctx, true),
            child: Text('Speichern',
                style: GoogleFonts.orbitron(color: kCyan, fontSize: 11)),
          ),
        ],
      ),
    );
    if (save == true && nameCtrl.text.trim().isNotEmpty) {
      await AddressBook.add(AddressEntry(
        name: nameCtrl.text.trim(),
        address: address,
        chain: _chain.isSolana ? 'mainnet-beta' : 'any',
      ));
    }
  }

  Future<void> _go() async {
    final raw = _recipient.text.trim();
    if (raw.isEmpty) {
      setState(() => _error = 'Empfänger fehlt.');
      return;
    }
    final baseUnits = decimalToBaseUnits(_amount.text, _selectedDecimals);
    if (baseUnits == null) {
      setState(() => _error = 'Betrag ungültig.');
      return;
    }
    // Address validation is chain-specific (base58 vs. EIP-55 hex).
    String validated;
    try {
      validated = _chain.isSolana
          ? ArgosWalletService.instance.validateAddress(raw)
          : await ArgosWalletService.instance.ethValidateAddress(raw);
    } catch (e) {
      setState(() => _error = '$e');
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    // Pre-Send Risk-Check is Solana-mint-based today; run it only on Solana.
    // EVM risk scoring lands with the real Pylonyx engine (see backlog).
    if (_chain.isSolana) {
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
    }
    try {
      String sig;
      if (_chain.isSolana) {
        if (_asset == 'SOL') {
          sig = await ArgosWalletService.instance
              .sendSol(recipient: validated, lamports: baseUnits);
        } else {
          final tok = argosKnownTokens.firstWhere((t) => t.symbol == _asset);
          sig = await ArgosWalletService.instance.sendToken(
              mint: tok.mint, recipient: validated, amount: baseUnits);
        }
      } else {
        // EVM send: native ETH/MATIC vs. ERC-20.
        final net = _chain.backendId;
        if (_asset == _nativeSymbol) {
          sig = await ArgosWalletService.instance.ethSendNative(
            network: net,
            recipient: validated,
            wei: baseUnits.toString(),
          );
        } else {
          final tok = argosEvmKnownTokens
              .firstWhere((t) => t.chain == _chain && t.symbol == _asset);
          sig = await ArgosWalletService.instance.ethSendErc20(
            network: net,
            token: tok.address,
            recipient: validated,
            amount: baseUnits.toString(),
          );
        }
      }
      if (!mounted) return;
      setState(() {
        _busy = false;
        _confirmedSig = sig;
      });
      unawaited(_maybeOfferSave(validated));
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
              symbols: _assetSymbols,
              current: _asset,
              onChanged: (a) => setState(() => _asset = a),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _recipient,
              decoration: InputDecoration(
                labelText: _chain.isSolana
                    ? 'EMPFÄNGER · Solana-Adresse'
                    : 'EMPFÄNGER · ${_chain.label}-Adresse (0x…)',
                hintText: _chain.isSolana ? '9xKL…' : '0x…',
                suffixIcon: IconButton(
                  tooltip: 'Adressbuch',
                  icon: const Icon(Icons.contacts_outlined,
                      color: kCyan, size: 18),
                  onPressed: _pickFromBook,
                ),
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
  final List<String> symbols;
  final String current;
  final ValueChanged<String> onChanged;
  const _AssetChips({
    required this.symbols,
    required this.current,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final all = symbols;
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
        feeBps: ProService.swapFeeBps,
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
            Text(
                ProService.isPro
                    ? 'Gebühr: 0,25 % · Argos Pro aktiv ★'
                    : 'Gebühr: 0,5 % an Argos-Treasury',
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


/// Transaction history. Solana renders an in-app list (recent signatures
/// with success/fail + relative time + Solscan deep-link). EVM chains link
/// out to the chain's block explorer for the address (no standard JSON-RPC
/// address-history; a full EVM list needs an indexer — backlog).
class _HistorySheet extends StatefulWidget {
  final ArgosChain chain;
  final String address;
  const _HistorySheet({required this.chain, required this.address});

  @override
  State<_HistorySheet> createState() => _HistorySheetState();
}

class _HistorySheetState extends State<_HistorySheet> {
  bool _loading = true;
  String? _error;
  List<dynamic> _rows = const [];

  @override
  void initState() {
    super.initState();
    if (widget.chain.isSolana) {
      _load();
    } else {
      _loading = false;
    }
  }

  Future<void> _load() async {
    try {
      final rows =
          await ArgosWalletService.instance.recentSignatures(limit: 25);
      if (!mounted) return;
      setState(() {
        _rows = rows;
        _loading = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = e.toString();
        _loading = false;
      });
    }
  }

  String _explorerBase() {
    switch (widget.chain) {
      case ArgosChain.ethereum:
        return 'https://etherscan.io/address/';
      case ArgosChain.base:
        return 'https://basescan.org/address/';
      case ArgosChain.polygon:
        return 'https://polygonscan.com/address/';
      case ArgosChain.solanaDevnet:
        return 'https://solscan.io/account/';
      case ArgosChain.solanaMainnet:
        return 'https://solscan.io/account/';
    }
  }

  String _solscanTx(String sig) => widget.chain == ArgosChain.solanaDevnet
      ? 'https://solscan.io/tx/$sig?cluster=devnet'
      : 'https://solscan.io/tx/$sig';

  String _ago(int unixSec) {
    if (unixSec <= 0) return '';
    final then = DateTime.fromMillisecondsSinceEpoch(unixSec * 1000);
    final d = DateTime.now().difference(then);
    if (d.inMinutes < 1) return 'gerade eben';
    if (d.inMinutes < 60) return 'vor ${d.inMinutes} min';
    if (d.inHours < 24) return 'vor ${d.inHours} h';
    return 'vor ${d.inDays} d';
  }

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('VERLAUF · ${widget.chain.label.toUpperCase()}',
                style: GoogleFonts.orbitron(
                    color: kCyan,
                    fontSize: 14,
                    letterSpacing: 3,
                    fontWeight: FontWeight.w700)),
            const SizedBox(height: 16),
            if (!widget.chain.isSolana)
              Column(
                children: [
                  Text(
                    'EVM-Verlauf öffnet sich im Block-Explorer.',
                    textAlign: TextAlign.center,
                    style:
                        GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 12),
                  ),
                  const SizedBox(height: 16),
                  ElevatedButton.icon(
                    icon: const Icon(Icons.open_in_new, color: kCyan, size: 16),
                    label: Text('Auf Explorer ansehen',
                        style: GoogleFonts.orbitron(
                            color: kCyan, fontSize: 11, letterSpacing: 2)),
                    onPressed: () {
                      final url = '${_explorerBase()}${widget.address}';
                      Clipboard.setData(ClipboardData(text: url));
                      ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                        content: Text('Explorer-Link kopiert: $url',
                            style: GoogleFonts.spaceMono(
                                color: kCyan, fontSize: 10)),
                        backgroundColor: kBgCard,
                        duration: const Duration(seconds: 5),
                      ));
                    },
                  ),
                ],
              )
            else if (_loading)
              const Padding(
                padding: EdgeInsets.all(24),
                child: CircularProgressIndicator(color: kCyan, strokeWidth: 2),
              )
            else if (_error != null)
              Text(_error!,
                  textAlign: TextAlign.center,
                  style: GoogleFonts.spaceMono(color: kMagenta, fontSize: 11))
            else if (_rows.isEmpty)
              Padding(
                padding: const EdgeInsets.all(16),
                child: Text('Noch keine Transaktionen.',
                    style:
                        GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 12)),
              )
            else
              ConstrainedBox(
                constraints: BoxConstraints(
                    maxHeight: MediaQuery.of(context).size.height * 0.55),
                child: ListView.separated(
                  shrinkWrap: true,
                  itemCount: _rows.length,
                  separatorBuilder: (context, index) => const SizedBox(height: 6),
                  itemBuilder: (_, i) {
                    final r = _rows[i];
                    final sig = r.signatureB58 as String;
                    final failed = r.failed as bool;
                    final bt = (r.blockTime as BigInt).toInt();
                    return GestureDetector(
                      onTap: () {
                        final url = _solscanTx(sig);
                        Clipboard.setData(ClipboardData(text: url));
                        ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                          content: Text('Solscan-Link kopiert',
                              style:
                                  GoogleFonts.spaceMono(color: kCyan)),
                          backgroundColor: kBgCard,
                          duration: const Duration(seconds: 2),
                        ));
                      },
                      child: Container(
                        padding: const EdgeInsets.symmetric(
                            horizontal: 12, vertical: 10),
                        decoration: BoxDecoration(
                          color: kBgInput,
                          border: Border.all(
                              color: failed ? kMagDim : kCyanDim),
                        ),
                        child: Row(
                          children: [
                            Icon(
                              failed
                                  ? Icons.error_outline
                                  : Icons.check_circle_outline,
                              color: failed ? kMagenta : kGreen,
                              size: 16,
                            ),
                            const SizedBox(width: 10),
                            Expanded(
                              child: Text(
                                '${sig.substring(0, 8)}…${sig.substring(sig.length - 6)}',
                                style: GoogleFonts.spaceMono(
                                    color: kCyan, fontSize: 12),
                              ),
                            ),
                            Text(_ago(bt),
                                style: GoogleFonts.spaceMono(
                                    color: kWhiteDim, fontSize: 10)),
                            const SizedBox(width: 6),
                            const Icon(Icons.open_in_new,
                                color: kGrayText, size: 14),
                          ],
                        ),
                      ),
                    );
                  },
                ),
              ),
            const SizedBox(height: 16),
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: Text('SCHLIESSEN',
                  style: GoogleFonts.orbitron(
                      color: kWhiteDim, fontSize: 11, letterSpacing: 2)),
            ),
          ],
        ),
      ),
    );
  }
}


/// Solana NFT grid for the unlocked wallet. Images load from the metadata
/// URIs returned by Helius DAS (via the Argos backend proxy). Best-effort —
/// a broken image URI just shows a placeholder.
class _NftSheet extends StatefulWidget {
  final String owner;
  const _NftSheet({required this.owner});

  @override
  State<_NftSheet> createState() => _NftSheetState();
}

class _NftSheetState extends State<_NftSheet> {
  bool _loading = true;
  List<ArgosNft> _nfts = const [];

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final nfts = await fetchArgosNfts(widget.owner);
    if (!mounted) return;
    setState(() {
      _nfts = nfts;
      _loading = false;
    });
  }

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('NFTs',
                style: GoogleFonts.orbitron(
                    color: kCyan,
                    fontSize: 14,
                    letterSpacing: 3,
                    fontWeight: FontWeight.w700)),
            const SizedBox(height: 16),
            if (_loading)
              const Padding(
                padding: EdgeInsets.all(24),
                child: CircularProgressIndicator(color: kCyan, strokeWidth: 2),
              )
            else if (_nfts.isEmpty)
              Padding(
                padding: const EdgeInsets.all(20),
                child: Text('Keine NFTs in dieser Wallet.',
                    style:
                        GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 12)),
              )
            else
              ConstrainedBox(
                constraints: BoxConstraints(
                    maxHeight: MediaQuery.of(context).size.height * 0.6),
                child: GridView.builder(
                  shrinkWrap: true,
                  gridDelegate:
                      const SliverGridDelegateWithFixedCrossAxisCount(
                    crossAxisCount: 3,
                    crossAxisSpacing: 8,
                    mainAxisSpacing: 8,
                    childAspectRatio: 0.78,
                  ),
                  itemCount: _nfts.length,
                  itemBuilder: (_, i) {
                    final n = _nfts[i];
                    return Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Expanded(
                          child: Container(
                            decoration: BoxDecoration(
                              color: kBgInput,
                              border: Border.all(color: kCyanDim),
                            ),
                            clipBehavior: Clip.hardEdge,
                            child: (n.image != null && n.image!.isNotEmpty)
                                ? Image.network(
                                    n.image!,
                                    fit: BoxFit.cover,
                                    width: double.infinity,
                                    errorBuilder: (ctx2, err, stack) => const Center(
                                        child: Icon(Icons.broken_image,
                                            color: kGrayText, size: 24)),
                                    loadingBuilder: (c, child, prog) =>
                                        prog == null
                                            ? child
                                            : const Center(
                                                child: SizedBox(
                                                    width: 16,
                                                    height: 16,
                                                    child:
                                                        CircularProgressIndicator(
                                                            color: kCyan,
                                                            strokeWidth: 1.5))),
                                  )
                                : const Center(
                                    child: Icon(Icons.image,
                                        color: kGrayText, size: 24)),
                          ),
                        ),
                        const SizedBox(height: 4),
                        Text(n.name,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: GoogleFonts.spaceMono(
                                color: kWhite, fontSize: 9)),
                        if (n.collection != null)
                          Text(n.collection!,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: GoogleFonts.spaceMono(
                                  color: kWhiteDim, fontSize: 8)),
                      ],
                    );
                  },
                ),
              ),
            const SizedBox(height: 16),
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: Text('SCHLIESSEN',
                  style: GoogleFonts.orbitron(
                      color: kWhiteDim, fontSize: 11, letterSpacing: 2)),
            ),
          ],
        ),
      ),
    );
  }
}


/// Argos Pro upsell / status card. Pro is keyed by the Solana wallet
/// address (the subscription on Pylonyx links addr -> Stripe sub). Shows a
/// subscribe button (opens Stripe checkout in the browser) or an active
/// badge. Pro lowers the swap fee from 0,5 % to 0,25 %.
class _ProCard extends StatefulWidget {
  final String pubkey;
  const _ProCard({required this.pubkey});

  @override
  State<_ProCard> createState() => _ProCardState();
}

class _ProCardState extends State<_ProCard> {
  bool _busy = false;

  Future<void> _subscribe() async {
    setState(() => _busy = true);
    final url = await ProService.startCheckout(widget.pubkey);
    if (!mounted) return;
    setState(() => _busy = false);
    if (url == null) {
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(
        content: Text('Checkout konnte nicht gestartet werden.',
            style: GoogleFonts.spaceMono(color: kMagenta, fontSize: 11)),
        backgroundColor: kBgCard,
      ));
      return;
    }
    final uri = Uri.parse(url);
    if (await canLaunchUrl(uri)) {
      await launchUrl(uri, mode: LaunchMode.externalApplication);
    } else {
      // Fallback: copy the link.
      await Clipboard.setData(ClipboardData(text: url));
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(
        content: Text('Checkout-Link kopiert.',
            style: GoogleFonts.spaceMono(color: kCyan, fontSize: 11)),
        backgroundColor: kBgCard,
      ));
    }
  }

  @override
  Widget build(BuildContext context) {
    final isPro = ProService.isPro;
    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: kBgCard,
        border: Border.all(color: isPro ? kGreen : kYellow),
        boxShadow: isPro ? neonGlow(kGreen, radius: 10) : null,
      ),
      child: Row(
        children: [
          Icon(isPro ? Icons.star : Icons.star_border,
              color: isPro ? kGreen : kYellow, size: 22),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(isPro ? 'ARGOS PRO AKTIV' : 'ARGOS PRO',
                    style: GoogleFonts.orbitron(
                        color: isPro ? kGreen : kYellow,
                        fontSize: 12,
                        letterSpacing: 2,
                        fontWeight: FontWeight.w700)),
                const SizedBox(height: 2),
                Text(
                    isPro
                        ? 'Swap-Gebühr 0,25 % aktiv'
                        : '0,25 % Swap-Gebühr · 4 €/Monat',
                    style:
                        GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 10)),
              ],
            ),
          ),
          if (!isPro)
            ElevatedButton(
              onPressed: _busy ? null : _subscribe,
              style: ElevatedButton.styleFrom(
                padding:
                    const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
                side: const BorderSide(color: kYellow, width: 1.5),
                foregroundColor: kYellow,
              ),
              child: _busy
                  ? const SizedBox(
                      width: 14,
                      height: 14,
                      child: CircularProgressIndicator(
                          color: kYellow, strokeWidth: 1.5))
                  : Text('AKTIVIEREN',
                      style: GoogleFonts.orbitron(
                          fontSize: 10, letterSpacing: 1)),
            ),
        ],
      ),
    );
  }
}


/// Liquid staking via Jupiter: stake = swap SOL -> jitoSOL, unstake = swap
/// back. Yield (~7-8 % APY) accrues in the jitoSOL price. Reuses the swap
/// quote/execute path, so the Argos swap fee (Pro-aware) applies.
class _StakingSheet extends StatefulWidget {
  const _StakingSheet();
  @override
  State<_StakingSheet> createState() => _StakingSheetState();
}

class _StakingSheetState extends State<_StakingSheet> {
  final _svc = ArgosWalletService.instance;
  bool _unstake = false;
  final _amount = TextEditingController();
  String _stakedHuman = '…';
  rust.ArgosSwapPreview? _preview;
  String? _error;
  String? _doneSig;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    _loadStaked();
  }

  @override
  void dispose() {
    _amount.dispose();
    super.dispose();
  }

  Future<void> _loadStaked() async {
    try {
      final bal = await _svc.balanceToken(argosJitoSolMint);
      final pow = BigInt.from(10).pow(argosJitoSolDecimals);
      final whole = bal ~/ pow;
      final frac = (bal % pow)
          .toString()
          .padLeft(argosJitoSolDecimals, '0')
          .substring(0, 4);
      if (mounted) setState(() => _stakedHuman = '$whole.$frac');
    } catch (_) {
      if (mounted) setState(() => _stakedHuman = '0.0000');
    }
  }

  Future<void> _quote() async {
    final decimals = _unstake ? argosJitoSolDecimals : 9;
    final raw = decimalToBaseUnits(_amount.text, decimals);
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
      final input = _unstake ? argosJitoSolMint : argosWsolMint;
      final output = _unstake ? argosWsolMint : argosJitoSolMint;
      final p = await _svc.quoteSwap(
        inputMint: input,
        outputMint: output,
        amountIn: raw,
        slippageBps: 50,
        feeBps: ProService.swapFeeBps,
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
      final sig = await _svc.executeSwap();
      if (!mounted) return;
      setState(() {
        _busy = false;
        _doneSig = sig;
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
    if (_doneSig != null) {
      return SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(20),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Icon(Icons.check_circle_outline, color: kGreen, size: 56),
              const SizedBox(height: 12),
              Text(_unstake ? 'UNSTAKED' : 'GESTAKED',
                  style: GoogleFonts.orbitron(
                      color: kGreen,
                      fontSize: 16,
                      letterSpacing: 2,
                      fontWeight: FontWeight.w700)),
              const SizedBox(height: 12),
              SelectableText('${_doneSig!.substring(0, 16)}…',
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
            Text('STAKING',
                style: GoogleFonts.orbitron(
                    color: kCyan,
                    fontSize: 14,
                    letterSpacing: 3,
                    fontWeight: FontWeight.w700)),
            const SizedBox(height: 4),
            Text('Gestaked: $_stakedHuman jitoSOL',
                style: GoogleFonts.spaceMono(color: kGreen, fontSize: 12)),
            const SizedBox(height: 16),
            Row(
              children: [
                Expanded(
                  child: GestureDetector(
                    onTap: () => setState(() {
                      _unstake = false;
                      _preview = null;
                    }),
                    child: Container(
                      padding: const EdgeInsets.symmetric(vertical: 10),
                      decoration: BoxDecoration(
                        color: !_unstake ? kCyanDim : Colors.transparent,
                        border:
                            Border.all(color: !_unstake ? kCyan : kGray),
                      ),
                      child: Center(
                        child: Text('STAKEN',
                            style: GoogleFonts.orbitron(
                                color: !_unstake ? kCyan : kGrayText,
                                fontSize: 11,
                                letterSpacing: 2,
                                fontWeight: FontWeight.w700)),
                      ),
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: GestureDetector(
                    onTap: () => setState(() {
                      _unstake = true;
                      _preview = null;
                    }),
                    child: Container(
                      padding: const EdgeInsets.symmetric(vertical: 10),
                      decoration: BoxDecoration(
                        color: _unstake ? kCyanDim : Colors.transparent,
                        border: Border.all(color: _unstake ? kCyan : kGray),
                      ),
                      child: Center(
                        child: Text('UNSTAKEN',
                            style: GoogleFonts.orbitron(
                                color: _unstake ? kCyan : kGrayText,
                                fontSize: 11,
                                letterSpacing: 2,
                                fontWeight: FontWeight.w700)),
                      ),
                    ),
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
                  labelText: _unstake ? 'BETRAG · jitoSOL' : 'BETRAG · SOL'),
              style: GoogleFonts.spaceMono(
                  color: kCyan, fontSize: 18, letterSpacing: 2),
              onChanged: (_) => setState(() => _preview = null),
            ),
            if (_preview != null) ...[
              const SizedBox(height: 10),
              Text(
                  'Du erhältst ~${_humanOut(_preview!.amountOutExpected)} ${_unstake ? "SOL" : "jitoSOL"}',
                  style: GoogleFonts.spaceMono(color: kCyan, fontSize: 12)),
            ],
            if (_error != null) ...[
              const SizedBox(height: 10),
              Text(_error!,
                  style:
                      GoogleFonts.spaceMono(color: kMagenta, fontSize: 11)),
            ],
            const SizedBox(height: 16),
            ElevatedButton(
              onPressed: _busy ? null : (_preview == null ? _quote : _execute),
              child: _busy
                  ? const SizedBox(
                      width: 18,
                      height: 18,
                      child: CircularProgressIndicator(
                          color: kCyan, strokeWidth: 2))
                  : Text(_preview == null
                      ? 'PREVIEW'
                      : (_unstake ? 'UNSTAKEN' : 'STAKEN')),
            ),
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: Text('ABBRECHEN',
                  style: GoogleFonts.orbitron(
                      color: kWhiteDim, fontSize: 11, letterSpacing: 2)),
            ),
          ],
        ),
      ),
    );
  }

  String _humanOut(BigInt raw) {
    final d = _unstake ? 9 : argosJitoSolDecimals;
    final pow = BigInt.from(10).pow(d);
    final whole = raw ~/ pow;
    final frac = (raw % pow).toString().padLeft(d, '0').substring(0, 4);
    return '$whole.$frac';
  }
}


/// Merchant payment request — generates a Solana Pay URL + QR that ANY wallet
/// can scan to pay the user a fixed amount. Unlike the in-chat payment
/// request (which is E2E to one contact), this is a public QR for a shop /
/// invoice / tip-jar use case.
class _MerchantSheet extends StatefulWidget {
  final String pubkey;
  const _MerchantSheet({required this.pubkey});
  @override
  State<_MerchantSheet> createState() => _MerchantSheetState();
}

class _MerchantSheetState extends State<_MerchantSheet> {
  String _asset = 'SOL';
  final _amount = TextEditingController();
  String? _payUrl;

  @override
  void dispose() {
    _amount.dispose();
    super.dispose();
  }

  void _generate() {
    final amt = _amount.text.trim().replaceAll(',', '.');
    if (amt.isEmpty || double.tryParse(amt) == null) return;
    final label = Uri.encodeComponent('Argos Zahlung');
    String url;
    if (_asset == 'SOL') {
      url = 'solana:${widget.pubkey}?amount=$amt&label=$label';
    } else {
      final tok = argosKnownTokens.firstWhere((t) => t.symbol == _asset);
      url =
          'solana:${widget.pubkey}?amount=$amt&spl-token=${tok.mint}&label=$label';
    }
    setState(() => _payUrl = url);
  }

  @override
  Widget build(BuildContext context) {
    final assets = ['SOL', ...argosKnownTokens.map((t) => t.symbol)];
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('ZAHLUNG ANFORDERN',
                style: GoogleFonts.orbitron(
                    color: kYellow,
                    fontSize: 14,
                    letterSpacing: 3,
                    fontWeight: FontWeight.w700)),
            const SizedBox(height: 16),
            if (_payUrl == null) ...[
              Row(
                children: assets.map((a) {
                  final sel = a == _asset;
                  return Padding(
                    padding: const EdgeInsets.only(right: 8),
                    child: GestureDetector(
                      onTap: () => setState(() => _asset = a),
                      child: Container(
                        padding: const EdgeInsets.symmetric(
                            horizontal: 14, vertical: 8),
                        decoration: BoxDecoration(
                          color: sel ? kCyanDim : Colors.transparent,
                          border: Border.all(color: sel ? kCyan : kGray),
                        ),
                        child: Text(a,
                            style: GoogleFonts.orbitron(
                                color: sel ? kCyan : kGrayText,
                                fontSize: 11,
                                fontWeight: FontWeight.w700)),
                      ),
                    ),
                  );
                }).toList(),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _amount,
                keyboardType:
                    const TextInputType.numberWithOptions(decimal: true),
                decoration: InputDecoration(labelText: 'BETRAG · $_asset'),
                style: GoogleFonts.spaceMono(
                    color: kCyan, fontSize: 18, letterSpacing: 2),
              ),
              const SizedBox(height: 16),
              SizedBox(
                width: double.infinity,
                child: ElevatedButton(
                  onPressed: _generate,
                  child: const Text('QR ERZEUGEN'),
                ),
              ),
            ] else ...[
              Container(
                padding: const EdgeInsets.all(16),
                decoration: BoxDecoration(
                    color: Colors.white,
                    border: Border.all(color: kCyan, width: 2)),
                child: QrImageView(
                  data: _payUrl!,
                  version: QrVersions.auto,
                  size: 220,
                  backgroundColor: Colors.white,
                  eyeStyle: const QrEyeStyle(
                      eyeShape: QrEyeShape.square, color: kBg),
                  dataModuleStyle: const QrDataModuleStyle(
                      dataModuleShape: QrDataModuleShape.square, color: kBg),
                ),
              ),
              const SizedBox(height: 12),
              Text('${_amount.text} $_asset',
                  style: GoogleFonts.orbitron(
                      color: kCyan, fontSize: 16, letterSpacing: 1)),
              const SizedBox(height: 8),
              Text('Jede Solana-Wallet kann diesen QR scannen und zahlen.',
                  textAlign: TextAlign.center,
                  style:
                      GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 10)),
              const SizedBox(height: 12),
              OutlinedButton.icon(
                icon: const Icon(Icons.copy, color: kCyan, size: 16),
                label: Text('LINK KOPIEREN',
                    style: GoogleFonts.orbitron(
                        color: kCyan, fontSize: 11, letterSpacing: 2)),
                onPressed: () {
                  Clipboard.setData(ClipboardData(text: _payUrl!));
                  ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                    content: Text('Zahlungslink kopiert',
                        style: GoogleFonts.spaceMono(color: kCyan)),
                    backgroundColor: kBgCard,
                  ));
                },
              ),
            ],
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: Text('SCHLIESSEN',
                  style: GoogleFonts.orbitron(
                      color: kWhiteDim, fontSize: 11, letterSpacing: 2)),
            ),
          ],
        ),
      ),
    );
  }
}
