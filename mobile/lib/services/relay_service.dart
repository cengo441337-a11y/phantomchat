// PhantomChat mobile relay listener service.
//
// Mirrors the Tauri Desktop's `start_listener` prefix-dispatch logic so a
// v3.0.0 Desktop user sending to a v3.x mobile user gets the right UI
// reaction for each wire prefix instead of the undecodable-garbage rendering
// that bit users in the v2 mobile build:
//
//   MLS-WLC2  → mlsJoinViaWelcome + mlsDirectoryInsert + 'mls_joined' event
//   MLS-WLC1  → legacy v1 fallback (placeholder inviter, joined event)
//   MLS-APP1  → mlsDecrypt + 'mls_message' / 'mls_epoch'
//   FILE1:01  → 'system' event with "file received, switch to Desktop"
//                 (file-transfer write-to-storage deferred to wave 7B-followup)
//   RCPT-1:   → 'receipt' event { msgId, kind: delivered|read }
//   TYPN-1:   → 'typing' event { fromLabel, ttlSecs }
//   (no prefix) → 'message' event with sealed-sender attribution
//
// Outgoing wire format encoding (build_*_payload helpers) lives here too so
// the Channels tab + 1:1 chat screens can ship the same shapes the Desktop
// expects on its receive side.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:path_provider/path_provider.dart';
import 'package:web_socket_channel/io.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import '../src/rust/api.dart' as rust;
import 'contact_directory.dart';

/// 8-byte ASCII tag prefixed to v1 MLS Welcome bytes. Carries no inviter
/// metadata; joiner displays inviter as `?<8hex>` until they reply
/// through their own contact path.
final Uint8List kMlsWlcPrefixV1 =
    Uint8List.fromList(utf8.encode('MLS-WLC1'));

/// 8-byte ASCII tag prefixed to v2 MLS Welcome bytes. Wire format:
///
///   MLS-WLC2 || ULEB128(meta_len) || meta_json || welcome_bytes
final Uint8List kMlsWlcPrefixV2 =
    Uint8List.fromList(utf8.encode('MLS-WLC2'));

/// 8-byte ASCII tag prefixed to MLS application/commit wire bytes.
final Uint8List kMlsAppPrefix = Uint8List.fromList(utf8.encode('MLS-APP1'));

/// 8-byte ASCII tag prefixed to file-transfer payloads. Wire format:
///
///   FILE1:01 || ULEB128(meta_len) || meta_json || raw_bytes
final Uint8List kFilePrefixV1 = Uint8List.fromList(utf8.encode('FILE1:01'));

/// 7-byte ASCII tag prefixed to delivery / read receipt envelopes.
final Uint8List kRcptPrefixV1 = Uint8List.fromList(utf8.encode('RCPT-1:'));

/// 7-byte ASCII tag prefixed to typing-indicator envelopes.
final Uint8List kTypnPrefixV1 = Uint8List.fromList(utf8.encode('TYPN-1:'));

const int kTypingTtlSecs = 5;

// ── Event envelope shipped to the UI ────────────────────────────────────────

/// One decoded relay event. `kind` mirrors the Tauri `app.emit("&lt;kind&gt;",
/// ...)` channel names so a future shared widget layer can dispatch on the
/// same string vocab without translation.
class RelayEvent {
  /// One of: "message", "mls_joined", "mls_message", "mls_epoch",
  /// "file_received", "receipt", "typing", "system", "error".
  final String kind;
  final Map<String, dynamic> payload;
  RelayEvent(this.kind, this.payload);

  @override
  String toString() => 'RelayEvent($kind, $payload)';
}

/// Singleton service. Holds the Dart-side stream sink the UI listens to.
/// On startup, [connect] opens one persistent WebSocket per configured
/// Nostr relay URL; the same channels are re-used for both directions
/// (subscribe + publish). Inbound `["EVENT", ...]` frames are handed to
/// the existing [feedEnvelope] dispatcher; outbound bytes are wrapped via
/// the Rust `nostr_build_event` helper and fanned out to every connected
/// relay in parallel by [publish].
class RelayService {
  RelayService._();
  static final RelayService instance = RelayService._();

  final StreamController<RelayEvent> _controller =
      StreamController<RelayEvent>.broadcast();

  Stream<RelayEvent> get events => _controller.stream;

  /// Sane defaults — same well-known Nostr relays the Desktop's
  /// `relays/src/lib.rs` factory uses. Persisted relay list overrides
  /// these on subsequent launches.
  static const List<String> defaultRelayUrls = <String>[
    'wss://relay.damus.io',
    'wss://nos.lol',
    'wss://relay.snort.social',
  ];

  /// One persistent connection slot per configured relay URL. A `null`
  /// value means the slot is currently waiting on a reconnect backoff.
  final Map<String, _RelayConn> _relayConns = <String, _RelayConn>{};

  /// Subscription id we send in our `REQ` frame (and which incoming
  /// `EVENT` frames echo back). Constant per process; relays don't care.
  static const String _subId = 'phantom-mob-1';

