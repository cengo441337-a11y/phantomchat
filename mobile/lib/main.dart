import 'package:flutter/material.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(const SafeRootApp());
}

class SafeRootApp extends StatelessWidget {
  const SafeRootApp({super.key});
  @override
  Widget build(BuildContext context) {
    return const MaterialApp(
      home: SafeHomeScreen(),
      debugShowCheckedModeBanner: false,
    );
  }
}

class SafeHomeScreen extends StatelessWidget {
  const SafeHomeScreen({super.key});
  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      body: Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            const Text('PHANTOM', style: TextStyle(color: Color(0xFFFF00FF), fontSize: 48, fontWeight: FontWeight.bold, letterSpacing: 10)),
            const SizedBox(height: 10),
            const Text('DIAGNOSTIC CORE - SAFE MODE', style: TextStyle(color: Colors.green, fontSize: 10, letterSpacing: 2)),
            const SizedBox(height: 40),
            const CircularProgressIndicator(color: Colors.green),
            const SizedBox(height: 40),
            const Text('If you see this, the Flutter Engine is functional.', style: TextStyle(color: Colors.white, fontSize: 12)),
            const Text('The issue is with P2P/Rust/Shaders.', style: TextStyle(color: Colors.grey, fontSize: 10)),
            const SizedBox(height: 20),
            OutlinedButton(
               style: OutlinedButton.styleFrom(side: const BorderSide(color: Colors.green)),
               onPressed: () {},
               child: const Text('SAFE BOOT OK', style: TextStyle(color: Colors.green)),
            ),
          ],
        ),
      )
    );
  }
}
