import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import '../models/contact.dart';
import '../models/identity.dart';
import '../services/storage_service.dart';
import '../theme.dart';
import '../widgets/glitch_text.dart';
import '../widgets/cyber_card.dart';
import 'chat.dart';
import 'channels.dart';
import 'package:path_provider/path_provider.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  List<PhantomContact> _contacts = [];
  PhantomIdentity? _identity;
  bool _loading = true;

  @override
  void initState() {
    super.initState();
    _load();
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

  void _showAddContact() {
    final ctrl = TextEditingController();
    final nameCtrl = TextEditingController();
    showModalBottomSheet(
      context: context,
      isScrollControlled: true,
      backgroundColor: kBgCard,
      shape: const RoundedRectangleBorder(borderRadius: BorderRadius.zero),
      builder: (ctx) => Padding(
        padding: EdgeInsets.only(
          left: 24, right: 24, top: 24,
          bottom: MediaQuery.of(ctx).viewInsets.bottom + 32,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(height: 1, color: kCyan.withOpacity(0.4)),
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
                    shadows: [Shadow(color: kCyan.withOpacity(0.5), blurRadius: 10)],
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
              decoration: const InputDecoration(hintText: 'phantom:eyJ...'),
            ),
            const SizedBox(height: 24),
            GestureDetector(
              onTap: () async {
                final contact = PhantomContact.fromPhantomId(
                  ctrl.text.trim(),
                  nameCtrl.text.trim().isEmpty ? 'UNKNOWN' : nameCtrl.text.trim().toUpperCase(),
                );
                if (contact == null) {
                  ScaffoldMessenger.of(context).showSnackBar(
                    const SnackBar(content: Text('! INVALID PHANTOM ID')),
                  );
                  return;
                }
                _contacts.add(contact);
                await StorageService.saveContacts(_contacts);
                setState(() {});
                if (ctx.mounted) Navigator.pop(ctx);
              },
              child: Container(
                width: double.infinity,
                padding: const EdgeInsets.symmetric(vertical: 16),
                decoration: BoxDecoration(
                  border: Border.all(color: kCyan, width: 1.5),
                  color: kCyan.withOpacity(0.08),
                  boxShadow: neonGlow(kCyan, radius: 8),
                ),
                child: Center(
                  child: Text(
                    'CONFIRM ADD',
                    style: GoogleFonts.orbitron(fontSize: 12, color: kCyan, letterSpacing: 2),
                  ),
                ),
              ),
            ),
          ],
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
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(height: 1, color: kMagenta.withOpacity(0.5)),
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
                      shadows: [Shadow(color: kMagenta.withOpacity(0.5), blurRadius: 12)],
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
                  color: kMagenta.withOpacity(0.08),
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
    return Scaffold(
      backgroundColor: kBg,
      body: GridBackground(
        child: SafeArea(
          child: Column(
            children: [
              _buildHeader(),
              Container(height: 1, color: kCyan.withOpacity(0.12)),
              Expanded(
                child: _loading
                    ? const Center(child: CircularProgressIndicator(color: kCyan, strokeWidth: 1.5))
                    : _contacts.isEmpty
                        ? _buildEmpty()
                        : _buildList(),
              ),
            ],
          ),
        ),
      ),
      floatingActionButton: _CyberFab(onTap: _showAddContact),
    );
  }

  Widget _buildHeader() {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 14),
      child: Row(
        children: [
          Container(
            width: 36, height: 36,
            decoration: BoxDecoration(
              border: Border.all(color: kCyan.withOpacity(0.6), width: 1),
              color: kCyanDim,
              boxShadow: neonGlow(kCyan, radius: 8),
            ),
            child: const Icon(Icons.shield_outlined, color: kCyan, size: 18),
          ),
          const SizedBox(width: 12),
          Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              GlitchText(
                text: 'PHANTOM',
                interval: const Duration(milliseconds: 200),
                style: GoogleFonts.orbitron(
                  fontSize: 18, fontWeight: FontWeight.w900,
                  color: kWhite, letterSpacing: 3,
                  shadows: [Shadow(color: kCyan.withOpacity(0.5), blurRadius: 8)],
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
                  Text('SECURE · ONLINE', style: GoogleFonts.spaceMono(fontSize: 9, color: kGreen, letterSpacing: 1)),
                ],
              ),
            ],
          ),
          const Spacer(),
          if (_contacts.isNotEmpty)
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              decoration: BoxDecoration(
                border: Border.all(color: kGrayText.withOpacity(0.3)),
                color: kBgCard,
              ),
              child: Text('${_contacts.length} NODES',
                style: GoogleFonts.spaceMono(fontSize: 9, color: kGrayText, letterSpacing: 1)),
            ),
          const SizedBox(width: 8),
          GestureDetector(
            onTap: _openChannels,
            child: Container(
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                border: Border.all(color: kCyan.withOpacity(0.4)),
                color: kBgCard,
              ),
              child: const Icon(Icons.groups_outlined, color: kCyan, size: 20),
            ),
          ),
          const SizedBox(width: 8),
          GestureDetector(
            onTap: _showMyId,
            child: Container(
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                border: Border.all(color: kGrayText.withOpacity(0.3)),
                color: kBgCard,
              ),
              child: const Icon(Icons.fingerprint, color: kGrayText, size: 20),
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
                    style: GoogleFonts.spaceMono(fontSize: 11, color: kGrayText.withOpacity(0.6), height: 1.5),
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
                  border: Border.all(color: kMagenta.withOpacity(0.6), width: 1.5),
                  color: kMagenta.withOpacity(0.06),
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
      itemBuilder: (ctx, i) => _ContactTile(
        contact: _contacts[i],
        onTap: () async {
          await Navigator.push(
            context,
            PageRouteBuilder(
              pageBuilder: (_, __, ___) => ChatScreen(contact: _contacts[i], identity: _identity!),
              transitionDuration: const Duration(milliseconds: 300),
              transitionsBuilder: (_, anim, __, child) => FadeTransition(opacity: anim, child: child),
            ),
          );
          _load();
        },
      ),
    );
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
          borderColor: kCyan.withOpacity(0.2),
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
          child: Row(
            children: [
              Container(
                width: 44, height: 44,
                decoration: BoxDecoration(
                  border: Border.all(color: kCyan.withOpacity(0.4)),
                  color: kCyanDim,
                ),
                child: Center(
                  child: Text(
                    contact.nickname[0],
                    style: GoogleFonts.orbitron(
                      fontSize: 18, fontWeight: FontWeight.w900,
                      color: kCyan,
                      shadows: [Shadow(color: kCyan.withOpacity(0.6), blurRadius: 8)],
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
