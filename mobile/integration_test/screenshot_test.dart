// Visual walk-through: drives the real app through every primary screen
// and captures a CPU-rendered screenshot of each via the Flutter test
// channel (binding.takeScreenshot → test_driver/integration_test.dart
// onScreenshot → mobile/screenshots/*.png). Works on a headless emulator
// where adb screencap returns black because of swiftshader + Impeller.
//
// Run:
//   flutter drive --driver=test_driver/integration_test.dart \
//     --target=integration_test/screenshot_test.dart -d emulator-5554
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import 'package:phantomchat/main.dart' as app;

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('walk every screen + screenshot', (tester) async {
    // ── Boot ──────────────────────────────────────────────────────────────
    app.main();
    await tester.pump();
    for (var i = 0; i < 25; i++) {
      await tester.pump(const Duration(milliseconds: 200));
    }
    // Android: turn the live surface into a capturable image. Must run once
    // before the first takeScreenshot; after this the engine renders into a
    // RenderRepaintBoundary the test channel can read.
    await binding.convertFlutterSurfaceToImage();
    await tester.pumpAndSettle();
    await binding.takeScreenshot('01_onboarding');

    // ── Keygen ────────────────────────────────────────────────────────────
    await tester.tap(find.text('INITIALIZE IDENTITY'));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    await tester.enterText(find.byType(TextField).first, 'demo');
    await tester.pumpAndSettle();
    await binding.takeScreenshot('02_keygen');

    await tester.tap(find.text('GENERATE KEYS'));
    await tester.pumpAndSettle(const Duration(seconds: 2));
    await binding.takeScreenshot('03_identity_ready');

    // ── PIN setup ─────────────────────────────────────────────────────────
    await tester.tap(find.text('[ ENTER PHANTOM ]'));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    await binding.takeScreenshot('04_set_pin');

    Future<void> tapDigit(String d) async {
      await tester.tap(find.text(d).first);
      await tester.pump(const Duration(milliseconds: 60));
    }

    for (final d in ['1', '2', '3', '4']) {
      await tapDigit(d);
    }
    await binding.takeScreenshot('05_pin_entered');
    await tester.tap(find.text('CONFIRM'));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    for (final d in ['1', '2', '3', '4']) {
      await tapDigit(d);
    }
    await tester.tap(find.text('CONFIRM'));
    await tester.pump();
    for (var i = 0; i < 60; i++) {
      if (find.byIcon(Icons.add).evaluate().isNotEmpty) break;
      await tester.pump(const Duration(milliseconds: 200));
    }
    await tester.pumpAndSettle(const Duration(seconds: 2));

    // ── Home ──────────────────────────────────────────────────────────────
    await binding.takeScreenshot('06_home');

    // ── Add contact modal ─────────────────────────────────────────────────
    await tester.tap(find.byIcon(Icons.add));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    await binding.takeScreenshot('07_add_contact');

    final modalFields = find.byType(TextField);
    await tester.enterText(modalFields.at(0), 'TESTPEER');
    await tester.enterText(
      modalFields.at(1),
      'phantom:'
      '0000000000000000000000000000000000000000000000000000000000000001:'
      '0000000000000000000000000000000000000000000000000000000000000002',
    );
    await tester.pumpAndSettle();
    await binding.takeScreenshot('08_contact_filled');

    await tester.tap(find.text('CONFIRM ADD'));
    await tester.pumpAndSettle(const Duration(seconds: 2));
    await binding.takeScreenshot('09_home_with_contact');

    // ── Chat screen ───────────────────────────────────────────────────────
    await tester.tap(find.text('TESTPEER').first);
    await tester.pumpAndSettle(const Duration(seconds: 2));
    await binding.takeScreenshot('10_chat_empty');

    await tester.enterText(find.byType(TextField), 'hello from the audit run');
    await tester.pumpAndSettle();
    await binding.takeScreenshot('11_chat_typed');

    if (find.byIcon(Icons.send_rounded).evaluate().isNotEmpty) {
      await tester.tap(find.byIcon(Icons.send_rounded));
      await tester.pumpAndSettle(const Duration(seconds: 3));
      await binding.takeScreenshot('12_chat_sent');
    }
  });
}
