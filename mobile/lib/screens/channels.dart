// MLS Channels (group chat) screen.
//
// Mirrors the Tauri Desktop's ChannelsPane — adapted to Flutter widgets
// with the same cyberpunk theme the rest of the mobile app uses. MVP
// scope: text-only group messages, single group, manual key-package
// pasting. File-transfer / multi-group / member-removal land in
// wave 7B-followup.
//
// Wiring: the screen subscribes to `RelayService.instance.events`
// `mls_message` / `mls_joined` / `mls_epoch` / `error` channels. Outbound
// MLS app messages flow through `mlsEncrypt` (Rust) → `MLS-APP1`-prefixed
// payload → `sendSealedV3` (Rust) per-member, mirroring Desktop's
// `mls_send` Tauri command.

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';

import '../services/relay_service.dart';
import '../src/rust/api.dart' as rust;
import '../theme.dart';
import '../widgets/cyber_card.dart';

class ChannelsScreen extends StatefulWidget {
  /// Storage directory for the MLS provider (`mls_state.bin` /
  /// `mls_meta.json`). On Android this is typically the app's internal
  /// data dir; resolved by the caller via path_provider.
  final String storageDir;

  /// Self-label baked into the local MLS member's BasicCredential.
  /// Forwarded to `mlsInit` on the first launch.
  final String selfLabel;

  const ChannelsScreen({
    super.key,
    required this.storageDir,
    required this.selfLabel,
  });

  @override
  State<ChannelsScreen> createState() => _ChannelsScreenState();
}

class _ChannelsScreenState extends State<ChannelsScreen> {
  final TextEditingController _msgCtrl = TextEditingController();
  final ScrollController _scrollCtrl = ScrollController();
  StreamSubscription<RelayEvent>? _sub;

  bool _initialised = false;
  bool _initialising = false;
  String? _initError;

  bool _inGroup = false;
  int _memberCount = 0;
  List<rust.MlsMemberInfoV3> _members = const [];
  String? _myKpBase64;

  // Local message log (MVP — not persisted across cold starts).
  final List<_GroupRow> _rows = [];

  @override
  void initState() {
    super.initState();
    _initBundle();
    _sub = RelayService.instance.events.listen(_handleRelay);
  }

  @override
  void dispose() {
    _sub?.cancel();
    _msgCtrl.dispose();
    _scrollCtrl.dispose();
    super.dispose();
  }

  Future<void> _initBundle() async {
    setState(() {
      _initialising = true;
      _initError = null;
    });
    try {
      await rust.mlsInit(
        identityLabel: widget.selfLabel,
        storageDir: widget.storageDir,
      );
      _refreshState();
      setState(() {
        _initialised = true;
        _initialising = false;
      });
    } catch (e) {
      setState(() {
        _initError = e.toString();
        _initialising = false;
      });
    }
  }

  Future<void> _refreshState() async {
    if (!mounted) return;
    final inGroup = rust.mlsInGroup();
    int count = 0;
    List<rust.MlsMemberInfoV3> rows = const [];
    if (inGroup) {
      count = rust.mlsMemberCount();
      try {
        rows = await rust.mlsListMembers();
      } catch (_) {}
    }
    if (!mounted) return;
    setState(() {
      _inGroup = inGroup;
      _memberCount = count;
      _members = rows;
    });
  }

  void _handleRelay(RelayEvent ev) {
    switch (ev.kind) {
      case 'mls_joined':
        _appendSystem(
          'Joined group as ${widget.selfLabel}, '
          '${ev.payload['memberCount']} members. Inviter: ${ev.payload['fromLabel']}.',
        );
        _refreshState();
        break;
      case 'mls_message':
        _appendIncoming(
          ev.payload['fromLabel'] as String,
          ev.payload['plaintext'] as String,
          ev.payload['ts'] as String,
        );
        _refreshState();
        break;
      case 'mls_epoch':
        _appendSystem(
          'Group epoch advanced — ${ev.payload['memberCount']} members.',
        );
        _refreshState();
        break;
      case 'error':
        _appendSystem('! ${ev.payload['detail']}');
        break;
    }
  }

