// Wave 11G+ — In-app diagnostics screen. Surfaces the LogService ring
// buffer + a state snapshot (version, identity-loaded, relay
// connectivity) so a user reporting "PIN-confirm hangs" / "messages
// don't arrive" / etc. can copy a meaningful dump without needing
// `adb logcat` over USB.
//
// Privacy: the dump can include phantom-IDs of contacts the user has
// chatted with (sender pubkeys land in `[relay]` log lines via
// `_resolveSenderLabel`). The "Kopieren" button warns about this; the
// dialog text spells out that the user should share via an encrypted
// channel (the very thing this app is for) and never paste into a
// public bug tracker.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:package_info_plus/package_info_plus.dart';

import '../services/log_service.dart';
import '../services/relay_service.dart';
import '../theme.dart';
import '../widgets/cyber_card.dart';

class DiagnosticsScreen extends StatefulWidget {
  const DiagnosticsScreen({super.key});

  @override
  State<DiagnosticsScreen> createState() => _DiagnosticsScreenState();
}

class _DiagnosticsScreenState extends State<DiagnosticsScreen> {
  String _version = '?';
  String _buildNumber = '?';
  int _relayCount = 0;
  bool _anyRelayConnected = false;
  String _logDump = '';

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    final pkg = await PackageInfo.fromPlatform();
    if (!mounted) return;
    final relay = RelayService.instance;
    setState(() {
      _version = pkg.version;
      _buildNumber = pkg.buildNumber;
      _relayCount = relay.relayUrls.length;
      _anyRelayConnected = relay.hasAnyConnection;
      _logDump = LogService().dump();
    });
  }

  /// Compose the full report — header lines (app version, device
  /// state, relay status) followed by the captured log buffer. Same
  /// shape used by both Copy + Share.
  String _composeReport() {
    final buf = StringBuffer();
    buf.writeln('=== PhantomChat Diagnostics ===');
    buf.writeln('app version : $_version+$_buildNumber');
    buf.writeln('relay count : $_relayCount');
    buf.writeln('relay open  : $_anyRelayConnected');
    buf.writeln('captured    : ${LogService().length} lines');
    buf.writeln('exported at : ${DateTime.now().toIso8601String()}');
    buf.writeln('');
    buf.writeln('=== Log (oldest → newest) ===');
    buf.write(_logDump);
    return buf.toString();
  }

  Future<void> _copyToClipboard() async {
    await Clipboard.setData(ClipboardData(text: _composeReport()));
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        backgroundColor: kBgCard,
        content: Text(
          'In Zwischenablage kopiert (${LogService().length} Zeilen). '
          'Über verschlüsselten Kanal teilen — kann Phantom-IDs enthalten.',
          style: GoogleFonts.spaceMono(fontSize: 12, color: kCyan),
        ),
        duration: const Duration(seconds: 4),
      ),
    );
  }

  void _clearBuffer() {
    LogService().clear();
    _refresh();
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
          'DIAGNOSTIK',
          style: GoogleFonts.orbitron(
            fontSize: 14,
            fontWeight: FontWeight.w700,
            color: kWhite,
            letterSpacing: 3,
          ),
        ),
        actions: [
          IconButton(
            icon: const Icon(Icons.refresh, color: kCyan),
            onPressed: _refresh,
            tooltip: 'aktualisieren',
          ),
        ],
      ),
      body: GridBackground(
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              CyberCard(
                borderColor: kCyan,
                padding: const EdgeInsets.all(14),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    _kvLine('Version', '$_version+$_buildNumber'),
                    _kvLine('Relays gesamt', '$_relayCount'),
                    _kvLine(
                      'Listener',
                      _anyRelayConnected ? 'CONNECTED' : 'NICHT VERBUNDEN',
                      valueColor: _anyRelayConnected ? kGreen : kMagenta,
                    ),
                    _kvLine('Log-Zeilen erfasst', '${LogService().length}'),
                  ],
                ),
              ),
              const SizedBox(height: 12),
              Row(
                children: [
                  Expanded(
                    child: _flatButton(
                      'KOPIEREN',
                      Icons.copy,
                      kCyan,
                      _copyToClipboard,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: _flatButton(
                      'BUFFER LEEREN',
                      Icons.delete_sweep,
                      kMagenta,
                      _clearBuffer,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 12),
              Expanded(
                child: CyberCard(
                  borderColor: kGray,
                  padding: const EdgeInsets.all(8),
                  child: SingleChildScrollView(
                    reverse: true,
                    child: SelectableText(
                      _logDump.isEmpty ? '(Buffer leer)' : _logDump,
                      style: GoogleFonts.spaceMono(
                        fontSize: 10,
                        color: kWhite,
                        height: 1.4,
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _kvLine(String key, String value, {Color valueColor = kWhite}) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 2),
      child: Row(
        children: [
          SizedBox(
            width: 140,
            child: Text(
              key,
              style: GoogleFonts.spaceMono(
                fontSize: 11,
                color: kGrayText,
                letterSpacing: 0.5,
              ),
            ),
          ),
          Expanded(
            child: Text(
              value,
              style: GoogleFonts.spaceMono(
                fontSize: 11,
                color: valueColor,
                letterSpacing: 0.5,
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
  }

  Widget _flatButton(String label, IconData icon, Color color, VoidCallback onTap) {
    return GestureDetector(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(vertical: 12),
        decoration: BoxDecoration(
          border: Border.all(color: color.withValues(alpha: 0.6)),
          color: color.withValues(alpha: 0.06),
        ),
        child: Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Icon(icon, color: color, size: 16),
            const SizedBox(width: 8),
            Text(
              label,
              style: GoogleFonts.orbitron(
                fontSize: 11,
                color: color,
                letterSpacing: 1.5,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
