// Wave 11G — modal triggered from the home-screen update banner.
//
// Three states:
//   - idle:        version + notes + size + [Download] / [Later]
//   - downloading: progress bar with bytes received / total + [Abort]
//   - installing:  spinner while we hand off to the package-installer
//
// This widget owns the lifetime of the download so a Navigator.pop() at
// any time cleanly cancels the in-flight HTTP request via the cancel
// flag. We deliberately don't persist anything across dismissal — if the
// user backs out, next launch they'll see the banner again and start
// over. APKs are small enough (<35 MB) that this is fine.

import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

import '../services/i18n.dart';
import '../services/update_service.dart';
import '../theme.dart';

class UpdateDialog extends StatefulWidget {
  final UpdateInfo info;

  const UpdateDialog({super.key, required this.info});

  @override
  State<UpdateDialog> createState() => _UpdateDialogState();
}

enum _Phase { idle, downloading, installing, error }

class _UpdateDialogState extends State<UpdateDialog> {
  _Phase _phase = _Phase.idle;
  int _received = 0;
  int _total = 0;
  String? _errorMessage;
  bool _aborted = false;

  Future<void> _startDownload() async {
    setState(() {
      _phase = _Phase.downloading;
      _received = 0;
      _total = widget.info.variant.sizeBytes;
      _errorMessage = null;
    });
    try {
      final apk = await UpdateService.downloadApk(
        widget.info,
        onProgress: (recv, total) {
          if (!mounted || _aborted) return;
          setState(() {
            _received = recv;
            // The manifest's size_bytes is authoritative — only fall
            // back to the response Content-Length if the manifest
            // omitted size (size_bytes==0).
            if (_total <= 0 && total > 0) _total = total;
          });
        },
      );
      if (_aborted || !mounted) return;
      setState(() => _phase = _Phase.installing);
      await UpdateService.installApk(apk);
      if (!mounted) return;
      // Don't auto-dismiss — once the system installer returns, our
      // process may have been replaced anyway. Just leave the dialog
      // showing the "installing" message.
    } catch (e) {
      if (!mounted || _aborted) return;
      final msg = e.toString().contains('sha256 mismatch')
          ? I18n.t('update.error.checksum')
          : I18n.t('update.error.download');
      setState(() {
        _phase = _Phase.error;
        _errorMessage = msg;
      });
    }
  }

  @override
  void dispose() {
    _aborted = true;
    super.dispose();
  }

  String _fmtBytes(int n) {
    if (n <= 0) return '—';
    const mb = 1024 * 1024;
    if (n >= mb) return '${(n / mb).toStringAsFixed(1)} MB';
    return '${(n / 1024).toStringAsFixed(0)} KB';
  }

