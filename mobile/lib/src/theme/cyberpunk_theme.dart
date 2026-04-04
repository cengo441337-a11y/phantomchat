import 'package:flutter/material.dart';

class CyberpunkTheme {
  static const Color neonGreen = Color(0xFF00FF9F);
  static const Color neonMagenta = Color(0xFFFF00FF);
  static const Color darkBackground = Color(0xFF020205);
  static const Color terminalGreen = Color(0xFF00FF00);

  static ThemeData get themeData {
    return ThemeData(
      brightness: Brightness.dark,
      scaffoldBackgroundColor: darkBackground,
      primaryColor: neonGreen,
      colorScheme: const ColorScheme.dark(
        primary: neonGreen,
        secondary: neonMagenta,
        surface: Color(0xFF111111),
      ),
      fontFamily: 'Courier', // Placeholder for a monospace font
      textTheme: const TextTheme(
        bodyLarge: TextStyle(color: neonGreen, fontSize: 16.0),
        bodyMedium: TextStyle(color: terminalGreen, fontSize: 14.0),
        displayLarge: TextStyle(
            color: neonMagenta,
            fontSize: 32.0,
            fontWeight: FontWeight.bold,
            letterSpacing: 2.0),
      ),
      appBarTheme: const AppBarTheme(
        backgroundColor: Colors.transparent,
        elevation: 0,
        centerTitle: true,
        titleTextStyle: TextStyle(
          color: neonGreen,
          fontSize: 24.0,
          fontWeight: FontWeight.bold,
          letterSpacing: 3.0,
        ),
      ),
      cardTheme: CardThemeData(
        color: const Color(0xFF1A1A1A),
        shape: RoundedRectangleBorder(
          side: const BorderSide(color: neonGreen, width: 1.0),
          borderRadius: BorderRadius.circular(4.0),
        ),
      ),
      elevatedButtonTheme: ElevatedButtonThemeData(
        style: ElevatedButton.styleFrom(
          backgroundColor: Colors.transparent,
          foregroundColor: neonGreen,
          side: const BorderSide(color: neonGreen, width: 2.0),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(2.0),
          ),
          textStyle: const TextStyle(
            fontWeight: FontWeight.bold,
            letterSpacing: 1.5,
          ),
        ),
      ),
    );
  }
}