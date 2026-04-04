import 'dart:async';
import 'dart:math';
import 'package:flutter/material.dart';

/// RGB-split glitch text — layers cyan + magenta channels with random offsets.
class GlitchText extends StatefulWidget {
  final String text;
  final TextStyle style;
  final bool active;
  final Duration interval;

  const GlitchText({
    super.key,
    required this.text,
    required this.style,
    this.active = true,
    this.interval = const Duration(milliseconds: 60),
  });

  @override
  State<GlitchText> createState() => _GlitchTextState();
}

class _GlitchTextState extends State<GlitchText> {
  final _rng = Random();
  double _offsetR = 0, _offsetB = 0;
  double _opacityR = 0, _opacityB = 0;
  Timer? _timer;
  int _tick = 0;

  @override
  void initState() {
    super.initState();
    if (widget.active) _start();
  }

  void _start() {
    _timer = Timer.periodic(widget.interval, (_) {
      if (!mounted) return;
      _tick++;
      // Glitch fires ~20% of ticks, harder glitch every ~50 ticks
      final shouldGlitch = _rng.nextInt(5) == 0;
      final hardGlitch = _tick % 47 < 2;
      setState(() {
        if (hardGlitch) {
          _offsetR = (_rng.nextDouble() * 10 - 5);
          _offsetB = (_rng.nextDouble() * 8 - 4);
          _opacityR = 0.85;
          _opacityB = 0.75;
        } else if (shouldGlitch) {
          _offsetR = (_rng.nextDouble() * 4 - 2);
          _offsetB = (_rng.nextDouble() * 3 - 1.5);
          _opacityR = 0.6;
          _opacityB = 0.5;
        } else {
          _offsetR = 0;
          _offsetB = 0;
          _opacityR = 0;
          _opacityB = 0;
        }
      });
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Stack(
      children: [
        // Magenta channel (red shift)
        if (_opacityR > 0)
          Transform.translate(
            offset: Offset(_offsetR, 0),
            child: Text(
              widget.text,
              style: widget.style.copyWith(
                color: const Color(0xFFFF0066).withOpacity(_opacityR),
                shadows: null,
              ),
            ),
          ),
        // Cyan channel (blue shift)
        if (_opacityB > 0)
          Transform.translate(
            offset: Offset(-_offsetB, 0),
            child: Text(
              widget.text,
              style: widget.style.copyWith(
                color: const Color(0xFF00F5FF).withOpacity(_opacityB),
                shadows: null,
              ),
            ),
          ),
        // Main text
        Text(widget.text, style: widget.style),
      ],
    );
  }
}

/// Scrambles characters randomly, then resolves to target text.
class DecryptText extends StatefulWidget {
  final String text;
  final TextStyle style;
  final Duration duration;

  const DecryptText({
    super.key,
    required this.text,
    required this.style,
    this.duration = const Duration(milliseconds: 800),
  });

  @override
  State<DecryptText> createState() => _DecryptTextState();
}

class _DecryptTextState extends State<DecryptText>
    with SingleTickerProviderStateMixin {
  static const _chars = 'ABCDEF0123456789#@!%\$&*<>';
  final _rng = Random();
  late String _current;
  Timer? _timer;
  int _resolved = 0;

  @override
  void initState() {
    super.initState();
    _current = widget.text.replaceAll(RegExp(r'\S'), '?');
    _start();
  }

  void _start() {
    final interval = Duration(
      milliseconds: widget.duration.inMilliseconds ~/ (widget.text.length * 3),
    );
    _timer = Timer.periodic(interval, (_) {
      if (!mounted) return;
      if (_resolved >= widget.text.length) {
        _timer?.cancel();
        return;
      }
      // Occasionally resolve next character
      if (_rng.nextInt(3) == 0) _resolved++;

      setState(() {
        final chars = List<String>.generate(widget.text.length, (i) {
          if (i < _resolved) return widget.text[i];
          if (widget.text[i] == ' ') return ' ';
          return _chars[_rng.nextInt(_chars.length)];
        });
        _current = chars.join();
      });
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Text(_current, style: widget.style);
  }
}
