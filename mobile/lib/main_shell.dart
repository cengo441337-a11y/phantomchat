import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

import 'screens/home.dart';
import 'screens/settings.dart';
import 'screens/wallet_screen.dart';
import 'theme.dart';

/// Root navigation shell for Argos.
///
/// Bottom-navigation with three first-class tabs:
///
/// 1. **Wallet** (default) — Argos Solana wallet: balance, send, receive,
///    swap, auto-swap-on-send. This is what new users see first; it
///    reflects that Argos is positioned as a wallet-first messenger,
///    not a messenger-with-wallet-bolted-on.
///
/// 2. **Chats** — the existing PhantomChat home screen (sealed-sender
///    1:1 + MLS group chat). Renamed visually but the underlying
///    Ratchet/MLS/Stealth-Address code is unchanged.
///
/// 3. **Settings** — relay, security, update-status, about.
///
/// State preserved across tab switches via [IndexedStack]. Per-tab state
/// (e.g. wallet unlock state, scroll position in chats) survives without
/// rebuilding the tab content from scratch.
class MainShell extends StatefulWidget {
  /// Which tab to open first. Defaults to the Wallet (index 0).
  final int initialIndex;
  const MainShell({super.key, this.initialIndex = 0});

  @override
  State<MainShell> createState() => _MainShellState();
}

class _MainShellState extends State<MainShell> {
  late int _index = widget.initialIndex.clamp(0, 2);

  // Tabs use IndexedStack so each retains its state — wallet stays
  // unlocked when the user dips into Chats, scroll position survives
  // tab toggles, etc.
  static const _tabs = <_TabSpec>[
    _TabSpec(icon: Icons.account_balance_wallet_outlined, label: 'WALLET'),
    _TabSpec(icon: Icons.chat_bubble_outline, label: 'CHATS'),
    _TabSpec(icon: Icons.settings_outlined, label: 'SETTINGS'),
  ];

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      // The tab screens (wallet/chats/settings) were written as full-screen
      // scaffolds that add MediaQuery.viewPadding.bottom themselves to clear
      // the system gesture bar. Inside this shell, the bottomNavigationBar
      // already reserves + pads that inset, so the inner screens must NOT
      // double-count it. removeBottom zeros their bottom inset, leaving the
      // shell's nav bar as the single owner of the system-bar area.
      body: MediaQuery.removePadding(
        context: context,
        removeBottom: true,
        child: IndexedStack(
          index: _index,
          children: const [
            ArgosWalletScreen(),
            HomeScreen(),
            SettingsScreen(),
          ],
        ),
      ),
      bottomNavigationBar: SafeArea(
        top: false,
        child: Container(
          decoration: BoxDecoration(
            color: kBgCard,
            border: Border(top: BorderSide(color: kCyanDim, width: 1)),
          ),
          padding: const EdgeInsets.symmetric(vertical: 6, horizontal: 4),
          child: Row(
            children: List.generate(_tabs.length, (i) {
              final selected = i == _index;
              return Expanded(
                child: GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTap: () => setState(() => _index = i),
                  child: Container(
                    padding: const EdgeInsets.symmetric(vertical: 8),
                    decoration: BoxDecoration(
                      color: selected ? kCyanDim : Colors.transparent,
                      border: Border(
                        top: BorderSide(
                          color: selected ? kCyan : Colors.transparent,
                          width: 2,
                        ),
                      ),
                    ),
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(
                          _tabs[i].icon,
                          color: selected ? kCyan : kGrayText,
                          size: 22,
                        ),
                        const SizedBox(height: 3),
                        Text(
                          _tabs[i].label,
                          style: GoogleFonts.orbitron(
                            fontSize: 9,
                            letterSpacing: 2,
                            fontWeight: FontWeight.w700,
                            color: selected ? kCyan : kGrayText,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              );
            }),
          ),
        ),
      ),
    );
  }
}

class _TabSpec {
  final IconData icon;
  final String label;
  const _TabSpec({required this.icon, required this.label});
}
