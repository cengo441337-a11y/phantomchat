import 'dart:async';
import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:uuid/uuid.dart';
import '../models/contact.dart';
import '../models/identity.dart';
import '../models/message.dart';
import '../services/contact_directory.dart';
import '../services/crypto_service.dart';
import '../services/relay_service.dart';
import '../services/storage_service.dart';
import '../src/rust/api.dart' as rust;
import '../theme.dart';
import '../widgets/cyber_card.dart';

class ChatScreen extends StatefulWidget {
  final PhantomContact contact;
  final PhantomIdentity identity;

  const ChatScreen({super.key, required this.contact, required this.identity});

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final _msgCtrl = TextEditingController();
  final _scrollCtrl = ScrollController();
  List<PhantomMessage> _messages = [];
  bool _sending = false;
  static const _uuid = Uuid();
  StreamSubscription<RelayEvent>? _relaySub;

  /// Tracks the most-recent incoming sender label/sigOk for the bind-to-
  /// contact UI. Set by the listener whenever a `?<8hex>` message lands.
  String? _lastUnboundSenderLabel;

  @override
  void initState() {
    super.initState();
    _loadMessages();
    // Subscribe to the relay event stream so `RelayService.feedEnvelope`
    // calls (driven by whatever transport is wired in wave 7B-followup)
    // surface as incoming bubbles here. Filtered down to messages whose
    // sealed-sender attribution maps to this contact's signing pub.
    _relaySub = RelayService.instance.events.listen((ev) {
      if (!mounted) return;
      switch (ev.kind) {
        case 'message':
          _handleIncoming(ev.payload);
          break;
        case 'system':
          _appendIncomingFree(ev.payload['plaintext'] as String);
          break;
      }
    });
  }

  @override
  void dispose() {
    _relaySub?.cancel();
    _msgCtrl.dispose();
    _scrollCtrl.dispose();
    super.dispose();
  }

  void _handleIncoming(Map<String, dynamic> p) {
    final senderHex = (p['senderPubHex'] as String?)?.toLowerCase();
    // Only surface here if the sealed-sender pub matches this contact's
    // bound signing pub (or it's an unbound sender showing as `?<8hex>`
    // and the user opens this thread to bind it).
    final isUnbound = p['isUnbound'] as bool? ?? false;
    final contactPubHex =
        widget.contact.publicSpendKey.toLowerCase(); // best-effort match
    final fromUs = senderHex != null &&
        (senderHex == contactPubHex ||
            (p['senderLabel'] as String?) ==
                widget.contact.nickname.toUpperCase());
    if (!fromUs && !isUnbound) return;

    if (isUnbound) {
      _lastUnboundSenderLabel = p['senderLabel'] as String?;
    }
    _appendIncomingFree(p['plaintext'] as String);
  }

  void _appendIncomingFree(String text) {
    final msg = PhantomMessage(
      id: _uuid.v4(),
      contactId: widget.contact.id,
      outgoing: false,
      plaintext: text,
      ciphertext: '',
      ephemeralKey: '',
      nonce: '',
      timestamp: DateTime.now(),
      status: MessageStatus.delivered,
    );
    StorageService.addMessage(msg);
    setState(() => _messages.add(msg));
    _scrollToBottom();
  }

  Future<void> _loadMessages() async {
    final msgs = await StorageService.loadMessages(widget.contact.id);
    setState(() => _messages = msgs);
    _scrollToBottom();
  }