  /// In-flight publish acks, keyed by Nostr event id. The publish call
  /// completes the inner completer when EITHER the first relay sends an
  /// `["OK", <id>, true, …]` OR the 5s timeout fires.
  final Map<String, Completer<bool>> _pendingAcks =
      <String, Completer<bool>>{};

  /// Have we kicked off [connect] yet? Idempotent guard so multiple
  /// screens calling `RelayService.instance.connect()` don't spawn N
  /// stacks of WebSockets.
  bool _connected = false;

  /// True if at least one underlying WebSocket is currently open. Drives
  /// the `> NO RELAYS REACHABLE` UI banner if it stays false for >5s
  /// after a publish attempt.
  bool get hasAnyConnection =>
      _relayConns.values.any((c) => c.isOpen);

  /// Snapshot of the configured relay URL list. Returned for UI status
  /// surfaces.
  List<String> get relayUrls => List.unmodifiable(_relayConns.keys);

  /// Feed one inbound wire blob (the bytes a relay handed us). The service
  /// runs `receiveFullV3` to decrypt + extract sealed-sender attribution,
  /// peeks for a known prefix, and emits the right [RelayEvent]. Envelopes
  /// not addressed to the local identity silently drop.
  Future<void> feedEnvelope(Uint8List wireBytes) async {
    rust.ReceivedFullV3? msg;
    try {
      msg = await rust.receiveFullV3(wireBytes: wireBytes);
    } catch (e) {
      _controller.add(RelayEvent('error', {'detail': 'receiveFullV3: $e'}));
      return;
    }
    if (msg == null) return;
    final plaintext = msg.plaintext;
    final senderPubHex = msg.senderPubHex;
    final sigOk = msg.sigOk;

    // ── 8-byte prefixes (MLS / FILE) ────────────────────────────────────────
    if (plaintext.length >= kMlsWlcPrefixV1.length) {
      final prefix = plaintext.sublist(0, kMlsWlcPrefixV1.length);
      if (_eq(prefix, kMlsWlcPrefixV2)) {
        await _handleMlsWelcomeV2(
          plaintext.sublist(kMlsWlcPrefixV2.length),
          senderPubHex,
          sigOk,
        );
        return;
      }
      if (_eq(prefix, kMlsWlcPrefixV1)) {
        await _handleMlsWelcomeV1(
          plaintext.sublist(kMlsWlcPrefixV1.length),
          senderPubHex,
          sigOk,
        );
        return;
      }
      if (_eq(prefix, kMlsAppPrefix)) {
        await _handleMlsApp(
          plaintext.sublist(kMlsAppPrefix.length),
          senderPubHex,
          sigOk,
        );
        return;
      }
      if (_eq(prefix, kFilePrefixV1)) {
        _handleFileV1(
          plaintext.sublist(kFilePrefixV1.length),
          senderPubHex,
          sigOk,
        );
        return;
      }
    }

    // ── 7-byte prefixes (RCPT / TYPN) ────────────────────────────────────────
    if (plaintext.length >= kRcptPrefixV1.length) {
      final prefix7 = plaintext.sublist(0, kRcptPrefixV1.length);
      if (_eq(prefix7, kRcptPrefixV1)) {
        _handleReceiptV1(plaintext.sublist(kRcptPrefixV1.length), senderPubHex,
            sigOk);
        return;
      }
      if (_eq(prefix7, kTypnPrefixV1)) {
        _handleTypingV1(plaintext.sublist(kTypnPrefixV1.length), senderPubHex,
            sigOk);
        return;
      }
    }

    // ── Plain 1:1 text (no prefix) ──────────────────────────────────────────
    final senderLabel = await _resolveSenderLabel(senderPubHex, sigOk);
    final ts = _now();
    _controller.add(RelayEvent('message', {
      'plaintext': utf8.decode(plaintext, allowMalformed: true),
      'timestamp': ts,
      'senderLabel': senderLabel.label,
      'sigOk': sigOk,
      'senderPubHex': senderPubHex,
      'msgId': computeMsgId(plaintext),
      'isUnbound': senderLabel.isUnbound,
    }));
  }

  // ── Prefix handlers ────────────────────────────────────────────────────────

  Future<void> _handleMlsWelcomeV2(
    Uint8List body,
    String? senderPubHex,
    bool sigOk,
  ) async {
    final decoded = decodeMlsWelcomeV2(body);
    if (decoded == null) {
      _controller.add(
          RelayEvent('error', {'detail': 'MLS-WLC2 decode failed'}));
      return;
    }
    final meta = decoded.$1;
    final welcome = decoded.$2;

    // Promote the inviter into the bundle directory BEFORE joining so the
    // very first incoming app message resolves to the human label.
    try {
      await rust.mlsDirectoryInsert(
        label: meta['inviter_label'] as String,
        address: meta['inviter_address'] as String,
        signingPubHex: meta['inviter_signing_pub_hex'] as String,
      );
    } catch (e) {
      _controller.add(
          RelayEvent('error', {'detail': 'mls_directory_insert: $e'}));
      return;
    }

    final fromLabel = (senderPubHex != null && !sigOk)
        ? 'INBOX!'
        : (meta['inviter_label'] as String);

    int memberCount;
    try {
      memberCount = await rust.mlsJoinViaWelcome(welcomeBytes: welcome);
    } catch (e) {
      _controller.add(
          RelayEvent('error', {'detail': 'mls_join_via_welcome: $e'}));
      return;
    }
    _controller.add(RelayEvent('mls_joined', {
      'fromLabel': fromLabel,
      'memberCount': memberCount,
    }));
  }

