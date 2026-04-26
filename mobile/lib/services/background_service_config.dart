// PhantomChat — Wave 8B
//
// Configuration for the `flutter_background_service` plugin. Owns the
// background-isolate entry point that drives the relay-listener loop
// while the user has the app closed.
//
// IMPORTANT — Hard constraint from PR #4:
//   Do NOT re-implement the prefix-dispatch logic in `RelayService` here.
//   We only ADD a thin hook that re-uses the existing API, identical to
//   what `chat.dart::initState` does in the foreground path.
//
// The Android-native side (`RelayForegroundService.kt` + the manifest
// declarations from this PR) handles the OS-level wake-lock + the
// persistent foreground notification. This file deals with the Dart
// half: spinning up the Rust core inside the background isolate and
// translating decoded `NetworkEvent`s into user-visible notifications.

import 'dart:async';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter_background_service/flutter_background_service.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../src/rust/frb_generated.dart';
import '../src/rust/network.dart';

/// Notification-channel IDs — must match `MainActivity.kt`.
const String channelBackground = 'phantomchat_background';
const String channelMessages = 'phantomchat_messages';
const String channelGroups = 'phantomchat_groups';

/// Foreground-service notification (the one Android requires while the
/// background isolate is running). The Kotlin service posts the *initial*
/// notification; this ID lets us update it from Dart with live status.
const int foregroundNotificationId = 0x9001;

/// Distinct base ID for incoming-message notifications so they don't
/// collide with the foreground-service notification.
const int messageNotificationIdBase = 0xA000;

/// SharedPreferences key — mirrored into the native receiver.
const String prefsAutostartOnBoot = 'phantom_bg_autostart_on_boot';

/// SharedPreferences key — set by the Settings UI when the user toggles
/// "Hintergrund-Empfang aktivieren".
const String prefsBgServiceEnabled = 'phantom_bg_service_enabled';

/// Status keys the foreground isolate writes back so the Settings screen
/// can render "Aktiv seit HH:MM:SS · X Relays · Y Nachrichten".
const String prefsBgStartedAtMs = 'phantom_bg_started_at_ms';
const String prefsBgRelayCount = 'phantom_bg_relay_count';
const String prefsBgMessagesReceived = 'phantom_bg_messages_received';

class PhantomBackgroundService {
  PhantomBackgroundService._();

  static final FlutterLocalNotificationsPlugin _localNotifications =
      FlutterLocalNotificationsPlugin();

  /// Call once from `main()` *before* `runApp` to register the isolate
  /// entry point with the plugin.
  static Future<void> initialize() async {
    final service = FlutterBackgroundService();

    // local-notifications plugin init for the foreground isolate's posts.
    await _localNotifications.initialize(
      const InitializationSettings(
        android: AndroidInitializationSettings('@mipmap/ic_launcher'),
      ),
    );

    await service.configure(
      androidConfiguration: AndroidConfiguration(
        onStart: _onBackgroundStart,
        autoStart: false, // Opt-in only — never auto-start.
        isForegroundMode: true,
        notificationChannelId: channelBackground,
        initialNotificationTitle: 'PhantomChat',
        initialNotificationContent: 'Hintergrund-Empfang aktiv',
        foregroundServiceNotificationId: foregroundNotificationId,
      ),
      iosConfiguration: IosConfiguration(
        autoStart: false,
        onForeground: _onBackgroundStart,
        // No iOS background-isolate impl yet — Wave 8C will tackle iOS
        // via NSE / VoIP push as a separate problem.
      ),
    );
  }

  /// User-facing toggle. Persists the opt-in flag so the BootReceiver
  /// can honour it after a reboot.
  static Future<bool> startService() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(prefsBgServiceEnabled, true);
    final service = FlutterBackgroundService();
    return service.startService();
  }

  static Future<void> stopService() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(prefsBgServiceEnabled, false);
    final service = FlutterBackgroundService();
    service.invoke('stopService');
  }

  static Future<bool> isRunning() {
    return FlutterBackgroundService().isRunning();
  }

  static Future<void> setAutostartOnBoot(bool enabled) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(prefsAutostartOnBoot, enabled);
  }

  static Future<bool> autostartOnBoot() async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getBool(prefsAutostartOnBoot) ?? false;
  }
}

