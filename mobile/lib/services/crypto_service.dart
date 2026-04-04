import 'dart:convert';
import 'dart:typed_data';
import 'package:cryptography/cryptography.dart';

class CryptoService {
  static final _x25519 = X25519();
  static final _chacha = Chacha20.poly1305Aead();

  // Generate a new X25519 key pair, returns {private: hex, public: hex}
  static Future<Map<String, String>> generateKeyPair() async {
    final pair = await _x25519.newKeyPair();
    final privBytes = await pair.extractPrivateKeyBytes();
    final pubKey = await pair.extractPublicKey();
    return {
      'private': _hex(Uint8List.fromList(privBytes)),
      'public': _hex(Uint8List.fromList(pubKey.bytes)),
    };
  }

  // ECDH: given our private key (hex) and their public key (hex) → shared secret bytes
  static Future<Uint8List> ecdh(String ourPrivHex, String theirPubHex) async {
    final privBytes = _unhex(ourPrivHex);
    final pubBytes = _unhex(theirPubHex);

    final ourPair = await _x25519.newKeyPairFromSeed(privBytes);
    final theirPub = SimplePublicKey(pubBytes, type: KeyPairType.x25519);
    final sharedSecret = await _x25519.sharedSecretKey(
      keyPair: ourPair,
      remotePublicKey: theirPub,
    );
    final sharedBytes = await sharedSecret.extractBytes();
    return Uint8List.fromList(sharedBytes);
  }

  // Encrypt a message to a recipient's public spend key
  // Returns: {ciphertext: hex, ephemeralKey: hex, nonce: hex}
  static Future<Map<String, String>> encrypt(
    String plaintext,
    String recipientSpendKeyHex,
  ) async {
    // Generate ephemeral key pair
    final ephemeral = await _x25519.newKeyPair();
    final ephPub = await ephemeral.extractPublicKey();
    final ephPriv = await ephemeral.extractPrivateKeyBytes();

    // ECDH with recipient's spend key
    final shared = await ecdh(
      _hex(Uint8List.fromList(ephPriv)),
      recipientSpendKeyHex,
    );

    // Use first 32 bytes as ChaCha20 key
    final secretKey = SecretKey(shared.sublist(0, 32));

    // Encrypt
    final plaintextBytes = utf8.encode(plaintext);
    final secretBox = await _chacha.encrypt(
      plaintextBytes,
      secretKey: secretKey,
    );

    return {
      'ciphertext': _hex(Uint8List.fromList([
        ...secretBox.cipherText,
        ...secretBox.mac.bytes,
      ])),
      'ephemeralKey': _hex(Uint8List.fromList(ephPub.bytes)),
      'nonce': _hex(Uint8List.fromList(secretBox.nonce)),
    };
  }

  // Decrypt using our private spend key
  static Future<String?> decrypt(
    String ciphertextHex,
    String ephemeralKeyHex,
    String nonceHex,
    String ourSpendKeyHex,
  ) async {
    try {
      // ECDH with ephemeral key
      final shared = await ecdh(ourSpendKeyHex, ephemeralKeyHex);
      final secretKey = SecretKey(shared.sublist(0, 32));

      final combined = _unhex(ciphertextHex);
      final macBytes = combined.sublist(combined.length - 16);
      final cipherBytes = combined.sublist(0, combined.length - 16);
      final nonceBytes = _unhex(nonceHex);

      final secretBox = SecretBox(
        cipherBytes,
        nonce: nonceBytes,
        mac: Mac(macBytes),
      );

      final plainBytes = await _chacha.decrypt(secretBox, secretKey: secretKey);
      return utf8.decode(plainBytes);
    } catch (_) {
      return null;
    }
  }

  // Hex encode
  static String _hex(Uint8List bytes) =>
      bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();

  // Hex decode
  static Uint8List _unhex(String hex) {
    final result = Uint8List(hex.length ~/ 2);
    for (int i = 0; i < result.length; i++) {
      result[i] = int.parse(hex.substring(i * 2, i * 2 + 2), radix: 16);
    }
    return result;
  }
}