  // ── UI actions ─────────────────────────────────────────────────────────────

  Future<void> _publishMyKp() async {
    try {
      final kp = await rust.mlsPublishKeyPackage();
      final b64 = base64Encode(kp);
      setState(() {
        _myKpBase64 = b64;
      });
      await Clipboard.setData(ClipboardData(text: b64));
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('> KEY PACKAGE COPIED')),
      );
    } catch (e) {
      _showErr('publish_key_package: $e');
    }
  }

  Future<void> _createGroup() async {
    try {
      final count = await rust.mlsCreateGroup();
      _appendSystem('Group created — $count member.');
      _refreshState();
    } catch (e) {
      _showErr('create_group: $e');
    }
  }

  Future<void> _showAddMember() async {
    final kpCtrl = TextEditingController();
    final labelCtrl = TextEditingController();
    final addrCtrl = TextEditingController();
    final pubCtrl = TextEditingController();
    await showModalBottomSheet(
      context: context,
      backgroundColor: kBgCard,
      isScrollControlled: true,
      shape: const RoundedRectangleBorder(borderRadius: BorderRadius.zero),
      builder: (ctx) => Padding(
        padding: EdgeInsets.only(
          left: 20,
          right: 20,
          top: 20,
          bottom: MediaQuery.of(ctx).viewInsets.bottom + 24,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(height: 1, color: kCyan.withOpacity(0.4)),
            const SizedBox(height: 16),
            Text(
              'ADD_MEMBER //',
              style: GoogleFonts.orbitron(
                  fontSize: 14, color: kCyan, letterSpacing: 2),
            ),
            const SizedBox(height: 12),
            _labeledField('LABEL:', labelCtrl, hint: 'alice'),
            _labeledField('ADDRESS:', addrCtrl, hint: 'phantom:...'),
            _labeledField('SIGNING PUB (HEX):', pubCtrl, hint: 'a1b2...'),
            _labeledField('KEY PACKAGE (BASE64):', kpCtrl,
                hint: 'paste their KP', maxLines: 4),
            const SizedBox(height: 12),
            GestureDetector(
              onTap: () async {
                Navigator.pop(ctx);
                await _addMember(
                  labelCtrl.text.trim(),
                  addrCtrl.text.trim(),
                  pubCtrl.text.trim(),
                  kpCtrl.text.trim(),
                );
              },
              child: Container(
                width: double.infinity,
                padding: const EdgeInsets.symmetric(vertical: 14),
                decoration: BoxDecoration(
                  border: Border.all(color: kCyan, width: 1.5),
                  color: kCyanDim,
                ),
                child: Center(
                  child: Text(
                    'COMMIT ADD',
                    style: GoogleFonts.orbitron(
                        fontSize: 12, color: kCyan, letterSpacing: 2),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Future<void> _addMember(
      String label, String address, String pubHex, String kpB64) async {
    if (label.isEmpty || address.isEmpty || pubHex.isEmpty || kpB64.isEmpty) {
      _showErr('all four fields required');
      return;
    }
    Uint8List kpBytes;
    try {
      kpBytes = base64Decode(kpB64);
    } catch (e) {
      _showErr('invalid base64: $e');
      return;
    }
    try {
      final pair = await rust.mlsAddMember(
        keyPackageBytes: kpBytes,
        newMemberLabel: label,
        newMemberAddress: address,
        newMemberSigningPubHex: pubHex,
      );
      final commitBytes = pair.$1;
      final welcomeBytes = pair.$2;
      _appendSystem(
        '$label added — commit ${commitBytes.length} B, welcome ${welcomeBytes.length} B. '
        'Use copy-buttons below to ship them via your normal sealed-sender pipe.',
      );
      // Stash the wire bytes on the row so the user can copy them out.
      _rows.add(_GroupRow.commitWelcome(commitBytes, welcomeBytes));
      _refreshState();
    } catch (e) {
      _showErr('add_member: $e');
    }
  }

  Future<void> _send() async {
    final text = _msgCtrl.text.trim();
    if (text.isEmpty || !_inGroup) return;
    _msgCtrl.clear();
    try {
      final wire = await rust.mlsEncrypt(plaintext: utf8.encode(text));
      _appendOutgoing(text);
      // The MLS-APP1 wire bytes need to go to every other group member via
      // sealed-sender 1:1. The Rust side gives us the ciphertext, so we
      // wrap with the prefix here. Actually shipping it is the relay
      // layer's job (Wave 7B-followup hooks the websocket transport in).
      final payload = BytesBuilder(copy: false)
        ..add(kMlsAppPrefix)
        ..add(wire);
      // For now we just stash a hint so the user knows the encryption
      // happened correctly even if no transport is wired yet.
      final size = payload.length;
      _appendSystem(
        'sent $size B MLS-APP1 ciphertext (transport hookup pending — wave 7B-followup).',
      );
    } catch (e) {
      _showErr('encrypt: $e');
    }
  }

  // ── Render ────────────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      body: GridBackground(
        child: SafeArea(
          child: Column(
            children: [
              _buildAppBar(),
              Container(height: 1, color: kCyan.withOpacity(0.12)),
              if (_initialising)
                const Padding(
                  padding: EdgeInsets.all(24),
                  child: Center(
                      child: CircularProgressIndicator(
                          color: kCyan, strokeWidth: 1.5)),
                ),
              if (_initError != null)
                Padding(
                  padding: const EdgeInsets.all(12),
                  child: Text(
                    '! MLS init failed: $_initError',
                    style: GoogleFonts.spaceMono(
                        color: kMagenta, fontSize: 11),
                  ),
                ),
              if (_initialised) _buildControls(),
              const Divider(color: kGray, height: 1),
              Expanded(child: _buildLog()),
              if (_inGroup) _buildInput(),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildAppBar() {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      child: Row(
        children: [
          GestureDetector(
            onTap: () => Navigator.pop(context),
            child: Container(
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                border: Border.all(color: kGray.withOpacity(0.5)),
                color: kBgCard,
              ),
              child: const Icon(Icons.arrow_back_ios_new,
                  color: kWhite, size: 14),
            ),
          ),
          const SizedBox(width: 14),
          Text('CHANNELS //',
              style: GoogleFonts.orbitron(
                  fontSize: 14,
                  color: kCyan,
                  letterSpacing: 2,
                  fontWeight: FontWeight.w700)),
          const Spacer(),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
            decoration: BoxDecoration(
              border: Border.all(color: kGrayText.withOpacity(0.3)),
              color: kBgCard,
            ),
            child: Text(
              _inGroup ? '$_memberCount MEMBERS' : 'NO GROUP',
              style: GoogleFonts.spaceMono(
                  fontSize: 9, color: kGrayText, letterSpacing: 1),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildControls() {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      child: Wrap(
        spacing: 8,
        runSpacing: 8,
        children: [
          _ctrlBtn('PUBLISH KP', _publishMyKp),
          if (!_inGroup) _ctrlBtn('CREATE GROUP', _createGroup),
          if (_inGroup) _ctrlBtn('+ MEMBER', _showAddMember),
          if (_inGroup)
            _ctrlBtn('LIST MEMBERS', () async {
              await _refreshState();
              if (!mounted) return;
              final names = _members
                  .map((m) =>
                      '${m.credentialLabel}${m.isSelf ? " (you)" : ""}${m.mappedContactLabel != null ? " → ${m.mappedContactLabel}" : ""}')
                  .join(', ');
              _appendSystem('members: $names');
            }),
          if (_myKpBase64 != null)
            Container(
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                  border: Border.all(color: kCyan.withOpacity(0.4))),
              child: Text(
                'KP: ${_myKpBase64!.substring(0, 24.clamp(0, _myKpBase64!.length))}…',
                style: GoogleFonts.spaceMono(fontSize: 10, color: kCyan),
              ),
            ),
        ],
      ),
    );
  }

  Widget _ctrlBtn(String label, VoidCallback onTap) {
    return GestureDetector(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        decoration: BoxDecoration(
          border: Border.all(color: kCyan, width: 1),
          color: kCyanDim,
          boxShadow: neonGlow(kCyan, radius: 6),
        ),
        child: Text(label,
            style: GoogleFonts.orbitron(
                fontSize: 10, color: kCyan, letterSpacing: 1.5)),
      ),
    );
  }

  Widget _buildLog() {
    return ListView.builder(
      controller: _scrollCtrl,
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      itemCount: _rows.length,
      itemBuilder: (ctx, i) => _rows[i].build(context),
    );
  }

  Widget _buildInput() {
    return Container(
      padding: EdgeInsets.only(
        left: 12,
        right: 12,
        top: 8,
        bottom: MediaQuery.of(context).viewInsets.bottom + 12,
      ),
      decoration: BoxDecoration(
        color: kBgCard,
        border: Border(top: BorderSide(color: kCyan.withOpacity(0.12))),
      ),
      child: Row(
        children: [
          Expanded(
            child: TextField(
              controller: _msgCtrl,
              style: GoogleFonts.spaceGrotesk(color: kWhite, fontSize: 14),
              decoration: InputDecoration(
                hintText: '> BROADCAST TO GROUP...',
                hintStyle:
                    GoogleFonts.spaceMono(color: kGrayText, fontSize: 11),
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(4),
                  borderSide: BorderSide(color: kGray.withOpacity(0.5)),
                ),
                contentPadding: const EdgeInsets.symmetric(
                    horizontal: 12, vertical: 10),
                filled: true,
                fillColor: kBgInput,
              ),
              onSubmitted: (_) => _send(),
              textInputAction: TextInputAction.send,
            ),
          ),
          const SizedBox(width: 10),
          GestureDetector(
            onTap: _send,
            child: Container(
              width: 40,
              height: 40,
              decoration: BoxDecoration(
                border: Border.all(color: kCyan, width: 1.5),
                color: kCyanDim,
                boxShadow: neonGlow(kCyan, radius: 6),
              ),
              child: const Icon(Icons.send_rounded, color: kCyan, size: 16),
            ),
          ),
        ],
      ),
    );
  }

  Widget _labeledField(String label, TextEditingController ctrl,
      {String? hint, int maxLines = 1}) {
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(label,
              style: GoogleFonts.spaceMono(
                  fontSize: 9, color: kCyan, letterSpacing: 1)),
          const SizedBox(height: 4),
          TextField(
            controller: ctrl,
            maxLines: maxLines,
            style: GoogleFonts.spaceMono(color: kWhite, fontSize: 11),
            decoration: InputDecoration(
              hintText: hint,
              hintStyle:
                  GoogleFonts.spaceMono(color: kGrayText, fontSize: 11),
              border: OutlineInputBorder(
                borderRadius: BorderRadius.circular(2),
                borderSide: BorderSide(color: kGray.withOpacity(0.4)),
              ),
              contentPadding:
                  const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
            ),
          ),
        ],
      ),
    );
  }

  // ── Log helpers ────────────────────────────────────────────────────────────

  void _appendIncoming(String fromLabel, String text, String ts) {
    setState(() {
      _rows.add(_GroupRow.incoming(fromLabel, text, ts));
    });
    _scrollAfter();
  }

  void _appendOutgoing(String text) {
    setState(() {
      _rows.add(_GroupRow.outgoing(text, _now()));
    });
    _scrollAfter();
  }

  void _appendSystem(String text) {
    setState(() {
      _rows.add(_GroupRow.system(text, _now()));
    });
    _scrollAfter();
  }

  void _showErr(String msg) {
    if (!mounted) return;
    ScaffoldMessenger.of(context)
        .showSnackBar(SnackBar(content: Text('! $msg')));
  }

  void _scrollAfter() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scrollCtrl.hasClients) {
        _scrollCtrl.animateTo(
          _scrollCtrl.position.maxScrollExtent,
          duration: const Duration(milliseconds: 200),
          curve: Curves.easeOut,
        );
      }
    });
  }

  String _now() {
    final n = DateTime.now();
    String two(int v) => v.toString().padLeft(2, '0');
    return '${two(n.hour)}:${two(n.minute)}:${two(n.second)}';
  }
}

class _GroupRow {
  final String kind; // "incoming" | "outgoing" | "system" | "commitwelcome"
  final String? fromLabel;
  final String text;
  final String ts;
  final Uint8List? commitBytes;
  final Uint8List? welcomeBytes;

  _GroupRow._({
    required this.kind,
    this.fromLabel,
    required this.text,
    required this.ts,
    this.commitBytes,
    this.welcomeBytes,
  });

  factory _GroupRow.incoming(String fromLabel, String text, String ts) =>
      _GroupRow._(
          kind: 'incoming', fromLabel: fromLabel, text: text, ts: ts);

  factory _GroupRow.outgoing(String text, String ts) =>
      _GroupRow._(kind: 'outgoing', text: text, ts: ts);

  factory _GroupRow.system(String text, String ts) =>
      _GroupRow._(kind: 'system', text: text, ts: ts);

  factory _GroupRow.commitWelcome(Uint8List commit, Uint8List welcome) =>
      _GroupRow._(
        kind: 'commitwelcome',
        text: 'commit/welcome ready — copy out via the buttons.',
        ts: '',
        commitBytes: commit,
        welcomeBytes: welcome,
      );

  Widget build(BuildContext context) {
    switch (kind) {
      case 'system':
        return Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: Text(
            '// $text  $ts',
            style:
                GoogleFonts.spaceMono(fontSize: 10, color: kGrayText),
          ),
        );
      case 'outgoing':
        return _bubble(
          align: Alignment.centerRight,
          color: kCyanDim,
          border: kCyan,
          fg: kWhite,
          text: text,
          ts: ts,
        );
      case 'incoming':
        return _bubble(
          align: Alignment.centerLeft,
          color: const Color(0xFF0D1520),
          border: kGray.withOpacity(0.4),
          fg: kWhite,
          text: text,
          ts: ts,
          headline: fromLabel,
        );
      case 'commitwelcome':
        return Padding(
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('// $text',
                  style: GoogleFonts.spaceMono(
                      fontSize: 10, color: kGreen)),
              const SizedBox(height: 4),
              Wrap(spacing: 8, children: [
                _copyChip('COMMIT', commitBytes!),
                _copyChip('WELCOME', welcomeBytes!),
              ]),
            ],
          ),
        );
    }
    return const SizedBox.shrink();
  }

  Widget _bubble({
    required Alignment align,
    required Color color,
    required Color border,
    required Color fg,
    required String text,
    required String ts,
    String? headline,
  }) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Align(
        alignment: align,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 320),
          child: Container(
            padding:
                const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            decoration: BoxDecoration(
              color: color,
              border: Border.all(color: border, width: 1),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                if (headline != null)
                  Text(headline,
                      style: GoogleFonts.orbitron(
                          fontSize: 10,
                          color: kCyan,
                          letterSpacing: 1)),
                Text(text,
                    style: GoogleFonts.spaceGrotesk(
                        fontSize: 13, color: fg, height: 1.3)),
                Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: Text(ts,
                      style: GoogleFonts.spaceMono(
                          fontSize: 9, color: kGrayText)),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _copyChip(String label, Uint8List bytes) {
    return _ChipButton(
      label: label,
      onTap: () async {
        await Clipboard.setData(ClipboardData(text: base64Encode(bytes)));
      },
    );
  }
}

class _ChipButton extends StatelessWidget {
  final String label;
  final VoidCallback onTap;
  const _ChipButton({required this.label, required this.onTap});

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        decoration: BoxDecoration(
          border: Border.all(color: kGreen.withOpacity(0.6)),
          color: kGreen.withOpacity(0.06),
        ),
        child: Text(
          'COPY $label',
          style: GoogleFonts.orbitron(
              fontSize: 10, color: kGreen, letterSpacing: 1.5),
        ),
      ),
    );
  }
}