  Future<void> _handleMlsWelcomeV1(
    Uint8List welcome,
    String? senderPubHex,
    bool sigOk,
  ) async {
    // No inviter metadata — render the inviter as `?<8hex>` to match
    // Desktop's legacy v1 path.
    final fromLabel = senderPubHex == null
        ? 'INBOX'
        : (!sigOk
            ? 'INBOX!'
            : '?${senderPubHex.substring(0, 8.clamp(0, senderPubHex.length))}');
    int memberCount;
    try {
      memberCount = await rust.mlsJoinViaWelcome(welcomeBytes: welcome);
    } catch (e) {
      _controller.add(
          RelayEvent('error', {'detail': 'mls_join_via_welcome (v1): $e'}));
      return;
    }
    _controller.add(RelayEvent('mls_joined', {
      'fromLabel': fromLabel,
      'memberCount': memberCount,
    }));
  }

  Future<void> _handleMlsApp(
    Uint8List wire,
    String? senderPubHex,
    bool sigOk,
  ) async {
    // Resolve sender via MLS directory (matches Desktop's
    // `resolve_mls_from_label`).
    String fromLabel;
    if (senderPubHex == null) {
      fromLabel = 'GROUP';
    } else if (!sigOk) {
      fromLabel = 'GROUP!';
    } else {
      try {
        final dir = await rust.mlsDirectory();
        final hit = dir.where(
          (m) => m.signingPubHex.toLowerCase() == senderPubHex.toLowerCase(),
        );
        fromLabel = hit.isNotEmpty
            ? hit.first.label
            : '?${senderPubHex.substring(0, 8.clamp(0, senderPubHex.length))}';
      } catch (_) {
        fromLabel =
            '?${senderPubHex.substring(0, 8.clamp(0, senderPubHex.length))}';
      }
    }

    Uint8List? plain;
    try {
      plain = await rust.mlsDecrypt(wireBytes: wire);
    } catch (e) {
      _controller.add(RelayEvent('error', {'detail': 'mls_decrypt: $e'}));
      return;
    }

    final ts = _now();
    final memberCount = rust.mlsMemberCount();
    if (plain == null) {
      _controller.add(RelayEvent('mls_epoch', {
        'memberCount': memberCount,
      }));
      return;
    }
    _controller.add(RelayEvent('mls_message', {
      'fromLabel': fromLabel,
      'plaintext': utf8.decode(plain, allowMalformed: true),
      'ts': ts,
      'memberCount': memberCount,
    }));
  }

  void _handleFileV1(
    Uint8List body,
    String? senderPubHex,
    bool sigOk,
  ) {
    // Wave-7B mobile catch-up: surface a system message rather than writing
    // to phone storage. File-transfer remains a Desktop-only feature
    // pending a phone-storage permission story (deferred to wave
    // 7B-followup).
    final decoded = decodeUleb128PrefixedJson(body);
    String filename = '<unknown>';
    int size = 0;
    if (decoded != null) {
      try {
        final meta = jsonDecode(utf8.decode(decoded.$1)) as Map<String, dynamic>;
        filename = (meta['filename'] as String?) ?? filename;
        size = (meta['size'] as num?)?.toInt() ?? size;
      } catch (_) {}
    }
    final fromHint = senderPubHex == null
        ? 'INBOX'
        : (!sigOk
            ? 'INBOX!'
            : '?${senderPubHex.substring(0, 8.clamp(0, senderPubHex.length))}');
    _controller.add(RelayEvent('system', {
      'plaintext':
          'File received from $fromHint: $filename (${_humanSize(size)}) — file transfer not yet supported on mobile, switch to Desktop.',
      'timestamp': _now(),
      'senderLabel': fromHint,
    }));
  }

  void _handleReceiptV1(
    Uint8List body,
    String? senderPubHex,
    bool sigOk,
  ) {
    if (senderPubHex == null || !sigOk) return; // useless without attribution
    final decoded = decodeUleb128PrefixedJson(body);
    if (decoded == null) return;
    Map<String, dynamic> meta;
    try {
      meta = jsonDecode(utf8.decode(decoded.$1)) as Map<String, dynamic>;
    } catch (_) {
      return;
    }
    final msgId = meta['msg_id'] as String?;
    final kind = meta['kind'] as String?;
    if (msgId == null || (kind != 'delivered' && kind != 'read')) return;
    _controller.add(RelayEvent('receipt', {
      'fromPubHex': senderPubHex,
      'msgId': msgId,
      'kind': kind,
    }));
  }

