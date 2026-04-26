// Wave 11B — voice-message wire format + persistence helpers.
//
// Wire envelope inside a `sendSealedV3` plaintext:
//   "VOICE-1:" || u8(codec_id) || u32_le(duration_ms) || u32_le(audio_len) || audio
// codec_id: 0x00 = opus-in-ogg (Android default), 0x01 = AAC-in-m4a (iOS / fallback).
//
// `PhantomMessage` only stores strings — to avoid touching the model we
// stuff a `voice://<codec>/<duration_ms>/<filename>` reference into
// `plaintext`; the audio bytes live in `<app-cache>/voice/<filename>`.

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:path_provider/path_provider.dart';

/// 8-byte ASCII tag at the start of a voice-message plaintext.
final Uint8List kVoicePrefixV1 =
    Uint8List.fromList(utf8.encode('VOICE-1:'));

/// 1 (codec) + 4 (duration_ms LE) + 4 (audio_len LE) header following
/// the prefix.
const int kVoiceHeaderLen = 1 + 4 + 4;

/// Codec identifiers — see file-level doc.
const int kCodecOpusOgg = 0x00;
const int kCodecAacM4a = 0x01;

/// File extension corresponding to a wire codec_id. Used both for the
/// recorder's output path and to name the persisted-cache file so a
/// future debug "open in player" intent has the right MIME hint.
String extensionForCodec(int codecId) {
  switch (codecId) {
    case kCodecOpusOgg:
      return 'ogg';
    case kCodecAacM4a:
      return 'm4a';
    default:
      return 'bin';
  }
}

/// Encode a recorded clip as a wire-format `Uint8List` ready for the
/// sealed-sender plaintext slot. Caller passes the raw codec bytes (the
/// file the recorder wrote) plus duration and codec id.
Uint8List encodeVoiceWire({
  required int codecId,
  required int durationMs,
  required Uint8List audio,
}) {
  final out = BytesBuilder();
  out.add(kVoicePrefixV1);
  out.addByte(codecId & 0xFF);
  // u32 little-endian — match what the desktop side expects.
  final hdr = ByteData(8);
  hdr.setUint32(0, durationMs & 0xFFFFFFFF, Endian.little);
  hdr.setUint32(4, audio.length & 0xFFFFFFFF, Endian.little);
  out.add(hdr.buffer.asUint8List());
  out.add(audio);
  return out.toBytes();
}

/// Result of a successful voice-wire decode.
class DecodedVoice {
  final int codecId;
  final int durationMs;
  final Uint8List audio;
  DecodedVoice({
    required this.codecId,
    required this.durationMs,
    required this.audio,
  });
}

/// Try to peel a voice-wire envelope off a plaintext that already had the
/// 8-byte prefix matched + stripped. Returns null on a malformed header.
DecodedVoice? decodeVoiceBody(Uint8List body) {
  if (body.length < kVoiceHeaderLen) return null;
  final codecId = body[0];
  final bd = ByteData.sublistView(body, 1, kVoiceHeaderLen);
  final durationMs = bd.getUint32(0, Endian.little);
  final audioLen = bd.getUint32(4, Endian.little);
  if (body.length < kVoiceHeaderLen + audioLen) return null;
  final audio = Uint8List.sublistView(
    body,
    kVoiceHeaderLen,
    kVoiceHeaderLen + audioLen,
  );
  return DecodedVoice(
    codecId: codecId,
    durationMs: durationMs,
    audio: audio,
  );
}

/// Persist a decoded clip into the app cache and return a stable
/// `voice://<codec>/<duration_ms>/<filename>` reference suitable for
/// stuffing into [PhantomMessage.plaintext]. The audio bytes themselves
/// are written under `<cache>/voice/<filename>`.
Future<String> persistVoiceClip({
  required int codecId,
  required int durationMs,
  required Uint8List audio,
  required String filename,
}) async {
  final dir = await voiceCacheDir();
  final f = File('${dir.path}/$filename');
  await f.writeAsBytes(audio, flush: true);
  return 'voice://$codecId/$durationMs/$filename';
}

/// Returns (and lazily creates) the on-device cache dir used for
/// persisted voice clips. Lives under the standard app-cache root so the
/// OS is allowed to evict it under storage pressure — that's fine, the
/// chat row degrades to "[voice clip unavailable]" if the file is gone.
Future<Directory> voiceCacheDir() async {
  final root = await getApplicationCacheDirectory();
  final dir = Directory('${root.path}/voice');
  if (!await dir.exists()) {
    await dir.create(recursive: true);
  }
  return dir;
}

/// Prefix that marks a [PhantomMessage.plaintext] as a voice reference
/// (see file-level doc). Consumers strip this and parse the remaining
/// `<codec>/<duration_ms>/<filename>` triplet.
const String kVoiceRefPrefix = 'voice://';

/// Parsed form of a `voice://...` reference stored on a [PhantomMessage].
class VoiceRef {
  final int codecId;
  final int durationMs;
  final String filename;
  VoiceRef({
    required this.codecId,
    required this.durationMs,
    required this.filename,
  });

  /// Resolve the on-disk path under the voice cache dir. Doesn't check
  /// existence — caller decides whether to display or to fall back to
  /// "unavailable".
  Future<String> resolvePath() async {
    final dir = await voiceCacheDir();
    return '${dir.path}/$filename';
  }
}

/// Parse a `voice://<codec>/<duration_ms>/<filename>` reference. Returns
/// null on a non-matching string so callers can detect "this is just text
/// after all".
VoiceRef? parseVoiceRef(String s) {
  if (!s.startsWith(kVoiceRefPrefix)) return null;
  final rest = s.substring(kVoiceRefPrefix.length);
  final parts = rest.split('/');
  if (parts.length < 3) return null;
  final codecId = int.tryParse(parts[0]);
  final durationMs = int.tryParse(parts[1]);
  if (codecId == null || durationMs == null) return null;
  // Filename can contain extra slashes only if a hostile peer crafts one,
  // but we strip path traversal up-front by joining everything from index
  // 2 onward and filtering `..`/`/` defensively.
  final raw = parts.sublist(2).join('/');
  if (raw.contains('..') || raw.startsWith('/')) return null;
  return VoiceRef(
    codecId: codecId,
    durationMs: durationMs,
    filename: raw,
  );
}

/// Format a duration as `m:ss` for the bubble label.
String formatDurationMs(int ms) {
  final totalSecs = (ms / 1000).round();
  final m = totalSecs ~/ 60;
  final s = totalSecs % 60;
  return '$m:${s.toString().padLeft(2, '0')}';
}
