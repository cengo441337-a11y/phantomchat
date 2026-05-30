import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:package_info_plus/package_info_plus.dart';

import '../services/app_lock_service.dart';
import '../services/battery_opt_service.dart';
import '../services/relay_service.dart';
import '../services/update_service.dart';
import '../theme.dart';
import '../widgets/cyber_card.dart';
import '../widgets/update_dialog.dart';
import 'diagnostics.dart';
import 'relay_manager.dart';
import 'wallet_screen.dart';

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

  String _version = '…';
  String _buildNumber = '';
  bool _checkingUpdate = false;

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
    String version = '?';
    String build = '';
    try {
      final pkg = await PackageInfo.fromPlatform();
      version = pkg.version;
      build = pkg.buildNumber;
    } catch (_) {}
    if (!mounted) return;
    setState(() {
      _bioOnLaunchEnabled       = results[0];
      _bioAvailable             = results[1];
      _batteryPlatformSupported = results[2];
      _batteryOptDisabled       = results[3];
      _version                  = version;
      _buildNumber              = build;
      _loading = false;
    });
  }

  /// Manual "check for updates" — what the auto-banner does on home-mount,
  /// but on demand from Settings so the user can pull the latest signed APK
  /// from updates.dc-infosec.de any time, not only when a banner happens to
  /// appear. Up-to-date / offline → a clear SnackBar instead of silence.
  Future<void> _checkForUpdate() async {
    if (_checkingUpdate) return;
    setState(() => _checkingUpdate = true);
    UpdateInfo? info;
    try {
      info = await UpdateService.checkForUpdate();
    } catch (_) {
      info = null;
    }
    if (!mounted) return;
    setState(() => _checkingUpdate = false);
    if (info != null) {
      await showDialog<void>(
        context: context,
        barrierDismissible: false,
        builder: (_) => UpdateDialog(info: info!),
      );
    } else {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            'Du hast die neueste Version ($_version). '
            'Falls kein Internet besteht, später erneut versuchen.',
            style: GoogleFonts.spaceMono(fontSize: 12),
          ),
          backgroundColor: kBgCard,
        ),
      );
    }
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
                padding: EdgeInsets.only(
                  left: 20, right: 20, top: 20,
                  bottom: 20 + MediaQuery.of(context).viewPadding.bottom,
                ),
                children: [
                  // App-update first so "Nach Updates suchen" is the most
                  // prominent action in Settings — users reported they
                  // couldn't find it when it sat further down.
                  SizedBox(height: MediaQuery.of(context).viewPadding.top > 0 ? 0 : 8),
                  _sectionHeader('APP-UPDATE'),
                  const SizedBox(height: 12),
                  _updateCard(),
                  const SizedBox(height: 32),
                  _sectionHeader('WALLET · ARGOS'),
                  const SizedBox(height: 12),
                  _walletCard(),
                  const SizedBox(height: 32),
                  _sectionHeader('VERBINDUNG'),
                  const SizedBox(height: 12),
                  _relayCard(),
                  const SizedBox(height: 32),
                  _sectionHeader('AI-ASSISTENT'),
                  const SizedBox(height: 12),
                  _aiBridgeCard(),
                  const SizedBox(height: 32),
                  _sectionHeader('SICHERHEIT'),
                  const SizedBox(height: 12),
                  _bioOnLaunchCard(),
                  const SizedBox(height: 32),
                  if (_batteryPlatformSupported) ...[
                    _sectionHeader('HINTERGRUND-AKTIVITÄT'),
                    const SizedBox(height: 12),
                    _batteryOptCard(),
                    const SizedBox(height: 32),
                  ],
                  _sectionHeader('DIAGNOSE'),
                  const SizedBox(height: 12),
                  _diagnosticsCard(),
                  const SizedBox(height: 32),
                  _sectionHeader('ÜBER'),
                  const SizedBox(height: 12),
                  _aboutCard(),
                  const SizedBox(height: 24),
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
                  border: Border.all(color: stateColor.withValues(alpha: 0.6)),
                  color: stateColor.withValues(alpha: 0.08),
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
                  color: kMagenta.withValues(alpha: 0.08),
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

  /// Relay connection status + the configured relay list. Read-only for
  /// now — surfaces whether the app is reachable (the messaging transport)
  /// and which relays it talks to, so a user debugging "messages don't
  /// arrive" can see the connection state at a glance.
  /// Tappable relay card — opens the full RelayManager (add / remove /
  /// reorder / live test). Previous incarnation was a read-only list
  /// which is exactly what users complained about.
  Widget _relayCard() {
    final urls = RelayService.instance.relayUrls;
    final connected = RelayService.instance.hasAnyConnection;
    final stateColor = connected ? kGreen : kMagenta;
    return GestureDetector(
      onTap: () => Navigator.of(context).push(
        MaterialPageRoute(builder: (_) => const RelayManagerScreen()),
      ).then((_) => setState(() {})),
      child: CyberCard(
        borderColor: kCyan,
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(connected ? Icons.cloud_done_outlined : Icons.cloud_off_outlined,
                    color: stateColor, size: 22),
                const SizedBox(width: 12),
                Expanded(
                  child: Text('Relays verwalten',
                      style: GoogleFonts.spaceGrotesk(
                          fontSize: 14, fontWeight: FontWeight.w600, color: kWhite)),
                ),
                Container(
                  padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                  decoration: BoxDecoration(
                    border: Border.all(color: stateColor.withValues(alpha: 0.6)),
                    color: stateColor.withValues(alpha: 0.08),
                  ),
                  child: Text(connected ? 'VERBUNDEN' : 'GETRENNT',
                      style: GoogleFonts.spaceMono(
                          fontSize: 10, color: stateColor, letterSpacing: 1.5)),
                ),
                const SizedBox(width: 8),
                const Icon(Icons.chevron_right, color: kCyan, size: 20),
              ],
            ),
            const SizedBox(height: 12),
            ...urls.take(3).map((u) => Padding(
                  padding: const EdgeInsets.only(bottom: 6),
                  child: Row(
                    children: [
                      Icon(Icons.circle, size: 7,
                          color: u.contains('dc-infosec.de') ? kCyan : kGrayText),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(u,
                            style: GoogleFonts.spaceMono(
                                fontSize: 11,
                                color: u.contains('dc-infosec.de') ? kWhite : kGrayText)),
                      ),
                    ],
                  ),
                )),
            if (urls.length > 3)
              Padding(
                padding: const EdgeInsets.only(bottom: 6),
                child: Text('  + ${urls.length - 3} weitere',
                    style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText)),
              ),
            const SizedBox(height: 4),
            Text(
              'Tippen um Relays hinzuzufuegen, zu entfernen, Reihenfolge '
              'zu aendern oder die Erreichbarkeit zu testen.',
              style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText, height: 1.5),
            ),
          ],
        ),
      ),
    );
  }

  /// AI-Bridge info card. Mobile is the CLIENT of the bridge, not the host
  /// — the LLM runs on the user's desktop via `phantomchat ai-bridge`
  /// (ClaudeCli / Ollama / OpenAI / Anthropic API). The card explains the
  /// setup and links to the project docs so curious users don't get lost.
