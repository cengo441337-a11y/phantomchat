import 'package:flutter/material.dart';
import 'package:qr_flutter/qr_flutter.dart';
import 'package:mobile/src/theme/cyberpunk_theme.dart';

class IdentityQRView extends StatelessWidget {
  final String phantomId;
  const IdentityQRView({super.key, required this.phantomId});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        title: const Text('MY PHANTOM IDENTITY', style: TextStyle(color: CyberpunkTheme.neonMagenta, fontSize: 16)),
        backgroundColor: Colors.black,
        leading: IconButton(icon: const Icon(Icons.close, color: Colors.white), onPressed: () => Navigator.pop(context)),
      ),
      body: Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            const Text('SCAN TO LINK', style: TextStyle(color: CyberpunkTheme.neonGreen, letterSpacing: 4, fontWeight: FontWeight.bold)),
            const SizedBox(height: 40),
            Container(
              padding: const EdgeInsets.all(16),
              decoration: BoxDecoration(color: Colors.white, border: Border.all(color: CyberpunkTheme.neonGreen, width: 4)),
              child: QrImageView(
                data: phantomId,
                version: QrVersions.auto,
                size: 250.0,
              ),
            ),
            const SizedBox(height: 40),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 40),
              child: Text(phantomId, style: const TextStyle(color: Colors.grey, fontSize: 10, fontFamily: 'monospace'), textAlign: TextAlign.center),
            ),
          ],
        ),
      ),
    );
  }
}
