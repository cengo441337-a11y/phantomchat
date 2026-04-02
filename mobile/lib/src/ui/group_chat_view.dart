import 'package:flutter/material.dart';
import 'package:mobile/src/theme/cyberpunk_theme.dart';
import 'package:mobile/src/rust/api.dart';
import 'package:mobile/src/rust/network.dart';

class GroupChatView extends StatefulWidget {
  final String groupId;
  const GroupChatView({super.key, required this.groupId});

  @override
  State<GroupChatView> createState() => _GroupChatViewState();
}

class _GroupChatViewState extends State<GroupChatView> {
  final TextEditingController _controller = TextEditingController();
  final List<String> _messages = [];

  @override
  void initState() {
    super.initState();
    // In real app, listen to the global node stream and filter for this groupId
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        title: Text('GROUP: ${widget.groupId}', style: const TextStyle(color: CyberpunkTheme.neonGreen, fontSize: 14)),
        backgroundColor: Colors.black,
      ),
      body: Column(
        children: [
          Expanded(
            child: ListView.builder(
              padding: const EdgeInsets.all(20),
              itemCount: _messages.length,
              itemBuilder: (context, index) => _buildMessageBubble(_messages[index]),
            ),
          ),
          _buildInputArea(),
        ],
      ),
    );
  }

  Widget _buildMessageBubble(String msg) {
    return Align(
      alignment: Alignment.centerLeft,
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 4),
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(border: Border.all(color: CyberpunkTheme.neonGreen, width: 1), color: Colors.black),
        child: Text(msg, style: const TextStyle(color: CyberpunkTheme.neonGreen, fontSize: 12)),
      ),
    );
  }

  Widget _buildInputArea() {
    return Container(
      padding: const EdgeInsets.all(20),
      color: Colors.black,
      child: Row(
        children: [
          Expanded(
            child: TextField(
              controller: _controller,
              style: const TextStyle(color: Colors.white),
              decoration: const InputDecoration(hintText: 'BROADCAST...', hintStyle: TextStyle(color: Colors.grey), border: InputBorder.none),
            ),
          ),
          IconButton(
            icon: const Icon(Icons.send, color: CyberpunkTheme.neonGreen),
            onPressed: () async {
              final msg = _controller.text;
              if (msg.isNotEmpty) {
                 await sendGroupMessage(groupId: widget.groupId, message: msg);
                 setState(() { _messages.add(msg); });
                 _controller.clear();
              }
            },
          ),
        ],
      ),
    );
  }
}
