import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

// Core palette
const kBg       = Color(0xFF050507);
const kBgCard   = Color(0xFF080C12);
const kBgInput  = Color(0xFF0B1018);
const kCyan     = Color(0xFF00F5FF);
const kCyanDim  = Color(0x2200F5FF);
const kCyanMid  = Color(0x6600F5FF);
const kMagenta  = Color(0xFFFF0066);
const kMagDim   = Color(0x22FF0066);
const kGreen    = Color(0xFF39FF14);
const kYellow   = Color(0xFFFFD700);
const kGray     = Color(0xFF2E3D50);
const kGrayText = Color(0xFF4A5A6E);
const kWhite    = Color(0xFFCDD9E8);
const kWhiteDim = Color(0xFF6E7F90);

// Legacy aliases so existing screens compile
const kNeon    = kCyan;
const kNeonDim = kCyanDim;
const kNeonText = kCyan;
const kRed     = kMagenta;

ThemeData phantomTheme() {
  final base = ThemeData.dark();
  return base.copyWith(
    scaffoldBackgroundColor: kBg,
    colorScheme: const ColorScheme.dark(
      primary: kCyan,
      secondary: kMagenta,
      surface: kBgCard,
      error: kMagenta,
    ),
    textTheme: GoogleFonts.spaceGroteskTextTheme(base.textTheme).apply(
      bodyColor: kWhite,
      displayColor: kWhite,
    ),
    appBarTheme: AppBarTheme(
      backgroundColor: kBg,
      elevation: 0,
      titleTextStyle: GoogleFonts.orbitron(
        color: kWhite,
        fontSize: 16,
        fontWeight: FontWeight.w700,
        letterSpacing: 1,
      ),
      iconTheme: const IconThemeData(color: kWhite),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: kBgInput,
      contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(4),
        borderSide: const BorderSide(color: kGray),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(4),
        borderSide: const BorderSide(color: kGray),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(4),
        borderSide: const BorderSide(color: kCyan, width: 1.5),
      ),
      hintStyle: GoogleFonts.spaceMono(color: kGrayText, fontSize: 13),
      labelStyle: GoogleFonts.spaceMono(color: kGrayText, fontSize: 13),
    ),
    elevatedButtonTheme: ElevatedButtonThemeData(
      style: ElevatedButton.styleFrom(
        backgroundColor: Colors.transparent,
        foregroundColor: kCyan,
        side: const BorderSide(color: kCyan, width: 1.5),
        padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 16),
        shape: const RoundedRectangleBorder(borderRadius: BorderRadius.zero),
        elevation: 0,
        shadowColor: Colors.transparent,
        textStyle: GoogleFonts.orbitron(
          fontWeight: FontWeight.w700,
          fontSize: 13,
          letterSpacing: 2,
        ),
      ),
    ),
    dividerColor: kGray,
    cardColor: kBgCard,
    snackBarTheme: SnackBarThemeData(
      backgroundColor: kBgCard,
      contentTextStyle: GoogleFonts.spaceMono(color: kWhite, fontSize: 12),
    ),
  );
}

// Shared glow shadow helper
List<BoxShadow> neonGlow(Color color, {double radius = 12}) => [
  BoxShadow(color: color.withOpacity(0.4), blurRadius: radius, spreadRadius: 0),
  BoxShadow(color: color.withOpacity(0.1), blurRadius: radius * 3, spreadRadius: 2),
];
