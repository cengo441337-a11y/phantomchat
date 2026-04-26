// Stub so the `record_linux` package resolves to a no-op on Android-only
// builds. Upstream `record_linux 0.7.2` has fallen out of sync with
// `record_platform_interface 1.5.x` (missing `startStream`, mismatched
// `hasPermission`), and `flutter build apk` refuses to compile across
// the gap even though we never ship a Linux build of PhantomChat mobile.
//
// All methods inherit RecordPlatform's default `throw UnimplementedError`
// behaviour. RecordPlatform doesn't actually require any concrete methods
// in 1.5.x — abstract methods on PlatformInterface that subclasses
// override. Since this stub never runs (Linux is not a target), no method
// will ever be called. We just need a class that the pub solver accepts.

import 'package:record_platform_interface/record_platform_interface.dart';

class RecordLinux extends RecordPlatform {
  static void registerWith() {
    RecordPlatform.instance = RecordLinux();
  }
}