Widget _walletCard() {    return GestureDetector(      onTap: () => Navigator.of(context).push(        MaterialPageRoute(builder: (_) => const ArgosWalletScreen()),      ).then((_) => setState(() {})),      child: CyberCard(        borderColor: kCyan,        padding: const EdgeInsets.all(16),        child: Row(          children: [            const Icon(Icons.account_balance_wallet_outlined, color: kCyan, size: 22),            const SizedBox(width: 12),            Expanded(              child: Column(                crossAxisAlignment: CrossAxisAlignment.start,                children: [                  Text("Argos Wallet öffnen",                      style: GoogleFonts.spaceGrotesk(                          fontSize: 14, fontWeight: FontWeight.w600, color: kWhite)),                  const SizedBox(height: 4),                  Text("Non-custodial Solana · Send · Swap · Auto-Swap-on-Send",                      style: GoogleFonts.spaceMono(                          fontSize: 10, color: kWhiteDim)),                ],              ),            ),            const Icon(Icons.chevron_right, color: kCyan, size: 20),          ],        ),      ),    );  }
  Widget _aiBridgeCard() {
    return CyberCard(
      borderColor: kMagenta,
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              const Icon(Icons.smart_toy_outlined, color: kMagenta, size: 22),
              const SizedBox(width: 12),
              Expanded(
                child: Text('AI-Assistent (Home-LLM)',
                    style: GoogleFonts.spaceGrotesk(
                        fontSize: 14, fontWeight: FontWeight.w600, color: kWhite)),
              ),
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                decoration: BoxDecoration(
                  border: Border.all(color: kMagenta.withValues(alpha: 0.6)),
                  color: kMagenta.withValues(alpha: 0.08),
                ),
                child: Text('DESKTOP-SEITIG',
                    style: GoogleFonts.spaceMono(
                        fontSize: 9, color: kMagenta, letterSpacing: 1.5)),
              ),
            ],
          ),
          const SizedBox(height: 10),
          Text(
            'Die KI-Bridge laeuft auf deinem Home-PC (PhantomChat Desktop). '
            'Provider waehlbar dort: Claude CLI (Pro-Abo), Ollama (lokal), '
            'OpenAI-kompatibel oder Anthropic API. Auf dem Handy '
            'konfigurierst du nur, welcher Kontakt ueber die Bridge '
            'antworten soll — das passiert beim Hinzufuegen des '
            'Bridge-Kontakts (Erlaube als AI-Antwort).',
            style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText, height: 1.5),
          ),
          const SizedBox(height: 10),
          GestureDetector(
            onTap: () {
              Clipboard.setData(const ClipboardData(
                  text: 'https://github.com/cengo441337-a11y/phantomchat#wave-11--ai-bridge'));
              ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                content: Text('Setup-Doku-Link kopiert',
                  style: GoogleFonts.spaceMono(fontSize: 12)),
                backgroundColor: kBgCard,
              ));
            },
            child: Row(
              children: [
                const Icon(Icons.link, color: kMagenta, size: 16),
                const SizedBox(width: 8),
                Expanded(
                  child: Text('Setup-Anleitung (Desktop)',
                    style: GoogleFonts.spaceMono(fontSize: 11, color: kMagenta)),
                ),
                const Icon(Icons.copy, color: kGrayText, size: 14),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// App-update card: shows the installed version and a manual
  /// "Nach Updates suchen" button (the auto-banner runs on home-mount, but
  /// users asked for a deliberate check). Pulls the signed APK manifest
  /// from updates.dc-infosec.de via the existing [UpdateService].
  Widget _updateCard() {
    return CyberCard(
      borderColor: kGreen,
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              const Icon(Icons.system_update_alt, color: kGreen, size: 22),
              const SizedBox(width: 12),
              Expanded(
                child: Text('App-Version',
                    style: GoogleFonts.spaceGrotesk(
                        fontSize: 14, fontWeight: FontWeight.w600, color: kWhite)),
              ),
              Text('v$_version${_buildNumber.isNotEmpty ? '+$_buildNumber' : ''}',
                  style: GoogleFonts.spaceMono(fontSize: 12, color: kGreen)),
            ],
          ),
          const SizedBox(height: 10),
          Text(
            'Sideload-Installationen bekommen keine Play-Store-Updates. '
            'Hier prüfst du manuell auf eine neuere signierte Version.',
            style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText, height: 1.5),
          ),
          const SizedBox(height: 14),
          GestureDetector(
            onTap: _checkingUpdate ? null : _checkForUpdate,
            child: Container(
              width: double.infinity,
              padding: const EdgeInsets.symmetric(vertical: 12),
              decoration: BoxDecoration(
                border: Border.all(color: kGreen, width: 1.5),
                color: kGreen.withValues(alpha: 0.08),
              ),
              child: Center(
                child: _checkingUpdate
                    ? const SizedBox(
                        width: 16, height: 16,
                        child: CircularProgressIndicator(color: kGreen, strokeWidth: 1.5))
                    : Text('NACH UPDATES SUCHEN',
                        style: GoogleFonts.orbitron(
                            fontSize: 11, color: kGreen, letterSpacing: 2)),
              ),
            ),
          ),
        ],
      ),
    );
  }

  /// About card: version, project links and the honest "research project"
  /// framing. Tapping the download URL copies it to the clipboard.
  Widget _aboutCard() {
    return CyberCard(
      borderColor: kGray,
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              const Icon(Icons.shield_outlined, color: kCyan, size: 22),
              const SizedBox(width: 12),
              Text('PhantomChat',
                  style: GoogleFonts.orbitron(
                      fontSize: 14, fontWeight: FontWeight.w700, color: kWhite,
                      letterSpacing: 1.5)),
            ],
          ),
          const SizedBox(height: 12),
          _aboutRow('Version', 'v$_version${_buildNumber.isNotEmpty ? '+$_buildNumber' : ''}'),
          _aboutRow('Verschlüsselung', 'X25519 · ChaCha20-Poly1305 · MLS'),
          _aboutRow('Daten', 'Lokal · kein Konto · kein Cloud-Backup'),
          const SizedBox(height: 10),
          GestureDetector(
            onTap: () {
              Clipboard.setData(const ClipboardData(
                  text: 'https://updates.dc-infosec.de/download/'));
              ScaffoldMessenger.of(context).showSnackBar(
                SnackBar(
                  content: Text('Download-Link kopiert',
                      style: GoogleFonts.spaceMono(fontSize: 12)),
                  backgroundColor: kBgCard,
                ),
              );
            },
            child: Row(
              children: [
                const Icon(Icons.link, color: kCyan, size: 16),
                const SizedBox(width: 8),
                Expanded(
                  child: Text('updates.dc-infosec.de/download',
                      style: GoogleFonts.spaceMono(fontSize: 11, color: kCyan)),
                ),
                const Icon(Icons.copy, color: kGrayText, size: 14),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _aboutRow(String label, String value) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 110,
            child: Text(label,
                style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText)),
          ),
          Expanded(
            child: Text(value,
                style: GoogleFonts.spaceMono(fontSize: 11, color: kWhite)),
          ),
        ],
      ),
    );
  }

  /// Tappable card that opens the in-app diagnostics screen. The
  /// screen surfaces the LogService ring buffer + a state snapshot
  /// (version, listener status, contact count) and gives the user a
  /// "Kopieren" button to share the dump via an encrypted channel.
  ///
  /// Replaces the prior path of "user opens Linux box, runs adb
  /// logcat over USB, pipes to me" — which was a non-starter for any
  /// non-developer user reporting a real-device bug.
  Widget _diagnosticsCard() {
    return GestureDetector(
      onTap: () => Navigator.of(context).push(
        MaterialPageRoute(builder: (_) => const DiagnosticsScreen()),
      ),
      child: CyberCard(
        borderColor: kCyan,
        padding: const EdgeInsets.all(16),
        child: Row(
          children: [
            const Icon(Icons.bug_report_outlined, color: kCyan, size: 22),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'Diagnose & Logs',
                    style: GoogleFonts.spaceGrotesk(
                      fontSize: 14,
                      fontWeight: FontWeight.w600,
                      color: kWhite,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    'App-Status, Relay-Verbindung, in-app Log-Buffer · '
                    'kopierbar für Bug-Reports.',
                    style: GoogleFonts.spaceMono(
                      fontSize: 11,
                      color: kGrayText,
                    ),
                  ),
                ],
              ),
            ),
            const Icon(Icons.chevron_right, color: kCyan, size: 20),
          ],
        ),
      ),
    );
  }
}
