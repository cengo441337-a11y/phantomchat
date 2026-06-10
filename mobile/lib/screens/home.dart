import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import '../models/contact.dart';
import '../models/identity.dart';
import '../services/storage_service.dart';
import '../theme.dart';
import '../widgets/glitch_text.dart';
import '../widgets/cyber_card.dart';
import '../services/update_service.dart';
import '../widgets/update_banner.dart';
import '../widgets/update_dialog.dart';
import 'chat.dart';
import 'qr_scan.dart';
import 'settings.dart';
import 'channels.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:path_provider/path_provider.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> with WidgetsBindingObserver {
  List<PhantomContact> _contacts = [];
  PhantomIdentity? _identity;
  bool _loading = true;

  /// Update available (set by the auto-check on boot). Drives the badge on
  /// the header's update icon: tapping the icon either opens the install
  /// dialog (when `_update != null`) or runs a manual check and shows a
  /// SnackBar (when null). Users repeatedly reported they couldn't find
  /// the update button when it lived in Settings — putting it directly
  /// in the header makes it impossible to miss.
  UpdateInfo? _update;
  bool _checkingUpdate = false;

  /// Installed semver, shown next to "SECURE · ONLINE" in the header so the
  /// user can always confirm which build they are running without diving
  /// into Settings → ÜBER.
  String _appVersion = '';

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _load();
    _silentUpdateCheck();
  }

  Future<void> _load() async {
    final identity = await StorageService.loadIdentity();
    final contacts = await StorageService.loadContacts();
    setState(() {
      _identity = identity;
      _contacts = contacts;
      _loading = false;
    });
  }

  /// Boot-time background update probe — drives the badge on the header's
  /// update icon without showing any UI. Failure → no badge (the manual
  /// check via the icon still works). Also pulls the installed semver so
  /// the header subtitle can show it next to "SECURE · ONLINE".
  Future<void> _silentUpdateCheck() async {
    try {
      final pkg = await PackageInfo.fromPlatform();
      if (mounted) setState(() => _appVersion = pkg.version);
    } catch (_) {/* silent */}
    try {
      final info = await UpdateService.checkForUpdate();
      if (!mounted) return;
      setState(() => _update = info);
    } catch (_) {
      /* silent */
    }
  }

  /// Header update-icon handler: if an update is known, open the install
  /// dialog directly; otherwise run a fresh manual check with visible
  /// feedback so the user knows the button actually did something.
  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // On resume we kick a fresh background-check so the home-screen banner
    // can switch from "installed v1.2.X" to "update available v1.2.Y"
    // without waiting for the user to tap. Also persists the outcome via
    // UpdateStateStore so the next cold-start can render an immediate
    // status without a network round-trip.
    if (state == AppLifecycleState.resumed) {
      unawaited(_runBackgroundUpdateCheck());
    }
  }

  Future<void> _runBackgroundUpdateCheck() async {
    if (_checkingUpdate) return;
    UpdateInfo? info;
    try {
      // Use the rich variant so persistence happens as a side effect.
      final result = await UpdateService.backgroundCheck();
      if (result.lastOutcome == 'updateAvailable') {
        // Fetch the full info struct so the home banner can show notes.
        info = await UpdateService.checkForUpdate();
      }
    } catch (_) {
      info = null;
    }
    if (!mounted) return;
    setState(() {
      _update = info;
    });
  }

    Future<void> _onUpdateTap() async {
    if (_update != null) {
      await showDialog<void>(
        context: context,
        barrierDismissible: false,
        builder: (_) => UpdateDialog(info: _update!),
      );
      return;
    }
    if (_checkingUpdate) return;
    setState(() => _checkingUpdate = true);
    UpdateCheckResult? result;
    try {
      result = await UpdateService.checkForUpdateRich();
    } catch (_) {
      result = null;
    }
    if (!mounted) return;
    setState(() {
      _checkingUpdate = false;
      _update = result?.info;
    });
    if (result == null) {
      _showUpdateSnack(
        'Update-Server nicht erreichbar. Bitte spaeter erneut versuchen.',
        kMagenta,
        Icons.cloud_off_outlined,
      );
      return;
    }
    switch (result.outcome) {
      case UpdateCheckOutcome.updateAvailable:
        if (result.info != null) {
          await showDialog<void>(
            context: context,
            barrierDismissible: false,
            builder: (_) => UpdateDialog(info: result!.info!),
          );
        }
        break;
      case UpdateCheckOutcome.alreadyLatest:
        final manifest = result.manifestVersion ?? '?';
        final code = result.manifestCode;
        _showUpdateSnack(
          'Aktuell auf v${result.installedVersion}'
              ' (Manifest v$manifest${code != null ? ' / build $code' : ''}).',
          kGreen,
          Icons.check_circle_outline,
        );
        break;
      case UpdateCheckOutcome.manifestUnreachable:
        _showUpdateSnack(
          'Update-Server antwortet nicht oder Manifest ungueltig. '
              'Installiert: v${result.installedVersion}.',
          kMagenta,
          Icons.cloud_off_outlined,
        );
        break;
    }
  }

  void _showUpdateSnack(String msg, Color color, IconData icon) {
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Row(
          children: [
            Icon(icon, color: color, size: 18),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                msg,
                style: GoogleFonts.spaceMono(fontSize: 12, color: kWhite),
              ),
            ),
          ],
        ),
        backgroundColor: kBgCard,
        duration: const Duration(seconds: 5),
      ),
    );
  }

  void _showAddContact() {
    final ctrl = TextEditingController();
    final nameCtrl = TextEditingController();
    String? errorMsg; // sheet-local — surfaced inline so the user actually sees it
    bool busy = false;
    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      backgroundColor: kBgCard,
      shape: const RoundedRectangleBorder(borderRadius: BorderRadius.zero),
      builder: (ctx) => StatefulBuilder(
        builder: (ctx, setSheetState) => Padding(
          padding: EdgeInsets.only(
            left: 24, right: 24, top: 24,
            bottom: (MediaQuery.of(ctx).viewInsets.bottom > 0
                    ? MediaQuery.of(ctx).viewInsets.bottom
                    : MediaQuery.of(ctx).viewPadding.bottom) +
                32,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Container(height: 1, color: kCyan.withValues(alpha: 0.4)),
              const SizedBox(height: 20),
              Row(
                children: [
                  Container(width: 3, height: 22, color: kCyan),
                  const SizedBox(width: 12),
                  Text(
                    'ADD_CONTACT //',
                    style: GoogleFonts.orbitron(
                      fontSize: 16, fontWeight: FontWeight.w700,
                      color: kWhite, letterSpacing: 1.5,
                      shadows: [Shadow(color: kCyan.withValues(alpha: 0.5), blurRadius: 10)],
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 24),
              Text('NICKNAME:', style: GoogleFonts.spaceMono(fontSize: 10, color: kCyan, letterSpacing: 1)),
              const SizedBox(height: 8),
              TextField(
                controller: nameCtrl,
                style: GoogleFonts.spaceMono(color: kWhite, fontSize: 14),
                decoration: const InputDecoration(hintText: 'ghost · cipher · zero'),
              ),
              const SizedBox(height: 16),
              Text('PHANTOM_ID:', style: GoogleFonts.spaceMono(fontSize: 10, color: kCyan, letterSpacing: 1)),
              const SizedBox(height: 8),
              TextField(
                controller: ctrl,
                style: GoogleFonts.spaceMono(color: kCyan, fontSize: 11),
                maxLines: 3,
                decoration: const InputDecoration(hintText: 'phantom:abc123...:def456...'),
              ),
              if (errorMsg != null) ...[
                const SizedBox(height: 8),
                Text(
                  '! $errorMsg',
                  style: GoogleFonts.spaceMono(
                    fontSize: 11,
                    color: kMagenta,
                    letterSpacing: 1,
                  ),
                ),
              ],
              const SizedBox(height: 12),
              // QR-scan shortcut — populates the PHANTOM_ID field above with
              // whatever the camera reads, so the rest of the flow (validation,
              // CONFIRM ADD) is identical for typed and scanned IDs.
              GestureDetector(
                onTap: () async {
                  final scanned = await Navigator.of(ctx).push<String>(
                    MaterialPageRoute(builder: (_) => const QrScanScreen()),
                  );
                  if (scanned != null && scanned.isNotEmpty) {
                    ctrl.text = scanned.trim();
                    setSheetState(() => errorMsg = null);
                  }
                },
                child: Container(
                  width: double.infinity,
                  padding: const EdgeInsets.symmetric(vertical: 12),
                  decoration: BoxDecoration(
                    border: Border.all(color: kMagenta.withValues(alpha: 0.6), width: 1.2),
                    color: kMagenta.withValues(alpha: 0.06),
                  ),
                  child: Center(
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        const Icon(Icons.qr_code_scanner, color: kMagenta, size: 18),
                        const SizedBox(width: 10),
                        Text(
                          'SCAN QR',
                          style: GoogleFonts.orbitron(fontSize: 11, color: kMagenta, letterSpacing: 2),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
              const SizedBox(height: 16),
              GestureDetector(
                onTap: busy ? null : () async {
                  final raw = ctrl.text.trim();
                  if (raw.isEmpty) {
                    setSheetState(() => errorMsg = 'Phantom-ID darf nicht leer sein');
                    return;
                  }
                  final contact = PhantomContact.fromPhantomId(
                    raw,
                    nameCtrl.text.trim().isEmpty ? 'UNKNOWN' : nameCtrl.text.trim().toUpperCase(),
                  );
                  if (contact == null) {
                    setSheetState(() => errorMsg =
                        'Ungültiges Format. Erwartet: phantom:<view_hex>:<spend_hex>');
                    return;
                  }
                  if (_contacts.any((c) => c.publicSpendKey == contact.publicSpendKey)) {
                    setSheetState(() => errorMsg = 'Kontakt mit dieser Spend-Key existiert bereits');
                    return;
                  }
                  setSheetState(() {
                    busy = true;
                    errorMsg = null;
                  });
                  try {
                    _contacts.add(contact);
                    await StorageService.saveContacts(_contacts);
                  } catch (e) {
                    setSheetState(() {
                      busy = false;
                      errorMsg = 'Speichern fehlgeschlagen: $e';
                    });
                    _contacts.removeLast();
                    return;
                  }
                  if (!mounted) return;
                  setState(() {});
                  if (ctx.mounted) Navigator.pop(ctx);
                },
                child: Container(
                  width: double.infinity,
                  padding: const EdgeInsets.symmetric(vertical: 16),
                  decoration: BoxDecoration(
                    border: Border.all(color: busy ? kGrayText : kCyan, width: 1.5),
                    color: kCyan.withValues(alpha: 0.08),
                    boxShadow: busy ? null : neonGlow(kCyan, radius: 8),
                  ),
                  child: Center(
                    child: Text(
                      busy ? 'SAVING…' : 'CONFIRM ADD',
                      style: GoogleFonts.orbitron(
                        fontSize: 12,
                        color: busy ? kGrayText : kCyan,
                        letterSpacing: 2,
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  void _showMyId() {
    if (_identity == null) return;
    showModalBottomSheet(
      context: context,
      backgroundColor: kBgCard,
      shape: const RoundedRectangleBorder(borderRadius: BorderRadius.zero),
      builder: (ctx) => Padding(
        padding: EdgeInsets.fromLTRB(
            24, 24, 24, 24 + MediaQuery.of(ctx).viewPadding.bottom),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(height: 1, color: kMagenta.withValues(alpha: 0.5)),
            const SizedBox(height: 20),
            Row(
              children: [
                Container(width: 3, height: 22, color: kMagenta),
                const SizedBox(width: 12),
                Text(
                  'MY_PHANTOM_ID //',
                  style: GoogleFonts.orbitron(
                    fontSize: 14, fontWeight: FontWeight.w700,
                    color: kWhite, letterSpacing: 1.5,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 20),
            CyberCard(
              borderColor: kMagenta,
              glow: true,
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    _identity!.nickname.toUpperCase(),
                    style: GoogleFonts.orbitron(
                      fontSize: 20, fontWeight: FontWeight.w900,
                      color: kWhite, letterSpacing: 2,
                      shadows: [Shadow(color: kMagenta.withValues(alpha: 0.5), blurRadius: 12)],
                    ),
                  ),
                  const SizedBox(height: 12),
                  SelectableText(
                    _identity!.phantomId,
                    style: GoogleFonts.spaceMono(fontSize: 10, color: kMagenta, height: 1.6),
                  ),
                ],
              ),
            ),
            const SizedBox(height: 16),
            GestureDetector(
              onTap: () {
                Clipboard.setData(ClipboardData(text: _identity!.phantomId));
                Navigator.pop(ctx);
                ScaffoldMessenger.of(context).showSnackBar(
                  const SnackBar(content: Text('> PHANTOM ID COPIED TO CLIPBOARD')),
                );
              },
              child: Container(
                width: double.infinity,
                padding: const EdgeInsets.symmetric(vertical: 14),
                decoration: BoxDecoration(
                  border: Border.all(color: kMagenta, width: 1.5),
                  color: kMagenta.withValues(alpha: 0.08),
                ),
                child: Center(
                  child: Text(
                    'COPY TO CLIPBOARD',
                    style: GoogleFonts.orbitron(fontSize: 12, color: kMagenta, letterSpacing: 2),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    // System UI insets — the gesture/navigation bar at the bottom + the
    // status bar at the top. SafeArea(bottom: false) so we can extend the
    // grid all the way down (pretty), then add the inset back as bottom
    // padding inside Expanded so contacts and the FAB are never hidden by
    // the gesture bar. Reported regression: bottom nav bar was overlapping
    // UI on real phones.
    final bottomInset = MediaQuery.of(context).viewPadding.bottom;
    return Scaffold(
      backgroundColor: kBg,
      body: GridBackground(
        child: SafeArea(
          bottom: false,
          // Wave 11G — wrap the home body in `UpdateBanner` so a yellow
          // "update available" strip shows above the contact list when
          // a newer APK has been published. The banner is invisible
          // when no update is pending, so existing layout is preserved.
          child: UpdateBanner(
            child: Column(
              children: [
                _buildHeader(),
                Container(height: 1, color: kCyan.withValues(alpha: 0.12)),
                Expanded(
                  child: Padding(
                    // Reserve room for the gesture bar PLUS the FAB so the
                    // last contact entry can be scrolled to without being
                    // hidden behind it.
                    padding: EdgeInsets.only(bottom: bottomInset + 80),
                    child: _loading
                        ? const Center(child: CircularProgressIndicator(color: kCyan, strokeWidth: 1.5))
                        : _contacts.isEmpty
                            ? _buildEmpty()
                            : _buildList(),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
      floatingActionButtonLocation: FloatingActionButtonLocation.endFloat,
      floatingActionButton: Padding(
        // Lift the FAB above the gesture bar.
        padding: EdgeInsets.only(bottom: bottomInset),
        child: _CyberFab(onTap: _showAddContact),
      ),
    );
  }

  Widget _buildHeader() {
    return Padding(
      // Tighter horizontal padding so all five icons + title fit on
      // narrower phones without the title wrapping to two lines.
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 14),
      child: Row(
        children: [
          Container(
            width: 36, height: 36,
            decoration: BoxDecoration(
              border: Border.all(color: kCyan.withValues(alpha: 0.6), width: 1),
              color: kCyanDim,
              boxShadow: neonGlow(kCyan, radius: 8),
            ),
            child: const Icon(Icons.shield_outlined, color: kCyan, size: 18),
          ),
          const SizedBox(width: 10),
          // Wrap the title block in Expanded so on narrower devices it
          // shrinks instead of pushing the trailing icon row off-screen.
          // Without this the header overflows ~20 px on a 392 dp viewport
          // (Pixel 4-class) once all four trailing buttons are present.
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                GlitchText(
                  text: 'ARGOS',
                  interval: const Duration(milliseconds: 200),
                  style: GoogleFonts.orbitron(
                    fontSize: 18, fontWeight: FontWeight.w900,
                    color: kWhite, letterSpacing: 3,
                    shadows: [Shadow(color: kCyan.withValues(alpha: 0.5), blurRadius: 8)],
                  ),
                ),
                Row(
                  children: [
                    Container(
                      width: 6, height: 6,
                      decoration: const BoxDecoration(
                        color: kGreen, shape: BoxShape.circle,
                        boxShadow: [BoxShadow(color: kGreen, blurRadius: 6)],
                      ),
                    ),
                    const SizedBox(width: 5),
                    Flexible(
                      child: Text(
                        // Persistent version label so users can always see
                        // which build they are running without digging into
                        // Settings. Cyan separator + grey version keeps the
                        // green "online" pill the visual anchor.
                        _appVersion.isEmpty
                            ? 'SECURE · ONLINE'
                            : 'SECURE · ONLINE · v$_appVersion',
                        style: GoogleFonts.spaceMono(fontSize: 9, color: kGreen, letterSpacing: 1),
                        overflow: TextOverflow.fade,
                        softWrap: false,
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
          // NODES badge only on screens wide enough to absorb it without
          // pushing the title to two lines (post-v1.1.9, with the update
          // icon added we have 4 trailing icons; 360 dp class phones can't
          // also fit the badge).
          if (_contacts.isNotEmpty && MediaQuery.of(context).size.width > 400) ...[
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
              decoration: BoxDecoration(
                border: Border.all(color: kGrayText.withValues(alpha: 0.3)),
                color: kBgCard,
              ),
              child: Text('${_contacts.length}',
                style: GoogleFonts.spaceMono(fontSize: 10, color: kGrayText, letterSpacing: 1)),
            ),
            const SizedBox(width: 4),
          ],
          GestureDetector(
            onTap: _openChannels,
            child: Container(
              padding: const EdgeInsets.all(6),
              decoration: BoxDecoration(
                border: Border.all(color: kCyan.withValues(alpha: 0.4)),
                color: kBgCard,
              ),
              child: const Icon(Icons.groups_outlined, color: kCyan, size: 18),
            ),
          ),
          const SizedBox(width: 4),
          GestureDetector(
            onTap: _showMyId,
            child: Container(
              padding: const EdgeInsets.all(6),
              decoration: BoxDecoration(
                border: Border.all(color: kGrayText.withValues(alpha: 0.3)),
                color: kBgCard,
              ),
              child: const Icon(Icons.fingerprint, color: kGrayText, size: 18),
            ),
          ),
          const SizedBox(width: 4),
          // Update icon — green/badged when an update is pending so users
          // can't miss it. Tapping with no update available runs a fresh
          // manual check + SnackBar feedback. Reported regression: when
          // the only update entry point lived in Settings, users couldn't
          // find it; this header slot is the fix.
          GestureDetector(
            onTap: _onUpdateTap,
            child: Stack(
              clipBehavior: Clip.none,
              children: [
                Container(
                  padding: const EdgeInsets.all(6),
                  decoration: BoxDecoration(
                    border: Border.all(
                      color: _update != null
                          ? kGreen
                          : kGrayText.withValues(alpha: 0.3),
                      width: _update != null ? 1.5 : 1,
                    ),
                    color: _update != null
                        ? kGreen.withValues(alpha: 0.08)
                        : kBgCard,
                    boxShadow: _update != null
                        ? [BoxShadow(color: kGreen.withValues(alpha: 0.5), blurRadius: 8)]
                        : null,
                  ),
                  child: _checkingUpdate
                      ? const SizedBox(
                          width: 18, height: 18,
                          child: CircularProgressIndicator(color: kGreen, strokeWidth: 1.5),
                        )
                      : Icon(
                          Icons.system_update_alt,
                          color: _update != null ? kGreen : kGrayText,
                          size: 18,
                        ),
                ),
                if (_update != null)
                  Positioned(
                    right: -3, top: -3,
                    child: Container(
                      width: 10, height: 10,
                      decoration: const BoxDecoration(
                        color: kGreen, shape: BoxShape.circle,
                        boxShadow: [BoxShadow(color: kGreen, blurRadius: 6)],
                      ),
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(width: 4),
          GestureDetector(
            onTap: () {
              Navigator.of(context).push(
                MaterialPageRoute(builder: (_) => const SettingsScreen()),
              );
            },
            child: Container(
              padding: const EdgeInsets.all(6),
              decoration: BoxDecoration(
                border: Border.all(color: kGrayText.withValues(alpha: 0.3)),
                color: kBgCard,
              ),
              child: const Icon(Icons.settings_outlined, color: kGrayText, size: 18),
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _openChannels() async {
    final dir = await getApplicationSupportDirectory();
    final mlsDir = '${dir.path}/mls';
    final selfLabel = (_identity?.nickname.isNotEmpty ?? false)
        ? _identity!.nickname
        : 'phantom';
    if (!mounted) return;
    await Navigator.push(
      context,
      MaterialPageRoute(
        builder: (_) =>
            ChannelsScreen(storageDir: mlsDir, selfLabel: selfLabel),
      ),
    );
  }

  Widget _buildEmpty() {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(40),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            CyberCard(
              borderColor: kGray,
              padding: const EdgeInsets.all(28),
              cut: 20,
              child: Column(
                children: [
                  const Icon(Icons.wifi_tethering_off, color: kGrayText, size: 40),
                  const SizedBox(height: 16),
                  Text(
                    'NO NODES\nCONNECTED',
                    textAlign: TextAlign.center,
                    style: GoogleFonts.orbitron(
                      fontSize: 16, fontWeight: FontWeight.w700,
                      color: kGrayText, letterSpacing: 2, height: 1.3,
                    ),
                  ),
                  const SizedBox(height: 12),
                  Text(
                    'Share your Phantom ID and\nadd contacts by theirs.',
                    textAlign: TextAlign.center,
                    style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText.withValues(alpha: 0.6), height: 1.5),
                  ),
                ],
              ),
            ),
            const SizedBox(height: 24),
            GestureDetector(
              onTap: _showMyId,
              child: Container(
                padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
                decoration: BoxDecoration(
                  border: Border.all(color: kMagenta.withValues(alpha: 0.6), width: 1.5),
                  color: kMagenta.withValues(alpha: 0.06),
                ),
                child: Text('BROADCAST MY ID',
                  style: GoogleFonts.orbitron(fontSize: 11, color: kMagenta, letterSpacing: 2)),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildList() {
    return ListView.builder(
      padding: const EdgeInsets.only(top: 4, bottom: 80),
      itemCount: _contacts.length,
      itemBuilder: (ctx, i) {
        final c = _contacts[i];
        // Each row is wrapped in a Dismissible so a left-swipe reveals
        // a "delete" affordance — same gesture every native messenger
        // uses, no extra long-press/tap-and-hold modal needed.
        // `confirmDismiss` shows an AlertDialog so a stray swipe can't
        // wipe a contact silently. After confirmation we remove from
        // both the in-memory list AND `contacts.json`; conversation
        // history is left intact (a future "wipe history" toggle can
        // chain off this if needed).
        return Dismissible(
          key: ValueKey('contact-${c.id}'),
          direction: DismissDirection.endToStart,
          background: Container(
            color: kMagenta.withValues(alpha: 0.18),
            alignment: Alignment.centerRight,
            padding: const EdgeInsets.symmetric(horizontal: 28),
            child: Icon(Icons.delete_outline, color: kMagenta),
          ),
          confirmDismiss: (_) => _confirmDeleteContact(c),
          onDismissed: (_) => _deleteContact(c),
          child: _ContactTile(
            contact: c,
            onTap: () async {
              await Navigator.push(
                context,
                PageRouteBuilder(
                  pageBuilder: (_, _, _) => ChatScreen(contact: c, identity: _identity!),
                  transitionDuration: const Duration(milliseconds: 300),
                  transitionsBuilder: (_, anim, _, child) => FadeTransition(opacity: anim, child: child),
                ),
              );
              _load();
            },
          ),
        );
      },
    );
  }

  /// Native confirm dialog before destroying a contact entry. Returns
  /// the boolean `Dismissible.confirmDismiss` expects.
  Future<bool> _confirmDeleteContact(PhantomContact c) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: kBgCard,
        title: Text(
          'Kontakt löschen?',
          style: TextStyle(color: kMagenta, fontFamily: 'monospace', letterSpacing: 1),
        ),
        content: Text(
          'Kontakt "${c.nickname}" wird endgültig aus deiner Liste entfernt. '
          'Verlauf bleibt erhalten — der Eintrag kann nur durch erneutes '
          'Hinzufügen wiederhergestellt werden.',
          style: TextStyle(color: kWhite),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx, false),
            child: Text('ABBRECHEN', style: TextStyle(color: kGrayText)),
          ),
          TextButton(
            onPressed: () => Navigator.pop(ctx, true),
            child: Text('LÖSCHEN', style: TextStyle(color: kMagenta)),
          ),
        ],
      ),
    );
    return ok ?? false;
  }

  Future<void> _deleteContact(PhantomContact c) async {
    final removed = c;
    setState(() {
      _contacts.removeWhere((x) => x.id == c.id);
    });
    try {
      await StorageService.saveContacts(_contacts);
    } catch (e) {
      // Persist failure: revert in-memory state so the row pops back.
      if (!mounted) return;
      setState(() => _contacts.add(removed));
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Löschen fehlgeschlagen: $e')),
      );
    }
  }
}

class _ContactTile extends StatelessWidget {
  final PhantomContact contact;
  final VoidCallback onTap;
  const _ContactTile({required this.contact, required this.onTap});

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
        child: CyberCard(
          borderColor: kCyan.withValues(alpha: 0.2),
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
          child: Row(
            children: [
              Container(
                width: 44, height: 44,
                decoration: BoxDecoration(
                  border: Border.all(color: kCyan.withValues(alpha: 0.4)),
                  color: kCyanDim,
                ),
                child: Center(
                  child: Text(
                    contact.nickname[0],
                    style: GoogleFonts.orbitron(
                      fontSize: 18, fontWeight: FontWeight.w900,
                      color: kCyan,
                      shadows: [Shadow(color: kCyan.withValues(alpha: 0.6), blurRadius: 8)],
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 14),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      contact.nickname.toUpperCase(),
                      style: GoogleFonts.orbitron(fontSize: 13, fontWeight: FontWeight.w700, color: kWhite, letterSpacing: 1),
                    ),
                    const SizedBox(height: 3),
                    Text(
                      contact.lastMessage ?? '// NO MESSAGES YET',
                      maxLines: 1, overflow: TextOverflow.ellipsis,
                      style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText),
                    ),
                  ],
                ),
              ),
              Column(
                crossAxisAlignment: CrossAxisAlignment.end,
                children: [
                  if (contact.lastMessageAt != null)
                    Text(_fmt(contact.lastMessageAt!),
                      style: GoogleFonts.spaceMono(fontSize: 10, color: kGrayText)),
                  const SizedBox(height: 4),
                  const Icon(Icons.lock_outline, size: 11, color: kGreen),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }

  String _fmt(DateTime dt) {
    final d = DateTime.now().difference(dt);
    if (d.inMinutes < 60) return '${d.inMinutes}m';
    if (d.inHours < 24) return '${d.inHours}h';
    return '${dt.day}.${dt.month}';
  }
}

class _CyberFab extends StatelessWidget {
  final VoidCallback onTap;
  const _CyberFab({required this.onTap});

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: Container(
        width: 56, height: 56,
        decoration: BoxDecoration(
          border: Border.all(color: kCyan, width: 1.5),
          color: kCyanDim,
          boxShadow: neonGlow(kCyan, radius: 12),
        ),
        child: const Icon(Icons.add, color: kCyan, size: 24),
      ),
    );
  }
}
