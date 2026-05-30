// Screenshot-capable integration test driver.
//
// Pairs with `integration_test/screenshot_test.dart`. Each
// `binding.takeScreenshot(name)` in the test streams the CPU-rendered
// frame bytes back here over the Flutter test channel — no GPU / adb
// screencap involved, so it works on a headless emulator where
// swiftshader's framebuffer is not screencap-readable.
//
// Run:
//   flutter drive \
//     --driver=test_driver/integration_test.dart \
//     --target=integration_test/screenshot_test.dart \
//     -d emulator-5554
//
// PNGs land in mobile/screenshots/<name>.png.
import 'dart:io';
import 'package:integration_test/integration_test_driver_extended.dart';

Future<void> main() async {
  await integrationDriver(
    onScreenshot:
        (String name, List<int> bytes, [Map<String, Object?>? args]) async {
      final file = File('screenshots/$name.png');
      await file.create(recursive: true);
      await file.writeAsBytes(bytes);
      // ignore: avoid_print
      print('[driver] wrote screenshots/$name.png (${bytes.length} bytes)');
      return true;
    },
  );
}
