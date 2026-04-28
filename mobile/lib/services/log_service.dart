// Capture every `debugPrint` call into a fixed-size ring buffer that
// the diagnostics screen can dump on demand. Pre-1.1.4 the only way to
// see what `[relay] error: …` / `[setPin] pbkdf2(50000)=…ms` etc. were
// reporting was via `adb logcat` over USB — useless for real users.
//
// Wire-up (call once from `main.dart`, BEFORE any `debugPrint` calls):
//
//     LogService.install();
//
// After that, every existing `debugPrint(...)` call across the app
// transparently feeds this buffer in addition to its normal stdout
// destination. No call-site changes needed.

import 'dart:collection';

import 'package:flutter/foundation.dart';

/// Process-lifetime log capture. Singleton on purpose — `debugPrint` is
/// a top-level function pointer; intercepting it twice would chain the
/// hooks and break logs after the second install.
class LogService {
  /// Ring buffer cap. ~500 lines × ~120 chars = ~60 KB — fits easily
  /// in RAM, plenty for a typical bug-report scope (last few seconds
  /// to minutes of activity), and stays small enough that the
  /// diagnostics screen renders without paging.
  static const int _maxLines = 500;

  static final LogService _instance = LogService._();
  factory LogService() => _instance;
  LogService._();

  final Queue<_LogLine> _buffer = Queue<_LogLine>();
  bool _installed = false;
  // Cache the original `debugPrint` so we can chain to it instead of
  // shadowing — `flutter run` console output keeps working.
  DebugPrintCallback? _previous;

  /// Replace the global `debugPrint` callback with a tee that pushes
  /// into the buffer AND forwards to the previous callback. Idempotent
  /// — second call is a no-op.
  void install() {
    if (_installed) return;
    _previous = debugPrint;
    debugPrint = (String? message, {int? wrapWidth}) {
      if (message != null) {
        _push(message);
      }
      // Always forward — keeps `flutter run` / `adb logcat` parity
      // with what the in-app dump shows.
      _previous?.call(message, wrapWidth: wrapWidth);
    };
    _installed = true;
  }

  void _push(String message) {
    _buffer.addLast(_LogLine(DateTime.now(), message));
    while (_buffer.length > _maxLines) {
      _buffer.removeFirst();
    }
  }

  /// Append a line manually (for events that don't go through
  /// debugPrint — e.g. caught exceptions you want to record without
  /// also printing).
  void log(String message) => _push(message);

  /// Dump the buffer as a single string, newest at the bottom. The
  /// diagnostics screen wraps this in a header that includes the
  /// app version + device info — so callers don't need to.
  String dump() {
    final buf = StringBuffer();
    for (final line in _buffer) {
      buf
        ..write(line.ts.toIso8601String())
        ..write('  ')
        ..writeln(line.message);
    }
    return buf.toString();
  }

  /// Number of buffered lines. Diagnostic-screen header uses this.
  int get length => _buffer.length;

  /// Wipe the buffer. Diagnostic screen exposes a button for the user
  /// to clear the log before reproducing a specific bug — narrows the
  /// dump to just the events that matter.
  void clear() => _buffer.clear();
}

class _LogLine {
  final DateTime ts;
  final String message;
  const _LogLine(this.ts, this.message);
}
