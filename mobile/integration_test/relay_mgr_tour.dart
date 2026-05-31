// Tour the v1.1.11 features: settings page with new AI-bridge card + the
// tappable VERBINDUNG card opening the full RelayManager screen.
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

  testWidgets('relay manager tour', (tester) async {
    app.main();
    await tester.pump();
    for (var i = 0; i < 20; i++) {
      await tester.pump(const Duration(milliseconds: 200));
    }
    await tester.tap(find.text('INITIALIZE IDENTITY'));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    await tester.enterText(find.byType(TextField).first, 'v111');
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

    // Settings via the gear icon.
    await tester.tap(find.byIcon(Icons.settings_outlined));
    await tester.pumpAndSettle(const Duration(seconds: 1));
    // ignore: avoid_print
    print('[tour] settings_top');
    await hold(tester);

    // Scroll once so VERBINDUNG + AI-ASSISTENT come into view.
    final list = find.byType(ListView);
    if (list.evaluate().isNotEmpty) {
      await tester.drag(list, const Offset(0, -260));
      await tester.pumpAndSettle();
      // ignore: avoid_print
      print('[tour] settings_relay_ai');
      await hold(tester);
    }

    // Tap the relay card to open the RelayManager.
    final relayTile = find.text('Relays verwalten');
    if (relayTile.evaluate().isNotEmpty) {
      await tester.tap(relayTile);
      await tester.pumpAndSettle(const Duration(seconds: 1));
      // ignore: avoid_print
      print('[tour] relay_manager');
      await hold(tester);

      // Tap the test/probe button on the first row.
      final probe = find.byIcon(Icons.network_check).first;
      if (probe.evaluate().isNotEmpty) {
        await tester.tap(probe);
        await tester.pumpAndSettle(const Duration(seconds: 3));
        // ignore: avoid_print
        print('[tour] relay_probe');
        await hold(tester);
      }

      // Tap the FAB to open the add dialog.
      final fab = find.byType(FloatingActionButton);
      if (fab.evaluate().isNotEmpty) {
        await tester.tap(fab);
        await tester.pumpAndSettle(const Duration(seconds: 1));
        // ignore: avoid_print
        print('[tour] add_relay_sheet');
        await hold(tester);
      }
    }
    // ignore: avoid_print
    print('[tour] DONE');
  });
}
