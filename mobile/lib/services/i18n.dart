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
    },
  };
}
