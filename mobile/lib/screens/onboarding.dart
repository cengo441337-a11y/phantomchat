import 'dart:async';
import 'dart:math';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import '../models/identity.dart';
import '../services/crypto_service.dart';
import '../services/storage_service.dart';
import '../theme.dart';
import '../widgets/glitch_text.dart';
import '../widgets/cyber_card.dart';
import 'home.dart';

class OnboardingScreen extends StatefulWidget {
  const OnboardingScreen({super.key});

  @override
  State<OnboardingScreen> createState() => _OnboardingScreenState();
}

class _OnboardingScreenState extends State<OnboardingScreen>
    with TickerProviderStateMixin {
  int _step = 0;
  final _nameCtrl = TextEditingController();
  PhantomIdentity? _identity;

  late AnimationController _fadeCtrl;
  late Animation<double> _fadeAnim;
  late AnimationController _bootCtrl;

  // Boot sequence
  List<String> _bootLines = [];
  Timer? _bootTimer;
  int _bootIndex = 0;
  bool _bootDone = false;

  static const _bootSequence = [
    '> PHANTOM OS v1.0.0',
    '> LOADING CRYPTO MODULE...',
    '> X25519 ECDH: OK',
    '> CHACHA20-POLY1305: OK',
    '> SECURE STORAGE: OK',
    '> NETWORK LAYER: ISOLATED',
    '> SYSTEM ONLINE',
  ];

  @override
  void initState() {
    super.initState();
    _fadeCtrl = AnimationController(vsync: this, duration: const Duration(milliseconds: 500));
    _fadeAnim = CurvedAnimation(parent: _fadeCtrl, curve: Curves.easeOut);
    _bootCtrl = AnimationController(vsync: this, duration: const Duration(milliseconds: 800));
    _fadeCtrl.forward();
    _runBoot();
  }

  void _runBoot() {
    _bootTimer = Timer.periodic(const Duration(milliseconds: 180), (_) {
      if (!mounted) return;
      if (_bootIndex < _bootSequence.length) {
        setState(() => _bootLines.add(_bootSequence[_bootIndex++]));
      } else {
        _bootTimer?.cancel();
        setState(() => _bootDone = true);
      }
    });
  }

  @override
  void dispose() {
    _fadeCtrl.dispose();
    _bootCtrl.dispose();
    _bootTimer?.cancel();
    _nameCtrl.dispose();
    super.dispose();
  }

  Future<void> _generateIdentity() async {
    if (_nameCtrl.text.trim().isEmpty) return;
    FocusScope.of(context).unfocus();
    setState(() => _step = 2);

    final viewKeys = await CryptoService.generateKeyPair();
    final spendKeys = await CryptoService.generateKeyPair();

    final identity = PhantomIdentity(
      id: spendKeys['public']!,
      nickname: _nameCtrl.text.trim(),
      privateViewKey: viewKeys['private']!,
      publicViewKey: viewKeys['public']!,
      privateSpendKey: spendKeys['private']!,
      publicSpendKey: spendKeys['public']!,
      createdAt: DateTime.now(),
    );

    await StorageService.saveIdentity(identity);
    if (mounted) setState(() { _identity = identity; _step = 3; });
  }

  void _stepTo(int step) {
    _fadeCtrl.reset();
    setState(() => _step = step);
    _fadeCtrl.forward();
  }

  void _finish() {
    Navigator.of(context).pushReplacement(
      PageRouteBuilder(
        pageBuilder: (_, __, ___) => const HomeScreen(),
        transitionDuration: const Duration(milliseconds: 400),
        transitionsBuilder: (_, anim, __, child) =>
            FadeTransition(opacity: anim, child: child),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      body: GridBackground(
        child: SafeArea(
          child: FadeTransition(
            opacity: _fadeAnim,
            child: _buildStep(),
          ),
        ),
      ),
    );
  }

  Widget _buildStep() {
    switch (_step) {
      case 0: return _buildWelcome();
      case 1: return _buildNameInput();
      case 2: return _buildGenerating();
      case 3: return _buildDone();
      default: return _buildWelcome();
    }
  }

  // ── STEP 0: Welcome ─────────────────────────────────────────────────────────
  Widget _buildWelcome() {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 28),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 48),

          // Logo + Title
          Center(
            child: Column(
              children: [
                PulseRing(
                  color: kCyan,
                  size: 96,
                  child: const Icon(Icons.shield_outlined, color: kCyan, size: 32),
                ),
                const SizedBox(height: 28),
                GlitchText(
                  text: 'PHANTOM',
                  style: GoogleFonts.orbitron(
                    fontSize: 46,
                    fontWeight: FontWeight.w900,
                    color: kWhite,
                    letterSpacing: 6,
                    shadows: [
                      Shadow(color: kCyan.withOpacity(0.8), blurRadius: 20),
                      Shadow(color: kCyan.withOpacity(0.3), blurRadius: 40),
                    ],
                  ),
                ),
                const SizedBox(height: 6),
                Text(
                  'DECENTRALIZED ENCRYPTED MESSENGER',
                  style: GoogleFonts.spaceMono(
                    fontSize: 9,
                    color: kCyan,
                    letterSpacing: 2,
                  ),
                ),
              ],
            ),
          ),

          const SizedBox(height: 36),

          // Boot sequence
          CyberCard(
            borderColor: kCyan,
            padding: const EdgeInsets.all(16),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                for (final line in _bootLines)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 3),
                    child: Text(
                      line,
                      style: GoogleFonts.spaceMono(
                        fontSize: 11,
                        color: line.contains('OK')
                            ? kGreen
                            : line.contains('ONLINE')
                                ? kCyan
                                : kGrayText,
                        letterSpacing: 0.5,
                      ),
                    ),
                  ),
                if (!_bootDone)
                  Row(
                    children: [
                      Text('> ', style: GoogleFonts.spaceMono(fontSize: 11, color: kCyan)),
                      const BlinkCursor(),
                    ],
                  ),
              ],
            ),
          ),

          const Spacer(),

          // Feature pills
          Row(
            children: [
              _Pill(label: 'X25519', color: kCyan),
              const SizedBox(width: 8),
              _Pill(label: 'ChaCha20', color: kCyan),
              const SizedBox(width: 8),
              _Pill(label: 'ZERO-LOG', color: kMagenta),
              const SizedBox(width: 8),
              _Pill(label: 'NO-SERVER', color: kMagenta),
            ],
          ),

          const SizedBox(height: 24),

          // CTA
          _CyberButton(
            label: 'INITIALIZE IDENTITY',
            onTap: _bootDone ? () => _stepTo(1) : null,
            width: double.infinity,
          ),

          const SizedBox(height: 12),
          Center(
            child: Text(
              'NO ACCOUNT · NO EMAIL · CRYPTOGRAPHY ONLY',
              style: GoogleFonts.spaceMono(fontSize: 9, color: kGrayText, letterSpacing: 1.5),
            ),
          ),
          const SizedBox(height: 24),
        ],
      ),
    );
  }

  // ── STEP 1: Name input ───────────────────────────────────────────────────────
  Widget _buildNameInput() {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 28),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 52),

          Text(
            '> ASSIGN\nCODENAME',
            style: GoogleFonts.orbitron(
              fontSize: 36,
              fontWeight: FontWeight.w900,
              color: kWhite,
              letterSpacing: 2,
              height: 1.1,
              shadows: [Shadow(color: kCyan.withOpacity(0.5), blurRadius: 16)],
            ),
          ),
          const SizedBox(height: 6),
          Text(
            '// STORED LOCALLY — NEVER TRANSMITTED',
            style: GoogleFonts.spaceMono(fontSize: 10, color: kGrayText),
          ),
          const SizedBox(height: 40),

          // Input inside CyberCard frame
          CyberCard(
            borderColor: kCyan,
            glow: true,
            padding: const EdgeInsets.all(16),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'CODENAME_INPUT:',
                  style: GoogleFonts.spaceMono(fontSize: 10, color: kCyan, letterSpacing: 1),
                ),
                const SizedBox(height: 10),
                TextField(
                  controller: _nameCtrl,
                  autofocus: true,
                  style: GoogleFonts.orbitron(color: kWhite, fontSize: 18, letterSpacing: 2),
                  decoration: InputDecoration(
                    border: InputBorder.none,
                    enabledBorder: InputBorder.none,
                    focusedBorder: InputBorder.none,
                    hintText: 'GHOST · CIPHER · NEXUS',
                    hintStyle: GoogleFonts.orbitron(fontSize: 14, color: kGray, letterSpacing: 2),
                    filled: false,
                    contentPadding: EdgeInsets.zero,
                  ),
                  onSubmitted: (_) => _generateIdentity(),
                ),
              ],
            ),
          ),

          const Spacer(),

          _CyberButton(
            label: 'GENERATE KEYS',
            onTap: _generateIdentity,
            width: double.infinity,
            color: kCyan,
          ),
          const SizedBox(height: 10),
          _CyberButton(
            label: '← BACK',
            onTap: () => _stepTo(0),
            width: double.infinity,
            color: kGrayText,
          ),
          const SizedBox(height: 24),
        ],
      ),
    );
  }

  // ── STEP 2: Generating ───────────────────────────────────────────────────────
  Widget _buildGenerating() {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(40),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            PulseRing(
              color: kMagenta,
              size: 100,
              child: const Icon(Icons.lock_open_outlined, color: kMagenta, size: 32),
            ),
            const SizedBox(height: 32),
            Text(
              'GENERATING\nCRYPTOGRAPHIC\nKEYS',
              textAlign: TextAlign.center,
              style: GoogleFonts.orbitron(
                fontSize: 22,
                fontWeight: FontWeight.w900,
                color: kWhite,
                letterSpacing: 3,
                height: 1.3,
                shadows: [Shadow(color: kMagenta.withOpacity(0.6), blurRadius: 20)],
              ),
            ),
            const SizedBox(height: 24),
            const _ScrambleBar(),
          ],
        ),
      ),
    );
  }

  // ── STEP 3: Done ─────────────────────────────────────────────────────────────
  Widget _buildDone() {
    final identity = _identity!;
    final shortId = identity.id.substring(0, 16);
    final formatted = '${shortId.substring(0,4)} ${shortId.substring(4,8)} ${shortId.substring(8,12)} ${shortId.substring(12,16)}';

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 28),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 48),

          // Status badge
          Row(
            children: [
              Container(
                width: 8,
                height: 8,
                decoration: const BoxDecoration(
                  color: kGreen,
                  shape: BoxShape.circle,
                  boxShadow: [BoxShadow(color: kGreen, blurRadius: 8)],
                ),
              ),
              const SizedBox(width: 10),
              Text(
                'IDENTITY CREATED',
                style: GoogleFonts.spaceMono(
                  fontSize: 11,
                  color: kGreen,
                  letterSpacing: 2,
                ),
              ),
            ],
          ),
          const SizedBox(height: 20),

          GlitchText(
            text: 'WELCOME,\n${identity.nickname.toUpperCase()}.',
            interval: const Duration(milliseconds: 120),
            style: GoogleFonts.orbitron(
              fontSize: 34,
              fontWeight: FontWeight.w900,
              color: kWhite,
              height: 1.1,
              letterSpacing: 2,
              shadows: [Shadow(color: kCyan.withOpacity(0.4), blurRadius: 12)],
            ),
          ),

          const SizedBox(height: 32),

          CyberCard(
            borderColor: kCyan,
            glow: true,
            padding: const EdgeInsets.all(20),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    const Icon(Icons.fingerprint, color: kCyan, size: 14),
                    const SizedBox(width: 8),
                    Text(
                      'PHANTOM_ID //',
                      style: GoogleFonts.spaceMono(
                        fontSize: 10,
                        color: kCyan,
                        letterSpacing: 2,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 14),
                Text(
                  formatted,
                  style: GoogleFonts.orbitron(
                    fontSize: 24,
                    fontWeight: FontWeight.w900,
                    color: kWhite,
                    letterSpacing: 3,
                    shadows: [Shadow(color: kCyan.withOpacity(0.6), blurRadius: 12)],
                  ),
                ),
                const SizedBox(height: 14),
                Row(
                  children: [
                    Expanded(
                      child: Text(
                        'Share your Phantom ID to receive encrypted messages. Only contacts with your ID can reach you.',
                        style: GoogleFonts.spaceGrotesk(fontSize: 12, color: kGrayText, height: 1.5),
                      ),
                    ),
                    const SizedBox(width: 12),
                    GestureDetector(
                      onTap: () {
                        Clipboard.setData(ClipboardData(text: identity.phantomId));
                        ScaffoldMessenger.of(context).showSnackBar(
                          const SnackBar(content: Text('> PHANTOM ID COPIED')),
                        );
                      },
                      child: CyberCard(
                        borderColor: kCyan,
                        bgColor: kCyanDim,
                        padding: const EdgeInsets.all(10),
                        cut: 6,
                        child: const Icon(Icons.copy_all_outlined, color: kCyan, size: 18),
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),

          const SizedBox(height: 16),

          // Warning
          CyberCard(
            borderColor: kMagenta,
            padding: const EdgeInsets.all(14),
            cut: 8,
            child: Row(
              children: [
                const Icon(Icons.warning_amber_rounded, color: kMagenta, size: 16),
                const SizedBox(width: 12),
                Expanded(
                  child: Text(
                    '! KEYS EXIST ONLY ON THIS DEVICE. NO CLOUD BACKUP.',
                    style: GoogleFonts.spaceMono(
                      fontSize: 10,
                      color: kMagenta,
                      letterSpacing: 0.5,
                      height: 1.4,
                    ),
                  ),
                ),
              ],
            ),
          ),

          const Spacer(),

          _CyberButton(
            label: '[ ENTER PHANTOM ]',
            onTap: _finish,
            width: double.infinity,
            color: kCyan,
            glow: true,
          ),
          const SizedBox(height: 24),
        ],
      ),
    );
  }
}

