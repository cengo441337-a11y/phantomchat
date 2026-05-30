// Verify the v1.1.9 fix: prominent update icon in the Home header +
// gesture-bar safe padding on Home FAB / Chat input. Pauses on each
// screen so an external grim loop captures it.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import 'package:phantomchat/main.dart' as app;

Future<void> hold(WidgetTester tester) async {
  for (var i = 0; i < 25; i++) {
    await tester.pump(const Duration(milliseconds: 200));
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('header update icon + chat safe-area', (tester) async {
    app.main();
    await tester.pump();
    for (var i = 0; i < 20; i++) {
      await tester.pump(const Duration(milliseconds: 200));
    }
    await tester.tap(find.text('INITIALIZE IDENTITY'));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    await tester.enterText(find.byType(TextField).first, 'v119');
    await tester.pumpAndSettle();
    await tester.tap(find.text('GENERATE KEYS'));
    await tester.pumpAndSettle(const Duration(seconds: 2));
    await tester.tap(find.text('[ ENTER PHANTOM ]'));
    await tester.pumpAndSettle(const Duration(seconds: 1));

    Future<void> tapDigit(String d) async {
      await tester.tap(find.text(d).first);
      await tester.pump(const Duration(milliseconds: 60));
    }

    for (final d in ['1', '2', '3', '4']) {
      await tapDigit(d);
    }
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

    // Home with the new prominent update icon in the header.
    // ignore: avoid_print
    print('[tour] home_with_update_icon');
    await hold(tester);

    // Add a contact so we can open the chat.
    await tester.tap(find.byIcon(Icons.add));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    final fields = find.byType(TextField);
    await tester.enterText(fields.at(0), 'BOB');
    await tester.enterText(
      fields.at(1),
      'phantom:'
      '0000000000000000000000000000000000000000000000000000000000000001:'
      '0000000000000000000000000000000000000000000000000000000000000002',
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('CONFIRM ADD'));
    await tester.pumpAndSettle(const Duration(seconds: 2));

    // Chat: input row should sit above the gesture bar now.
    await tester.tap(find.text('BOB').first);
    await tester.pumpAndSettle(const Duration(seconds: 2));
    // ignore: avoid_print
    print('[tour] chat_with_safe_area');
    await hold(tester);

    await tester.enterText(find.byType(TextField), 'kein nav-bar overlap mehr');
    await tester.pumpAndSettle();
    // ignore: avoid_print
    print('[tour] chat_typed_safe');
    await hold(tester);
    // ignore: avoid_print
    print('[tour] DONE');
  });
}
