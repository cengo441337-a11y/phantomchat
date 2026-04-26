import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:audioplayers/audioplayers.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';
import 'package:permission_handler/permission_handler.dart';
import 'package:record/record.dart';
import 'package:uuid/uuid.dart';
import '../models/contact.dart';
import '../models/identity.dart';
import '../models/message.dart';
import '../services/contact_directory.dart';
import '../services/crypto_service.dart';
import '../services/i18n.dart';
import '../services/relay_service.dart';
import '../services/storage_service.dart';
import '../services/voice_message.dart' as voice;
import '../src/rust/api.dart' as rust;
import '../theme.dart';
import '../widgets/cyber_card.dart';

class ChatScreen extends StatefulWidget {
  final PhantomContact contact;
  final PhantomIdentity identity;

  const ChatScreen({super.key, required this.contact, required this.identity});

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final _msgCtrl = TextEditingController();
  final _scrollCtrl = ScrollController();
  List<PhantomMessage> _messages = [];
  bool _sending = false;
  static const _uuid = Uuid();
  StreamSubscription<RelayEvent>? _relaySub;

  /// Tracks the most-recent incoming sender label/sigOk for the bind-to-
  /// contact UI. Set by the listener whenever a `?<8hex>` message lands.
  String? _lastUnboundSenderLabel;

  // ── Wave 11B — voice-recording state ────────────────────────────────
  /// `record` audio recorder. One instance per chat screen; the package
  /// is stateful (locks the mic device) so we explicitly dispose it.
  final AudioRecorder _recorder = AudioRecorder();

  /// True between `start` and `stop`. Drives the mic-button colour and
  /// the elapsed-time ticker. Not the same as `_recorder.isRecording()`
  /// — we mirror it as plain Dart state so `setState` rebuilds work
  /// without an `await` round-trip.
  bool _recording = false;

  /// True when a press has crossed the cancel-threshold (drag-left). On
  /// release the recording is stopped + discarded instead of sent.
  bool _cancelArmed = false;

  /// Wallclock at which the current recording started. Drives the
  /// `m:ss` counter on the recording overlay and the upper 60-second
  /// hard cap.
  DateTime? _recordingStart;

  /// Periodic timer that calls `setState` every 200ms while recording
  /// so the elapsed-time counter ticks. Cancelled in `dispose` and on
  /// stop.
  Timer? _recordingTicker;

  /// Hard cap. The `record` package doesn't expose a max-duration
  /// callback so we enforce it Dart-side: when the elapsed time crosses
  /// 60s the ticker auto-fires `_finishRecording(send: true)`.
  static const Duration _maxRecordingDuration = Duration(seconds: 60);

  /// Path on disk the recorder writes to. Keep a handle so we can
  /// `File(path).readAsBytes()` post-stop, regardless of which codec the
  /// platform negotiated.
  String? _recordingPath;

  /// Wire-level codec id for the recording in flight. Decided at
  /// `start` based on platform / encoder availability and stamped into
  /// the wire envelope on send.
  int _recordingCodecId = voice.kCodecOpusOgg;

  @override
  void initState() {
    super.initState();
    _loadMessages();
    // Subscribe to the relay event stream so `RelayService.feedEnvelope`
    // calls — now driven by the per-relay WebSockets opened below in
    // `RelayService.connect` — surface as incoming bubbles here. Filtered
    // down to messages whose sealed-sender attribution maps to this
    // contact's signing pub.
    _relaySub = RelayService.instance.events.listen((ev) {
      if (!mounted) return;
      switch (ev.kind) {
        case 'message':
          _handleIncoming(ev.payload);
          break;
        case 'voice':
          // Voice carries a `voice://...` reference in the `plaintext`
          // slot — same routing as a plain text message, but the bubble
          // renderer special-cases the prefix to draw a player.
          _handleIncoming(ev.payload);
          break;
        case 'system':
          _appendIncomingFree(ev.payload['plaintext'] as String);
          break;
      }
    });
    // Wave 7B2 — kick off the relay-publish path. Idempotent across screen
    // rebuilds (the singleton's `_connected` guard short-circuits after
    // the first call). Default URLs are the Damus / nos.lol / snort
    // triple, same as Desktop.
    unawaited(RelayService.instance.connect());
  }

  @override
  void dispose() {
    _relaySub?.cancel();
    _recordingTicker?.cancel();
    // Fire-and-forget — we're tearing down anyway.
    unawaited(_recorder.dispose());
    _msgCtrl.dispose();
    _scrollCtrl.dispose();
    super.dispose();
  }

  void _handleIncoming(Map<String, dynamic> p) {
    final senderHex = (p['senderPubHex'] as String?)?.toLowerCase();
    // Only surface here if the sealed-sender pub matches this contact's
    // bound signing pub (or it's an unbound sender showing as `?<8hex>`
    // and the user opens this thread to bind it).
    final isUnbound = p['isUnbound'] as bool? ?? false;
    final contactPubHex =
        widget.contact.publicSpendKey.toLowerCase(); // best-effort match
    final fromUs = senderHex != null &&
        (senderHex == contactPubHex ||
            (p['senderLabel'] as String?) ==
                widget.contact.nickname.toUpperCase());
    if (!fromUs && !isUnbound) return;

    if (isUnbound) {
      _lastUnboundSenderLabel = p['senderLabel'] as String?;
    }
    _appendIncomingFree(p['plaintext'] as String);
  }