  void _handleTypingV1(
    Uint8List body,
    String? senderPubHex,
    bool sigOk,
  ) {
    if (senderPubHex == null || !sigOk) return;
    final decoded = decodeUleb128PrefixedJson(body);
    if (decoded == null) return;
    Map<String, dynamic> meta;
    try {
      meta = jsonDecode(utf8.decode(decoded.$1)) as Map<String, dynamic>;
    } catch (_) {
      return;
    }
    final ttl = ((meta['ttl_secs'] as num?)?.toInt() ?? kTypingTtlSecs)
        .clamp(1, 30);
    _controller.add(RelayEvent('typing', {
      'fromPubHex': senderPubHex,
      'ttlSecs': ttl,
    }));
  }

  // ── Sender attribution (1:1 text path) ─────────────────────────────────────

  Future<({String label, bool isUnbound})> _resolveSenderLabel(
    String? senderPubHex,
    bool sigOk,
  ) async {
    if (senderPubHex == null) return (label: 'INBOX', isUnbound: false);
    if (!sigOk) return (label: 'INBOX!', isUnbound: false);
    final hex = senderPubHex.toLowerCase();
    final book = await ContactDirectory.load();
    for (final c in book) {
      final cHex = c.signingPubHex?.toLowerCase();
      if (cHex != null && cHex == hex) {
        return (label: c.label, isUnbound: false);
      }
    }
    // Stash for the upcoming `bindLastUnboundSender` call (mirrors Desktop's
    // `last_unbound_sender` AppState slot).
    ContactDirectory.lastUnboundSenderPubHex = hex;
    return (label: '?${hex.substring(0, 8)}', isUnbound: true);
  }

  String _now() {
    final n = DateTime.now();
    String two(int v) => v.toString().padLeft(2, '0');
    return '${two(n.hour)}:${two(n.minute)}:${two(n.second)}';
  }

  // ── Connection management ────────────────────────────────────────────────
  //
  // Per-relay persistent WebSocket. The first call wins — subsequent calls
  // return immediately so screen-rebuilds don't pile up sockets. Pass
  // `force: true` from a settings-screen "reconnect now" button when we
  // eventually ship one (deferred — see PR description).

  /// Open one WebSocket per `relayUrls`. Idempotent. Uses
  /// [defaultRelayUrls] if the persisted [relays.json] file doesn't exist
  /// yet (or the caller passes an empty list). Each socket auto-reconnects
  /// with exponential backoff (1s → 60s, capped, ±jitter) on close/error,
  /// and re-sends the subscription `REQ` on every successful reconnect.
  Future<void> connect({List<String>? relayUrls}) async {
    if (_connected) return;
    _connected = true;
    final urls = relayUrls ?? await loadPersistedRelayUrls();
    for (final url in urls) {
      final conn = _RelayConn(url, _onWsFrame, _onWsStateChange);
      _relayConns[url] = conn;
      // Kicks off the connect-loop in a microtask; doesn't block here so
      // the UI thread stays responsive even if all 3 relays are slow.
      conn.start();
    }
  }

  /// Tear down every open socket. Called on app shutdown / sign-out so we
  /// don't leak background reconnect timers across hot-restart in dev.
  Future<void> disconnect() async {
    for (final c in _relayConns.values) {
      await c.close();
    }
    _relayConns.clear();
    _connected = false;
  }

  /// Wrap `wireBytes` in a NIP-01 EVENT (kind 1059, hex content) and
  /// broadcast to every connected relay. Returns `true` if AT LEAST ONE
  /// relay sent an `["OK", <id>, true, …]` ack inside [timeout]. Returns
  /// `false` if no socket was open OR every relay either rejected or
  /// timed out.
  Future<bool> publish(
    Uint8List wireBytes, {
    Duration timeout = const Duration(seconds: 5),
  }) async {
    final openConns =
        _relayConns.values.where((c) => c.isOpen).toList(growable: false);
    if (openConns.isEmpty) {
      return false;
    }
    rust.NostrEventWire wrapped;
    try {
      wrapped = await rust.nostrBuildEvent(envelopeBytes: wireBytes);
    } catch (e) {
      _controller.add(RelayEvent('error', {'detail': 'nostr_build_event: $e'}));
      return false;
    }
    final completer = Completer<bool>();
    _pendingAcks[wrapped.eventId] = completer;
    // Timeout arms here — completes false if no relay acks in time.
    final timer = Timer(timeout, () {
      if (!completer.isCompleted) {
        _pendingAcks.remove(wrapped.eventId);
        completer.complete(false);
      }
    });
    // Fire all sends in parallel; per-socket failures are logged but
    // don't abort the publish (another relay may still ack).
    for (final c in openConns) {
      try {
        c.send(wrapped.publishJson);
      } catch (e) {
        _controller.add(RelayEvent('error',
            {'detail': 'publish to ${c.url}: $e'}));
      }
    }
    final result = await completer.future;
    timer.cancel();
    return result;
  }

