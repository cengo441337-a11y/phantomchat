import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import '../models/identity.dart';
import '../services/crypto_service.dart';
import '../services/storage_service.dart';
import '../theme.dart';
import 'home.dart';

class OnboardingScreen extends StatefulWidget {
  const OnboardingScreen({super.key});

  @override
  State<OnboardingScreen> createState() => _OnboardingScreenState();
}

class _OnboardingScreenState extends State<OnboardingScreen>
    with TickerProviderStateMixin {
  int _step = 0; // 0=welcome, 1=name, 2=generating, 3=done
  final _nameCtrl = TextEditingController();
  PhantomIdentity? _identity;
  bool _generating = false;
  late AnimationController _glitchCtrl;
  late AnimationController _fadeCtrl;
  late Animation<double> _fadeAnim;
  int _glitchFrame = 0;
  Timer? _glitchTimer;

  @override
  void initState() {
    super.initState();
    _glitchCtrl = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 100),
    );
    _fadeCtrl = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 600),
    );
    _fadeAnim = CurvedAnimation(parent: _fadeCtrl, curve: Curves.easeOut);
    _fadeCtrl.forward();
    _startGlitch();
  }

  void _startGlitch() {
    _glitchTimer = Timer.periodic(const Duration(milliseconds: 80), (_) {
      if (mounted) setState(() => _glitchFrame = (_glitchFrame + 1) % 8);
    });
  }

  @override
  void dispose() {
    _glitchCtrl.dispose();
    _fadeCtrl.dispose();
    _glitchTimer?.cancel();
    _nameCtrl.dispose();
    super.dispose();
  }

  Future<void> _generateIdentity() async {
    if (_nameCtrl.text.trim().isEmpty) return;
    setState(() {
      _step = 2;
      _generating = true;
    });

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

    setState(() {
      _identity = identity;
      _generating = false;
      _step = 3;
    });
  }

  void _finish() {
    Navigator.of(context).pushReplacement(
      MaterialPageRoute(builder: (_) => const HomeScreen()),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      body: SafeArea(
        child: FadeTransition(
          opacity: _fadeAnim,
          child: _buildStep(),
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

  Widget _buildWelcome() {
    return Padding(
      padding: const EdgeInsets.all(32),
      child: Column(
        children: [
          const Spacer(),
          _PhantomLogo(glitchFrame: _glitchFrame),
          const SizedBox(height: 40),
          Text(
            'PHANTOM',
            style: GoogleFonts.spaceGrotesk(
              fontSize: 48,
              fontWeight: FontWeight.w800,
              color: kWhite,
              letterSpacing: -2,
            ),
          ),
          const SizedBox(height: 8),
          Text(
            'End-to-end verschlüsselt.\nKein Server. Keine Spuren.',
            textAlign: TextAlign.center,
            style: GoogleFonts.spaceGrotesk(
              fontSize: 16,
              color: kWhiteDim,
              height: 1.5,
            ),
          ),
          const Spacer(),
          _NeonDivider(),
          const SizedBox(height: 32),
          Row(
            children: [
              _FeaturePill(icon: Icons.lock_outline, label: 'X25519'),
              const SizedBox(width: 8),
              _FeaturePill(icon: Icons.shuffle, label: 'ChaCha20'),
              const SizedBox(width: 8),
              _FeaturePill(icon: Icons.visibility_off, label: 'Zero-Meta'),
            ],
          ),
          const SizedBox(height: 32),
          SizedBox(
            width: double.infinity,
            child: ElevatedButton(
              onPressed: () {
                _fadeCtrl.reset();
                setState(() => _step = 1);
                _fadeCtrl.forward();
              },
              child: const Text('PHANTOM ERSTELLEN'),
            ),
          ),
          const SizedBox(height: 16),
          Text(
            'Kein Account. Keine E-Mail. Nur Kryptografie.',
            style: GoogleFonts.spaceGrotesk(fontSize: 12, color: kGray),
          ),
          const SizedBox(height: 16),
        ],
      ),
    );
  }

  Widget _buildNameInput() {
    return Padding(
      padding: const EdgeInsets.all(32),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 40),
          Text(
            'Dein\nCodename.',
            style: GoogleFonts.spaceGrotesk(
              fontSize: 42,
              fontWeight: FontWeight.w800,
              color: kWhite,
              letterSpacing: -1.5,
              height: 1.1,
            ),
          ),
          const SizedBox(height: 12),
          Text(
            'Wird lokal gespeichert — niemals übertragen.',
            style: GoogleFonts.spaceGrotesk(fontSize: 14, color: kGray),
          ),
          const SizedBox(height: 40),
          TextField(
            controller: _nameCtrl,
            autofocus: true,
            style: GoogleFonts.spaceGrotesk(color: kWhite, fontSize: 16),
            decoration: const InputDecoration(
              hintText: 'z.B. Ghost, Cipher, Nexus...',
              prefixIcon: Icon(Icons.person_outline, color: kNeon, size: 20),
            ),
            onSubmitted: (_) => _generateIdentity(),
          ),
          const Spacer(),
          SizedBox(
            width: double.infinity,
            child: ElevatedButton(
              onPressed: _generateIdentity,
              child: const Text('SCHLÜSSEL GENERIEREN'),
            ),
          ),
          const SizedBox(height: 12),
          Center(
            child: TextButton(
              onPressed: () {
                _fadeCtrl.reset();
                setState(() => _step = 0);
                _fadeCtrl.forward();
              },
              child: Text(
                'Zurück',
                style: GoogleFonts.spaceGrotesk(color: kGray),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildGenerating() {
    return const Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          _GeneratingIndicator(),
          SizedBox(height: 24),
          Text(
            'Generiere kryptografische\nSchlüssel...',
            textAlign: TextAlign.center,
            style: TextStyle(color: kWhiteDim, fontSize: 16, height: 1.5),
          ),
        ],
      ),
    );
  }

  Widget _buildDone() {
    final identity = _identity!;
    final shortId = identity.id.substring(0, 16);

    return Padding(
      padding: const EdgeInsets.all(32),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 40),
          Row(
            children: [
              Container(
                padding: const EdgeInsets.all(8),
                decoration: BoxDecoration(
                  color: kNeonDim,
                  borderRadius: BorderRadius.circular(8),
                ),
                child: const Icon(Icons.check, color: kNeon, size: 20),
              ),
              const SizedBox(width: 12),
              Text(
                'Schlüssel generiert',
                style: GoogleFonts.spaceGrotesk(
                  color: kNeon,
                  fontWeight: FontWeight.w700,
                  fontSize: 16,
                ),
              ),
            ],
          ),
          const SizedBox(height: 24),
          Text(
            'Willkommen,\n${identity.nickname}.',
            style: GoogleFonts.spaceGrotesk(
              fontSize: 38,
              fontWeight: FontWeight.w800,
              color: kWhite,
              height: 1.1,
              letterSpacing: -1,
            ),
          ),
          const SizedBox(height: 32),
          Container(
            padding: const EdgeInsets.all(20),
            decoration: BoxDecoration(
              color: kBgCard,
              borderRadius: BorderRadius.circular(16),
              border: Border.all(color: kNeonDim),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'DEINE PHANTOM ID',
                  style: GoogleFonts.spaceGrotesk(
                    fontSize: 10,
                    fontWeight: FontWeight.w700,
                    color: kNeon,
                    letterSpacing: 1.5,
                  ),
                ),
                const SizedBox(height: 12),
                Text(
                  '${shortId.substring(0, 4)} ${shortId.substring(4, 8)} ${shortId.substring(8, 12)} ${shortId.substring(12, 16)}',
                  style: GoogleFonts.spaceMono(
                    fontSize: 22,
                    fontWeight: FontWeight.w700,
                    color: kWhite,
                    letterSpacing: 2,
                  ),
                ),
                const SizedBox(height: 16),
                Row(
                  children: [
                    Expanded(
                      child: Text(
                        'Teile deine Phantom ID mit Kontakten, um verschlüsselt zu kommunizieren.',
                        style: GoogleFonts.spaceGrotesk(
                          fontSize: 12,
                          color: kGray,
                          height: 1.5,
                        ),
                      ),
                    ),
                    IconButton(
                      onPressed: () {
                        Clipboard.setData(
                          ClipboardData(text: identity.phantomId),
                        );
                        ScaffoldMessenger.of(context).showSnackBar(
                          const SnackBar(
                            content: Text('Phantom ID kopiert'),
                            backgroundColor: kBgCard,
                          ),
                        );
                      },
                      icon: const Icon(Icons.copy_outlined, color: kNeon, size: 18),
                    ),
                  ],
                ),
              ],
            ),
          ),
          const Spacer(),
          Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              color: const Color(0xFF1A0D00),
              borderRadius: BorderRadius.circular(12),
              border: Border.all(color: const Color(0xFF3D2000)),
            ),
            child: Row(
              children: [
                const Icon(Icons.warning_amber_outlined, color: Color(0xFFFFAA00), size: 18),
                const SizedBox(width: 12),
                Expanded(
                  child: Text(
                    'Schlüssel sind nur auf diesem Gerät gespeichert. Backup-Funktion folgt in v2.',
                    style: GoogleFonts.spaceGrotesk(
                      fontSize: 12,
                      color: const Color(0xFFFFAA00),
                      height: 1.4,
                    ),
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 20),
          SizedBox(
            width: double.infinity,
            child: ElevatedButton(
              onPressed: _finish,
              child: const Text('LOS GEHT\'S'),
            ),
          ),
        ],
      ),
    );
  }
}

class _PhantomLogo extends StatelessWidget {
  final int glitchFrame;
  const _PhantomLogo({required this.glitchFrame});

  @override
  Widget build(BuildContext context) {
    return Stack(
      alignment: Alignment.center,
      children: [
        Container(
          width: 100,
          height: 100,
          decoration: BoxDecoration(
            shape: BoxShape.circle,
            color: kNeonDim,
            border: Border.all(color: kNeon, width: 1.5),
            boxShadow: [
              BoxShadow(
                color: kNeon.withOpacity(0.3),
                blurRadius: 30,
                spreadRadius: 5,
              ),
            ],
          ),
        ),
        Transform.translate(
          offset: Offset(
            (glitchFrame % 3 == 0) ? 2.0 : 0,
            0,
          ),
          child: Icon(
            Icons.shield_outlined,
            size: 44,
            color: kNeon.withOpacity(glitchFrame % 5 == 0 ? 0.5 : 1.0),
          ),
        ),
      ],
    );
  }
}

class _NeonDivider extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Expanded(
          child: Container(
            height: 1,
            decoration: const BoxDecoration(
              gradient: LinearGradient(
                colors: [Colors.transparent, kNeonDim, Colors.transparent],
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class _FeaturePill extends StatelessWidget {
  final IconData icon;
  final String label;
  const _FeaturePill({required this.icon, required this.label});

  @override
  Widget build(BuildContext context) {
    return Expanded(
      child: Container(
        padding: const EdgeInsets.symmetric(vertical: 8, horizontal: 4),
        decoration: BoxDecoration(
          color: kBgCard,
          borderRadius: BorderRadius.circular(8),
          border: Border.all(color: const Color(0xFF1E2733)),
        ),
        child: Column(
          children: [
            Icon(icon, size: 16, color: kNeon),
            const SizedBox(height: 4),
            Text(
              label,
              style: GoogleFonts.spaceMono(fontSize: 9, color: kGray),
            ),
          ],
        ),
      ),
    );
  }
}

class _GeneratingIndicator extends StatefulWidget {
  const _GeneratingIndicator();

  @override
  State<_GeneratingIndicator> createState() => _GeneratingIndicatorState();
}

class _GeneratingIndicatorState extends State<_GeneratingIndicator>
    with SingleTickerProviderStateMixin {
  late AnimationController _ctrl;

  @override
  void initState() {
    super.initState();
    _ctrl = AnimationController(vsync: this, duration: const Duration(seconds: 1))
      ..repeat();
  }

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 60,
      height: 60,
      child: CircularProgressIndicator(
        strokeWidth: 2,
        color: kNeon,
        backgroundColor: kNeonDim,
      ),
    );
  }
}
