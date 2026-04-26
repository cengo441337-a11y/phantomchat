import 'package:flutter/material.dart';
import '../theme.dart';

/// Card with cut top-right corner and optional neon border + glow.
class CyberCard extends StatelessWidget {
  final Widget child;
  final Color borderColor;
  final Color bgColor;
  final double cut;
  final bool glow;
  final EdgeInsetsGeometry? padding;
  final VoidCallback? onTap;

  const CyberCard({
    super.key,
    required this.child,
    this.borderColor = kCyan,
    this.bgColor = kBgCard,
    this.cut = 14,
    this.glow = false,
    this.padding,
    this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    Widget content = ClipPath(
      clipper: _CutClipper(cut: cut),
      child: Container(
        color: bgColor,
        padding: padding,
        child: child,
      ),
    );

    content = CustomPaint(
      painter: _CutBorderPainter(
        borderColor: borderColor,
        cut: cut,
        glow: glow,
      ),
      child: content,
    );

    if (onTap != null) {
      content = GestureDetector(onTap: onTap, child: content);
    }

    return content;
  }
}

class _CutClipper extends CustomClipper<Path> {
  final double cut;
  _CutClipper({required this.cut});

  @override
  Path getClip(Size size) => _cutPath(size, cut);

  @override
  bool shouldReclip(_CutClipper old) => old.cut != cut;
}

class _CutBorderPainter extends CustomPainter {
  final Color borderColor;
  final double cut;
  final bool glow;

  _CutBorderPainter({
    required this.borderColor,
    required this.cut,
    required this.glow,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final path = _cutPath(size, cut);

    if (glow) {
      canvas.drawPath(
        path,
        Paint()
          ..color = borderColor.withValues(alpha: 0.25)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 6
          ..maskFilter = const MaskFilter.blur(BlurStyle.outer, 8),
      );
    }

    canvas.drawPath(
      path,
      Paint()
        ..color = borderColor.withValues(alpha: 0.5)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.0,
    );

    // Corner accent: bright dot at cut point
    canvas.drawCircle(
      Offset(size.width - cut, 0),
      2,
      Paint()..color = borderColor,
    );
    canvas.drawCircle(
      Offset(0, size.height - cut),
      2,
      Paint()..color = borderColor.withValues(alpha: 0.4),
    );
  }

  @override
  bool shouldRepaint(_CutBorderPainter old) =>
      old.borderColor != borderColor || old.glow != glow;
}

Path _cutPath(Size size, double cut) => Path()
  ..moveTo(0, 0)
  ..lineTo(size.width - cut, 0)
  ..lineTo(size.width, cut)
  ..lineTo(size.width, size.height)
  ..lineTo(cut, size.height)
  ..lineTo(0, size.height - cut)
  ..close();

/// Subtle dot-grid background painter.
class GridBackground extends StatelessWidget {
  final Widget child;
  final Color dotColor;

  const GridBackground({
    super.key,
    required this.child,
    this.dotColor = const Color(0xFF0A1525),
  });

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      painter: _GridPainter(dotColor: dotColor),
      child: child,
    );
  }
}

class _GridPainter extends CustomPainter {
  final Color dotColor;
  _GridPainter({required this.dotColor});

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()..color = dotColor;
    const step = 28.0;
    for (double x = 0; x <= size.width; x += step) {
      for (double y = 0; y <= size.height; y += step) {
        canvas.drawCircle(Offset(x, y), 1.2, paint);
      }
    }
    // Horizontal scan line hint at 40% height
    canvas.drawLine(
      Offset(0, size.height * 0.4),
      Offset(size.width, size.height * 0.4),
      Paint()
        ..color = const Color(0x0800F5FF)
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_GridPainter old) => old.dotColor != dotColor;
}

/// Blinking cursor widget.
class BlinkCursor extends StatefulWidget {
  final Color color;
  const BlinkCursor({super.key, this.color = kCyan});

  @override
  State<BlinkCursor> createState() => _BlinkCursorState();
}

class _BlinkCursorState extends State<BlinkCursor>
    with SingleTickerProviderStateMixin {
  late AnimationController _ctrl;

  @override
  void initState() {
    super.initState();
    _ctrl = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 530),
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
      builder: (_, _) => Opacity(
        opacity: _ctrl.value > 0.5 ? 1 : 0,
        child: Container(
          width: 8,
          height: 16,
          color: widget.color,
        ),
      ),
    );
  }
}

/// Pulsing neon ring widget.
class PulseRing extends StatefulWidget {
  final Widget child;
  final Color color;
  final double size;

  const PulseRing({
    super.key,
    required this.child,
    this.color = kCyan,
    this.size = 90,
  });

  @override
  State<PulseRing> createState() => _PulseRingState();
}

class _PulseRingState extends State<PulseRing>
    with SingleTickerProviderStateMixin {
  late AnimationController _ctrl;
  late Animation<double> _anim;

  @override
  void initState() {
    super.initState();
    _ctrl = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 1800),
    )..repeat(reverse: true);
    _anim = CurvedAnimation(parent: _ctrl, curve: Curves.easeInOut);
  }

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _anim,
      builder: (_, child) => SizedBox(
        width: widget.size,
        height: widget.size,
        child: Stack(
          alignment: Alignment.center,
          children: [
            // Outer pulse ring
            Container(
              width: widget.size * (0.9 + _anim.value * 0.1),
              height: widget.size * (0.9 + _anim.value * 0.1),
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                border: Border.all(
                  color: widget.color.withValues(alpha: 0.15 + _anim.value * 0.1),
                  width: 1,
                ),
                boxShadow: [
                  BoxShadow(
                    color: widget.color.withValues(alpha: 0.1 + _anim.value * 0.15),
                    blurRadius: 16 + _anim.value * 8,
                    spreadRadius: 2,
                  ),
                ],
              ),
            ),
            // Inner ring
            Container(
              width: widget.size * 0.72,
              height: widget.size * 0.72,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: widget.color.withValues(alpha: 0.06),
                border: Border.all(
                  color: widget.color.withValues(alpha: 0.4 + _anim.value * 0.3),
                  width: 1.5,
                ),
              ),
            ),
            ?child,
          ],
        ),
      ),
      child: widget.child,
    );
  }
}