  // ── Persistence (relays.json) ────────────────────────────────────────────
  //
  // Stored next to the identity file in ApplicationDocumentsDirectory.
  // Same shape as Desktop's `relays.json` — a JSON array of URL strings
  // — so a future shared-storage path can read either.

  static Future<File> _relayConfigFile() async {
    final dir = await getApplicationDocumentsDirectory();
    return File('${dir.path}/relays.json');
  }

  /// Load the persisted relay URL list, or [defaultRelayUrls] on first
  /// launch. The first-launch case writes the defaults back so the file
  /// shows up in storage inspectors immediately.
  static Future<List<String>> loadPersistedRelayUrls() async {
    try {
      final f = await _relayConfigFile();
      if (!await f.exists()) {
        await savePersistedRelayUrls(defaultRelayUrls);
        return defaultRelayUrls;
      }
      final raw = await f.readAsString();
      final list = (jsonDecode(raw) as List<dynamic>)
          .map((e) => e as String)
          .toList();
      if (list.isEmpty) return defaultRelayUrls;
      return list;
    } catch (_) {
      // Corrupt file / permission denied — fall back to defaults rather
      // than leaving the app silent (defaults still publish anywhere).
      return defaultRelayUrls;
    }
  }

  /// Persist the relay URL list. Idempotent — overwrites the file.
  static Future<void> savePersistedRelayUrls(List<String> urls) async {
    try {
      final f = await _relayConfigFile();
      await f.writeAsString(jsonEncode(urls));
    } catch (_) {
      // Best-effort. A failure here only costs us the defaults on next
      // launch; not a crash-worthy condition.
    }
  }

  // ── Socket frame dispatch ────────────────────────────────────────────────

  /// Per-socket text-frame callback. Splits Nostr command frames:
  ///
  ///   `["EVENT", subId, eventObj]`  → extract envelope + feedEnvelope
  ///   `["OK", eventId, accepted, msg]` → resolve pending publish ack
  ///   `["EOSE", subId]` / `["NOTICE", msg]` → ignored (no UI surface)
  Future<void> _onWsFrame(String url, String text) async {
    // Cheap prefix peek before paying the JSON parse cost. Most frames in
    // a busy chat are EVENT, so check that one first.
    if (text.startsWith('["EVENT"')) {
      Uint8List? envelope;
      try {
        envelope = await rust.nostrExtractEventPayload(frameText: text);
      } catch (e) {
        _controller.add(RelayEvent(
            'error', {'detail': 'nostr_extract_event_payload: $e'}));
        return;
      }
      if (envelope == null) return;
      // feedEnvelope handles its own error reporting via the events stream.
      await feedEnvelope(envelope);
      return;
    }
    if (text.startsWith('["OK"')) {
      try {
        final arr = jsonDecode(text) as List<dynamic>;
        if (arr.length >= 3) {
          final eventId = arr[1] as String;
          final accepted = arr[2] as bool;
          final completer = _pendingAcks.remove(eventId);
          if (completer != null && !completer.isCompleted) {
            completer.complete(accepted);
          }
        }
      } catch (_) {
        // Malformed OK — ignore; the timeout will close the publish.
      }
      return;
    }
    // EOSE / NOTICE / unknown — silent drop is fine for v1.
  }

  /// Per-socket state change. Currently only used to surface a `system`
  /// event with the connect/disconnect transition so the chat screens can
  /// flash a banner. Keep it best-effort — losing one of three relays is
  /// not user-actionable.
  void _onWsStateChange(String url, _ConnState state) {
    // Re-send the subscription REQ on every successful (re)connect so a
    // recovering link starts streaming events immediately.
    if (state == _ConnState.connected) {
      final c = _relayConns[url];
      if (c != null) {
        unawaited(_resendSubscription(c));
      }
    }
  }

  Future<void> _resendSubscription(_RelayConn c) async {
    try {
      final req = await rust.nostrSubscriptionReq(subId: _subId);
      // The connection may have flipped to disconnected during the await
      // of the (sync-on-Rust-side but async-over-FRB) `nostrSubscriptionReq`
      // call; `send` itself raises StateError which we swallow.
      c.send(req);
    } catch (_) {/* connect-state may have flipped already */}
  }
}

// ── Wire encoders / decoders (parity with Tauri side) ───────────────────────

/// Decode the `MLS-WLC2` body (after the 8-byte prefix has been stripped).
/// Returns `(meta_json_decoded, welcome_bytes)`.
(Map<String, dynamic>, Uint8List)? decodeMlsWelcomeV2(Uint8List body) {
  final lp = readUleb128(body);
  if (lp == null) return null;
  final metaLen = lp.$1;
  final consumed = lp.$2;
  final metaEnd = consumed + metaLen;
  if (body.length < metaEnd) return null;
  Map<String, dynamic> meta;
  try {
    meta = jsonDecode(utf8.decode(body.sublist(consumed, metaEnd)))
        as Map<String, dynamic>;
  } catch (_) {
    return null;
  }
  return (meta, body.sublist(metaEnd));
}

