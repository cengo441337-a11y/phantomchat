import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

import '../services/app_lock_service.dart';
import '../services/battery_opt_service.dart';
import '../theme.dart';
import '../widgets/cyber_card.dart';

/// PhantomChat settings panel — Wave 8a.
///
/// Two sections for now:
///
/// 1. **Sicherheit** — opt-in for the lightweight "biometric-on-launch"
///    quick-lock. Independent of the PIN flow; default OFF.
///
/// 2. **Hintergrund-Aktivität** — current battery-optimisation state plus a
///    button to launch the system "ignore battery optimisations" dialog.
///    Hidden entirely on iOS / desktop where the plugin no-ops.
///
/// Reads its own state on every `build` via [FutureBuilder] so toggles take
/// effect immediately without an explicit refresh hop.
class SettingsScreen extends StatefulWidget {
  const SettingsScreen({super.key});

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  bool _bioOnLaunchEnabled = false;
  bool _bioAvailable = false;
  bool _batteryOptDisabled = false;
  bool _batteryPlatformSupported = false;
  bool _loading = true;

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    final results = await Future.wait<bool>([
      AppLockService.bioOnLaunchEnabled(),
      AppLockService.biometricAvailable(),
      BatteryOptService.platformSupported(),
      BatteryOptService.isOptimizationDisabled(),
    ]);
    if (!mounted) return;
    setState(() {
      _bioOnLaunchEnabled       = results[0];
      _bioAvailable             = results[1];
      _batteryPlatformSupported = results[2];
      _batteryOptDisabled       = results[3];
      _loading = false;
    });
  }

  Future<void> _toggleBioOnLaunch(bool enable) async {
    if (enable && !_bioAvailable) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('! Biometrie auf diesem Gerät nicht verfügbar')),
      );
      return;
    }
    await AppLockService.setBioOnLaunchEnabled(enable);
    if (!mounted) return;
    setState(() => _bioOnLaunchEnabled = enable);
  }

  Future<void> _requestDisableBatteryOpt() async {
    await BatteryOptService.requestDisableOptimization();
    // The plugin returns synchronously after the user dismisses the system
    // dialog; re-query the actual state rather than trusting that return
    // value (some OEM ROMs lie about it).
    final now = await BatteryOptService.isOptimizationDisabled();
    if (!mounted) return;
    setState(() => _batteryOptDisabled = now);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      appBar: AppBar(
        backgroundColor: kBg,
        leading: IconButton(
          icon: const Icon(Icons.arrow_back, color: kCyan),
          onPressed: () => Navigator.pop(context),
        ),
        title: Text(
          'EINSTELLUNGEN',
          style: GoogleFonts.orbitron(
            fontSize: 14,
            fontWeight: FontWeight.w700,
            color: kWhite,
            letterSpacing: 3,
          ),
        ),
      ),
      body: GridBackground(
        child: _loading
            ? const Center(child: CircularProgressIndicator(color: kCyan, strokeWidth: 1.5))
            : ListView(
                padding: const EdgeInsets.all(20),
                children: [
                  _sectionHeader('SICHERHEIT'),
                  const SizedBox(height: 12),
                  _bioOnLaunchCard(),
                  const SizedBox(height: 32),
                  if (_batteryPlatformSupported) ...[
                    _sectionHeader('HINTERGRUND-AKTIVITÄT'),
                    const SizedBox(height: 12),
                    _batteryOptCard(),
                  ],
                ],
              ),
      ),
    );
  }

  Widget _sectionHeader(String label) {
    return Row(
      children: [
        Container(width: 3, height: 18, color: kCyan),
        const SizedBox(width: 10),
        Text(
          label,
          style: GoogleFonts.orbitron(
            fontSize: 12,
            fontWeight: FontWeight.w700,
            color: kWhite,
            letterSpacing: 2.5,
          ),
        ),
      ],
    );
  }

  Widget _bioOnLaunchCard() {
    return CyberCard(
      borderColor: kCyan,
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              const Icon(Icons.fingerprint, color: kCyan, size: 22),
              const SizedBox(width: 12),
              Expanded(
                child: Text(
                  'Biometrie bei Start anfordern',
                  style: GoogleFonts.spaceGrotesk(
                    fontSize: 14,
                    fontWeight: FontWeight.w600,
                    color: kWhite,
                  ),
                ),
              ),
              Switch(
                value: _bioOnLaunchEnabled,
                activeThumbColor: kCyan,
                inactiveTrackColor: kGray,
                onChanged: _toggleBioOnLaunch,
              ),
            ],
          ),
          const SizedBox(height: 10),
          Text(
            _bioAvailable
                ? 'Beim App-Start wird ein Fingerabdruck oder PIN '
                    'verlangt, bevor Inhalte sichtbar werden. Schützt '
                    'Chat-Vorschauen vor flüchtigen Blicken.'
                : '! Auf diesem Gerät ist keine Biometrie eingerichtet.',
            style: GoogleFonts.spaceMono(
              fontSize: 11,
              color: _bioAvailable ? kGrayText : kMagenta,
              height: 1.5,
            ),
          ),
        ],
      ),
    );
  }

  Widget _batteryOptCard() {
    final stateLabel = _batteryOptDisabled ? 'INAKTIV' : 'AKTIV';
    final stateColor = _batteryOptDisabled ? kGreen : kMagenta;
    return CyberCard(
      borderColor: kMagenta,
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              const Icon(Icons.battery_saver, color: kMagenta, size: 22),
              const SizedBox(width: 12),
              Expanded(
                child: Text(
                  'Akku-Optimierung',
                  style: GoogleFonts.spaceGrotesk(
                    fontSize: 14,
                    fontWeight: FontWeight.w600,
                    color: kWhite,
                  ),
                ),
              ),
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                decoration: BoxDecoration(
                  border: Border.all(color: stateColor.withOpacity(0.6)),
                  color: stateColor.withOpacity(0.08),
                ),
                child: Text(
                  stateLabel,
                  style: GoogleFonts.spaceMono(
                    fontSize: 10,
                    color: stateColor,
                    letterSpacing: 1.5,
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 10),
          Text(
            _batteryOptDisabled
                ? 'Hintergrund-Aktivität ist gesichert. Nachrichten '
                    'kommen auch dann an, wenn die App nicht im '
                    'Vordergrund ist.'
                : 'Das System darf PhantomChat im Hintergrund anhalten. '
                    'Auf manchen Geräten (Xiaomi, Huawei, OnePlus, Samsung) '
                    'gehen dadurch Nachrichten verloren.',
            style: GoogleFonts.spaceMono(
              fontSize: 11,
              color: kGrayText,
              height: 1.5,
            ),
          ),
          if (!_batteryOptDisabled) ...[
            const SizedBox(height: 14),
            GestureDetector(
              onTap: _requestDisableBatteryOpt,
              child: Container(
                width: double.infinity,
                padding: const EdgeInsets.symmetric(vertical: 12),
                decoration: BoxDecoration(
                  border: Border.all(color: kMagenta, width: 1.5),
                  color: kMagenta.withOpacity(0.08),
                ),
                child: Center(
                  child: Text(
                    'AKKU-OPTIMIERUNG DEAKTIVIEREN',
                    style: GoogleFonts.orbitron(
                      fontSize: 11,
                      color: kMagenta,
                      letterSpacing: 2,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ],
      ),
    );
  }
}
