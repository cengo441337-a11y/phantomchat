import 'package:flutter/material.dart';
import 'package:mobile/src/rust/api.dart';
import 'package:mobile/src/theme/cyberpunk_theme.dart';

class ChatView extends StatefulWidget {
  final String peerId;
  final String myPhantomId;

  const ChatView({super.key, required this.peerId, required this.myPhantomId});

  @override
  State<ChatView> createState() => _ChatViewState();
}

class _ChatViewState extends State<ChatView> {
  final TextEditingController _controller = TextEditingController();
  final List<ChatMessage> _messages = [];

  void _sendMessage() {
    if (_controller.text.isEmpty) return;
    final text = _controller.text;
    _controller.clear();

    setState(() {
      _messages.insert(0, ChatMessage(text: text, isMe: true));
    });

    sendSecureMessage(
      targetPeerId: widget.peerId,
      phantomId: widget.myPhantomId,
      message: text,
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Text(
          "SECURE CHANNEL: ${widget.peerId.substring(0, 8)}...",
          style: const TextStyle(fontSize: 14, letterSpacing: 1.0),
        ),
        actions: [
          const Icon(Icons.lock_outline, color: CyberpunkTheme.neonGreen),
          const SizedBox(width: 16),
        ],
      ),
      body: Column(
        children: [
          Expanded(
            child: ListView.builder(
              reverse: true,
              padding: const EdgeInsets.all(16),
              itemCount: _messages.length,
              itemBuilder: (context, index) {
                final msg = _messages[index];
                return _ChatBubble(message: msg);
              },
            ),
          ),
          _buildInput(),
        ],
      ),
    );
  }

  Widget _buildInput() {
    return Container(
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: Colors.black,
        border: Border(top: BorderSide(color: CyberpunkTheme.neonGreen.withOpacity(0.3))),
      ),
      child: Row(
        children: [
          Expanded(
            child: TextField(
              controller: _controller,
              style: const TextStyle(color: CyberpunkTheme.terminalGreen),
              decoration: InputDecoration(
                hintText: "TRANSMIT DATA...",
                hintStyle: TextStyle(color: CyberpunkTheme.terminalGreen.withOpacity(0.3)),
                border: OutlineInputBorder(
                  borderSide: const BorderSide(color: CyberpunkTheme.neonGreen),
                  borderRadius: BorderRadius.circular(0),
                ),
              ),
              onSubmitted: (_) => _sendMessage(),
            ),
          ),
          const SizedBox(width: 12),
          IconButton(
            icon: const Icon(Icons.send, color: CyberpunkTheme.neonMagenta),
            onPressed: _sendMessage,
          ),
        ],
      ),
    );
  }
}

class ChatMessage {
  final String text;
  final bool isMe;

  ChatMessage({required this.text, required this.isMe});
}

class _ChatBubble extends StatelessWidget {
  final ChatMessage message;

  const _ChatBubble({required this.message});

  @override
  Widget build(BuildContext context) {
    return Align(
      alignment: message.isMe ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 4),
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
        decoration: BoxDecoration(
          color: message.isMe ? CyberpunkTheme.neonMagenta.withOpacity(0.1) : CyberpunkTheme.neonGreen.withOpacity(0.05),
          border: Border.all(
            color: message.isMe ? CyberpunkTheme.neonMagenta : CyberpunkTheme.neonGreen,
            width: 1,
          ),
          borderRadius: BorderRadius.circular(2),
        ),
        child: Text(
          message.text,
          style: TextStyle(
            color: message.isMe ? Colors.white : CyberpunkTheme.terminalGreen,
            fontSize: 14,
            fontFamily: 'monospace',
          ),
        ),
      ),
    );
  }
}