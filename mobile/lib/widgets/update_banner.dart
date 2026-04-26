// Wave 11G — top-of-home banner that surfaces when a newer APK is
// available. Polls `UpdateService.checkForUpdate()` once on mount; if it
// resolves to non-null, slots a single-line cyber-themed banner above
// whatever child is below us. Tap → opens `UpdateDialog`.
//
// Constraints (from the wave-spec):
// - The check runs ONCE per launch, not on a timer. Recurring polls would
//   waste battery and the user is going to relaunch the app eventually.
// - Failure (offline / 404 / malformed JSON) just hides the banner —
//   never crash the boot path. `UpdateService.checkForUpdate()` already
//   swallows everything, so we just react to a `null` result.
// - The banner is dismissible per-process (X button); state lives in
//   memory, so it reappears on next launch if the user ignored it.

import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

import '../services/i18n.dart';
import '../services/update_service.dart';
import '../theme.dart';
import 'update_dialog.dart';

class UpdateBanner extends StatefulWidget {
  final Widget child;

  const UpdateBanner({super.key, required this.child});

  @override
  State<UpdateBanner> createState() => _UpdateBannerState();
}

class _UpdateBannerState extends State<UpdateBanner> {
  UpdateInfo? _info;
  bool _dismissed = false;

  @override
  void initState() {
    super.initState();
    _check();
  }

  Future<void> _check() async {
    // Defer briefly so we don't compete with the rest of boot. The home
    // screen's _load() also runs on first frame; by waiting one frame we
    // give it a clean shot at the disk + UI.
    await Future<void>.delayed(const Duration(milliseconds: 500));
    if (!mounted) return;
    final info = await UpdateService.checkForUpdate();
    if (!mounted) return;
    setState(() => _info = info);
  }

  void _openDialog() {
    final info = _info;
    if (info == null) return;
    showDialog<void>(
      context: context,
      barrierDismissible: true,
      builder: (_) => UpdateDialog(info: info),
    );
  }

  @override
  Widget build(BuildContext context) {
    final info = _info;
    if (info == null || _dismissed) return widget.child;

    return Column(
      children: [
        Material(
          color: Colors.transparent,
          child: InkWell(
            onTap: _openDialog,
            child: Container(
              width: double.infinity,
              decoration: BoxDecoration(
                color: kYellow.withValues(alpha: 0.10),
                border: Border(
                  bottom: BorderSide(
                    color: kYellow.withValues(alpha: 0.6),
                    width: 1,
                  ),
                ),
              ),
              padding: const EdgeInsets.fromLTRB(14, 8, 8, 8),
              child: Row(
                children: [
                  const Icon(
                    Icons.system_update_alt,
                    color: kYellow,
                    size: 16,
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      I18n.tf('update.banner', {'version': info.newVersion}),
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: GoogleFonts.orbitron(
                        fontSize: 10,
                        color: kYellow,
                        letterSpacing: 1.2,
                        shadows: [
                          Shadow(
                            color: kYellow.withValues(alpha: 0.4),
                            blurRadius: 6,
                          ),
                        ],
                      ),
                    ),
                  ),
                  IconButton(
                    onPressed: () => setState(() => _dismissed = true),
                    icon: const Icon(Icons.close, color: kYellow, size: 16),
                    padding: EdgeInsets.zero,
                    constraints: const BoxConstraints(
                      minWidth: 28,
                      minHeight: 28,
                    ),
                    tooltip: I18n.t('update.dialog.dismiss'),
                  ),
                ],
              ),
            ),
          ),
        ),
        Expanded(child: widget.child),
      ],
    );
  }
}
