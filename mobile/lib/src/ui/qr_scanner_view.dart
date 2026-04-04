import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';
import 'package:mobile/src/theme/cyberpunk_theme.dart';

class QRScannerView extends StatefulWidget {
  const QRScannerView({super.key});
  @override
  State<QRScannerView> createState() => _QRScannerViewState();
}

class _QRScannerViewState extends State<QRScannerView> {
  final MobileScannerController controller = MobileScannerController();

  @override
  void dispose() {
    controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('SCAN PHANTOM ID', style: TextStyle(color: CyberpunkTheme.neonGreen, letterSpacing: 2)),
        backgroundColor: Colors.black,
        leading: IconButton(icon: const Icon(Icons.arrow_back, color: CyberpunkTheme.neonMagenta), onPressed: () => Navigator.pop(context)),
      ),
      body: Stack(
        children: [
          MobileScanner(
            controller: controller,
            onDetect: (capture) {
              final List<Barcode> barcodes = capture.barcodes;
              for (final barcode in barcodes) {
                if (barcode.rawValue != null) {
                   Navigator.pop(context, barcode.rawValue);
                   break;
                }
              }
            },
          ),
          _buildScanningOverlay(),
        ],
      ),
    );
  }

  Widget _buildScanningOverlay() {
    return Column(
      children: [
        const Expanded(child: SizedBox()),
        Center(
          child: Container(
            width: 250, height: 250,
            decoration: BoxDecoration(border: Border.all(color: CyberpunkTheme.neonGreen, width: 2), color: Colors.transparent),
            child: const Center(child: Text('LINKING...', style: TextStyle(color: CyberpunkTheme.neonGreen, fontSize: 10, letterSpacing: 4))),
          ),
        ),
        const Expanded(child: SizedBox()),
        const Padding(padding: EdgeInsets.all(40), child: Text('ALIGN QR WITH RADAR GRID', style: TextStyle(color: Colors.white, fontSize: 12, letterSpacing: 1))),
      ],
    );
  }
}
