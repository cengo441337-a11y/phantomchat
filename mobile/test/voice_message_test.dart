// Tests for the voice-message wire codec + ref parser.
//
// Three concerns covered:
//   * round-trip — encodeVoiceWire then decodeVoiceBody (sans the 8-byte
//     prefix that the sealed-sender path strips for us) must reproduce the
//     codec / duration / payload bit-for-bit.
//   * ref parser — voice://… strings must reject path traversal so a
//     hostile peer can't escape the per-app cache dir via `..` segments
//     or absolute paths.
//   * edge cases — the codec field is u8, duration is u32 LE; we exercise
//     0-duration, the 60s upper bound the recorder caps at, and an
//     unknown codec id (0xFF) round-tripping unchanged so an older client
//     can still decode something a newer one emitted.

import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:phantomchat/services/voice_message.dart';

void main() {
  group('encodeVoiceWire / decodeVoiceBody', () {
    test('round-trip preserves codec, duration, and bytes', () {
      final audio = Uint8List.fromList(
        List<int>.generate(64, (i) => i & 0xFF),
      );
      final wire = encodeVoiceWire(
        codecId: kCodecOpusOgg,
        durationMs: 5000,
        audio: audio,
      );

      // Strip the 8-byte VOICE-1: prefix the way the sealed-sender path
      // does before handing the body to decodeVoiceBody.
      expect(wire.length > kVoicePrefixV1.length, true);
      final body = Uint8List.sublistView(wire, kVoicePrefixV1.length);

      final decoded = decodeVoiceBody(body);
      expect(decoded, isNotNull);
      expect(decoded!.codecId, kCodecOpusOgg);
      expect(decoded.durationMs, 5000);
      expect(decoded.audio, equals(audio));
    });

    test('0-duration clip round-trips', () {
      final wire = encodeVoiceWire(
        codecId: kCodecAacM4a,
        durationMs: 0,
        audio: Uint8List(0),
      );
      final body = Uint8List.sublistView(wire, kVoicePrefixV1.length);
      final decoded = decodeVoiceBody(body);
      expect(decoded, isNotNull);
      expect(decoded!.durationMs, 0);
      expect(decoded.audio.length, 0);
    });

    test('60s upper-bound duration round-trips', () {
      final wire = encodeVoiceWire(
        codecId: kCodecOpusOgg,
        durationMs: 60000,
        audio: Uint8List.fromList([1, 2, 3]),
      );
      final body = Uint8List.sublistView(wire, kVoicePrefixV1.length);
      final decoded = decodeVoiceBody(body);
      expect(decoded, isNotNull);
      expect(decoded!.durationMs, 60000);
    });

    test('unknown codec_id 0xFF survives round-trip', () {
      // An older client must still see the bytes even if it can't render
      // them — the wire is forward-compatible by design.
      final wire = encodeVoiceWire(
        codecId: 0xFF,
        durationMs: 1234,
        audio: Uint8List.fromList([0xDE, 0xAD]),
      );
      final body = Uint8List.sublistView(wire, kVoicePrefixV1.length);
      final decoded = decodeVoiceBody(body);
      expect(decoded, isNotNull);
      expect(decoded!.codecId, 0xFF);
      expect(extensionForCodec(0xFF), 'bin');
    });

    test('truncated body returns null (audio_len exceeds remaining)', () {
      // Hand-craft a header that claims 1000 bytes of audio but provides
      // only 4. decodeVoiceBody must refuse rather than read past the end.
      final hdr = ByteData(kVoiceHeaderLen);
      hdr.setUint8(0, kCodecOpusOgg);
      hdr.setUint32(1, 100, Endian.little);
      hdr.setUint32(5, 1000, Endian.little);
      final body = Uint8List.fromList(
        hdr.buffer.asUint8List() + [1, 2, 3, 4],
      );
      expect(decodeVoiceBody(body), isNull);
    });
  });

  group('parseVoiceRef', () {
    test('valid ref parses', () {
      final ref = parseVoiceRef('voice://0/5000/abc.ogg');
      expect(ref, isNotNull);
      expect(ref!.codecId, 0);
      expect(ref.durationMs, 5000);
      expect(ref.filename, 'abc.ogg');
    });

    test('rejects ../ path traversal', () {
      expect(
        parseVoiceRef('voice://0/1000/../../../etc/passwd'),
        isNull,
      );
    });

    test('rejects absolute filename', () {
      expect(parseVoiceRef('voice://0/1000//etc/passwd'), isNull);
    });

    test('rejects non-numeric codec / duration', () {
      expect(parseVoiceRef('voice://abc/1000/x.ogg'), isNull);
      expect(parseVoiceRef('voice://0/abc/x.ogg'), isNull);
    });

    test('rejects strings missing the prefix', () {
      expect(parseVoiceRef('not a voice ref'), isNull);
      expect(parseVoiceRef('http://0/1000/x.ogg'), isNull);
    });
  });
}
