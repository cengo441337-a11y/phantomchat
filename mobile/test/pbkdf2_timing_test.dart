import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

class _Args {
  final String pin;
  final List<int> salt;
  final int iters;
  const _Args(this.pin, this.salt, this.iters);
}

Future<List<int>> _derive(_Args a) async {
  final pbkdf2 = Pbkdf2(
    macAlgorithm: Hmac.sha256(),
    iterations: a.iters,
    bits: 256,
  );
  final key = await pbkdf2.deriveKey(
    secretKey: SecretKey(utf8.encode(a.pin)),
    nonce: a.salt,
  );
  return key.extractBytes();
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('PBKDF2 50k completes in reasonable time on host (proxy for slow device)', () async {
    final salt = List<int>.generate(16, (i) => i);
    for (final iters in [50000, 100000, 600000]) {
      final t0 = DateTime.now().millisecondsSinceEpoch;
      // compute() runs the function in a background isolate
      final out = await compute(_derive, _Args('1234', salt, iters));
      final t1 = DateTime.now().millisecondsSinceEpoch;
      // ignore: avoid_print
      print('PBKDF2 iters=$iters → ${t1 - t0} ms (out len=${out.length})');
    }
  }, timeout: const Timeout(Duration(seconds: 120)));
}
