import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:mobile_scanner/mobile_scanner.dart';
import 'package:permission_handler/permission_handler.dart';

import '../theme.dart';
import '../widgets/cyber_card.dart';

/// "Add contact via QR" screen — Wave 8a.
///
/// Justifies the CAMERA permission (Option B from the polish task). Opens
/// the back camera through `mobile_scanner`, watches for any barcode
/// detected, and pops with the raw payload string when one is found.
///
/// The caller is responsible for parsing — typically by passing the result
/// through `PhantomContact.fromPhantomId(...)`. We deliberately do NOT
/// touch storage from this screen so it stays a pure single-purpose UI:
/// it scans, it returns a string, that's it.
///
/// Usage:
///
///     final raw = await Navigator.of(context).push<String>(
///       MaterialPageRoute(builder: (_) => const QrScanScreen()),
///     );
///     if (raw == null) return; // user cancelled
///     final contact = PhantomContact.fromPhantomId(raw, nickname);
class QrScanScreen extends StatefulWidget {
  const QrScanScreen({super.key});

  @override
  State<QrScanScreen> createState() => _QrScanScreenState();
}

class _QrScanScreenState extends State<QrScanScreen> {
  final MobileScannerController _controller = MobileScannerController(
    detectionSpeed: DetectionSpeed.normal,
    facing: CameraFacing.back,
  );

  bool _torchOn = false;
  bool _handled = false;
  // Permission state — `null` while we're awaiting the system prompt,
  // `true` once granted (camera shows live), `false` if the user denied
  // (we render the rationale + "open settings" CTA instead of an opaque
  // black view, which was 1.0.6's silent failure mode).
  bool? _camGranted;

  @override
  void initState() {
    super.initState();
    _ensureCameraPermission();
  }