  @override
  Widget build(BuildContext context) {
    return Dialog(
      backgroundColor: kBgCard,
      shape: const RoundedRectangleBorder(borderRadius: BorderRadius.zero),
      insetPadding: const EdgeInsets.symmetric(horizontal: 24, vertical: 32),
      child: Container(
        decoration: BoxDecoration(
          border: Border.all(color: kCyan.withValues(alpha: 0.6), width: 1.2),
          color: kBgCard,
        ),
        padding: const EdgeInsets.all(20),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: _buildBody(),
        ),
      ),
    );
  }

  List<Widget> _buildBody() {
    final info = widget.info;
    final header = Row(
      children: [
        Container(width: 3, height: 22, color: kCyan),
        const SizedBox(width: 12),
        Expanded(
          child: Text(
            I18n.t('update.dialog.title'),
            style: GoogleFonts.orbitron(
              fontSize: 14,
              fontWeight: FontWeight.w700,
              color: kWhite,
              letterSpacing: 1.5,
              shadows: [
                Shadow(color: kCyan.withValues(alpha: 0.5), blurRadius: 10),
              ],
            ),
          ),
        ),
      ],
    );

    final versionLine = Text(
      I18n.tf('update.dialog.version', {
        'current': info.currentVersion,
        'new': info.newVersion,
      }),
      style: GoogleFonts.spaceMono(fontSize: 12, color: kCyan, letterSpacing: 1),
    );

    final sizeLine = Text(
      I18n.tf('update.dialog.size', {
        'size': _fmtBytes(info.variant.sizeBytes),
      }),
      style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText),
    );

    switch (_phase) {
      case _Phase.idle:
        return [
          header,
          const SizedBox(height: 16),
          versionLine,
          const SizedBox(height: 12),
          if (info.notes.isNotEmpty) ...[
            Text(
              I18n.t('update.dialog.notes'),
              style: GoogleFonts.spaceMono(
                fontSize: 10,
                color: kCyan,
                letterSpacing: 1,
              ),
            ),
            const SizedBox(height: 6),
            Container(
              constraints: const BoxConstraints(maxHeight: 140),
              child: SingleChildScrollView(
                child: Text(
                  info.notes,
                  style: GoogleFonts.spaceMono(
                    fontSize: 11,
                    color: kWhite,
                    height: 1.4,
                  ),
                ),
              ),
            ),
            const SizedBox(height: 12),
          ],
          sizeLine,
          const SizedBox(height: 20),
          Row(
            children: [
              Expanded(
                child: GestureDetector(
                  onTap: () => Navigator.of(context).pop(),
                  child: Container(
                    padding: const EdgeInsets.symmetric(vertical: 14),
                    decoration: BoxDecoration(
                      border: Border.all(
                        color: kGrayText.withValues(alpha: 0.5),
                        width: 1.2,
                      ),
                    ),
                    child: Center(
                      child: Text(
                        I18n.t('update.dialog.dismiss'),
                        style: GoogleFonts.orbitron(
                          fontSize: 11,
                          color: kGrayText,
                          letterSpacing: 2,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                flex: 2,
                child: GestureDetector(
                  onTap: _startDownload,
                  child: Container(
                    padding: const EdgeInsets.symmetric(vertical: 14),
                    decoration: BoxDecoration(
                      border: Border.all(color: kCyan, width: 1.5),
                      color: kCyan.withValues(alpha: 0.08),
                      boxShadow: neonGlow(kCyan, radius: 8),
                    ),
                    child: Center(
                      child: Text(
                        I18n.t('update.dialog.download'),
                        style: GoogleFonts.orbitron(
                          fontSize: 11,
                          color: kCyan,
                          letterSpacing: 2,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ];

      case _Phase.downloading:
        final pct = _total > 0 ? (_received / _total).clamp(0.0, 1.0) : null;
        return [
          header,
          const SizedBox(height: 20),
          Text(
            I18n.t('update.downloading'),
            style: GoogleFonts.spaceMono(fontSize: 12, color: kCyan),
          ),
          const SizedBox(height: 12),
          LinearProgressIndicator(
            value: pct,
            color: kCyan,
            backgroundColor: kBgInput,
            minHeight: 4,
          ),
          const SizedBox(height: 8),
          Text(
            '${_fmtBytes(_received)} / ${_fmtBytes(_total)}',
            style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText),
          ),
          const SizedBox(height: 20),
          GestureDetector(
            onTap: () {
              _aborted = true;
              Navigator.of(context).pop();
            },
            child: Container(
              padding: const EdgeInsets.symmetric(vertical: 12),
              width: double.infinity,
              decoration: BoxDecoration(
                border: Border.all(color: kMagenta.withValues(alpha: 0.6)),
              ),
              child: Center(
                child: Text(
                  I18n.t('update.abort'),
                  style: GoogleFonts.orbitron(
                    fontSize: 11,
                    color: kMagenta,
                    letterSpacing: 2,
                  ),
                ),
              ),
            ),
          ),
        ];

      case _Phase.installing:
        return [
          header,
          const SizedBox(height: 20),
          Row(
            children: [
              const SizedBox(
                width: 16,
                height: 16,
                child: CircularProgressIndicator(
                  strokeWidth: 1.5,
                  color: kCyan,
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Text(
                  I18n.t('update.installing'),
                  style: GoogleFonts.spaceMono(fontSize: 12, color: kCyan),
                ),
              ),
            ],
          ),
        ];

      case _Phase.error:
        return [
          header,
          const SizedBox(height: 20),
          Text(
            _errorMessage ?? I18n.t('update.error.download'),
            style: GoogleFonts.spaceMono(fontSize: 12, color: kMagenta),
          ),
          const SizedBox(height: 20),
          GestureDetector(
            onTap: () => Navigator.of(context).pop(),
            child: Container(
              padding: const EdgeInsets.symmetric(vertical: 12),
              width: double.infinity,
              decoration: BoxDecoration(
                border: Border.all(color: kGrayText.withValues(alpha: 0.5)),
              ),
              child: Center(
                child: Text(
                  I18n.t('update.dialog.dismiss'),
                  style: GoogleFonts.orbitron(
                    fontSize: 11,
                    color: kGrayText,
                    letterSpacing: 2,
                  ),
                ),
              ),
            ),
          ),
        ];
    }
  }
}