/// Background-isolate entry point. Runs in its own isolate — no access
/// to the UI's state, only to plugins + SharedPreferences.
@pragma('vm:entry-point')
void _onBackgroundStart(ServiceInstance service) async {
  // Allow the ServiceInstance to use Flutter plugins.
  DartPluginRegistrant.ensureInitialized();

  // Bring the Rust core online inside this isolate. RustLib.init() is
  // safe to call from any isolate; the FFI layer is process-global.
  try {
    await RustLib.init();
  } catch (e) {
    debugPrint('[bg] RustLib.init failed: $e');
    service.stopSelf();
    return;
  }

  final prefs = await SharedPreferences.getInstance();
  await prefs.setInt(prefsBgStartedAtMs, DateTime.now().millisecondsSinceEpoch);
  await prefs.setInt(prefsBgMessagesReceived, 0);

  // Listen for the explicit "stopService" signal coming from
  // `PhantomBackgroundService.stopService()`.
  service.on('stopService').listen((_) {
    service.stopSelf();
  });

  // Drive the relay-listener loop. The original Wave 8B plan was to bridge
  // a `start_network_node()` Rust stream into this isolate, but that Rust
  // entry point was never finished — `mobile/rust/src/api.rs` does not
  // expose it. Until the bridge lands the bg-service stays alive (so
  // Android keeps the process for the foreground notification + boot
  // autostart contract) but emits no events. `_handleEvent` is preserved
  // below so the wiring is one line away once the Rust stream exists.
  StreamSubscription<NetworkEvent>? sub;
  // ignore: dead_code
  if (false) {
    sub = const Stream<NetworkEvent>.empty().listen(
      (event) => _handleEvent(event, prefs),
      onError: (Object e) => debugPrint('[bg] relay error: $e'),
    );
  }

  service.on('stopService').listen((_) async {
    await sub?.cancel();
  });
}

Future<void> _handleEvent(NetworkEvent event, SharedPreferences prefs) async {
  switch (event) {
    case NetworkEvent_NodeStarted():
      // node is up — nothing user-facing yet.
      break;
    case NetworkEvent_PeerDiscovered(:final peerId):
      final count = (prefs.getInt(prefsBgRelayCount) ?? 0) + 1;
      await prefs.setInt(prefsBgRelayCount, count);
      debugPrint('[bg] peer discovered: $peerId (relays now=$count)');
      break;
    case NetworkEvent_MessageReceived(:final from, :final message):
      await _bumpReceived(prefs);
      await _postDirectMessage(from: from, body: message);
      break;
    case NetworkEvent_GroupMessageReceived(
      :final groupId,
      :final from,
      :final message,
    ):
      await _bumpReceived(prefs);
      await _postGroupMessage(groupId: groupId, from: from, body: message);
      break;
    case NetworkEvent_Error(:final message):
      debugPrint('[bg] relay event error: $message');
      break;
  }
}

Future<void> _bumpReceived(SharedPreferences prefs) async {
  final n = (prefs.getInt(prefsBgMessagesReceived) ?? 0) + 1;
  await prefs.setInt(prefsBgMessagesReceived, n);
}

String _truncate(String s, int max) {
  if (s.length <= max) return s;
  return '${s.substring(0, max)}…';
}

int _idForMessage(String key) {
  // 31-bit positive hash → Android notification id space.
  return messageNotificationIdBase + (key.hashCode & 0x7fffffff) % 0x10000;
}

Future<void> _postDirectMessage({
  required String from,
  required String body,
}) async {
  const androidDetails = AndroidNotificationDetails(
    channelMessages,
    'Nachrichten',
    channelDescription: 'Eingehende verschlüsselte 1:1-Nachrichten.',
    importance: Importance.high,
    priority: Priority.high,
    category: AndroidNotificationCategory.message,
  );
  await PhantomBackgroundService._localNotifications.show(
    _idForMessage('dm:$from'),
    'Nachricht von $from',
    _truncate(body, 80),
    const NotificationDetails(android: androidDetails),
  );
}

Future<void> _postGroupMessage({
  required String groupId,
  required String from,
  required String body,
}) async {
  const androidDetails = AndroidNotificationDetails(
    channelGroups,
    'Gruppen',
    channelDescription: 'Eingehende Gruppen-Nachrichten (MLS).',
    importance: Importance.high,
    priority: Priority.high,
    category: AndroidNotificationCategory.message,
  );
  await PhantomBackgroundService._localNotifications.show(
    _idForMessage('grp:$groupId:$from'),
    'Gruppe — $from',
    _truncate(body, 80),
    const NotificationDetails(android: androidDetails),
  );
}

/// Post a "tampered message" warning (sig_ok=false). Exposed publicly so
/// the foreground RelayService (PR #4) can raise it too without dragging
/// in the full background-service stack.
Future<void> postTamperedMessageNotification() async {
  final androidDetails = AndroidNotificationDetails(
    channelMessages,
    'Nachrichten',
    channelDescription: 'Eingehende verschlüsselte 1:1-Nachrichten.',
    importance: Importance.high,
    priority: Priority.high,
    category: AndroidNotificationCategory.error,
    enableVibration: true,
    vibrationPattern: Int64List.fromList(<int>[0, 400, 200, 400, 200, 800]),
  );
  await PhantomBackgroundService._localNotifications.show(
    messageNotificationIdBase + 1,
    '⚠ Unverifizierte Nachricht',
    'Signaturprüfung fehlgeschlagen — nicht öffnen ohne weitere Verifikation',
    NotificationDetails(android: androidDetails),
  );
}
