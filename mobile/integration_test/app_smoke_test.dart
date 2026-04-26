import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import 'package:phantomchat/main.dart' as app;

/// End-to-end smoke that exercises the user-facing paths reported broken
/// in 1.0.3 (PIN-confirm hang, Add-Contact silent fail, missing QR-Scan
/// button) on a real device + real platform plugins.
///
/// Run from a host terminal:
///   flutter test integration_test/app_smoke_test.dart
/// against a connected Android emulator/device. The test installs a test
/// runner APK on the device and drives the app via WidgetTester — no
/// screencap or visual feedback required, all assertions are on the
/// widget tree.
void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('onboarding → PIN setup → home → add contact → QR button',
      (tester) async {
    // ── 1. Boot the app ──────────────────────────────────────────────────
    // Pumps main.dart, runs RustLib.init(), reads identity flag from
    // secure storage. After this returns the onboarding splash is on screen.
    //
    // We can't use pumpAndSettle here: the onboarding screen runs a
    // `Timer.periodic` boot sequence (7 lines @ 180 ms = 1.3 s) plus a
    // fade-in AnimationController. pumpAndSettle waits until the tree is
    // quiet — the periodic timer means it never goes quiet — so we pump
    // in discrete slices long enough for the boot timer to self-cancel.
    app.main();
    // First pump lets main()'s `runApp` install the widget tree.
    await tester.pump();
    // Then drain ~3 s in 200 ms slices: covers the 1.3 s boot sequence,
    // the 500 ms fade-in, plus a generous margin for the Rust core boot
    // banner and async storage probes.
    for (var i = 0; i < 25; i++) {
      await tester.pump(const Duration(milliseconds: 200));
    }

    expect(find.text('INITIALIZE IDENTITY'), findsOneWidget,
        reason: 'onboarding boot sequence did not finish');
    await tester.tap(find.text('INITIALIZE IDENTITY'));
    await tester.pumpAndSettle(const Duration(seconds: 1));

    // ── 2. Codename + GENERATE KEYS ──────────────────────────────────────
    final codenameField = find.byType(TextField).first;
    await tester.enterText(codenameField, 'smoketest');
    await tester.pumpAndSettle();

    expect(find.text('GENERATE KEYS'), findsOneWidget);
    await tester.tap(find.text('GENERATE KEYS'));
    // Identity generation is deterministic (X25519 keygen), should be
    // sub-100 ms, but the screen transitions over a 400 ms fade.
    await tester.pumpAndSettle(const Duration(seconds: 2));

    // ── 3. ENTER PHANTOM → Lock setup ────────────────────────────────────
    expect(find.text('[ ENTER PHANTOM ]'), findsOneWidget);
    await tester.tap(find.text('[ ENTER PHANTOM ]'));
    await tester.pumpAndSettle(const Duration(seconds: 1));

    expect(find.text('> SET PIN'), findsOneWidget,
        reason: 'lock screen did not enter setupMode');

    // Tap 1234 on the numeric pad. Each digit is a Text widget inside a
    // _PadKey GestureDetector; tap the first matching ancestor that
    // responds to taps.
    Future<void> tapDigit(String d) async {
      await tester.tap(find.text(d).first);
      await tester.pump(const Duration(milliseconds: 50));
    }

    for (final d in ['1', '2', '3', '4']) {
      await tapDigit(d);
    }
    expect(find.text('CONFIRM'), findsOneWidget);
    await tester.tap(find.text('CONFIRM'));
    await tester.pumpAndSettle(const Duration(seconds: 1));

    // ── 4. CONFIRM PIN ───────────────────────────────────────────────────
    expect(find.text('> CONFIRM PIN'), findsOneWidget);
    expect(find.text('Repeat PIN to confirm'), findsOneWidget);

    for (final d in ['1', '2', '3', '4']) {
      await tapDigit(d);
    }
    final t0 = DateTime.now();
    await tester.tap(find.text('CONFIRM'));
    // The setPin() call kicks off PBKDF2 in a background isolate. We need
    // long enough for the isolate spawn + 50k iters + storage writes.
    // Budget is 10 s on emulator-class hardware; on a real device sub-1 s.
    await tester.pump();
    for (var i = 0; i < 50; i++) {
      if (find.text('Securing PIN…').evaluate().isEmpty) break;
      await tester.pump(const Duration(milliseconds: 200));
    }
    await tester.pumpAndSettle(const Duration(seconds: 3));
    final pinElapsed = DateTime.now().difference(t0);
    // ignore: avoid_print
    print('[smoke] PIN setup elapsed: ${pinElapsed.inMilliseconds} ms');
    expect(pinElapsed.inSeconds < 15, isTrue,
        reason: 'PIN setup took longer than 15 s — '
            'the 600k → 50k PBKDF2 fix did not land');

    // ── 5. Home screen ───────────────────────────────────────────────────
    // After PIN setup we should be on HomeScreen with a "+" FAB.
    expect(find.byIcon(Icons.add), findsOneWidget,
        reason: 'home screen FAB not found — PIN setup did not complete');

    // ── 6. Add Contact modal ─────────────────────────────────────────────
    await tester.tap(find.byIcon(Icons.add));
    await tester.pumpAndSettle(const Duration(seconds: 1));

    expect(find.text('ADD_CONTACT //'), findsOneWidget,
        reason: 'add-contact modal did not open');

    // QR-Scan button presence — fix for "qr code scan funktion war auch weg"
    expect(find.text('SCAN QR'), findsOneWidget,
        reason: 'QR-Scan button missing from Add-Contact modal');
    expect(find.byIcon(Icons.qr_code_scanner), findsOneWidget);

    // Fill nickname + phantom_id and confirm. The colon-hex format is
    // what desktop emits; the parser also accepts phantomx and legacy
    // base64 (see contact_format_test.dart).
    final modalFields = find.byType(TextField);
    expect(modalFields, findsNWidgets(2));
    await tester.enterText(modalFields.at(0), 'TESTPEER');
    await tester.enterText(
      modalFields.at(1),
      'phantom:'
      '0000000000000000000000000000000000000000000000000000000000000001:'
      '0000000000000000000000000000000000000000000000000000000000000002',
    );
    await tester.pumpAndSettle();

    expect(find.text('CONFIRM ADD'), findsOneWidget);
    await tester.tap(find.text('CONFIRM ADD'));
    await tester.pumpAndSettle(const Duration(seconds: 2));

    // After save the modal closes and we're back on HomeScreen, with the
    // new contact rendered in the list. The contact's nickname is what
    // we typed, uppercased.
    expect(find.text('TESTPEER'), findsOneWidget,
        reason: 'newly-added contact not visible in list');

    // ignore: avoid_print
    print('[smoke] all 4 user-facing paths verified end-to-end');
  });
}