// ── Shared Widgets ────────────────────────────────────────────────────────────

class _Pill extends StatelessWidget {
  final String label;
  final Color color;
  const _Pill({required this.label, required this.color});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
      decoration: BoxDecoration(
        border: Border.all(color: color.withOpacity(0.4)),
        color: color.withOpacity(0.06),
      ),
      child: Text(
        label,
        style: GoogleFonts.spaceMono(
          fontSize: 9,
          color: color,
          letterSpacing: 1,
        ),
      ),
    );
  }
}

class _CyberButton extends StatelessWidget {
  final String label;
  final VoidCallback? onTap;
  final double? width;
  final Color color;
  final bool glow;

  const _CyberButton({
    required this.label,
    required this.onTap,
    this.width,
    this.color = kCyan,
    this.glow = false,
  });

  @override
  Widget build(BuildContext context) {
    final active = onTap != null;
    return GestureDetector(
      onTap: onTap,
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 200),
        width: width,
        padding: const EdgeInsets.symmetric(vertical: 16, horizontal: 20),
        decoration: BoxDecoration(
          border: Border.all(color: active ? color : kGray, width: 1.5),
          color: active ? color.withOpacity(0.07) : Colors.transparent,
          boxShadow: (active && glow)
              ? [
                  BoxShadow(color: color.withOpacity(0.3), blurRadius: 16, spreadRadius: 0),
                  BoxShadow(color: color.withOpacity(0.1), blurRadius: 32, spreadRadius: 4),
                ]
              : null,
        ),
        child: Center(
          child: Text(
            label,
            style: GoogleFonts.orbitron(
              fontSize: 13,
              fontWeight: FontWeight.w700,
              color: active ? color : kGray,
              letterSpacing: 2,
            ),
          ),
        ),
      ),
    );
  }
}

class _ScrambleBar extends StatefulWidget {
  const _ScrambleBar();

  @override
  State<_ScrambleBar> createState() => _ScrambleBarState();
}

class _ScrambleBarState extends State<_ScrambleBar> {
  final _rng = Random();
  String _hex = '0' * 32;
  Timer? _timer;
  double _progress = 0;

  @override
  void initState() {
    super.initState();
    _timer = Timer.periodic(const Duration(milliseconds: 60), (_) {
      if (!mounted) return;
      setState(() {
        _progress = (_progress + 0.04).clamp(0, 1);
        _hex = List.generate(
          32,
          (_) => '0123456789ABCDEF'[_rng.nextInt(16)],
        ).join();
      });
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        Text(
          _hex,
          style: GoogleFonts.spaceMono(fontSize: 12, color: kCyan.withOpacity(0.6), letterSpacing: 2),
        ),
        const SizedBox(height: 16),
        Container(
          width: 200,
          height: 2,
          color: kGray,
          child: Align(
            alignment: Alignment.centerLeft,
            child: AnimatedContainer(
              duration: const Duration(milliseconds: 60),
              width: 200 * _progress,
              height: 2,
              color: kCyan,
            ),
          ),
        ),
      ],
    );
  }
}