  /// Explicitly request CAMERA at runtime. `mobile_scanner` claims to do
  /// this on Android 6+ but on real devices the surface stays opaque
  /// black if the permission was never granted — no prompt, no error,
  /// no recovery. We drive the permission flow ourselves so the user
  /// always gets the OS dialog, and on permanent-denial we link out to
  /// app-settings.
  Future<void> _ensureCameraPermission() async {
    final status = await Permission.camera.request();
    if (!mounted) return;
    setState(() => _camGranted = status.isGranted);
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _onDetect(BarcodeCapture capture) {
    if (_handled) return;
    for (final barcode in capture.barcodes) {
      final raw = barcode.rawValue;
      if (raw != null && raw.isNotEmpty) {
        _handled = true;
        HapticFeedback.heavyImpact();
        Navigator.of(context).pop<String>(raw);
        return;
      }
    }
  }

  Future<void> _toggleTorch() async {
    await _controller.toggleTorch();
    if (!mounted) return;
    setState(() => _torchOn = !_torchOn);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        backgroundColor: Colors.black,
        elevation: 0,
        leading: IconButton(
          icon: const Icon(Icons.arrow_back, color: kCyan),
          onPressed: () => Navigator.of(context).pop(),
        ),
        title: Text(
          'PHANTOM_ID SCAN',
          style: GoogleFonts.orbitron(
            fontSize: 14,
            fontWeight: FontWeight.w700,
            color: kWhite,
            letterSpacing: 3,
          ),
        ),
        actions: [
          IconButton(
            icon: Icon(
              _torchOn ? Icons.flash_on : Icons.flash_off,
              color: _torchOn ? kYellow : kGrayText,
            ),
            onPressed: _toggleTorch,
            tooltip: 'Taschenlampe',
          ),
        ],
      ),
      body: Stack(
        fit: StackFit.expand,
        children: [
          // Camera surface — only mounted once the OS has granted the
          // CAMERA permission. While we await the prompt we show a
          // loading hint; on denial we fall back to an explanatory
          // panel + an "app settings" CTA so the user never sees an
          // unexplained black surface (which is what 1.0.6 did when
          // mobile_scanner's implicit permission request was missed).
          if (_camGranted == true)
            MobileScanner(
              controller: _controller,
              onDetect: _onDetect,
              errorBuilder: (ctx, err, _) => _ScannerError(
                message: err.errorDetails?.message ?? err.errorCode.name,
              ),
            )
          else if (_camGranted == false)
            _CameraPermissionDenied(
              onRetry: _ensureCameraPermission,
            )
          else
            const Center(
              child: CircularProgressIndicator(
                strokeWidth: 1.6,
                valueColor: AlwaysStoppedAnimation<Color>(kCyan),
              ),
            ),
          // Reticle + cyberpunk frame overlay
          IgnorePointer(child: _Reticle()),
          // Bottom hint card
          Align(
            alignment: Alignment.bottomCenter,
            child: SafeArea(
              child: Padding(
                padding: const EdgeInsets.all(20),
                child: CyberCard(
                  borderColor: kCyan,
                  bgColor: const Color(0xCC050507),
                  padding: const EdgeInsets.all(14),
                  child: Row(
                    children: [
                      const Icon(Icons.qr_code_2, color: kCyan, size: 22),
                      const SizedBox(width: 12),
                      Expanded(
                        child: Text(
                          'QR-Code im Rahmen ausrichten — '
                          'Erkennung erfolgt automatisch.',
                          style: GoogleFonts.spaceMono(
                            fontSize: 11,
                            color: kWhite,
                            height: 1.5,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _Reticle extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return Center(
      child: SizedBox(
        width: 240,
        height: 240,
        child: CustomPaint(
          painter: _ReticlePainter(),
        ),
      ),
    );
  }
}

class _ReticlePainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = kCyan
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2;
    const corner = 28.0;

    // Top-left
    canvas.drawLine(const Offset(0, 0), const Offset(corner, 0), paint);
    canvas.drawLine(const Offset(0, 0), const Offset(0, corner), paint);
    // Top-right
    canvas.drawLine(Offset(size.width - corner, 0), Offset(size.width, 0), paint);
    canvas.drawLine(Offset(size.width, 0), Offset(size.width, corner), paint);
    // Bottom-left
    canvas.drawLine(Offset(0, size.height - corner), Offset(0, size.height), paint);
    canvas.drawLine(Offset(0, size.height), Offset(corner, size.height), paint);
    // Bottom-right
    canvas.drawLine(Offset(size.width - corner, size.height), Offset(size.width, size.height), paint);
    canvas.drawLine(Offset(size.width, size.height - corner), Offset(size.width, size.height), paint);

    // Centre crosshair
    final cx = size.width / 2;
    final cy = size.height / 2;
    final dim = Paint()
      ..color = kCyan.withValues(alpha: 0.35)
      ..strokeWidth = 1;
    canvas.drawLine(Offset(cx - 12, cy), Offset(cx + 12, cy), dim);
    canvas.drawLine(Offset(cx, cy - 12), Offset(cx, cy + 12), dim);
  }

  @override
  bool shouldRepaint(_) => false;
}

/// Rendered in place of the camera surface when the user denied (or the
/// system silently denied) the CAMERA permission. Offers a retry button —
/// `permission_handler` returns `permanentlyDenied` after the user taps
/// "don't allow" twice, so the retry path also opens app-settings so
/// they can re-grant manually.
class _CameraPermissionDenied extends StatelessWidget {
  final Future<void> Function() onRetry;
  const _CameraPermissionDenied({required this.onRetry});

  @override
  Widget build(BuildContext context) {
    return Container(
      color: Colors.black,
      padding: const EdgeInsets.all(32),
      child: Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            const Icon(Icons.no_photography, color: kMagenta, size: 48),
            const SizedBox(height: 16),
            Text(
              'KAMERA-FREIGABE FEHLT',
              textAlign: TextAlign.center,
              style: GoogleFonts.orbitron(
                fontSize: 14,
                color: kMagenta,
                letterSpacing: 2,
              ),
            ),
            const SizedBox(height: 8),
            Text(
              'PhantomChat braucht Kamera-Zugriff zum Scannen von Phantom-IDs. Sonst nichts — die Bilddaten verlassen das Gerät nie.',
              textAlign: TextAlign.center,
              style: GoogleFonts.spaceMono(
                fontSize: 11,
                color: kGrayText,
                height: 1.5,
              ),
            ),
            const SizedBox(height: 24),
            GestureDetector(
              onTap: () async {
                await onRetry();
                final stillDenied = !(await Permission.camera.isGranted);
                if (stillDenied) {
                  // Permanently-denied — only Settings can flip it.
                  await openAppSettings();
                }
              },
              child: Container(
                padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 10),
                decoration: BoxDecoration(
                  border: Border.all(color: kCyan, width: 1.4),
                  color: kCyan.withValues(alpha: 0.1),
                ),
                child: Text(
                  'KAMERA FREIGEBEN',
                  style: GoogleFonts.orbitron(
                    fontSize: 12,
                    color: kCyan,
                    letterSpacing: 2,
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ScannerError extends StatelessWidget {
  final String message;
  const _ScannerError({required this.message});

  @override
  Widget build(BuildContext context) {
    return Container(
      color: Colors.black,
      padding: const EdgeInsets.all(32),
      child: Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            const Icon(Icons.no_photography, color: kMagenta, size: 48),
            const SizedBox(height: 16),
            Text(
              'KAMERA NICHT VERFÜGBAR',
              textAlign: TextAlign.center,
              style: GoogleFonts.orbitron(
                fontSize: 14,
                color: kMagenta,
                letterSpacing: 2,
              ),
            ),
            const SizedBox(height: 8),
            Text(
              message,
              textAlign: TextAlign.center,
              style: GoogleFonts.spaceMono(
                fontSize: 11,
                color: kGrayText,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
