// Visual tour of the expanded Settings screen (relay status, app-update,
// about). Navigates onboarding → identity → PIN → home → settings, pausing
// + scrolling so the external grim loop captures each section.
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

  testWidgets('settings tour', (tester) async {
    app.main();
    await tester.pump();
    for (var i = 0; i < 20; i++) {
      await tester.pump(const Duration(milliseconds: 200));
    }
    await tester.tap(find.text('INITIALIZE IDENTITY'));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    await tester.enterText(find.byType(TextField).first, 'settings-demo');
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

    // Open settings via the gear icon in the home header.
    // ignore: avoid_print
    print('[tour] home');
    await hold(tester);
    await tester.tap(find.byIcon(Icons.settings_outlined));
    await tester.pumpAndSettle(const Duration(seconds: 2));
    // ignore: avoid_print
    print('[tour] settings_top');
    await hold(tester);

    final list = find.byType(ListView);
    if (list.evaluate().isNotEmpty) {
      await tester.drag(list, const Offset(0, -420));
      await tester.pumpAndSettle();
      // ignore: avoid_print
      print('[tour] settings_mid');
      await hold(tester);
      await tester.drag(list, const Offset(0, -460));
      await tester.pumpAndSettle();
      // ignore: avoid_print
      print('[tour] settings_bottom');
      await hold(tester);
    }
    // ignore: avoid_print
    print('[tour] DONE');
  });
}
