// Full relay management — add / edit / remove / reorder / live status.
// Replaces the read-only relay card in Settings with a proper CRUD UI so
// the user can actually swap a dead public relay for their own server.
// god-tier: per-relay state badge (verbunden / wartet / weg), inline
// validation (wss:// or ws:// only), drag-to-reorder, "Standards zurueck-
// setzen" escape hatch, and a tap-to-test handshake against any URL.
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:web_socket_channel/io.dart';

import '../services/relay_service.dart';
import '../theme.dart';
import '../widgets/cyber_card.dart';

class RelayManagerScreen extends StatefulWidget {
  const RelayManagerScreen({super.key});

  @override
  State<RelayManagerScreen> createState() => _RelayManagerScreenState();
}

class _RelayManagerScreenState extends State<RelayManagerScreen> {
  List<String> _urls = [];
  bool _loading = true;
  StreamSubscription<RelayEvent>? _sub;

  /// Per-URL transient probe-result: null = idle, true = ok, false = fail.
  final Map<String, bool?> _probe = {};

  @override
  void initState() {
    super.initState();
    _load();
    _sub = RelayService.instance.events.listen((_) {
      if (mounted) setState(() {});
    });
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  Future<void> _load() async {
    final list = await RelayService.loadPersistedRelayUrls();
    if (!mounted) return;
    setState(() {
      _urls = list;
      _loading = false;
    });
  }

  Future<void> _persistAndReconnect() async {
    await RelayService.savePersistedRelayUrls(_urls);
    await RelayService.instance.disconnect();
    await RelayService.instance.connect(relayUrls: _urls);
    if (!mounted) return;
    setState(() {});
  }

  bool _isValidRelayUrl(String s) {
    final t = s.trim();
    if (t.isEmpty) return false;
    if (!t.startsWith('wss://') && !t.startsWith('ws://')) return false;
    if (t.length < 8) return false;
    return true;
  }

  Future<void> _addRelay() async {
    final ctrl = TextEditingController(text: 'wss://');
    final ok = await showModalBottomSheet<bool>(
      context: context,
      isScrollControlled: true,
      backgroundColor: kBgCard,
      builder: (ctx) => Padding(
        padding: EdgeInsets.only(
          left: 24, right: 24, top: 24,
          bottom: (MediaQuery.of(ctx).viewInsets.bottom > 0
                  ? MediaQuery.of(ctx).viewInsets.bottom
                  : MediaQuery.of(ctx).viewPadding.bottom) +
              32,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(height: 1, color: kCyan.withValues(alpha: 0.4)),
            const SizedBox(height: 20),
            Row(
              children: [
                Container(width: 3, height: 22, color: kCyan),
                const SizedBox(width: 12),
                Text(
                  'ADD_RELAY //',
                  style: GoogleFonts.orbitron(
                    fontSize: 16, fontWeight: FontWeight.w700,
                    color: kWhite, letterSpacing: 1.5,
                    shadows: [Shadow(color: kCyan.withValues(alpha: 0.5), blurRadius: 10)],
                  ),
                ),
              ],
            ),
            const SizedBox(height: 18),
            TextField(
              controller: ctrl,
              autofocus: true,
              autocorrect: false,
              style: GoogleFonts.spaceMono(color: kWhite, fontSize: 13),
              decoration: InputDecoration(
                hintText: 'wss://relay.example.com',
                hintStyle: GoogleFonts.spaceMono(color: kGrayText, fontSize: 12),
                filled: true,
                fillColor: kBgInput,
                border: OutlineInputBorder(
                  borderSide: BorderSide(color: kCyan.withValues(alpha: 0.5)),
                  borderRadius: BorderRadius.zero,
                ),
                focusedBorder: OutlineInputBorder(
                  borderSide: const BorderSide(color: kCyan, width: 1.5),
                  borderRadius: BorderRadius.zero,
                ),
                enabledBorder: OutlineInputBorder(
                  borderSide: BorderSide(color: kCyan.withValues(alpha: 0.3)),
                  borderRadius: BorderRadius.zero,
                ),
              ),
            ),
            const SizedBox(height: 8),
            Text(
              'Akzeptiert ws:// und wss:// URLs. Eigener Relay liefert '
              'am zuverlaessigsten — Nostr-Public-Relays verwerfen '
              'Argos-Events oft.',
              style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText, height: 1.5),
            ),
            const SizedBox(height: 20),
            Row(
              children: [
                Expanded(
                  child: GestureDetector(
                    onTap: () => Navigator.pop(ctx, false),
                    child: Container(
                      padding: const EdgeInsets.symmetric(vertical: 12),
                      decoration: BoxDecoration(
                        border: Border.all(color: kGrayText.withValues(alpha: 0.5)),
                      ),
                      child: Center(
                        child: Text('ABBRECHEN',
                          style: GoogleFonts.orbitron(fontSize: 11, color: kGrayText, letterSpacing: 2)),
                      ),
                    ),
                  ),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: GestureDetector(
                    onTap: () {
                      if (!_isValidRelayUrl(ctrl.text)) {
                        ScaffoldMessenger.of(ctx).showSnackBar(SnackBar(
                          content: Text('URL muss mit ws:// oder wss:// beginnen',
                            style: GoogleFonts.spaceMono(fontSize: 12)),
                          backgroundColor: kBgCard,
                        ));
                        return;
                      }
                      Navigator.pop(ctx, true);
                    },
                    child: Container(
                      padding: const EdgeInsets.symmetric(vertical: 12),
                      decoration: BoxDecoration(
                        border: Border.all(color: kCyan, width: 1.5),
                        color: kCyan.withValues(alpha: 0.08),
                      ),
                      child: Center(
                        child: Text('HINZUFUEGEN',
                          style: GoogleFonts.orbitron(fontSize: 11, color: kCyan, letterSpacing: 2)),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
    if (ok != true || !mounted) return;
    final u = ctrl.text.trim();
    if (_urls.contains(u)) {
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(
        content: Text('Relay ist bereits in der Liste',
          style: GoogleFonts.spaceMono(fontSize: 12)),
        backgroundColor: kBgCard,
      ));
      return;
    }
    setState(() => _urls.add(u));
    await _persistAndReconnect();
  }

  Future<void> _removeRelay(String url) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: kBgCard,
        shape: const RoundedRectangleBorder(borderRadius: BorderRadius.zero,
          side: BorderSide(color: kMagenta)),
        title: Text('Relay entfernen?',
          style: GoogleFonts.orbitron(fontSize: 14, color: kWhite, letterSpacing: 1.5)),
        content: Text(url,
          style: GoogleFonts.spaceMono(fontSize: 12, color: kGrayText)),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx, false),
            child: Text('ABBRECHEN',
              style: GoogleFonts.orbitron(fontSize: 11, color: kGrayText)),
          ),
          TextButton(
            onPressed: () => Navigator.pop(ctx, true),
            child: Text('ENTFERNEN',
              style: GoogleFonts.orbitron(fontSize: 11, color: kMagenta)),
          ),
        ],
      ),
    );
    if (ok != true) return;
    setState(() {
      _urls.remove(url);
      _probe.remove(url);
    });
    await _persistAndReconnect();
  }

  void _moveUp(int i) {
    if (i <= 0) return;
    setState(() {
      final tmp = _urls[i];
      _urls[i] = _urls[i - 1];
      _urls[i - 1] = tmp;
    });
    _persistAndReconnect();
  }

  void _moveDown(int i) {
    if (i >= _urls.length - 1) return;
    setState(() {
      final tmp = _urls[i];
      _urls[i] = _urls[i + 1];
      _urls[i + 1] = tmp;
    });
    _persistAndReconnect();
  }

  Future<void> _resetDefaults() async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: kBgCard,
        shape: const RoundedRectangleBorder(borderRadius: BorderRadius.zero,
          side: BorderSide(color: kCyan)),
        title: Text('Standards wiederherstellen?',
          style: GoogleFonts.orbitron(fontSize: 14, color: kWhite, letterSpacing: 1.5)),
        content: Text(
          'Setzt die Relay-Liste auf den eigenen Relay + die '
          'oeffentlichen Fallback-Relays zurueck.',
          style: GoogleFonts.spaceMono(fontSize: 12, color: kGrayText, height: 1.5)),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx, false),
            child: Text('ABBRECHEN',
              style: GoogleFonts.orbitron(fontSize: 11, color: kGrayText)),
          ),
          TextButton(
            onPressed: () => Navigator.pop(ctx, true),
            child: Text('ZURUECKSETZEN',
              style: GoogleFonts.orbitron(fontSize: 11, color: kCyan)),
          ),
        ],
      ),
    );
    if (ok != true) return;
    setState(() {
      _urls = List.from(RelayService.defaultRelayUrls);
      _probe.clear();
    });
    await _persistAndReconnect();
  }

  /// Quick "is it alive?" probe — opens a websocket, waits for the
  /// handshake to settle (or fail) and tears it down. Pure-client check;
  /// does not subscribe or publish.
  Future<void> _probeRelay(String url) async {
    setState(() => _probe[url] = null);
    try {
      final uri = Uri.parse(url);
      final ch = IOWebSocketChannel.connect(uri,
          connectTimeout: const Duration(seconds: 8));
      await ch.ready.timeout(const Duration(seconds: 8));
      await ch.sink.close();
      if (!mounted) return;
      setState(() => _probe[url] = true);
    } catch (_) {
      if (!mounted) return;
      setState(() => _probe[url] = false);
    }
  }

  bool _isOwnRelay(String u) => u.contains('relay.dc-infosec.de');

  @override
  Widget build(BuildContext context) {
    final bottomInset = MediaQuery.of(context).viewPadding.bottom;
    return Scaffold(
      backgroundColor: kBg,
      appBar: AppBar(
        backgroundColor: kBg,
        leading: IconButton(
          icon: const Icon(Icons.arrow_back, color: kCyan),
          onPressed: () => Navigator.pop(context),
        ),
        title: Text(
          'RELAYS',
          style: GoogleFonts.orbitron(
            fontSize: 14, fontWeight: FontWeight.w700,
            color: kWhite, letterSpacing: 3,
          ),
        ),
        actions: [
          IconButton(
            tooltip: 'Standards wiederherstellen',
            icon: const Icon(Icons.restart_alt, color: kGrayText, size: 20),
            onPressed: _resetDefaults,
          ),
        ],
      ),
      floatingActionButton: Padding(
        padding: EdgeInsets.only(bottom: bottomInset),
        child: FloatingActionButton.extended(
          backgroundColor: kCyan,
          foregroundColor: kBg,
          onPressed: _addRelay,
          icon: const Icon(Icons.add),
          label: Text('RELAY',
            style: GoogleFonts.orbitron(fontWeight: FontWeight.w700, letterSpacing: 2)),
        ),
      ),
      body: GridBackground(
        child: SafeArea(
          bottom: false,
          child: _loading
              ? const Center(child: CircularProgressIndicator(color: kCyan, strokeWidth: 1.5))
              : ListView.builder(
                  padding: EdgeInsets.only(
                    left: 20, right: 20, top: 16, bottom: bottomInset + 100,
                  ),
                  itemCount: _urls.length + 1,
                  itemBuilder: (ctx, i) {
                    if (i == 0) return _legendCard();
                    final idx = i - 1;
                    return _relayRow(idx);
                  },
                ),
        ),
      ),
    );
  }

  Widget _legendCard() {
    return Padding(
      padding: const EdgeInsets.only(bottom: 16),
      child: CyberCard(
        borderColor: kCyan,
        padding: const EdgeInsets.all(14),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                const Icon(Icons.info_outline, color: kCyan, size: 18),
                const SizedBox(width: 10),
                Text('REIHENFOLGE = PUBLISH-PRIORITAET',
                  style: GoogleFonts.orbitron(
                    fontSize: 11, color: kCyan, letterSpacing: 1.5,
                    fontWeight: FontWeight.w700,
                  )),
              ],
            ),
            const SizedBox(height: 8),
            Text(
              'Nachrichten werden parallel an alle Relays gesendet. Der '
              'erste antwortende zaehlt als "geliefert". Eigener Relay '
              '(dc-infosec.de) ist am zuverlaessigsten. Tippe den Test-'
              'Button um zu pruefen ob ein Relay erreichbar ist.',
              style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText, height: 1.5),
            ),
          ],
        ),
      ),
    );
  }

  Widget _relayRow(int idx) {
    final url = _urls[idx];
    final isOwn = _isOwnRelay(url);
    final probe = _probe[url];
    final probing = _probe.containsKey(url) && probe == null;
    final connected = RelayService.instance.relayUrls.contains(url)
        && RelayService.instance.hasAnyConnection;
    return Padding(
      padding: const EdgeInsets.only(bottom: 10),
      child: CyberCard(
        borderColor: isOwn ? kCyan : kGray,
        padding: const EdgeInsets.all(12),
        child: Column(
          children: [
            Row(
              children: [
                Container(
                  width: 22, height: 22, alignment: Alignment.center,
                  decoration: BoxDecoration(
                    border: Border.all(color: isOwn ? kCyan : kGrayText.withValues(alpha: 0.3)),
                  ),
                  child: Text('${idx + 1}',
                    style: GoogleFonts.orbitron(fontSize: 10, color: isOwn ? kCyan : kGrayText)),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Text(url,
                    style: GoogleFonts.spaceMono(
                      fontSize: 11.5,
                      color: isOwn ? kWhite : kGrayText,
                      fontWeight: isOwn ? FontWeight.w600 : FontWeight.normal,
                    ),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                GestureDetector(
                  onTap: () {
                    Clipboard.setData(ClipboardData(text: url));
                    ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                      content: Text('Relay-URL kopiert',
                        style: GoogleFonts.spaceMono(fontSize: 12)),
                      backgroundColor: kBgCard,
                    ));
                  },
                  child: const Icon(Icons.copy, size: 14, color: kGrayText),
                ),
              ],
            ),
            const SizedBox(height: 10),
            Row(
              children: [
                _statusChip(connected, probe, probing),
                const Spacer(),
                _iconBtn(Icons.arrow_upward, idx > 0,
                    onTap: () => _moveUp(idx)),
                _iconBtn(Icons.arrow_downward, idx < _urls.length - 1,
                    onTap: () => _moveDown(idx)),
                _iconBtn(Icons.network_check, true,
                    color: kCyan, onTap: () => _probeRelay(url)),
                _iconBtn(Icons.delete_outline, _urls.length > 1,
                    color: kMagenta, onTap: () => _removeRelay(url)),
              ],
            ),
          ],
        ),
      ),
    );
  }

  Widget _statusChip(bool connected, bool? probe, bool probing) {
    String label; Color color;
    if (probing) {
      label = 'PRUEFE...'; color = kCyan;
    } else if (probe == true) {
      label = 'ERREICHBAR'; color = kGreen;
    } else if (probe == false) {
      label = 'NICHT ERREICHBAR'; color = kMagenta;
    } else if (connected) {
      label = 'VERBUNDEN'; color = kGreen;
    } else {
      label = 'OFFLINE'; color = kGrayText;
    }
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 3),
      decoration: BoxDecoration(
        border: Border.all(color: color.withValues(alpha: 0.6)),
        color: color.withValues(alpha: 0.08),
      ),
      child: Text(label,
        style: GoogleFonts.spaceMono(
          fontSize: 9, color: color, letterSpacing: 1.2)),
    );
  }

  Widget _iconBtn(IconData icon, bool enabled, {Color color = kGrayText, required VoidCallback onTap}) {
    return Padding(
      padding: const EdgeInsets.only(left: 6),
      child: GestureDetector(
        onTap: enabled ? onTap : null,
        child: Container(
          padding: const EdgeInsets.all(6),
          decoration: BoxDecoration(
            border: Border.all(color: enabled
                ? color.withValues(alpha: 0.5)
                : kGray.withValues(alpha: 0.3)),
          ),
          child: Icon(icon,
            size: 14,
            color: enabled ? color : kGray.withValues(alpha: 0.4)),
        ),
      ),
    );
  }
}