/// Encode a `MLS-WLC2` welcome wrapping payload from raw welcome bytes
/// + inviter meta. Output is the full payload — the caller wraps it in a
/// sealed-sender envelope with `sendSealedV3`.
Uint8List encodeMlsWelcomeV2({
  required String inviterLabel,
  required String inviterAddress,
  required String inviterSigningPubHex,
  required Uint8List welcomeBytes,
}) {
  final metaJson = utf8.encode(jsonEncode({
    'inviter_label': inviterLabel,
    'inviter_address': inviterAddress,
    'inviter_signing_pub_hex': inviterSigningPubHex,
  }));
  final out = BytesBuilder(copy: false);
  out.add(kMlsWlcPrefixV2);
  writeUleb128(out, metaJson.length);
  out.add(metaJson);
  out.add(welcomeBytes);
  return out.toBytes();
}

/// Encode a generic `<prefix> || ULEB128(meta_len) || meta_json` wire
/// frame for `RCPT-1:` / `TYPN-1:` / `FILE1:01`-meta cases.
Uint8List encodePrefixedJson(Uint8List prefix, Map<String, dynamic> meta) {
  final metaJson = utf8.encode(jsonEncode(meta));
  final out = BytesBuilder(copy: false);
  out.add(prefix);
  writeUleb128(out, metaJson.length);
  out.add(metaJson);
  return out.toBytes();
}

/// Decode `ULEB128(meta_len) || meta_json` and return `(json_bytes, rest)`.
/// Used by FILE1:01 / RCPT-1: / TYPN-1: receivers.
(Uint8List, Uint8List)? decodeUleb128PrefixedJson(Uint8List body) {
  final lp = readUleb128(body);
  if (lp == null) return null;
  final metaLen = lp.$1;
  final consumed = lp.$2;
  final metaEnd = consumed + metaLen;
  if (body.length < metaEnd) return null;
  return (body.sublist(consumed, metaEnd), body.sublist(metaEnd));
}

void writeUleb128(BytesBuilder out, int value) {
  var v = value;
  while (true) {
    final b = v & 0x7F;
    v >>= 7;
    if (v == 0) {
      out.addByte(b);
      return;
    }
    out.addByte(b | 0x80);
  }
}

(int, int)? readUleb128(Uint8List input) {
  var value = 0;
  var shift = 0;
  for (var i = 0; i < input.length; i++) {
    final b = input[i];
    value |= (b & 0x7F) << shift;
    if (b & 0x80 == 0) return (value, i + 1);
    shift += 7;
    if (shift >= 64) return null;
  }
  return null;
}

/// Stable per-message identifier — same recipe as Desktop's `compute_msg_id`
/// (sha256("v1|" || hex(plaintext)) truncated to 16 hex chars). Plaintext-
/// only so sender + receiver agree across the second boundary.
String computeMsgId(Uint8List plaintext) {
  // Lightweight sha256 via package:crypto would add a transitive; do it
  // ourselves to keep the dep surface tight.
  final hex = StringBuffer();
  for (final b in plaintext) {
    hex.write(b.toRadixString(16).padLeft(2, '0'));
  }
  final input = utf8.encode('v1|${hex.toString()}');
  final digest = _sha256(input);
  final hexOut = StringBuffer();
  for (final b in digest.take(8)) {
    hexOut.write(b.toRadixString(16).padLeft(2, '0'));
  }
  return hexOut.toString();
}

