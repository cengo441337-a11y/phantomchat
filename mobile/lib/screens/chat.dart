import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:uuid/uuid.dart';
import '../models/contact.dart';
import '../models/identity.dart';
import '../models/message.dart';
import '../services/crypto_service.dart';
import '../services/storage_service.dart';
import '../theme.dart';

class ChatScreen extends StatefulWidget {
  final PhantomContact contact;
  final PhantomIdentity identity;

  const ChatScreen({
    super.key,
    required this.contact,
    required this.identity,
  });

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final _msgCtrl = TextEditingController();
  final _scrollCtrl = ScrollController();
  List<PhantomMessage> _messages = [];
  bool _sending = false;
  static const _uuid = Uuid();

  @override
  void initState() {
    super.initState();
    _loadMessages();
  }

  @override
  void dispose() {
    _msgCtrl.dispose();
    _scrollCtrl.dispose();
    super.dispose();
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
          duration: const Duration(milliseconds: 300),
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

    try {
      // Encrypt with recipient's spend key
      final encrypted = await CryptoService.encrypt(
        text,
        widget.contact.publicSpendKey,
      );

      final msg = PhantomMessage(
        id: _uuid.v4(),
        contactId: widget.contact.id,
        outgoing: true,
        plaintext: text,
        ciphertext: encrypted['ciphertext']!,
        ephemeralKey: encrypted['ephemeralKey']!,
        nonce: encrypted['nonce']!,
        timestamp: DateTime.now(),
        status: MessageStatus.sent,
      );

      await StorageService.addMessage(msg);
      setState(() {
        _messages.add(msg);
        _sending = false;
      });

      // Update contact's last message
      widget.contact.lastMessage = text;
      widget.contact.lastMessageAt = DateTime.now();

      _scrollToBottom();
    } catch (e) {
      setState(() => _sending = false);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Fehler: $e'), backgroundColor: kRed),
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      appBar: AppBar(
        backgroundColor: kBg,
        leading: IconButton(
          icon: const Icon(Icons.arrow_back_ios, size: 18),
          onPressed: () => Navigator.pop(context),
        ),
        title: Row(
          children: [
            Container(
              width: 36,
              height: 36,
              decoration: BoxDecoration(
                color: kBgCard,
                shape: BoxShape.circle,
                border: Border.all(color: const Color(0xFF1E2733)),
              ),
              child: Center(
                child: Text(
                  widget.contact.nickname[0].toUpperCase(),
                  style: GoogleFonts.spaceGrotesk(
                    fontWeight: FontWeight.w700,
                    color: kNeonText,
                    fontSize: 14,
                  ),
                ),
              ),
            ),
            const SizedBox(width: 10),
            Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  widget.contact.nickname,
                  style: GoogleFonts.spaceGrotesk(
                    fontSize: 16,
                    fontWeight: FontWeight.w700,
                    color: kWhite,
                  ),
                ),
                Row(
                  children: [
                    const Icon(Icons.lock_outline, size: 10, color: kNeon),
                    const SizedBox(width: 3),
                    Text(
                      'Ende-zu-Ende verschlüsselt',
                      style: GoogleFonts.spaceGrotesk(
                        fontSize: 10,
                        color: kNeon,
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ],
        ),
        actions: [
          IconButton(
            icon: const Icon(Icons.info_outline, size: 20, color: kGray),
            onPressed: _showContactInfo,
          ),
        ],
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(1),
          child: Container(height: 1, color: const Color(0xFF1A2030)),
        ),
      ),
      body: Column(
        children: [
          Expanded(
            child: _messages.isEmpty
                ? _buildEmptyChat()
                : ListView.builder(
                    controller: _scrollCtrl,
                    padding: const EdgeInsets.symmetric(
                      horizontal: 16,
                      vertical: 12,
                    ),
                    itemCount: _messages.length,
                    itemBuilder: (ctx, i) {
                      final msg = _messages[i];
                      final showDate = i == 0 ||
                          !_sameDay(
                            _messages[i - 1].timestamp,
                            msg.timestamp,
                          );
                      return Column(
                        children: [
                          if (showDate) _DateDivider(date: msg.timestamp),
                          _MessageBubble(message: msg),
                        ],
                      );
                    },
                  ),
          ),
          _buildInput(),
        ],
      ),
    );
  }

  bool _sameDay(DateTime a, DateTime b) =>
      a.year == b.year && a.month == b.month && a.day == b.day;

  Widget _buildEmptyChat() {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(40),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            const Icon(Icons.lock_outline, color: kNeon, size: 40),
            const SizedBox(height: 16),
            Text(
              'Verschlüsselte Verbindung\nbereit',
              textAlign: TextAlign.center,
              style: GoogleFonts.spaceGrotesk(
                fontSize: 16,
                color: kWhiteDim,
                height: 1.5,
              ),
            ),
            const SizedBox(height: 8),
            Text(
              'Nachrichten werden mit X25519 +\nChaCha20-Poly1305 verschlüsselt.',
              textAlign: TextAlign.center,
              style: GoogleFonts.spaceGrotesk(fontSize: 12, color: kGray, height: 1.5),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildInput() {
    return Container(
      padding: EdgeInsets.only(
        left: 16,
        right: 16,
        top: 12,
        bottom: MediaQuery.of(context).viewInsets.bottom + 16,
      ),
      decoration: const BoxDecoration(
        color: kBg,
        border: Border(top: BorderSide(color: Color(0xFF1A2030))),
      ),
      child: Row(
        children: [
          Expanded(
            child: TextField(
              controller: _msgCtrl,
              style: GoogleFonts.spaceGrotesk(color: kWhite, fontSize: 15),
              decoration: const InputDecoration(
                hintText: 'Nachricht...',
                contentPadding: EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 12,
                ),
              ),
              onSubmitted: (_) => _send(),
              textInputAction: TextInputAction.send,
              maxLines: null,
            ),
          ),
          const SizedBox(width: 8),
          GestureDetector(
            onTap: _send,
            child: AnimatedContainer(
              duration: const Duration(milliseconds: 200),
              width: 46,
              height: 46,
              decoration: BoxDecoration(
                color: _sending ? kNeonDim : kNeon,
                borderRadius: BorderRadius.circular(12),
              ),
              child: _sending
                  ? const Padding(
                      padding: EdgeInsets.all(12),
                      child: CircularProgressIndicator(
                        strokeWidth: 2,
                        color: kNeon,
                      ),
                    )
                  : const Icon(Icons.send_rounded, color: kBg, size: 20),
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
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(24)),
      ),
      builder: (ctx) => Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              widget.contact.nickname,
              style: GoogleFonts.spaceGrotesk(
                fontSize: 22,
                fontWeight: FontWeight.w700,
                color: kWhite,
              ),
            ),
            const SizedBox(height: 16),
            _InfoRow(label: 'View Key', value: widget.contact.publicViewKey.substring(0, 32)),
            _InfoRow(label: 'Spend Key', value: widget.contact.publicSpendKey.substring(0, 32)),
            _InfoRow(
              label: 'Hinzugefügt',
              value: '${widget.contact.addedAt.day}.${widget.contact.addedAt.month}.${widget.contact.addedAt.year}',
            ),
            const SizedBox(height: 20),
            Text(
              'Alle Nachrichten sind Ende-zu-Ende verschlüsselt.\nNur du und ${widget.contact.nickname} können sie lesen.',
              style: GoogleFonts.spaceGrotesk(
                fontSize: 12,
                color: kGray,
                height: 1.5,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _MessageBubble extends StatelessWidget {
  final PhantomMessage message;
  const _MessageBubble({required this.message});

  @override
  Widget build(BuildContext context) {
    final isOut = message.outgoing;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 2),
      child: Align(
        alignment: isOut ? Alignment.centerRight : Alignment.centerLeft,
        child: GestureDetector(
          onLongPress: () {
            Clipboard.setData(ClipboardData(text: message.plaintext));
            ScaffoldMessenger.of(context).showSnackBar(
              const SnackBar(
                content: Text('Kopiert'),
                backgroundColor: kBgCard,
              ),
            );
          },
          child: Container(
            constraints: BoxConstraints(
              maxWidth: MediaQuery.of(context).size.width * 0.72,
            ),
            padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
            decoration: BoxDecoration(
              color: isOut ? kNeon : kBgCard,
              borderRadius: BorderRadius.only(
                topLeft: const Radius.circular(16),
                topRight: const Radius.circular(16),
                bottomLeft: Radius.circular(isOut ? 16 : 4),
                bottomRight: Radius.circular(isOut ? 4 : 16),
              ),
              border: isOut
                  ? null
                  : Border.all(color: const Color(0xFF1E2733)),
              boxShadow: isOut
                  ? [BoxShadow(color: kNeon.withOpacity(0.15), blurRadius: 8)]
                  : null,
            ),
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
                      _formatTime(message.timestamp),
                      style: GoogleFonts.spaceGrotesk(
                        fontSize: 10,
                        color: isOut ? kBg.withOpacity(0.5) : kGray,
                      ),
                    ),
                    if (isOut) ...[
                      const SizedBox(width: 3),
                      Icon(
                        Icons.done_all,
                        size: 12,
                        color: kBg.withOpacity(0.5),
                      ),
                    ],
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  String _formatTime(DateTime dt) {
    return '${dt.hour.toString().padLeft(2, '0')}:${dt.minute.toString().padLeft(2, '0')}';
  }
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
          const Expanded(child: Divider(color: Color(0xFF1A2030))),
          const SizedBox(width: 12),
          Text(
            '${date.day}.${date.month}.${date.year}',
            style: GoogleFonts.spaceGrotesk(fontSize: 11, color: kGray),
          ),
          const SizedBox(width: 12),
          const Expanded(child: Divider(color: Color(0xFF1A2030))),
        ],
      ),
    );
  }
}

class _InfoRow extends StatelessWidget {
  final String label;
  final String value;
  const _InfoRow({required this.label, required this.value});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        children: [
          Text(
            label,
            style: GoogleFonts.spaceGrotesk(
              fontSize: 12,
              fontWeight: FontWeight.w700,
              color: kGray,
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              value,
              style: GoogleFonts.spaceMono(fontSize: 10, color: kWhiteDim),
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
  }
}