  void _appendIncomingFree(String text) {
    final msg = PhantomMessage(
      id: _uuid.v4(),
      contactId: widget.contact.id,
      outgoing: false,
      plaintext: text,
      ciphertext: '',
      ephemeralKey: '',
      nonce: '',
      timestamp: DateTime.now(),
      status: MessageStatus.delivered,
    );
    StorageService.addMessage(msg);
    setState(() => _messages.add(msg));
    _scrollToBottom();
  }

  Future<void> _loadMessages() async {
    final msgs = await StorageService.loadMessages(widget.contact.id);
    setState(() => _messages = msgs);
    _scrollToBottom();
  }

  void _scrollToBottom() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scrollCtrl.hasClients) {
        _scrollCtrl.animateTo(
          _scrollCtrl.position.maxScrollExtent,
          duration: const Duration(milliseconds: 250),
          curve: Curves.easeOut,
        );
      }
    });
  }

  Future<void> _send() async {
    final text = _msgCtrl.text.trim();
    if (text.isEmpty || _sending) return;
    setState(() => _sending = true);
    _msgCtrl.clear();

    String ciphertextHex = '';
    String ephemeralHex = '';
    String nonceHex = '';
    bool relayDelivered = false;
    try {
      // v3 path — sealed-sender 1:1 via the wrapper Rust crate. Returns the
      // raw envelope wire bytes; we then ship them via every connected
      // Nostr relay (Wave 7B2 publish path). The Rust call is the slow
      // bit; the relay fan-out is parallel + bounded by `publish`'s
      // 5-second ack timeout.
      final wire = await rust.sendSealedV3(
        recipientAddress:
            'phantom:${widget.contact.publicViewKey}:${widget.contact.publicSpendKey}',
        plaintext: utf8.encode(text),
      );
      // Telemetry: we keep the wire bytes only as a debug-friendly hex
      // fingerprint on the row so a developer trace can confirm the
      // ciphertext was produced.
      ciphertextHex =
          wire.take(32).map((b) => b.toRadixString(16).padLeft(2, '0')).join();
      relayDelivered = await RelayService.instance.publish(wire);
    } catch (_) {
      // Fall back to the legacy demo crypto so first-launch (no v3
      // identity loaded yet) doesn't hard-fail. The demo is intentionally
      // NOT wire-compatible with Desktop — it's there to keep the UI
      // smoke-testable.
      try {
        final encrypted =
            await CryptoService.encrypt(text, widget.contact.publicSpendKey);
        ciphertextHex = encrypted['ciphertext']!;
        ephemeralHex = encrypted['ephemeralKey']!;
        nonceHex = encrypted['nonce']!;
      } catch (_) {
        setState(() => _sending = false);
        return;
      }
    }

    final msg = PhantomMessage(
      id: _uuid.v4(),
      contactId: widget.contact.id,
      outgoing: true,
      plaintext: text,
      ciphertext: ciphertextHex,
      ephemeralKey: ephemeralHex,
      nonce: nonceHex,
      timestamp: DateTime.now(),
      // `MessageStatus.sent` reflects "encrypted + handed to transport".
      // A future state-machine pass can flip this to `delivered` once an
      // explicit `RCPT-1:` receipt lands; for now `sent` covers the
      // not-yet-acked + acked cases (relay OK doesn't imply delivery).
      status: MessageStatus.sent,
    );
    await StorageService.addMessage(msg);
    widget.contact.lastMessage = text;
    widget.contact.lastMessageAt = DateTime.now();
    setState(() {
      _messages.add(msg);
      _sending = false;
    });
    _scrollToBottom();
    // Surface a visible failure if no relay accepted within the publish
    // timeout. Only triggered when the v3 path itself succeeded (so the
    // CryptoService fallback shouldn't show this banner — that path
    // doesn't run a relay publish).
    if (!relayDelivered && ciphertextHex.isNotEmpty && ephemeralHex.isEmpty) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text(
            '! FAILED TO PUBLISH — NO RELAY REACHED. RECIPIENT WILL '
            'NOT GET THIS MESSAGE UNTIL A RELAY ACCEPTS A RETRY.',
          ),
          duration: Duration(seconds: 4),
        ),
      );
    }
  }

  // ── Wave 11B — voice recording / sending ────────────────────────────

  /// Begin a press-and-hold recording. Asks for `RECORD_AUDIO` if not
  /// yet granted, picks the best codec the device supports (opus-ogg →
  /// AAC-m4a fallback), and starts the recorder. Spawns a 200ms ticker
  /// that both rebuilds the elapsed-time counter and enforces the
  /// 60-second hard cap.
  Future<void> _startRecording() async {
    if (_recording) return;
    // 1. Permission. `permission_handler` gives us a uniform API across
    //    Android (RECORD_AUDIO) and iOS (NSMicrophoneUsageDescription).
    final status = await Permission.microphone.request();
    if (!status.isGranted) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('! ${I18n.t('voice.permissionDenied')}')),
      );
      return;
    }
    // 2. Pick codec. `record` exposes `isEncoderSupported`; we prefer
    //    opus-ogg (small, voice-tuned) and fall back to AAC if the
    //    device's MediaCodec stack lacks an opus encoder. iOS hits the
    //    fallback by default (AVAudioRecorder doesn't write ogg).
    AudioEncoder encoder = AudioEncoder.opus;
    String ext = 'ogg';
    int codecId = voice.kCodecOpusOgg;
    try {
      final opusOk = await _recorder.isEncoderSupported(AudioEncoder.opus);
      if (!opusOk) {
        encoder = AudioEncoder.aacLc;
        ext = 'm4a';
        codecId = voice.kCodecAacM4a;
      }
    } catch (_) {
      encoder = AudioEncoder.aacLc;
      ext = 'm4a';
      codecId = voice.kCodecAacM4a;
    }
    // 3. Output path. `getApplicationCacheDirectory()` is the standard
    //    Android `cacheDir`; OS may evict under storage pressure but
    //    we send + persist immediately so that's not a real risk.
    final cacheDir = await voice.voiceCacheDir();
    final outPath =
        '${cacheDir.path}/tx_${DateTime.now().microsecondsSinceEpoch}.$ext';
    // 4. Configure: 24 kbps mono — matches the desktop side's bitrate
    //    target. `record` interprets `sampleRate` for AAC and ignores
    //    it for opus (opus is bandwidth-adaptive based on `bitRate`).
    final cfg = RecordConfig(
      encoder: encoder,
      bitRate: 24000,
      sampleRate: 16000,
      numChannels: 1,
    );
    try {
      await _recorder.start(cfg, path: outPath);
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('! REC FAIL: $e')),
      );
      return;
    }
    setState(() {
      _recording = true;
      _cancelArmed = false;
      _recordingStart = DateTime.now();
      _recordingPath = outPath;
      _recordingCodecId = codecId;
    });
    HapticFeedback.lightImpact();
    _recordingTicker = Timer.periodic(const Duration(milliseconds: 200), (_) {
      if (!mounted || !_recording) return;
      final elapsed =
          DateTime.now().difference(_recordingStart ?? DateTime.now());
      if (elapsed >= _maxRecordingDuration) {
        // Hard cap — auto-stop and send.
        unawaited(_finishRecording(send: true));
      } else {
        setState(() {});
      }
    });
  }

  /// Stop the recorder. Either ships the audio (if `send: true` and the
  /// clip is non-trivial) or discards it. Always resets the recording
  /// state so the mic button comes back.
  Future<void> _finishRecording({required bool send}) async {
    if (!_recording) return;
    final ticker = _recordingTicker;
    _recordingTicker = null;
    ticker?.cancel();

    final start = _recordingStart;
    String? path;
    try {
      path = await _recorder.stop();
    } catch (_) {
      path = _recordingPath;
    }
    final elapsedMs = start == null
        ? 0
        : DateTime.now().difference(start).inMilliseconds;
    final codecId = _recordingCodecId;

    setState(() {
      _recording = false;
      _cancelArmed = false;
      _recordingStart = null;
    });

    if (!send || _cancelArmed) {
      // Fire-and-forget delete.
      if (path != null) {
        unawaited(_safeDelete(path));
      }
      return;
    }
    // Drop micro-clips ( <300ms ) — almost certainly accidental taps.
    if (elapsedMs < 300 || path == null) {
      if (path != null) {
        unawaited(_safeDelete(path));
      }
      return;
    }
    await _sendVoice(path: path, durationMs: elapsedMs, codecId: codecId);
  }

  /// Best-effort delete of a recording-tmp file. Errors are swallowed —
  /// the OS will eventually evict the cache anyway.
  Future<void> _safeDelete(String path) async {
    try {
      await File(path).delete();
    } catch (_) {}
  }

  /// Encode + ship one recorded clip. Reads the on-disk file, wraps it
  /// in the `VOICE-1:` wire envelope, and hands it to `sendSealedV3` —
  /// same path text messages use, just with binary plaintext bytes.
  Future<void> _sendVoice({
    required String path,
    required int durationMs,
    required int codecId,
  }) async {
    setState(() => _sending = true);
    bool relayDelivered = false;
    String wireFingerprint = '';
    try {
      final audio = await File(path).readAsBytes();
      final wireEnv = voice.encodeVoiceWire(
        codecId: codecId,
        durationMs: durationMs,
        audio: audio,
      );
      // Persist a local copy under the canonical voice cache path so the
      // outgoing-bubble player can re-read it later. We move (rename)
      // the recorder's tmp file rather than re-writing the bytes.
      final ext = voice.extensionForCodec(codecId);
      final filename = 'tx_${DateTime.now().microsecondsSinceEpoch}.$ext';
      final canonical = '${(await voice.voiceCacheDir()).path}/$filename';
      try {
        await File(path).rename(canonical);
      } catch (_) {
        // Cross-fs rename can fail — copy + delete fallback.
        await File(canonical).writeAsBytes(audio);
        try {
          await File(path).delete();
        } catch (_) {}
      }
      // Hand the wire envelope to the sealed-sender layer.
      final wire = await rust.sendSealedV3(
        recipientAddress:
            'phantom:${widget.contact.publicViewKey}:${widget.contact.publicSpendKey}',
        plaintext: wireEnv,
      );
      wireFingerprint = wire
          .take(32)
          .map((b) => b.toRadixString(16).padLeft(2, '0'))
          .join();
      relayDelivered = await RelayService.instance.publish(wire);

      // Build the "voice ref" stored in the message row.
      final ref = 'voice://$codecId/$durationMs/$filename';
      final msg = PhantomMessage(
        id: _uuid.v4(),
        contactId: widget.contact.id,
        outgoing: true,
        plaintext: ref,
        ciphertext: wireFingerprint,
        ephemeralKey: '',
        nonce: '',
        timestamp: DateTime.now(),
        status: MessageStatus.sent,
      );
      await StorageService.addMessage(msg);
      widget.contact.lastMessage = '[${I18n.t('voice.label')}]';
      widget.contact.lastMessageAt = DateTime.now();
      if (!mounted) return;
      setState(() {
        _messages.add(msg);
        _sending = false;
      });
      _scrollToBottom();
      if (!relayDelivered) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('! VOICE NOT YET DELIVERED — NO RELAY ACK'),
            duration: Duration(seconds: 3),
          ),
        );
      }
    } catch (e) {
      if (!mounted) return;
      setState(() => _sending = false);
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('! VOICE SEND FAIL: $e')),
      );
    }
  }

  /// Surface a "Bind to contact" sheet when the most-recent inbound
  /// message arrived from a sealed-sender pubkey we don't have on file
  /// yet. Mirrors the Desktop's `bind_last_unbound_sender` action.
  Future<void> _showBindUnbound() async {
    if (ContactDirectory.lastUnboundSenderPubHex == null) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('> NO UNBOUND SENDER PENDING')),
      );
      return;
    }
    final result = await ContactDirectory.bindLastUnboundSender(
        widget.contact.nickname);
    if (!mounted) return;
    if (result.ok) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
            content: Text('> BOUND ${widget.contact.nickname}'.toUpperCase())),
      );
      setState(() {
        _lastUnboundSenderLabel = null;
      });
    } else {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('! ${result.error}')),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: kBg,
      body: GridBackground(
        child: SafeArea(
          child: Column(
            children: [
              _buildAppBar(),
              Container(height: 1, color: kCyan.withValues(alpha: 0.12)),
              Expanded(
                child: _messages.isEmpty ? _buildEmptyChat() : _buildMessages(),
              ),
              _buildInput(),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildAppBar() {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      child: Row(
        children: [
          GestureDetector(
            onTap: () => Navigator.pop(context),
            child: Container(
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                border: Border.all(color: kGray.withValues(alpha: 0.5)),
                color: kBgCard,
              ),
              child: const Icon(Icons.arrow_back_ios_new, color: kWhite, size: 14),
            ),
          ),
          const SizedBox(width: 14),
          Container(
            width: 38, height: 38,
            decoration: BoxDecoration(
              border: Border.all(color: kCyan.withValues(alpha: 0.5)),
              color: kCyanDim,
            ),
            child: Center(
              child: Text(
                widget.contact.nickname[0],
                style: GoogleFonts.orbitron(
                  fontSize: 16, fontWeight: FontWeight.w900,
                  color: kCyan,
                  shadows: [Shadow(color: kCyan.withValues(alpha: 0.6), blurRadius: 8)],
                ),
              ),
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  widget.contact.nickname.toUpperCase(),
                  style: GoogleFonts.orbitron(
                    fontSize: 14, fontWeight: FontWeight.w700,
                    color: kWhite, letterSpacing: 1,
                  ),
                ),
                Row(
                  children: [
                    const Icon(Icons.lock_outline, size: 10, color: kCyan),
                    const SizedBox(width: 4),
                    Text(
                      'E2E ENCRYPTED',
                      style: GoogleFonts.spaceMono(fontSize: 9, color: kCyan, letterSpacing: 1),
                    ),
                  ],
                ),
              ],
            ),
          ),
          if (_lastUnboundSenderLabel != null) ...[
            GestureDetector(
              onTap: _showBindUnbound,
              child: Container(
                padding: const EdgeInsets.symmetric(
                    horizontal: 10, vertical: 8),
                decoration: BoxDecoration(
                  border: Border.all(color: kMagenta.withValues(alpha: 0.6)),
                  color: kMagenta.withValues(alpha: 0.08),
                ),
                child: Text(
                  'BIND $_lastUnboundSenderLabel',
                  style: GoogleFonts.orbitron(
                      fontSize: 9, color: kMagenta, letterSpacing: 1),
                ),
              ),
            ),
            const SizedBox(width: 8),
          ],
          GestureDetector(
            onTap: _showContactInfo,
            child: Container(
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                border: Border.all(color: kGray.withValues(alpha: 0.3)),
                color: kBgCard,
              ),
              child: const Icon(Icons.info_outline, color: kGrayText, size: 16),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildEmptyChat() {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(40),
        child: CyberCard(
          borderColor: kGray,
          padding: const EdgeInsets.all(28),
          cut: 20,
          child: Column(
            children: [
              const Icon(Icons.lock_outline, color: kCyan, size: 36),
              const SizedBox(height: 16),
              Text(
                'ENCRYPTED CHANNEL\nINITIALIZED',
                textAlign: TextAlign.center,
                style: GoogleFonts.orbitron(
                  fontSize: 13, fontWeight: FontWeight.w700,
                  color: kCyan, letterSpacing: 2, height: 1.4,
                  shadows: [Shadow(color: kCyan.withValues(alpha: 0.4), blurRadius: 8)],
                ),
              ),
              const SizedBox(height: 12),
              Text(
                'X25519 + ChaCha20-Poly1305\nZero metadata. Zero logs.',
                textAlign: TextAlign.center,
                style: GoogleFonts.spaceMono(fontSize: 10, color: kGrayText, height: 1.6),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildMessages() {
    return ListView.builder(
      controller: _scrollCtrl,
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      itemCount: _messages.length,
      itemBuilder: (ctx, i) {
        final msg = _messages[i];
        final showDate = i == 0 || !_sameDay(_messages[i - 1].timestamp, msg.timestamp);
        return Column(
          children: [
            if (showDate) _DateDivider(date: msg.timestamp),
            _MsgBubble(message: msg),
          ],
        );
      },
    );
  }

  /// Press-origin captured on `onPointerDown` so `onPointerMove` can
  /// compute drag-left distance. Lives at the State level (not as a
  /// closure variable in `_buildMicButton`) so the value survives the
  /// rebuild that flips `_recording` true.
  Offset? _pressStart;

  Widget _buildInput() {
    return Container(
      padding: EdgeInsets.only(
        left: 14, right: 14, top: 10,
        bottom: MediaQuery.of(context).viewInsets.bottom + 14,
      ),
      decoration: BoxDecoration(
        color: kBgCard,
        border: Border(top: BorderSide(color: kCyan.withValues(alpha: 0.12))),
      ),
      // Stack lets us swap the textfield/recording-overlay without
      // rebuilding the mic-button Listener — the pointer event chain
      // started on press must keep pointing at the same Listener
      // instance for `onPointerUp` to fire as expected.
      child: Row(
        children: [
          Expanded(
            child: _recording ? _buildRecordingBar() : _buildTextBar(),
          ),
          const SizedBox(width: 10),
          if (!_recording &&
              _msgCtrl.text.trim().isNotEmpty)
            _buildSendButton()
          else
            _buildMicButton(),
        ],
      ),
    );
  }

  /// Default chat-input row (text field + lock indicator). The
  /// mic/send button is rendered separately by `_buildInput` so its
  /// `Listener` survives the textbar↔recordingbar swap.
  Widget _buildTextBar() {
    return Row(
      children: [
        // Encrypt indicator
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
          decoration: BoxDecoration(
            border: Border.all(color: kGreen.withValues(alpha: 0.4)),
            color: kGreen.withValues(alpha: 0.06),
          ),
          child: const Icon(Icons.lock_outline, color: kGreen, size: 14),
        ),
        const SizedBox(width: 10),
        Expanded(
          child: TextField(
            controller: _msgCtrl,
            style: GoogleFonts.spaceGrotesk(color: kWhite, fontSize: 15),
            // Trigger a rebuild on each keystroke so the mic↔send swap
            // stays in sync with the field's emptiness state.
            onChanged: (_) => setState(() {}),
            decoration: InputDecoration(
              hintText: '> TYPE MESSAGE...',
              hintStyle: GoogleFonts.spaceMono(color: kGrayText, fontSize: 12),
              border: OutlineInputBorder(
                borderRadius: BorderRadius.circular(4),
                borderSide: BorderSide(color: kGray.withValues(alpha: 0.5)),
              ),
              enabledBorder: OutlineInputBorder(
                borderRadius: BorderRadius.circular(4),
                borderSide: BorderSide(color: kGray.withValues(alpha: 0.4)),
              ),
              focusedBorder: OutlineInputBorder(
                borderRadius: BorderRadius.circular(4),
                borderSide: const BorderSide(color: kCyan, width: 1.5),
              ),
              filled: true,
              fillColor: kBgInput,
              contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
            ),
            onSubmitted: (_) => _send(),
            textInputAction: TextInputAction.send,
            maxLines: null,
          ),
        ),
      ],
    );
  }

  Widget _buildSendButton() {
    return GestureDetector(
      onTap: _send,
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 150),
        width: 44, height: 44,
        decoration: BoxDecoration(
          border: Border.all(color: _sending ? kGray : kCyan, width: 1.5),
          color: _sending ? kBgCard : kCyanDim,
          boxShadow: _sending ? null : neonGlow(kCyan, radius: 8),
        ),
        child: _sending
            ? const Padding(
                padding: EdgeInsets.all(12),
                child: CircularProgressIndicator(strokeWidth: 1.5, color: kCyan),
              )
            : const Icon(Icons.send_rounded, color: kCyan, size: 18),
      ),
    );
  }

  /// Press-and-hold mic. `Listener` (not `GestureDetector`) gives us
  /// `onPointerDown`/`onPointerUp`/`onPointerMove` raw events — robust
  /// against scroll-like gestures that would steal focus from a long
  /// press detector. We arm a "cancel" zone if the user drags more
  /// than 60px to the left of the press origin (familiar
  /// WhatsApp-style gesture). Stored on State so the value survives
  /// the rebuild that flips `_recording` true.
  Widget _buildMicButton() {
    final activeColor = _recording
        ? (_cancelArmed ? kMagenta : kRed)
        : kMagenta;
    return Listener(
      onPointerDown: (e) {
        _pressStart = e.position;
        unawaited(_startRecording());
      },
      onPointerMove: (e) {
        if (!_recording || _pressStart == null) return;
        final dx = e.position.dx - _pressStart!.dx;
        final shouldCancel = dx < -60;
        if (shouldCancel != _cancelArmed) {
          setState(() => _cancelArmed = shouldCancel);
        }
      },
      onPointerUp: (_) {
        _pressStart = null;
        unawaited(_finishRecording(send: !_cancelArmed));
      },
      onPointerCancel: (_) {
        _pressStart = null;
        unawaited(_finishRecording(send: false));
      },
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 150),
        width: 44, height: 44,
        decoration: BoxDecoration(
          border: Border.all(color: activeColor, width: 1.5),
          color: activeColor.withValues(alpha: 0.12),
          boxShadow: neonGlow(activeColor, radius: _recording ? 10 : 6),
        ),
        child: Tooltip(
          message: I18n.t('voice.pressToRecord'),
          child: Icon(
            _recording ? Icons.fiber_manual_record : Icons.mic,
            color: activeColor,
            size: _recording ? 16 : 20,
          ),
        ),
      ),
    );
  }

  /// Active-recording overlay shown in place of the text field. The
  /// mic-button itself is rendered by `_buildInput`, outside this
  /// widget, so pointer events stay attached to the same Listener
  /// across the textbar↔recordingbar swap.
  Widget _buildRecordingBar() {
    final start = _recordingStart;
    final elapsedMs =
        start == null ? 0 : DateTime.now().difference(start).inMilliseconds;
    final cancelHint = _cancelArmed
        ? I18n.t('voice.slideToCancel')
        : I18n.t('voice.releaseToSend');
    return Row(
      children: [
        const _RecordingDot(),
        const SizedBox(width: 10),
        Text(
          voice.formatDurationMs(elapsedMs),
          style: GoogleFonts.spaceMono(
            color: _cancelArmed ? kMagenta : kRed,
            fontSize: 13,
            fontWeight: FontWeight.w700,
          ),
        ),
        const SizedBox(width: 12),
        Expanded(
          child: Text(
            cancelHint,
            overflow: TextOverflow.ellipsis,
            style: GoogleFonts.spaceMono(
              color: kGrayText,
              fontSize: 11,
              letterSpacing: 1,
            ),
          ),
        ),
      ],
    );
  }

  void _showContactInfo() {
    showModalBottomSheet(
      context: context,
      backgroundColor: kBgCard,
      shape: const RoundedRectangleBorder(borderRadius: BorderRadius.zero),
      builder: (ctx) => Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(height: 1, color: kCyan.withValues(alpha: 0.3)),
            const SizedBox(height: 16),
            Text(
              'NODE_INFO //',
              style: GoogleFonts.orbitron(fontSize: 12, color: kCyan, letterSpacing: 2),
            ),
            const SizedBox(height: 16),
            _InfoRow('NAME', widget.contact.nickname.toUpperCase()),
            _InfoRow('VIEW_KEY', widget.contact.publicViewKey.substring(0, 24)),
            _InfoRow('SPEND_KEY', widget.contact.publicSpendKey.substring(0, 24)),
            _InfoRow(
              'ADDED',
              '${widget.contact.addedAt.day}.${widget.contact.addedAt.month}.${widget.contact.addedAt.year}',
            ),
            const SizedBox(height: 16),
            CyberCard(
              borderColor: kGreen.withValues(alpha: 0.4),
              bgColor: kGreen.withValues(alpha: 0.04),
              padding: const EdgeInsets.all(12),
              cut: 8,
              child: Row(
                children: [
                  const Icon(Icons.verified_outlined, color: kGreen, size: 14),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      'All messages encrypted with X25519 ECDH + ChaCha20-Poly1305. Zero server involvement.',
                      style: GoogleFonts.spaceMono(fontSize: 10, color: kGreen.withValues(alpha: 0.8), height: 1.5),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  bool _sameDay(DateTime a, DateTime b) =>
      a.year == b.year && a.month == b.month && a.day == b.day;
}

class _MsgBubble extends StatelessWidget {
  final PhantomMessage message;
  const _MsgBubble({required this.message});

  @override
  Widget build(BuildContext context) {
    final isOut = message.outgoing;
    // Detect a voice-message reference (`voice://<codec>/<ms>/<file>`)
    // stored in the plaintext slot. On a hit, swap the text body for a
    // play-button + duration. Long-press copies the wire ref so a
    // future debug "play in external app" can hook in without UI work.
    final voiceRef = voice.parseVoiceRef(message.plaintext);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Align(
        alignment: isOut ? Alignment.centerRight : Alignment.centerLeft,
        child: GestureDetector(
          onLongPress: () {
            Clipboard.setData(ClipboardData(text: message.plaintext));
            ScaffoldMessenger.of(context).showSnackBar(
              const SnackBar(content: Text('> COPIED')),
            );
          },
          child: ConstrainedBox(
            constraints: BoxConstraints(maxWidth: MediaQuery.of(context).size.width * 0.72),
            child: CustomPaint(
              painter: _BubblePainter(isOut: isOut),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.end,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (voiceRef != null)
                      _VoiceBubbleBody(ref: voiceRef, isOut: isOut)
                    else
                      Text(
                        message.plaintext,
                        style: GoogleFonts.spaceGrotesk(
                          fontSize: 14,
                          color: isOut ? kBg : kWhite,
                          height: 1.4,
                        ),
                      ),
                    const SizedBox(height: 4),
                    Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Text(
                          _fmt(message.timestamp),
                          style: GoogleFonts.spaceMono(
                            fontSize: 9,
                            color: isOut ? kBg.withValues(alpha: 0.5) : kGrayText,
                          ),
                        ),
                        if (isOut) ...[
                          const SizedBox(width: 4),
                          Icon(Icons.done_all, size: 10, color: kBg.withValues(alpha: 0.5)),
                        ],
                      ],
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  String _fmt(DateTime dt) =>
      '${dt.hour.toString().padLeft(2, '0')}:${dt.minute.toString().padLeft(2, '0')}';
}

/// Inline play-button + duration + progress bar for a voice message.
/// Owns its own [AudioPlayer] so two clips can play independently
/// (tap A, tap B → A pauses automatically because each player holds the
/// same OS audio focus, but the UI state stays per-row).
class _VoiceBubbleBody extends StatefulWidget {
  final voice.VoiceRef ref;
  final bool isOut;
  const _VoiceBubbleBody({required this.ref, required this.isOut});

  @override
  State<_VoiceBubbleBody> createState() => _VoiceBubbleBodyState();
}

class _VoiceBubbleBodyState extends State<_VoiceBubbleBody> {
  final AudioPlayer _player = AudioPlayer();
  bool _playing = false;
  Duration _position = Duration.zero;
  StreamSubscription<Duration>? _posSub;
  StreamSubscription<void>? _completeSub;
  StreamSubscription<PlayerState>? _stateSub;

  @override
  void initState() {
    super.initState();
    _posSub = _player.onPositionChanged.listen((p) {
      if (!mounted) return;
      setState(() => _position = p);
    });
    _completeSub = _player.onPlayerComplete.listen((_) {
      if (!mounted) return;
      setState(() {
        _playing = false;
        _position = Duration.zero;
      });
    });
    _stateSub = _player.onPlayerStateChanged.listen((s) {
      if (!mounted) return;
      setState(() => _playing = s == PlayerState.playing);
    });
  }

  @override
  void dispose() {
    _posSub?.cancel();
    _completeSub?.cancel();
    _stateSub?.cancel();
    unawaited(_player.dispose());
    super.dispose();
  }

  Future<void> _togglePlay() async {
    if (_playing) {
      await _player.pause();
      return;
    }
    final path = await widget.ref.resolvePath();
    final f = File(path);
    if (!await f.exists()) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('! ${I18n.t('voice.unavailable')}')),
      );
      return;
    }
    try {
      await _player.play(DeviceFileSource(path));
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('! PLAY FAIL: $e')),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final isOut = widget.isOut;
    final fg = isOut ? kBg : kCyan;
    final secondary = isOut ? kBg.withValues(alpha: 0.7) : kGrayText;
    final totalMs = widget.ref.durationMs;
    final progress = totalMs <= 0
        ? 0.0
        : (_position.inMilliseconds / totalMs).clamp(0.0, 1.0);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        GestureDetector(
          onTap: _togglePlay,
          child: Container(
            width: 32, height: 32,
            decoration: BoxDecoration(
              border: Border.all(color: fg.withValues(alpha: 0.6), width: 1.2),
              shape: BoxShape.circle,
              color: fg.withValues(alpha: 0.12),
            ),
            child: Icon(
              _playing ? Icons.pause : Icons.play_arrow,
              size: 18,
              color: fg,
            ),
          ),
        ),
        const SizedBox(width: 10),
        Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              I18n.t('voice.label'),
              style: GoogleFonts.orbitron(
                fontSize: 9,
                color: fg,
                letterSpacing: 1.2,
                fontWeight: FontWeight.w700,
              ),
            ),
            const SizedBox(height: 4),
            // Linear progress bar — fixed-width so it doesn't reflow
            // the bubble as the position changes.
            SizedBox(
              width: 120,
              child: LinearProgressIndicator(
                value: progress,
                minHeight: 3,
                backgroundColor: secondary.withValues(alpha: 0.25),
                valueColor: AlwaysStoppedAnimation(fg),
              ),
            ),
            const SizedBox(height: 4),
            Text(
              _playing
                  ? voice.formatDurationMs(_position.inMilliseconds)
                  : '${voice.formatDurationMs(totalMs)} · ${I18n.t('voice.tapToPlay')}',
              style: GoogleFonts.spaceMono(
                fontSize: 9,
                color: secondary,
              ),
            ),
          ],
        ),
      ],
    );
  }
}

/// Pulsing red dot used in the recording-overlay bar. Self-contained
/// `AnimationController` so it doesn't hold any State references and
/// can mount/unmount with the input-row swap.
class _RecordingDot extends StatefulWidget {
  const _RecordingDot();
  @override
  State<_RecordingDot> createState() => _RecordingDotState();
}

class _RecordingDotState extends State<_RecordingDot>
    with SingleTickerProviderStateMixin {
  late final AnimationController _ctrl;

  @override
  void initState() {
    super.initState();
    _ctrl = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 800),
    )..repeat(reverse: true);
  }

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _ctrl,
      builder: (ctx, _) => Container(
        width: 12, height: 12,
        decoration: BoxDecoration(
          shape: BoxShape.circle,
          color: kRed.withValues(alpha: 0.4 + 0.6 * _ctrl.value),
          boxShadow: [
            BoxShadow(
              color: kRed.withValues(alpha: 0.6 * _ctrl.value),
              blurRadius: 8,
            ),
          ],
        ),
      ),
    );
  }
}

class _BubblePainter extends CustomPainter {
  final bool isOut;
  _BubblePainter({required this.isOut});

  @override
  void paint(Canvas canvas, Size size) {
    const cut = 10.0;
    // Outgoing: cut top-left + bottom-right (like →), Incoming: cut top-right + bottom-left (like ←)
    final path = isOut
        ? (Path()
            ..moveTo(cut, 0)
            ..lineTo(size.width, 0)
            ..lineTo(size.width, size.height - cut)
            ..lineTo(size.width - cut, size.height)
            ..lineTo(0, size.height)
            ..lineTo(0, 0)
            ..close())
        : (Path()
            ..moveTo(0, 0)
            ..lineTo(size.width - cut, 0)
            ..lineTo(size.width, cut)
            ..lineTo(size.width, size.height)
            ..lineTo(cut, size.height)
            ..lineTo(0, size.height - cut)
            ..close());

    // Fill
    canvas.drawPath(
      path,
      Paint()..color = isOut ? kCyan : const Color(0xFF0D1520),
    );

    // Glow for outgoing
    if (isOut) {
      canvas.drawPath(
        path,
        Paint()
          ..color = kCyan.withValues(alpha: 0.2)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 4
          ..maskFilter = const MaskFilter.blur(BlurStyle.outer, 6),
      );
    }

    // Border
    canvas.drawPath(
      path,
      Paint()
        ..color = isOut ? kCyan.withValues(alpha: 0.6) : kGray.withValues(alpha: 0.4)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.0,
    );
  }

  @override
  bool shouldRepaint(_BubblePainter old) => old.isOut != isOut;
}

class _DateDivider extends StatelessWidget {
  final DateTime date;
  const _DateDivider({required this.date});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 12),
      child: Row(
        children: [
          Expanded(child: Container(height: 1, color: kGray.withValues(alpha: 0.3))),
          const SizedBox(width: 12),
          Text(
            '${date.day.toString().padLeft(2, '0')}.${date.month.toString().padLeft(2, '0')}.${date.year}',
            style: GoogleFonts.spaceMono(fontSize: 10, color: kGrayText),
          ),
          const SizedBox(width: 12),
          Expanded(child: Container(height: 1, color: kGray.withValues(alpha: 0.3))),
        ],
      ),
    );
  }
}

class _InfoRow extends StatelessWidget {
  final String label;
  final String value;
  const _InfoRow(this.label, this.value);

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        children: [
          Text(
            label,
            style: GoogleFonts.spaceMono(fontSize: 10, color: kCyan, letterSpacing: 1),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              value,
              style: GoogleFonts.spaceMono(fontSize: 10, color: kGrayText),
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
  }
}
