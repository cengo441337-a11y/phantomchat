import 'package:flutter/material.dart';
import 'package:mobile/src/theme/cyberpunk_theme.dart';
import 'package:mobile/src/rust/api.dart';
import 'package:mobile/src/ui/group_chat_view.dart';

class GroupsListView extends StatefulWidget {
  const GroupsListView({super.key});

  @override
  State<GroupsListView> createState() => _GroupsListViewState();
}

class _GroupsListViewState extends State<GroupsListView> {
  final List<String> _groups = [];

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        title: const Text('DECENTRALIZED GROUPS', style: TextStyle(color: CyberpunkTheme.neonGreen, fontSize: 16)),
        backgroundColor: Colors.black,
        leading: IconButton(icon: const Icon(Icons.arrow_back, color: Colors.white), onPressed: () => Navigator.pop(context)),
      ),
      body: Column(
        children: [
          const SizedBox(height: 20),
          _buildCreateGroupButton(),
          const SizedBox(height: 20),
          Expanded(child: _buildGroupsList()),
        ],
      ),
    );
  }

  Widget _buildCreateGroupButton() {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 40),
      child: OutlinedButton.icon(
        style: OutlinedButton.styleFrom(side: const BorderSide(color: CyberpunkTheme.neonGreen)),
        onPressed: _showCreateGroupDialog,
        icon: const Icon(Icons.add, color: CyberpunkTheme.neonGreen),
        label: const Text('INITIALIZE NEW GROUP', style: TextStyle(color: CyberpunkTheme.neonGreen)),
      ),
    );
  }

  Widget _buildGroupsList() {
    return _groups.isEmpty 
      ? const Center(child: Text('NO ACTIVE CHANNELS', style: TextStyle(color: Colors.grey)))
      : ListView.builder(
          itemCount: _groups.length,
          itemBuilder: (context, index) => ListTile(
            title: Text(_groups[index], style: const TextStyle(color: CyberpunkTheme.neonGreen)),
            subtitle: const Text('ENCRYPTED GOSSIPSUB', style: TextStyle(color: Colors.grey, fontSize: 10)),
            trailing: const Icon(Icons.chevron_right, color: CyberpunkTheme.neonGreen),
            onTap: () => Navigator.push(context, MaterialPageRoute(builder: (c) => GroupChatView(groupId: _groups[index]))),
          ),
        );
  }

  void _showCreateGroupDialog() {
     final controller = TextEditingController();
     showDialog(context: context, builder: (c) => AlertDialog(
       backgroundColor: Colors.black,
       title: const Text('GROUP INITIALIZATION', style: TextStyle(color: CyberpunkTheme.neonGreen)),
       content: TextField(
         controller: controller,
         style: const TextStyle(color: Colors.white),
         decoration: const InputDecoration(hintText: 'GROUP ID (e.g. STRIKE-ONE)', hintStyle: TextStyle(color: Colors.grey)),
       ),
       actions: [
         TextButton(onPressed: () => Navigator.pop(c), child: const Text('CANCEL')),
         TextButton(onPressed: () async {
            final id = controller.text;
            if (id.isNotEmpty) {
               await joinGroup(groupId: id);
               setState(() { _groups.add(id); });
               Navigator.pop(c);
            }
         }, child: const Text('INITIALIZE')),
       ],
     ));
  }
}
