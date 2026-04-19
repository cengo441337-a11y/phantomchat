import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'services/storage_service.dart';
import 'screens/onboarding.dart';
import 'screens/home.dart';
import 'theme.dart';
import 'widgets/app_lock_gate.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  SystemChrome.setPreferredOrientations([DeviceOrientation.portraitUp]);
  SystemChrome.setSystemUIOverlayStyle(
    const SystemUiOverlayStyle(
      statusBarColor: Colors.transparent,
      statusBarIconBrightness: Brightness.light,
      systemNavigationBarColor: kBg,
    ),
  );

  final hasIdentity = await StorageService.hasIdentity();

  runApp(PhantomApp(hasIdentity: hasIdentity));
}

class PhantomApp extends StatelessWidget {
  final bool hasIdentity;
  const PhantomApp({super.key, required this.hasIdentity});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'PhantomChat',
      theme: phantomTheme(),
      debugShowCheckedModeBanner: false,
      home: AppLockGate(
        child: hasIdentity ? const HomeScreen() : const OnboardingScreen(),
      ),
    );
  }
}
