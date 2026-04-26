// Wave 11B — minimal locale-aware string lookup.
//
// PhantomChat's existing UI hard-codes English strings with the Orbitron /
// SpaceMono cyber-aesthetic. To stay inside the "no new dependency" budget
// we don't pull `flutter_localizations` here — just a small static map
// keyed by `intl.Intl.getCurrentLocale()` short code (`de`, `en`, …).
//
// New keys added in 11B for the voice-message UX:
//   `voice.pressToRecord`, `voice.releaseToSend`, `voice.slideToCancel`,
//   `voice.recording`, `voice.label`, `voice.tapToPlay`.

import 'dart:ui' show PlatformDispatcher;

class I18n {
  /// Returns a locale-appropriate string for the given key. Falls back to
  /// English if the key is missing in the locale map, or to the key
  /// itself if it's missing in English too (so devs spot typos quickly).
  static String t(String key) {
    final lang = _shortLang();
    final map = _strings[lang] ?? _strings['en']!;
    return map[key] ?? _strings['en']![key] ?? key;
  }

  static String _shortLang() {
    final l = PlatformDispatcher.instance.locale.languageCode.toLowerCase();
    if (_strings.containsKey(l)) return l;
    return 'en';
  }

  static const Map<String, Map<String, String>> _strings = {
    'en': {
      'voice.pressToRecord': 'PRESS TO RECORD',
      'voice.releaseToSend': 'RELEASE TO SEND',
      'voice.slideToCancel': 'SLIDE TO CANCEL',
      'voice.recording': 'RECORDING…',
      'voice.label': 'VOICE MESSAGE',
      'voice.tapToPlay': 'TAP TO PLAY',
      'voice.permissionDenied': 'MIC PERMISSION DENIED',
      'voice.unavailable': '[VOICE CLIP UNAVAILABLE]',
      // Wave 11G+ — chat send-path errors. Surfaced via SnackBar when
      // sealed-sender encryption fails so the user can't mistake a
      // legacy-fallback ghost-send for delivery (the fallback was
      // removed in this wave because it is not wire-compatible with
      // Desktop).
      'chat.errors.sendFailed':
          '! SEND FAILED — MESSAGE NOT DELIVERED. {err}',
      'chat.errors.voiceSendFailed': '! VOICE SEND FAILED: {err}',
      // Wave 11G — APK auto-update.
      'update.banner': 'UPDATE AVAILABLE: v{version} — TAP TO DOWNLOAD',
      'update.dialog.title': 'NEW VERSION AVAILABLE',
      'update.dialog.version': 'v{current} → v{new}',
      'update.dialog.notes': 'CHANGES:',
      'update.dialog.size': 'DOWNLOAD SIZE: {size}',
      'update.dialog.download': 'DOWNLOAD + INSTALL',
      'update.dialog.dismiss': 'LATER',
      'update.downloading': 'DOWNLOADING…',
      'update.installing': 'LAUNCHING INSTALLER…',
      'update.error.download': 'DOWNLOAD FAILED',
      'update.error.checksum': 'CHECKSUM MISMATCH — ABORTED',
      'update.error.install': 'COULD NOT LAUNCH INSTALLER',
      'update.abort': 'ABORT',
    },
    'de': {
      'voice.pressToRecord': 'ZUM AUFNEHMEN GEDRÜCKT HALTEN',
      'voice.releaseToSend': 'LOSLASSEN ZUM SENDEN',
      'voice.slideToCancel': 'NACH LINKS WISCHEN ZUM ABBRECHEN',
      'voice.recording': 'AUFNAHME…',
      'voice.label': 'SPRACHNACHRICHT',
      'voice.tapToPlay': 'ZUM ABSPIELEN TIPPEN',
      'voice.permissionDenied': 'MIKROFONZUGRIFF VERWEIGERT',
      'voice.unavailable': '[SPRACHCLIP NICHT VERFÜGBAR]',
      'chat.errors.sendFailed':
          '! SENDEN FEHLGESCHLAGEN — NACHRICHT NICHT ZUGESTELLT. {err}',
      'chat.errors.voiceSendFailed':
          '! SPRACHNACHRICHT FEHLGESCHLAGEN: {err}',
      // Wave 11G — APK auto-update.
      'update.banner': 'UPDATE VERFÜGBAR: v{version} — JETZT HERUNTERLADEN',
      'update.dialog.title': 'NEUE VERSION VERFÜGBAR',
      'update.dialog.version': 'v{current} → v{new}',
      'update.dialog.notes': 'ÄNDERUNGEN:',
      'update.dialog.size': 'DOWNLOAD-GRÖSSE: {size}',
      'update.dialog.download': 'HERUNTERLADEN + INSTALLIEREN',
      'update.dialog.dismiss': 'SPÄTER',
      'update.downloading': 'WIRD HERUNTERGELADEN…',
      'update.installing': 'INSTALLER WIRD GESTARTET…',
      'update.error.download': 'DOWNLOAD FEHLGESCHLAGEN',
      'update.error.checksum': 'PRÜFSUMME UNGÜLTIG — ABGEBROCHEN',
      'update.error.install': 'INSTALLER KONNTE NICHT GESTARTET WERDEN',
      'update.abort': 'ABBRECHEN',
    },
  };

  /// Like [t] but substitutes `{key}` placeholders from the given map.
  /// Used by the Wave 11G update strings which embed a version / size.
  static String tf(String key, Map<String, String> args) {
    var s = t(key);
    args.forEach((k, v) {
      s = s.replaceAll('{$k}', v);
    });
    return s;
  }
}
