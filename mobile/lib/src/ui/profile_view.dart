import 'package:flutter/material.dart';
import 'package:image_picker/image_picker.dart';
import 'package:mobile/src/theme/cyberpunk_theme.dart';
import 'package:mobile/lib/services/ipfs_service.dart';
import 'package:mobile/lib/services/privacy_service.dart';
import 'package:mobile/lib/src/rust/api.dart';
import 'package:mobile/lib/src/ui/privacy_settings_view.dart';

class ProfileView extends StatefulWidget {
  final String phantomId;
  const ProfileView({super.key, required this.phantomId});

  @override
  State<ProfileView> createState() => _ProfileViewState();
}

class _ProfileViewState extends State<ProfileView> {
  String? _avatarCid;
  bool _uploading = false;
  PrivacyMode _privacyMode = PrivacyMode.dailyUse;

  @override
  void initState() {
    super.initState();
    PrivacyService.loadMode().then((m) => setState(() => _privacyMode = m));
  }

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
            const SizedBox(height: 32),
            _buildPrivacyTile(),
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

  Widget _buildPrivacyTile() {
    final isStealth = _privacyMode == PrivacyMode.maximumStealth;
    return GestureDetector(
      onTap: () async {
        await Navigator.push(
          context,
          MaterialPageRoute(builder: (_) => const PrivacySettingsView()),
        );
        // Refresh mode indicator after returning from settings
        final updated = await PrivacyService.loadMode();
        setState(() => _privacyMode = updated);
      },
      child: Container(
        margin: const EdgeInsets.symmetric(horizontal: 40),
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        decoration: BoxDecoration(
          border: Border.all(
            color: isStealth ? CyberpunkTheme.neonMagenta : CyberpunkTheme.neonGreen,
          ),
          color: (isStealth ? CyberpunkTheme.neonMagenta : CyberpunkTheme.neonGreen)
              .withOpacity(0.05),
        ),
        child: Row(
          children: [
            Icon(
              isStealth ? Icons.visibility_off_outlined : Icons.shield_outlined,
              color: isStealth ? CyberpunkTheme.neonMagenta : CyberpunkTheme.neonGreen,
              size: 20,
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    isStealth ? 'MAXIMUM STEALTH' : 'DAILY USE',
                    style: TextStyle(
                      color: isStealth ? CyberpunkTheme.neonMagenta : CyberpunkTheme.neonGreen,
                      fontSize: 12,
                      letterSpacing: 2,
                      fontFamily: 'Courier',
                    ),
                  ),
                  Text(
                    isStealth ? 'Relay-only · Tor/Nym SOCKS5' : 'libp2p + Dandelion++',
                    style: const TextStyle(
                      color: Colors.white38,
                      fontSize: 10,
                      fontFamily: 'Courier',
                    ),
                  ),
                ],
              ),
            ),
            const Icon(Icons.chevron_right, color: Colors.white38, size: 18),
          ],
        ),
      ),
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
