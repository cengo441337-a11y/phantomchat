import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

const kBg = Color(0xFF080B0F);
const kBgCard = Color(0xFF0D1117);
const kBgInput = Color(0xFF131920);
const kNeon = Color(0xFF00FFB2);
const kNeonDim = Color(0xFF00FFB240);
const kNeonText = Color(0xFF00E5A0);
const kRed = Color(0xFFFF3860);
const kGray = Color(0xFF5A6475);
const kWhite = Color(0xFFE8F0F8);
const kWhiteDim = Color(0xFF8899AA);

ThemeData phantomTheme() {
  final base = ThemeData.dark();
  return base.copyWith(
    scaffoldBackgroundColor: kBg,
    colorScheme: const ColorScheme.dark(
      primary: kNeon,
      secondary: kNeonText,
      surface: kBgCard,
      error: kRed,
    ),
    textTheme: GoogleFonts.spaceGroteskTextTheme(base.textTheme).copyWith(
      bodyLarge: GoogleFonts.spaceGrotesk(color: kWhite),
      bodyMedium: GoogleFonts.spaceGrotesk(color: kWhiteDim),
      titleLarge: GoogleFonts.spaceGrotesk(
        color: kWhite,
        fontWeight: FontWeight.w700,
        letterSpacing: -0.5,
      ),
      headlineLarge: GoogleFonts.spaceGrotesk(
        color: kWhite,
        fontWeight: FontWeight.w800,
        letterSpacing: -1,
      ),
    ),
    appBarTheme: AppBarTheme(
      backgroundColor: kBg,
      elevation: 0,
      titleTextStyle: GoogleFonts.spaceGrotesk(
        color: kWhite,
        fontSize: 18,
        fontWeight: FontWeight.w700,
        letterSpacing: -0.3,
      ),
      iconTheme: const IconThemeData(color: kWhite),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: kBgInput,
      contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: const BorderSide(color: Color(0xFF1E2733)),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: const BorderSide(color: Color(0xFF1E2733)),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: const BorderSide(color: kNeon, width: 1.5),
      ),
      hintStyle: GoogleFonts.spaceGrotesk(color: kGray, fontSize: 14),
      labelStyle: GoogleFonts.spaceGrotesk(color: kGray, fontSize: 14),
    ),
    elevatedButtonTheme: ElevatedButtonThemeData(
      style: ElevatedButton.styleFrom(
        backgroundColor: kNeon,
        foregroundColor: kBg,
        padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 14),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
        textStyle: GoogleFonts.spaceGrotesk(
          fontWeight: FontWeight.w700,
          fontSize: 15,
          letterSpacing: 0.5,
        ),
      ),
    ),
    dividerColor: const Color(0xFF1A2030),
    cardColor: kBgCard,
  );
}