bool _eq(Uint8List a, Uint8List b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

String _humanSize(int bytes) {
  if (bytes < 1024) return '$bytes B';
  if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
  return '${(bytes / (1024 * 1024)).toStringAsFixed(2)} MB';
}

// ── Pure-Dart SHA-256 (avoids pulling package:crypto) ───────────────────────
// Tiny implementation; only used for the 16-hex msg_id in computeMsgId. Not
// constant-time, not for crypto-grade work.

const List<int> _k = [
  0x428a2f98,
  0x71374491,
  0xb5c0fbcf,
  0xe9b5dba5,
  0x3956c25b,
  0x59f111f1,
  0x923f82a4,
  0xab1c5ed5,
  0xd807aa98,
  0x12835b01,
  0x243185be,
  0x550c7dc3,
  0x72be5d74,
  0x80deb1fe,
  0x9bdc06a7,
  0xc19bf174,
  0xe49b69c1,
  0xefbe4786,
  0x0fc19dc6,
  0x240ca1cc,
  0x2de92c6f,
  0x4a7484aa,
  0x5cb0a9dc,
  0x76f988da,
  0x983e5152,
  0xa831c66d,
  0xb00327c8,
  0xbf597fc7,
  0xc6e00bf3,
  0xd5a79147,
  0x06ca6351,
  0x14292967,
  0x27b70a85,
  0x2e1b2138,
  0x4d2c6dfc,
  0x53380d13,
  0x650a7354,
  0x766a0abb,
  0x81c2c92e,
  0x92722c85,
  0xa2bfe8a1,
  0xa81a664b,
  0xc24b8b70,
  0xc76c51a3,
  0xd192e819,
  0xd6990624,
  0xf40e3585,
  0x106aa070,
  0x19a4c116,
  0x1e376c08,
  0x2748774c,
  0x34b0bcb5,
  0x391c0cb3,
  0x4ed8aa4a,
  0x5b9cca4f,
  0x682e6ff3,
  0x748f82ee,
  0x78a5636f,
  0x84c87814,
  0x8cc70208,
  0x90befffa,
  0xa4506ceb,
  0xbef9a3f7,
  0xc67178f2,
];

int _rotr(int x, int n) =>
    ((x >> n) | (x << (32 - n))) & 0xFFFFFFFF;

Uint8List _sha256(List<int> data) {
  final padded = _padSha256(data);
  var h0 = 0x6a09e667;
  var h1 = 0xbb67ae85;
  var h2 = 0x3c6ef372;
  var h3 = 0xa54ff53a;
  var h4 = 0x510e527f;
  var h5 = 0x9b05688c;
  var h6 = 0x1f83d9ab;
  var h7 = 0x5be0cd19;

  for (var blk = 0; blk < padded.length; blk += 64) {
    final w = List<int>.filled(64, 0);
    for (var i = 0; i < 16; i++) {
      w[i] = (padded[blk + i * 4] << 24) |
          (padded[blk + i * 4 + 1] << 16) |
          (padded[blk + i * 4 + 2] << 8) |
          padded[blk + i * 4 + 3];
      w[i] &= 0xFFFFFFFF;
    }
    for (var i = 16; i < 64; i++) {
      final s0 = _rotr(w[i - 15], 7) ^ _rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
      final s1 =
          _rotr(w[i - 2], 17) ^ _rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) & 0xFFFFFFFF;
    }
    var a = h0, b = h1, c = h2, d = h3;
    var e = h4, f = h5, g = h6, h = h7;
    for (var i = 0; i < 64; i++) {
      final s1 = _rotr(e, 6) ^ _rotr(e, 11) ^ _rotr(e, 25);
      final ch = (e & f) ^ ((~e & 0xFFFFFFFF) & g);
      final temp1 = (h + s1 + ch + _k[i] + w[i]) & 0xFFFFFFFF;
      final s0 = _rotr(a, 2) ^ _rotr(a, 13) ^ _rotr(a, 22);
      final maj = (a & b) ^ (a & c) ^ (b & c);
      final temp2 = (s0 + maj) & 0xFFFFFFFF;
      h = g;
      g = f;
      f = e;
      e = (d + temp1) & 0xFFFFFFFF;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) & 0xFFFFFFFF;
    }
    h0 = (h0 + a) & 0xFFFFFFFF;
    h1 = (h1 + b) & 0xFFFFFFFF;
    h2 = (h2 + c) & 0xFFFFFFFF;
    h3 = (h3 + d) & 0xFFFFFFFF;
    h4 = (h4 + e) & 0xFFFFFFFF;
    h5 = (h5 + f) & 0xFFFFFFFF;
    h6 = (h6 + g) & 0xFFFFFFFF;
    h7 = (h7 + h) & 0xFFFFFFFF;
  }
  final out = Uint8List(32);
  void write(int v, int off) {
    out[off] = (v >> 24) & 0xFF;
    out[off + 1] = (v >> 16) & 0xFF;
    out[off + 2] = (v >> 8) & 0xFF;
    out[off + 3] = v & 0xFF;
  }

  write(h0, 0);
  write(h1, 4);
  write(h2, 8);
  write(h3, 12);
  write(h4, 16);
  write(h5, 20);
  write(h6, 24);
  write(h7, 28);
  return out;
}

Uint8List _padSha256(List<int> data) {
  final bitLen = data.length * 8;
  final padLen = (56 - (data.length + 1) % 64 + 64) % 64;
  final out = Uint8List(data.length + 1 + padLen + 8);
  out.setRange(0, data.length, data);
  out[data.length] = 0x80;
  // Big-endian 64-bit bit length at the end.
  for (var i = 0; i < 8; i++) {
    out[out.length - 1 - i] = (bitLen >> (i * 8)) & 0xFF;
  }
  return out;
}

// ── Per-relay WebSocket with auto-reconnect (file-private) ──────────────────
//
// One instance per relay URL. Owns a WebSocketChannel + a backoff loop that
// transparently reconnects on close/error. `RelayService` only holds the
// callbacks (frame + state); this class encapsulates the lifetime.
//
// Backoff curve (matches `phantomchat_relays::NostrRelay::subscribe`):
//   attempt 0 → 1s, 1 → 2s, 2 → 4s, …, 6+ → 60s, all + 0..5s jitter.

enum _ConnState { connecting, connected, disconnected }

