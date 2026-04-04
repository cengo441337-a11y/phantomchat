import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import '../models/contact.dart';
import '../models/identity.dart';
import '../services/storage_service.dart';
import '../theme.dart';
import 'chat.dart';

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
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(24)),
      ),
      builder: (ctx) => Padding(
        padding: EdgeInsets.only(
          left: 24, right: 24, top: 24,
          bottom: MediaQuery.of(ctx).viewInsets.bottom + 24,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Center(
              child: Container(
                width: 40,
                height: 4,
                decoration: BoxDecoration(
                  color: kGray,
                  borderRadius: BorderRadius.circular(2),
                ),
              ),
            ),
            const SizedBox(height: 20),
            Text(
              'Kontakt hinzufügen',
              style: GoogleFonts.spaceGrotesk(
                fontSize: 22,
                fontWeight: FontWeight.w700,
                color: kWhite,
              ),
            ),
            const SizedBox(height: 20),
            TextField(
              controller: nameCtrl,
              style: GoogleFonts.spaceGrotesk(color: kWhite),
              decoration: const InputDecoration(
                hintText: 'Spitzname',
                prefixIcon: Icon(Icons.person_outline, color: kNeon, size: 18),
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: ctrl,
              style: GoogleFonts.spaceMono(color: kWhite, fontSize: 12),
              maxLines: 3,
              decoration: const InputDecoration(
                hintText: 'Phantom ID (phantom:...)',
                prefixIcon: Padding(
                  padding: EdgeInsets.only(top: 14),
                  child: Icon(Icons.key_outlined, color: kNeon, size: 18),
                ),
                alignLabelWithHint: true,
              ),
            ),
            const SizedBox(height: 20),
            SizedBox(
              width: double.infinity,
              child: ElevatedButton(
                onPressed: () async {
                  final contact = PhantomContact.fromPhantomId(
                    ctrl.text.trim(),
                    nameCtrl.text.trim().isEmpty ? 'Unbekannt' : nameCtrl.text.trim(),
                  );
                  if (contact == null) {
                    ScaffoldMessenger.of(context).showSnackBar(
                      const SnackBar(
                        content: Text('Ungültige Phantom ID'),
                        backgroundColor: kRed,
                      ),
                    );
                    return;
                  }
                  _contacts.add(contact);
                  await StorageService.saveContacts(_contacts);
                  setState(() {});
                  if (ctx.mounted) Navigator.pop(ctx);
                },
                child: const Text('HINZUFÜGEN'),
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
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(24)),
      ),
      builder: (ctx) => Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 40,
              height: 4,
              decoration: BoxDecoration(
                color: kGray,
                borderRadius: BorderRadius.circular(2),
              ),
            ),
            const SizedBox(height: 24),
            Container(
              width: 64,
              height: 64,
              decoration: BoxDecoration(
                color: kNeonDim,
                shape: BoxShape.circle,
                border: Border.all(color: kNeon),
              ),
              child: const Icon(Icons.shield_outlined, color: kNeon, size: 28),
            ),
            const SizedBox(height: 16),
            Text(
              _identity!.nickname,
              style: GoogleFonts.spaceGrotesk(
                fontSize: 24,
                fontWeight: FontWeight.w700,
                color: kWhite,
              ),
            ),
            const SizedBox(height: 8),
            Text(
              'DEINE PHANTOM ID',
              style: GoogleFonts.spaceGrotesk(
                fontSize: 10,
                fontWeight: FontWeight.w700,
                color: kNeon,
                letterSpacing: 1.5,
              ),
            ),
            const SizedBox(height: 16),
            Container(
              padding: const EdgeInsets.all(16),
              decoration: BoxDecoration(
                color: kBgInput,
                borderRadius: BorderRadius.circular(12),
                border: Border.all(color: kNeonDim),
              ),
              child: SelectableText(
                _identity!.phantomId,
                style: GoogleFonts.spaceMono(
                  fontSize: 10,
                  color: kNeonText,
                  height: 1.6,
                ),
              ),
            ),
            const SizedBox(height: 20),
            SizedBox(
              width: double.infinity,
              child: ElevatedButton.icon(
                onPressed: () {
                  Clipboard.setData(ClipboardData(text: _identity!.phantomId));
                  Navigator.pop(ctx);
                  ScaffoldMessenger.of(context).showSnackBar(
                    const SnackBar(
                      content: Text('Phantom ID kopiert'),
                      backgroundColor: kBgCard,
                    ),
                  );
                },
                icon: const Icon(Icons.copy_outlined, size: 16),
                label: const Text('KOPIEREN'),
              ),
            ),
            const SizedBox(height: 12),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      appBar: AppBar(
        backgroundColor: kBg,
        title: Row(
          children: [
            Container(
              width: 32,
              height: 32,
              decoration: BoxDecoration(
                color: kNeonDim,
                shape: BoxShape.circle,
                border: Border.all(color: kNeon, width: 1),
              ),
              child: const Icon(Icons.shield_outlined, color: kNeon, size: 16),
            ),
            const SizedBox(width: 10),
            Text(
              'PHANTOM',
              style: GoogleFonts.spaceGrotesk(
                fontSize: 18,
                fontWeight: FontWeight.w800,
                color: kWhite,
                letterSpacing: -0.5,
              ),
            ),
          ],
        ),
        actions: [
          IconButton(
            icon: const Icon(Icons.qr_code_outlined, color: kWhiteDim),
            onPressed: _showMyId,
            tooltip: 'Meine Phantom ID',
          ),
        ],
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(1),
          child: Container(height: 1, color: const Color(0xFF1A2030)),
        ),
      ),
      body: _loading
          ? const Center(child: CircularProgressIndicator(color: kNeon, strokeWidth: 2))
          : _contacts.isEmpty
              ? _buildEmpty()
              : _buildList(),
      floatingActionButton: FloatingActionButton(
        onPressed: _showAddContact,
        backgroundColor: kNeon,
        foregroundColor: kBg,
        child: const Icon(Icons.add),
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
            Container(
              width: 80,
              height: 80,
              decoration: BoxDecoration(
                color: kBgCard,
                shape: BoxShape.circle,
                border: Border.all(color: const Color(0xFF1E2733)),
              ),
              child: const Icon(Icons.person_add_outlined, color: kGray, size: 32),
            ),
            const SizedBox(height: 24),
            Text(
              'Noch keine Kontakte',
              style: GoogleFonts.spaceGrotesk(
                fontSize: 20,
                fontWeight: FontWeight.w700,
                color: kWhite,
              ),
            ),
            const SizedBox(height: 8),
            Text(
              'Teile deine Phantom ID und\nfüge Kontakte per ID hinzu.',
              textAlign: TextAlign.center,
              style: GoogleFonts.spaceGrotesk(
                fontSize: 14,
                color: kGray,
                height: 1.5,
              ),
            ),
            const SizedBox(height: 32),
            OutlinedButton.icon(
              onPressed: _showMyId,
              style: OutlinedButton.styleFrom(
                side: const BorderSide(color: kNeon),
                foregroundColor: kNeon,
                padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 12),
              ),
              icon: const Icon(Icons.share_outlined, size: 16),
              label: const Text('MEINE ID TEILEN'),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildList() {
    return ListView.builder(
      padding: const EdgeInsets.symmetric(vertical: 8),
      itemCount: _contacts.length,
      itemBuilder: (ctx, i) {
        final contact = _contacts[i];
        return _ContactTile(
          contact: contact,
          onTap: () async {
            await Navigator.push(
              context,
              MaterialPageRoute(
                builder: (_) => ChatScreen(
                  contact: contact,
                  identity: _identity!,
                ),
              ),
            );
            _load();
          },
          onDelete: () async {
            _contacts.removeAt(i);
            await StorageService.saveContacts(_contacts);
            setState(() {});
          },
        );
      },
    );
  }
}

class _ContactTile extends StatelessWidget {
  final PhantomContact contact;
  final VoidCallback onTap;
  final VoidCallback onDelete;

  const _ContactTile({
    required this.contact,
    required this.onTap,
    required this.onDelete,
  });

  @override
  Widget build(BuildContext context) {
    final initials = contact.nickname.isNotEmpty
        ? contact.nickname[0].toUpperCase()
        : '?';

    return InkWell(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
        decoration: const BoxDecoration(
          border: Border(bottom: BorderSide(color: Color(0xFF0F1520))),
        ),
        child: Row(
          children: [
            Container(
              width: 48,
              height: 48,
              decoration: BoxDecoration(
                color: kBgCard,
                shape: BoxShape.circle,
                border: Border.all(color: const Color(0xFF1E2733)),
              ),
              child: Center(
                child: Text(
                  initials,
                  style: GoogleFonts.spaceGrotesk(
                    fontSize: 18,
                    fontWeight: FontWeight.w700,
                    color: kNeonText,
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
                    contact.nickname,
                    style: GoogleFonts.spaceGrotesk(
                      fontSize: 16,
                      fontWeight: FontWeight.w600,
                      color: kWhite,
                    ),
                  ),
                  const SizedBox(height: 2),
                  Text(
                    contact.lastMessage ?? 'Noch keine Nachrichten',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: GoogleFonts.spaceGrotesk(
                      fontSize: 13,
                      color: kGray,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 8),
            Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                if (contact.lastMessageAt != null)
                  Text(
                    _formatTime(contact.lastMessageAt!),
                    style: GoogleFonts.spaceGrotesk(fontSize: 11, color: kGray),
                  ),
                const SizedBox(height: 4),
                const Icon(Icons.lock_outline, size: 12, color: kGray),
              ],
            ),
          ],
        ),
      ),
    );
  }

  String _formatTime(DateTime dt) {
    final now = DateTime.now();
    final diff = now.difference(dt);
    if (diff.inMinutes < 60) return '${diff.inMinutes}m';
    if (diff.inHours < 24) return '${diff.inHours}h';
    return '${dt.day}.${dt.month}';
  }
}
