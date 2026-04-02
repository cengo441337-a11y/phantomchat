import 'package:flutter/material.dart';
import 'package:image_picker/image_picker.dart';
import 'package:mobile/src/theme/cyberpunk_theme.dart';
import 'package:mobile/lib/services/ipfs_service.dart';
import 'package:mobile/lib/src/rust/api.dart';

class ProfileView extends StatefulWidget {
  final String phantomId;
  const ProfileView({super.key, required this.phantomId});

  @override
  State<ProfileView> createState() => _ProfileViewState();
}

class _ProfileViewState extends State<ProfileView> {
  String? _avatarCid;
  bool _uploading = false;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        title: const Text('IDENTITY SETTINGS', style: TextStyle(color: CyberpunkTheme.neonMagenta, fontSize: 16)),
        backgroundColor: Colors.black,
        leading: IconButton(icon: const Icon(Icons.close, color: Colors.white), onPressed: () => Navigator.pop(context)),
      ),
      body: Center(
        child: Column(
          children: [
            const SizedBox(height: 40),
            _buildAvatarFrame(),
            const SizedBox(height: 40),
            _buildIdentityGrid(),
            const Spacer(),
            Padding(
               padding: const EdgeInsets.all(40),
               child: OutlinedButton(
                 style: OutlinedButton.styleFrom(side: const BorderSide(color: Colors.red)),
                 onPressed: () => Navigator.pop(context),
                 child: const Text('DISMISS', style: TextStyle(color: Colors.red)),
               ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildAvatarFrame() {
    return GestureDetector(
      onTap: _pickImage,
      child: Container(
        width: 150, height: 150,
        decoration: BoxDecoration(border: Border.all(color: CyberpunkTheme.neonGreen, width: 2), color: Colors.black),
        child: Stack(
           children: [
             if (_avatarCid != null) Image.network(IpfsService.getUrl(_avatarCid!), fit: BoxFit.cover, errorBuilder: (c, e, s) => const Center(child: Icon(Icons.error, color: Colors.red))),
             if (_uploading) const Center(child: CircularProgressIndicator(color: CyberpunkTheme.neonGreen)),
             if (!_uploading && _avatarCid == null) const Center(child: Icon(Icons.add_a_photo, color: CyberpunkTheme.neonGreen, size: 40)),
           ],
        ),
      ),
    );
  }

  Widget _buildIdentityGrid() {
    return Column(
      children: [
        const Text('PUBLIC CID', style: TextStyle(color: Colors.grey, fontSize: 10)),
        const SizedBox(height: 8),
        Text(_avatarCid ?? 'NONE', style: const TextStyle(color: CyberpunkTheme.neonGreen, fontSize: 12, fontFamily: 'monospace')),
        const SizedBox(height: 20),
        const Text('DECENTRALIZED ANCHOR', style: TextStyle(color: CyberpunkTheme.neonGreen, letterSpacing: 2, fontSize: 8)),
      ],
    );
  }

  Future<void> _pickImage() async {
    final picker = ImagePicker();
    final file = await picker.pickImage(source: ImageSource.gallery, maxWidth: 512, maxHeight: 512);
    if (file != null) {
      setState(() { _uploading = true; });
      final cid = await IpfsService.uploadImage(file);
      if (cid != null) {
        await updateAvatarCid(cid: cid);
        setState(() { _avatarCid = cid; });
      }
      setState(() { _uploading = false; });
    }
  }
}
