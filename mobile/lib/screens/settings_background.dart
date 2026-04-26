// PhantomChat — Wave 8B
//
// Settings screen for the background relay-listener service.
// Lets the user opt into:
//   - Persistent foreground service (default OFF)
//   - Auto-start on device boot (default OFF)
// And shows live status: uptime, relay count, messages received.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../services/background_service_config.dart';
import '../theme.dart';
import '../widgets/cyber_card.dart';

class BackgroundSettingsScreen extends StatefulWidget {
  const BackgroundSettingsScreen({super.key});

  @override
  State<BackgroundSettingsScreen> createState() =>
      _BackgroundSettingsScreenState();
}

class _BackgroundSettingsScreenState extends State<BackgroundSettingsScreen> {
  bool _serviceEnabled = false;
  bool _autostart = false;
  bool _running = false;
  DateTime? _startedAt;
  int _relayCount = 0;
  int _messages = 0;
  Timer? _refresh;

  @override
  void initState() {
    super.initState();
    _load();
    _refresh = Timer.periodic(const Duration(seconds: 1), (_) => _refreshStatus());
  }

  @override
  void dispose() {
    _refresh?.cancel();
    super.dispose();
  }

  Future<void> _load() async {
    final prefs = await SharedPreferences.getInstance();
    final running = await PhantomBackgroundService.isRunning();
    setState(() {
      _serviceEnabled = prefs.getBool(prefsBgServiceEnabled) ?? false;
      _autostart = prefs.getBool(prefsAutostartOnBoot) ?? false;
      _running = running;
      final startedMs = prefs.getInt(prefsBgStartedAtMs);
      _startedAt = startedMs != null
          ? DateTime.fromMillisecondsSinceEpoch(startedMs)
          : null;
      _relayCount = prefs.getInt(prefsBgRelayCount) ?? 0;
      _messages = prefs.getInt(prefsBgMessagesReceived) ?? 0;
    });
  }

  Future<void> _refreshStatus() async {
    if (!mounted) return;
    final prefs = await SharedPreferences.getInstance();
    final running = await PhantomBackgroundService.isRunning();
    if (!mounted) return;
    setState(() {
      _running = running;
      final startedMs = prefs.getInt(prefsBgStartedAtMs);
      _startedAt = startedMs != null
          ? DateTime.fromMillisecondsSinceEpoch(startedMs)
          : null;
      _relayCount = prefs.getInt(prefsBgRelayCount) ?? 0;
      _messages = prefs.getInt(prefsBgMessagesReceived) ?? 0;
    });
  }

  Future<void> _toggleService(bool enable) async {
    if (enable) {
      await PhantomBackgroundService.startService();
    } else {
      await PhantomBackgroundService.stopService();
    }
    setState(() => _serviceEnabled = enable);
    await _refreshStatus();
  }

  Future<void> _toggleAutostart(bool enable) async {
    await PhantomBackgroundService.setAutostartOnBoot(enable);
    setState(() => _autostart = enable);
  }

  String _formatUptime() {
    if (_startedAt == null || !_running) return '--:--:--';
    final d = DateTime.now().difference(_startedAt!);
    String two(int n) => n.toString().padLeft(2, '0');
    return '${two(d.inHours)}:${two(d.inMinutes.remainder(60))}:${two(d.inSeconds.remainder(60))}';
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      appBar: AppBar(
        backgroundColor: kBg,
        elevation: 0,
        title: Text(
          'HINTERGRUND // BACKGROUND',
          style: GoogleFonts.orbitron(
            color: kCyan,
            fontSize: 14,
            letterSpacing: 2,
            fontWeight: FontWeight.w700,
          ),
        ),
      ),
      body: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            CyberCard(
              borderColor: _running ? kGreen : kGray,
              padding: const EdgeInsets.all(16),
              cut: 12,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    _running ? 'STATUS // ACTIVE' : 'STATUS // INACTIVE',
                    style: GoogleFonts.orbitron(
                      color: _running ? kGreen : kGrayText,
                      fontSize: 12,
                      letterSpacing: 2,
                    ),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'Aktiv seit ${_formatUptime()} · '
                    '$_relayCount Relays verbunden · '
                    '$_messages Nachrichten empfangen',
                    style: GoogleFonts.spaceMono(
                      fontSize: 11,
                      color: kWhite,
                      height: 1.4,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(height: 12),
            SwitchListTile(
              tileColor: kBgCard,
              title: Text(
                'Hintergrund-Empfang aktivieren',
                style: GoogleFonts.spaceGrotesk(
                  color: kWhite,
                  fontSize: 14,
                ),
              ),
              subtitle: Text(
                'Persistente Benachrichtigung; Nachrichten kommen auch bei geschlossener App an.',
                style: GoogleFonts.spaceMono(
                  color: kGrayText,
                  fontSize: 10,
                ),
              ),
              value: _serviceEnabled,
              activeThumbColor: kCyan,
              onChanged: _toggleService,
            ),
            const SizedBox(height: 8),
            SwitchListTile(
              tileColor: kBgCard,
              title: Text(
                'Bei Geräte-Start automatisch starten',
                style: GoogleFonts.spaceGrotesk(
                  color: kWhite,
                  fontSize: 14,
                ),
              ),
              subtitle: Text(
                'Standard: AUS — Privacy by default.',
                style: GoogleFonts.spaceMono(
                  color: kGrayText,
                  fontSize: 10,
                ),
              ),
              value: _autostart,
              activeThumbColor: kCyan,
              onChanged: _toggleAutostart,
            ),
            const SizedBox(height: 16),
            Text(
              'Hinweis: Einige Hersteller (Xiaomi, Huawei, OnePlus) beenden Hintergrund-Dienste aggressiv. Siehe README → "Background-Empfang auf Android".',
              style: GoogleFonts.spaceMono(
                fontSize: 10,
                color: kGrayText,
                height: 1.5,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