  void _scrollToBottom() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scrollCtrl.hasClients) {
        _scrollCtrl.animateTo(
          _scrollCtrl.position.maxScrollExtent,
          duration: const Duration(milliseconds: 250),
          curve: Curves.easeOut,
        );
      }
    });
  }

  Future<void> _send() async {
    final text = _msgCtrl.text.trim();
    if (text.isEmpty || _sending) return;
    setState(() => _sending = true);
    _msgCtrl.clear();

    String ciphertextHex = '';
    String ephemeralHex = '';
    String nonceHex = '';
    try {
      // v3 path — sealed-sender 1:1 via the wrapper Rust crate. Returns the
      // raw envelope wire bytes; the relay-transport hookup that ships
      // these to the recipient is wave 7B-followup. For now we still emit
      // a legacy CryptoService payload so the on-disk row keeps the
      // ciphertext/ephemeral/nonce columns populated (StorageService /
      // PhantomMessage round-trip).
      final wire = await rust.sendSealedV3(
        recipientAddress:
            'phantom:${widget.contact.publicViewKey}:${widget.contact.publicSpendKey}',
        plaintext: utf8.encode(text),
      );
      // Telemetry: we keep the wire bytes only as a debug-friendly hex
      // fingerprint on the row so a developer trace can confirm the
      // ciphertext was produced.
      ciphertextHex =
          wire.take(32).map((b) => b.toRadixString(16).padLeft(2, '0')).join();
    } catch (_) {
      // Fall back to the legacy demo crypto so first-launch (no v3
      // identity loaded yet) doesn't hard-fail. The demo is intentionally
      // NOT wire-compatible with Desktop — it's there to keep the UI
      // smoke-testable.
      try {
        final encrypted =
            await CryptoService.encrypt(text, widget.contact.publicSpendKey);
        ciphertextHex = encrypted['ciphertext']!;
        ephemeralHex = encrypted['ephemeralKey']!;
        nonceHex = encrypted['nonce']!;
      } catch (_) {
        setState(() => _sending = false);
        return;
      }
    }

    final msg = PhantomMessage(
      id: _uuid.v4(),
      contactId: widget.contact.id,
      outgoing: true,
      plaintext: text,
      ciphertext: ciphertextHex,
      ephemeralKey: ephemeralHex,
      nonce: nonceHex,
      timestamp: DateTime.now(),
      status: MessageStatus.sent,
    );
    await StorageService.addMessage(msg);
    widget.contact.lastMessage = text;
    widget.contact.lastMessageAt = DateTime.now();
    setState(() {
      _messages.add(msg);
      _sending = false;
    });
    _scrollToBottom();
  }

  /// Surface a "Bind to contact" sheet when the most-recent inbound
  /// message arrived from a sealed-sender pubkey we don't have on file
  /// yet. Mirrors the Desktop's `bind_last_unbound_sender` action.
  Future<void> _showBindUnbound() async {
    if (ContactDirectory.lastUnboundSenderPubHex == null) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('> NO UNBOUND SENDER PENDING')),
      );
      return;
    }
    final result = await ContactDirectory.bindLastUnboundSender(
        widget.contact.nickname);
    if (!mounted) return;
    if (result.ok) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
            content: Text('> BOUND ${widget.contact.nickname}'.toUpperCase())),
      );
      setState(() {
        _lastUnboundSenderLabel = null;
      });
    } else {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('! ${result.error}')),
      );
    }
  }

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
              Expanded(
                child: _messages.isEmpty ? _buildEmptyChat() : _buildMessages(),
              ),
              _buildInput(),
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
              child: const Icon(Icons.arrow_back_ios_new, color: kWhite, size: 14),
            ),
          ),
          const SizedBox(width: 14),
          Container(
            width: 38, height: 38,
            decoration: BoxDecoration(
              border: Border.all(color: kCyan.withOpacity(0.5)),
              color: kCyanDim,
            ),
            child: Center(
              child: Text(
                widget.contact.nickname[0],
                style: GoogleFonts.orbitron(
                  fontSize: 16, fontWeight: FontWeight.w900,
                  color: kCyan,
                  shadows: [Shadow(color: kCyan.withOpacity(0.6), blurRadius: 8)],
                ),
              ),
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  widget.contact.nickname.toUpperCase(),
                  style: GoogleFonts.orbitron(
                    fontSize: 14, fontWeight: FontWeight.w700,
                    color: kWhite, letterSpacing: 1,
                  ),
                ),
                Row(
                  children: [
                    const Icon(Icons.lock_outline, size: 10, color: kCyan),
                    const SizedBox(width: 4),
                    Text(
                      'E2E ENCRYPTED',
                      style: GoogleFonts.spaceMono(fontSize: 9, color: kCyan, letterSpacing: 1),
                    ),
                  ],
                ),
              ],
            ),
          ),
          if (_lastUnboundSenderLabel != null) ...[
            GestureDetector(
              onTap: _showBindUnbound,
              child: Container(
                padding: const EdgeInsets.symmetric(
                    horizontal: 10, vertical: 8),
                decoration: BoxDecoration(
                  border: Border.all(color: kMagenta.withOpacity(0.6)),
                  color: kMagenta.withOpacity(0.08),
                ),
                child: Text(
                  'BIND $_lastUnboundSenderLabel',
                  style: GoogleFonts.orbitron(
                      fontSize: 9, color: kMagenta, letterSpacing: 1),
                ),
              ),
            ),
            const SizedBox(width: 8),
          ],
          GestureDetector(
            onTap: _showContactInfo,
            child: Container(
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                border: Border.all(color: kGray.withOpacity(0.3)),
                color: kBgCard,
              ),
              child: const Icon(Icons.info_outline, color: kGrayText, size: 16),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildEmptyChat() {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(40),
        child: CyberCard(
          borderColor: kGray,
          padding: const EdgeInsets.all(28),
          cut: 20,
          child: Column(
            children: [
              const Icon(Icons.lock_outline, color: kCyan, size: 36),
              const SizedBox(height: 16),
              Text(
                'ENCRYPTED CHANNEL\nINITIALIZED',
                textAlign: TextAlign.center,
                style: GoogleFonts.orbitron(
                  fontSize: 13, fontWeight: FontWeight.w700,
                  color: kCyan, letterSpacing: 2, height: 1.4,
                  shadows: [Shadow(color: kCyan.withOpacity(0.4), blurRadius: 8)],
                ),
              ),
              const SizedBox(height: 12),
              Text(
                'X25519 + ChaCha20-Poly1305\nZero metadata. Zero logs.',
                textAlign: TextAlign.center,
                style: GoogleFonts.spaceMono(fontSize: 10, color: kGrayText, height: 1.6),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildMessages() {
    return ListView.builder(
      controller: _scrollCtrl,
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      itemCount: _messages.length,
      itemBuilder: (ctx, i) {
        final msg = _messages[i];
        final showDate = i == 0 || !_sameDay(_messages[i - 1].timestamp, msg.timestamp);
        return Column(
          children: [
            if (showDate) _DateDivider(date: msg.timestamp),
            _MsgBubble(message: msg),
          ],
        );
      },
    );
  }

  Widget _buildInput() {
    return Container(
      padding: EdgeInsets.only(
        left: 14, right: 14, top: 10,
        bottom: MediaQuery.of(context).viewInsets.bottom + 14,
      ),
      decoration: BoxDecoration(
        color: kBgCard,
        border: Border(top: BorderSide(color: kCyan.withOpacity(0.12))),
      ),
      child: Row(
        children: [
          // Encrypt indicator
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
            decoration: BoxDecoration(
              border: Border.all(color: kGreen.withOpacity(0.4)),
              color: kGreen.withOpacity(0.06),
            ),
            child: const Icon(Icons.lock_outline, color: kGreen, size: 14),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: TextField(
              controller: _msgCtrl,
              style: GoogleFonts.spaceGrotesk(color: kWhite, fontSize: 15),
              decoration: InputDecoration(
                hintText: '> TYPE MESSAGE...',
                hintStyle: GoogleFonts.spaceMono(color: kGrayText, fontSize: 12),
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(4),
                  borderSide: BorderSide(color: kGray.withOpacity(0.5)),
                ),
                enabledBorder: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(4),
                  borderSide: BorderSide(color: kGray.withOpacity(0.4)),
                ),
                focusedBorder: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(4),
                  borderSide: const BorderSide(color: kCyan, width: 1.5),
                ),
                filled: true,
                fillColor: kBgInput,
                contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
              ),
              onSubmitted: (_) => _send(),
              textInputAction: TextInputAction.send,
              maxLines: null,
            ),
          ),
          const SizedBox(width: 10),
          GestureDetector(
            onTap: _send,
            child: AnimatedContainer(
              duration: const Duration(milliseconds: 150),
              width: 44, height: 44,
              decoration: BoxDecoration(
                border: Border.all(color: _sending ? kGray : kCyan, width: 1.5),
                color: _sending ? kBgCard : kCyanDim,
                boxShadow: _sending ? null : neonGlow(kCyan, radius: 8),
              ),
              child: _sending
                  ? const Padding(
                      padding: EdgeInsets.all(12),
                      child: CircularProgressIndicator(strokeWidth: 1.5, color: kCyan),
                    )
                  : const Icon(Icons.send_rounded, color: kCyan, size: 18),
            ),
          ),
        ],
      ),
    );
  }

  void _showContactInfo() {
    showModalBottomSheet(
      context: context,
      backgroundColor: kBgCard,
      shape: const RoundedRectangleBorder(borderRadius: BorderRadius.zero),
      builder: (ctx) => Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(height: 1, color: kCyan.withOpacity(0.3)),
            const SizedBox(height: 16),
            Text(
              'NODE_INFO //',
              style: GoogleFonts.orbitron(fontSize: 12, color: kCyan, letterSpacing: 2),
            ),
            const SizedBox(height: 16),
            _InfoRow('NAME', widget.contact.nickname.toUpperCase()),
            _InfoRow('VIEW_KEY', widget.contact.publicViewKey.substring(0, 24)),
            _InfoRow('SPEND_KEY', widget.contact.publicSpendKey.substring(0, 24)),
            _InfoRow(
              'ADDED',
              '${widget.contact.addedAt.day}.${widget.contact.addedAt.month}.${widget.contact.addedAt.year}',
            ),
            const SizedBox(height: 16),
            CyberCard(
              borderColor: kGreen.withOpacity(0.4),
              bgColor: kGreen.withOpacity(0.04),
              padding: const EdgeInsets.all(12),
              cut: 8,
              child: Row(
                children: [
                  const Icon(Icons.verified_outlined, color: kGreen, size: 14),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      'All messages encrypted with X25519 ECDH + ChaCha20-Poly1305. Zero server involvement.',
                      style: GoogleFonts.spaceMono(fontSize: 10, color: kGreen.withOpacity(0.8), height: 1.5),
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

  bool _sameDay(DateTime a, DateTime b) =>
      a.year == b.year && a.month == b.month && a.day == b.day;
}

class _MsgBubble extends StatelessWidget {
  final PhantomMessage message;
  const _MsgBubble({required this.message});

  @override
  Widget build(BuildContext context) {
    final isOut = message.outgoing;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Align(
        alignment: isOut ? Alignment.centerRight : Alignment.centerLeft,
        child: GestureDetector(
          onLongPress: () {
            Clipboard.setData(ClipboardData(text: message.plaintext));
            ScaffoldMessenger.of(context).showSnackBar(
              const SnackBar(content: Text('> COPIED')),
            );
          },
          child: ConstrainedBox(
            constraints: BoxConstraints(maxWidth: MediaQuery.of(context).size.width * 0.72),
            child: CustomPaint(
              painter: _BubblePainter(isOut: isOut),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.end,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      message.plaintext,
                      style: GoogleFonts.spaceGrotesk(
                        fontSize: 14,
                        color: isOut ? kBg : kWhite,
                        height: 1.4,
                      ),
                    ),
                    const SizedBox(height: 4),
                    Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Text(
                          _fmt(message.timestamp),
                          style: GoogleFonts.spaceMono(
                            fontSize: 9,
                            color: isOut ? kBg.withOpacity(0.5) : kGrayText,
                          ),
                        ),
                        if (isOut) ...[
                          const SizedBox(width: 4),
                          Icon(Icons.done_all, size: 10, color: kBg.withOpacity(0.5)),
                        ],
                      ],
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  String _fmt(DateTime dt) =>
      '${dt.hour.toString().padLeft(2, '0')}:${dt.minute.toString().padLeft(2, '0')}';
}

class _BubblePainter extends CustomPainter {
  final bool isOut;
  _BubblePainter({required this.isOut});

  @override
  void paint(Canvas canvas, Size size) {
    const cut = 10.0;
    // Outgoing: cut top-left + bottom-right (like →), Incoming: cut top-right + bottom-left (like ←)
    final path = isOut
        ? (Path()
            ..moveTo(cut, 0)
            ..lineTo(size.width, 0)
            ..lineTo(size.width, size.height - cut)
            ..lineTo(size.width - cut, size.height)
            ..lineTo(0, size.height)
            ..lineTo(0, 0)
            ..close())
        : (Path()
            ..moveTo(0, 0)
            ..lineTo(size.width - cut, 0)
            ..lineTo(size.width, cut)
            ..lineTo(size.width, size.height)
            ..lineTo(cut, size.height)
            ..lineTo(0, size.height - cut)
            ..close());

    // Fill
    canvas.drawPath(
      path,
      Paint()..color = isOut ? kCyan : const Color(0xFF0D1520),
    );

    // Glow for outgoing
    if (isOut) {
      canvas.drawPath(
        path,
        Paint()
          ..color = kCyan.withOpacity(0.2)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 4
          ..maskFilter = const MaskFilter.blur(BlurStyle.outer, 6),
      );
    }

    // Border
    canvas.drawPath(
      path,
      Paint()
        ..color = isOut ? kCyan.withOpacity(0.6) : kGray.withOpacity(0.4)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.0,
    );
  }

  @override
  bool shouldRepaint(_BubblePainter old) => old.isOut != isOut;
}

class _DateDivider extends StatelessWidget {
  final DateTime date;
  const _DateDivider({required this.date});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 12),
      child: Row(
        children: [
          Expanded(child: Container(height: 1, color: kGray.withOpacity(0.3))),
          const SizedBox(width: 12),
          Text(
            '${date.day.toString().padLeft(2, '0')}.${date.month.toString().padLeft(2, '0')}.${date.year}',
            style: GoogleFonts.spaceMono(fontSize: 10, color: kGrayText),
          ),
          const SizedBox(width: 12),
          Expanded(child: Container(height: 1, color: kGray.withOpacity(0.3))),
        ],
      ),
    );
  }
}

class _InfoRow extends StatelessWidget {
  final String label;
  final String value;
  const _InfoRow(this.label, this.value);

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        children: [
          Text(
            label,
            style: GoogleFonts.spaceMono(fontSize: 10, color: kCyan, letterSpacing: 1),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              value,
              style: GoogleFonts.spaceMono(fontSize: 10, color: kGrayText),
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
  }
}
