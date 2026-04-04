import 'package:flutter/material.dart';
import 'package:mobile/src/theme/cyberpunk_theme.dart';
import 'package:mobile/lib/services/privacy_service.dart';

/// Privacy settings screen — lets the user toggle between:
///
///   DailyUse        — libp2p + Dandelion++ + Nostr/TLS + light cover traffic
///   MaximumStealth  — relay-only, all traffic routed through Tor/Nym SOCKS5
///                     + aggressive cover traffic
class PrivacySettingsView extends StatefulWidget {
  const PrivacySettingsView({super.key});

  @override
  State<PrivacySettingsView> createState() => _PrivacySettingsViewState();
}

class _PrivacySettingsViewState extends State<PrivacySettingsView> {
  PrivacyMode _mode       = PrivacyMode.dailyUse;
  bool        _useNym     = false;
  bool        _loading    = true;
  bool        _applying   = false;
  String?     _errorMsg;

  late TextEditingController _proxyController;

  @override
  void initState() {
    super.initState();
    _proxyController = TextEditingController();
    _loadSettings();
  }

  @override
  void dispose() {
    _proxyController.dispose();
    super.dispose();
  }

  Future<void> _loadSettings() async {
    final mode  = await PrivacyService.loadMode();
    final proxy = await PrivacyService.loadProxyAddr();
    final nym   = await PrivacyService.loadUseNym();
    setState(() {
      _mode = mode;
      _proxyController.text = proxy;
      _useNym  = nym;
      _loading = false;
    });
  }

