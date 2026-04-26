// Stub so the `record_linux` package resolves to a no-op on Android-only
// builds. Upstream `record_linux 0.7.2` has fallen out of sync with
// `record_platform_interface 1.5.x`, and `flutter build apk` refuses to
// compile across the gap even though we never ship a Linux build of
// PhantomChat mobile.
//
// Every method throws `UnsupportedError` at runtime — this code never
// executes (Linux is not a target). We just need a class that satisfies
// the abstract contract for pub-solver + analyze-time.

import 'dart:async';
import 'dart:typed_data';

import 'package:record_platform_interface/record_platform_interface.dart';

class RecordLinux extends RecordPlatform {
  static void registerWith() {
    RecordPlatform.instance = RecordLinux();
  }

  static Never _unsupported() =>
      throw UnsupportedError('record_linux stub: Linux is not a build target');

  @override
  Future<void> create(String recorderId) async => _unsupported();

  @override
  Future<bool> hasPermission(String recorderId, {bool request = true}) async =>
      _unsupported();

  @override
  Future<bool> isPaused(String recorderId) async => _unsupported();

  @override
  Future<bool> isRecording(String recorderId) async => _unsupported();

  @override
  Future<void> pause(String recorderId) async => _unsupported();

  @override
  Future<void> resume(String recorderId) async => _unsupported();

  @override
  Future<void> start(
    String recorderId,
    RecordConfig config, {
    required String path,
  }) async =>
      _unsupported();

  @override
  Future<Stream<Uint8List>> startStream(
    String recorderId,
    RecordConfig config,
  ) async =>
      _unsupported();

  @override
  Future<String?> stop(String recorderId) async => _unsupported();

  @override
  Future<void> cancel(String recorderId) async => _unsupported();

  @override
  Future<void> dispose(String recorderId) async => _unsupported();

  @override
  Future<Amplitude> getAmplitude(String recorderId) async => _unsupported();

  @override
  Future<bool> isEncoderSupported(
    String recorderId,
    AudioEncoder encoder,
  ) async =>
      _unsupported();

  @override
  Future<List<InputDevice>> listInputDevices(String recorderId) async =>
      _unsupported();

  @override
  Stream<RecordState> onStateChanged(String recorderId) =>
      Stream<RecordState>.empty();
}