class _RelayConn {
  final String url;
  final Future<void> Function(String url, String text) _onFrame;
  final void Function(String url, _ConnState state) _onState;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _sub;
  bool _isOpen = false;
  bool _closed = false;
  int _attempt = 0;
  Timer? _reconnectTimer;
  static final Random _rng = Random();

  _RelayConn(this.url, this._onFrame, this._onState);

  bool get isOpen => _isOpen;

  void start() {
    // Fire-and-forget: the connect-loop owns the lifetime, errors surface
    // through the state callback rather than blocking the caller.
    unawaited(_connectLoop());
  }

  Future<void> _connectLoop() async {
    while (!_closed) {
      _onState(url, _ConnState.connecting);
      try {
        // `IOWebSocketChannel.connect` constructs the channel synchronously;
        // the `ready` future completes once the TLS + WebSocket upgrade
        // finishes (or throws on DNS / refused / TLS / 4xx). The
        // `connectTimeout` cap stops a half-open relay from wedging this
        // loop indefinitely.
        _channel = IOWebSocketChannel.connect(
          Uri.parse(url),
          pingInterval: const Duration(seconds: 30),
          connectTimeout: const Duration(seconds: 10),
        );
        await _channel!.ready;
        _isOpen = true;
        _attempt = 0;
        _onState(url, _ConnState.connected);
        // We only return from this future when the stream errors / closes,
        // which is exactly when we want the loop to retry.
        final completer = Completer<void>();
        _sub = _channel!.stream.listen(
          (raw) {
            if (raw is String) {
              // Don't await — handler runs on its own microtask so a slow
              // frame doesn't block the next one.
              unawaited(_onFrame(url, raw));
            } else if (raw is List<int>) {
              try {
                unawaited(_onFrame(url, utf8.decode(raw)));
              } catch (_) {/* binary frames not part of NIP-01 */}
            }
          },
          onError: (Object e) {
            if (!completer.isCompleted) completer.complete();
          },
          onDone: () {
            if (!completer.isCompleted) completer.complete();
          },
          cancelOnError: true,
        );
        await completer.future;
      } catch (_) {
        // Connect failed (TLS, DNS, refused) — fall through to backoff.
      }
      // ── Disconnect cleanup ─────────────────────────────────────────────
      _isOpen = false;
      await _sub?.cancel();
      _sub = null;
      try {
        await _channel?.sink.close();
      } catch (_) {}
      _channel = null;
      if (_closed) break;
      _onState(url, _ConnState.disconnected);

      // Exponential backoff with jitter; same curve as Desktop's
      // NostrRelay subscribe loop. attempt 0 → 1s, capped at 60s.
      _attempt = min(_attempt + 1, 6);
      final base = (1 << _attempt).clamp(1, 60);
      final jitter = _rng.nextInt(5);
      final wait = Duration(seconds: base + jitter);
      final waitCompleter = Completer<void>();
      _reconnectTimer = Timer(wait, () {
        if (!waitCompleter.isCompleted) waitCompleter.complete();
      });
      await waitCompleter.future;
    }
  }

  void send(String text) {
    final ch = _channel;
    if (ch == null || !_isOpen) {
      throw StateError('relay $url not connected');
    }
    ch.sink.add(text);
  }

  Future<void> close() async {
    _closed = true;
    _reconnectTimer?.cancel();
    await _sub?.cancel();
    _sub = null;
    try {
      await _channel?.sink.close();
    } catch (_) {}
    _channel = null;
    _isOpen = false;
  }
}

// ── Outgoing wire-prefix encoders (parity with Desktop's send helpers) ─────
//
// The receive-side prefix dispatch already lives at the top of this file.
// These mirror the build_*_payload helpers Desktop uses on its Tauri side
// so the chat / channels screens can wrap their plain bytes in the right
// frame before handing them to `RelayService.publish`.

/// `MLS-APP1 || ciphertext` — wraps the bytes returned by `mlsEncrypt`
/// for transport via sealed-sender 1:1 (per-member fan-out is the
/// caller's job).
Uint8List encodeMlsApp(Uint8List ciphertext) {
  final out = BytesBuilder(copy: false);
  out.add(kMlsAppPrefix);
  out.add(ciphertext);
  return out.toBytes();
}

/// `RCPT-1: || ULEB128(len) || {"msg_id": ..., "kind": "delivered"|"read"}`.
Uint8List encodeReceipt({required String msgId, required String kind}) {
  return encodePrefixedJson(kRcptPrefixV1, <String, dynamic>{
    'msg_id': msgId,
    'kind': kind,
  });
}

/// `TYPN-1: || ULEB128(len) || {"ttl_secs": <int>}`. Default TTL is
/// [kTypingTtlSecs] so a brief tap of the keyboard surfaces as a 5-second
/// `is typing…` indicator on the receiver.
Uint8List encodeTyping({int ttlSecs = kTypingTtlSecs}) {
  return encodePrefixedJson(kTypnPrefixV1, <String, dynamic>{
    'ttl_secs': ttlSecs,
  });
}