  Future<void> _applyMode(PrivacyMode newMode) async {
    setState(() { _applying = true; _errorMsg = null; });

    final err = await PrivacyService.setMode(
      mode:      newMode,
      proxyAddr: _proxyController.text.trim().isEmpty ? null : _proxyController.text.trim(),
      useNym:    _useNym,
    );

    setState(() {
      _applying = false;
      if (err == null) {
        _mode = newMode;
      } else {
        _errorMsg = err;
      }
    });

    if (err == null && mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          backgroundColor: CyberpunkTheme.neonGreen.withOpacity(0.15),
          content: Text(
            newMode == PrivacyMode.maximumStealth
                ? 'MAXIMUM STEALTH ACTIVATED'
                : 'DAILY USE MODE ACTIVE',
            style: const TextStyle(
              color: CyberpunkTheme.neonGreen,
              fontFamily: 'Courier',
              letterSpacing: 2,
            ),
          ),
          duration: const Duration(seconds: 2),
        ),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        backgroundColor: Colors.black,
        leading: IconButton(
          icon: const Icon(Icons.arrow_back, color: CyberpunkTheme.neonGreen),
          onPressed: () => Navigator.pop(context),
        ),
        title: const Text(
          'PRIVACY MODE',
          style: TextStyle(
            color: CyberpunkTheme.neonMagenta,
            fontSize: 16,
            letterSpacing: 3,
          ),
        ),
      ),
      body: _loading
          ? const Center(child: CircularProgressIndicator(color: CyberpunkTheme.neonGreen))
          : SingleChildScrollView(
              padding: const EdgeInsets.all(20),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  _buildModeCard(
                    mode:        PrivacyMode.dailyUse,
                    title:       'DAILY USE',
                    subtitle:    'libp2p + Dandelion++ + Nostr/TLS',
                    description: 'IP gegenüber direkten Peers verschleiert.\n'
                                 'Niedriger Akkuverbrauch, geringe Latenz.\n'
                                 'Leichter Cover Traffic (30–180 s).',
                    icon:        Icons.shield_outlined,
                    color:       CyberpunkTheme.neonGreen,
                  ),
                  const SizedBox(height: 16),
                  _buildModeCard(
                    mode:        PrivacyMode.maximumStealth,
                    title:       'MAXIMUM STEALTH',
                    subtitle:    'Relay-Only · Tor/Nym SOCKS5',
                    description: 'libp2p deaktiviert. Alle Verbindungen\n'
                                 'über anonymisierenden Proxy-Tunnel.\n'
                                 'Schutz gegen globale passive Angreifer.\n'
                                 'Aggressiver Cover Traffic (5–15 s).',
                    icon:        Icons.visibility_off_outlined,
                    color:       CyberpunkTheme.neonMagenta,
                  ),

                  // Proxy config — only relevant for MaximumStealth
                  AnimatedOpacity(
                    opacity: _mode == PrivacyMode.maximumStealth ? 1.0 : 0.35,
                    duration: const Duration(milliseconds: 300),
                    child: _buildProxyConfig(),
                  ),

                  if (_errorMsg != null) ...[
                    const SizedBox(height: 16),
                    Container(
                      padding: const EdgeInsets.all(12),
                      decoration: BoxDecoration(
                        border: Border.all(color: Colors.red),
                        color: Colors.red.withOpacity(0.07),
                      ),
                      child: Text(
                        _errorMsg!,
                        style: const TextStyle(color: Colors.red, fontSize: 12, fontFamily: 'Courier'),
                      ),
                    ),
                  ],

                  const SizedBox(height: 32),
                  _buildWarningBox(),
                ],
              ),
            ),
    );
  }

  Widget _buildModeCard({
    required PrivacyMode mode,
    required String title,
    required String subtitle,
    required String description,
    required IconData icon,
    required Color color,
  }) {
    final selected = _mode == mode;
    return GestureDetector(
      onTap: _applying ? null : () => _applyMode(mode),
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 200),
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(
          color: selected ? color.withOpacity(0.07) : Colors.transparent,
          border: Border.all(
            color: selected ? color : Colors.grey.withOpacity(0.4),
            width: selected ? 1.5 : 1.0,
          ),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Icon(icon, color: selected ? color : Colors.grey, size: 32),
            const SizedBox(width: 16),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Text(
                        title,
                        style: TextStyle(
                          color: selected ? color : Colors.grey,
                          fontSize: 14,
                          fontWeight: FontWeight.bold,
                          letterSpacing: 2,
                          fontFamily: 'Courier',
                        ),
                      ),
                      const Spacer(),
                      if (_applying && _mode != mode)
                        const SizedBox(
                          width: 14, height: 14,
                          child: CircularProgressIndicator(
                            strokeWidth: 1.5,
                            color: CyberpunkTheme.neonGreen,
                          ),
                        )
                      else if (selected)
                        Icon(Icons.check_circle, color: color, size: 18),
                    ],
                  ),
                  const SizedBox(height: 4),
                  Text(
                    subtitle,
                    style: TextStyle(
                      color: selected ? color.withOpacity(0.7) : Colors.grey.withOpacity(0.6),
                      fontSize: 10,
                      letterSpacing: 1,
                      fontFamily: 'Courier',
                    ),
                  ),
                  const SizedBox(height: 8),
                  Text(
                    description,
                    style: const TextStyle(
                      color: Colors.white54,
                      fontSize: 11,
                      fontFamily: 'Courier',
                      height: 1.5,
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildProxyConfig() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const SizedBox(height: 24),
        const Text(
          'PROXY CONFIGURATION',
          style: TextStyle(
            color: CyberpunkTheme.neonMagenta,
            fontSize: 10,
            letterSpacing: 3,
            fontFamily: 'Courier',
          ),
        ),
        const SizedBox(height: 12),
        // Tor / Nym toggle
        Row(
          children: [
            _proxyChip(label: 'TOR', selected: !_useNym, onTap: () {
              setState(() {
                _useNym = false;
                _proxyController.text = PrivacyService.defaultTorProxy;
              });
            }),
            const SizedBox(width: 8),
            _proxyChip(label: 'NYM', selected: _useNym, onTap: () {
              setState(() {
                _useNym = true;
                _proxyController.text = PrivacyService.defaultNymProxy;
              });
            }),
          ],
        ),
        const SizedBox(height: 12),
        // Manual proxy address input
        TextField(
          controller: _proxyController,
          style: const TextStyle(
            color: CyberpunkTheme.neonGreen,
            fontFamily: 'Courier',
            fontSize: 13,
          ),
          decoration: InputDecoration(
            labelText: 'SOCKS5 ADDRESS',
            labelStyle: const TextStyle(color: Colors.grey, fontSize: 10, letterSpacing: 2),
            hintText: '127.0.0.1:9050',
            hintStyle: TextStyle(color: Colors.grey.withOpacity(0.4), fontSize: 12),
            enabledBorder: const OutlineInputBorder(
              borderSide: BorderSide(color: Colors.grey),
            ),
            focusedBorder: const OutlineInputBorder(
              borderSide: BorderSide(color: CyberpunkTheme.neonMagenta),
            ),
          ),
        ),
      ],
    );
  }

  Widget _proxyChip({
    required String label,
    required bool selected,
    required VoidCallback onTap,
  }) {
    return GestureDetector(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
        decoration: BoxDecoration(
          color: selected ? CyberpunkTheme.neonMagenta.withOpacity(0.15) : Colors.transparent,
          border: Border.all(
            color: selected ? CyberpunkTheme.neonMagenta : Colors.grey.withOpacity(0.4),
          ),
        ),
        child: Text(
          label,
          style: TextStyle(
            color: selected ? CyberpunkTheme.neonMagenta : Colors.grey,
            fontSize: 11,
            letterSpacing: 2,
            fontFamily: 'Courier',
          ),
        ),
      ),
    );
  }

  Widget _buildWarningBox() {
    if (_mode != PrivacyMode.maximumStealth) return const SizedBox.shrink();
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        border: Border.all(color: CyberpunkTheme.neonMagenta.withOpacity(0.5)),
        color: CyberpunkTheme.neonMagenta.withOpacity(0.04),
      ),
      child: const Row(
        children: [
          Icon(Icons.warning_amber_outlined,
              color: CyberpunkTheme.neonMagenta, size: 18),
          SizedBox(width: 10),
          Expanded(
            child: Text(
              'Erhöhter Akkuverbrauch und Latenz.\n'
              'Tor/Nym muss lokal laufen.',
              style: TextStyle(
                color: CyberpunkTheme.neonMagenta,
                fontSize: 11,
                fontFamily: 'Courier',
                height: 1.5,
              ),
            ),
          ),
        ],
      ),
    );
  }
}
