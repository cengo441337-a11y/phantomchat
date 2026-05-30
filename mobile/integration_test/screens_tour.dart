// UX tour: navigates every primary screen via the widget tester (which
// reliably delivers taps, unlike adb input on this emulator) and PAUSES on
// each screen long enough for an external grim screenshot loop to capture
// it. Assertions double as proof the navigation actually works.
//
//   flutter drive --no-enable-impeller \
//     --driver=test_driver/integration_test.dart \
//     --target=integration_test/screens_tour.dart -d emulator-5554
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import 'package:phantomchat/main.dart' as app;

Future<void> hold(WidgetTester tester) async {
  // Keep the frame pumping for ~5 s so the grim loop lands a shot, without
  // pumpAndSettle (the boot screen has an endless timer).
  for (var i = 0; i < 25; i++) {
    await tester.pump(const Duration(milliseconds: 200));
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('tour every screen with pauses', (tester) async {
    app.main();
    await tester.pump();
    for (var i = 0; i < 20; i++) {
      await tester.pump(const Duration(milliseconds: 200));
    }
    // SCREEN 1: onboarding
    // ignore: avoid_print
    print('[tour] onboarding');
    await hold(tester);

    // SCREEN 2: keygen
    await tester.tap(find.text('INITIALIZE IDENTITY'));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    await tester.enterText(find.byType(TextField).first, 'audit-demo');
    await tester.pumpAndSettle();
    // ignore: avoid_print
    print('[tour] keygen');
    await hold(tester);

    // SCREEN 3: identity ready
    await tester.tap(find.text('GENERATE KEYS'));
    await tester.pumpAndSettle(const Duration(seconds: 2));
    // ignore: avoid_print
    print('[tour] identity_ready');
    await hold(tester);

    // SCREEN 4: set PIN
    await tester.tap(find.text('[ ENTER PHANTOM ]'));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    // ignore: avoid_print
    print('[tour] set_pin');
    await hold(tester);

    Future<void> tapDigit(String d) async {
      await tester.tap(find.text(d).first);
      await tester.pump(const Duration(milliseconds: 60));
    }

    for (final d in ['1', '2', '3', '4']) {
      await tapDigit(d);
    }
    await tester.tap(find.text('CONFIRM'));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    // SCREEN 5: confirm PIN
    // ignore: avoid_print
    print('[tour] confirm_pin');
    await hold(tester);
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

    // SCREEN 6: home
    // ignore: avoid_print
    print('[tour] home');
    await hold(tester);

    // SCREEN 7: add contact
    await tester.tap(find.byIcon(Icons.add));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    // ignore: avoid_print
    print('[tour] add_contact');
    await hold(tester);

    final modalFields = find.byType(TextField);
    await tester.enterText(modalFields.at(0), 'ALICE');
    await tester.enterText(
      modalFields.at(1),
      'phantom:'
      '0000000000000000000000000000000000000000000000000000000000000001:'
      '0000000000000000000000000000000000000000000000000000000000000002',
    );
    await tester.pumpAndSettle();
    // ignore: avoid_print
    print('[tour] contact_filled');
    await hold(tester);

    await tester.tap(find.text('CONFIRM ADD'));
    await tester.pumpAndSettle(const Duration(seconds: 2));
    // SCREEN 8: home with contact
    // ignore: avoid_print
    print('[tour] home_with_contact');
    await hold(tester);

    // SCREEN 9: chat
    await tester.tap(find.text('ALICE').first);
    await tester.pumpAndSettle(const Duration(seconds: 2));
    // ignore: avoid_print
    print('[tour] chat');
    await hold(tester);

    await tester.enterText(find.byType(TextField), 'hello from the audit run');
    await tester.pumpAndSettle();
    // ignore: avoid_print
    print('[tour] chat_typed');
    await hold(tester);

    if (find.byIcon(Icons.send_rounded).evaluate().isNotEmpty) {
      await tester.tap(find.byIcon(Icons.send_rounded));
      await tester.pumpAndSettle(const Duration(seconds: 3));
      // ignore: avoid_print
      print('[tour] chat_sent');
      await hold(tester);
    }
    // ignore: avoid_print
    print('[tour] DONE');
  });
}
