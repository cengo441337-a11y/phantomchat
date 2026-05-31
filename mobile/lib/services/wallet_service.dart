import 'dart:io';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:path_provider/path_provider.dart';

import '../src/rust/api/wallet.dart' as rust;

/// Dart-side facade over the Argos wallet FFI.
///
/// One source of truth for the storage path, secure-storage PIN cache, and
/// the cached "currently unlocked" pubkey so the UI doesn't need to hit the
/// FFI on every rebuild.
class ArgosWalletService {
  ArgosWalletService._();
  static final ArgosWalletService instance = ArgosWalletService._();

  static const _secure = FlutterSecureStorage(
    aOptions: AndroidOptions(encryptedSharedPreferences: true),
  );

  static const _kStoredPubkey = 'argos.pubkey.b58';
  static const _kStoredNetwork = 'argos.network';
  static const _kAutoLockPin = 'argos.autounlock.pin';

  String? _cachedPubkey;
  String? _cachedNetwork;

  /// On-disk path of the encrypted wallet blob. Persistent across launches.
  Future<String> walletPath() async {
    final dir = await getApplicationDocumentsDirectory();
    return '${dir.path}${Platform.pathSeparator}argos_wallet.enc.json';
  }

  /// Whether a wallet exists on disk (regardless of whether it's unlocked).
  Future<bool> hasWallet() async {
    final p = await walletPath();
    final exists = File(p).existsSync();
    return exists;
  }

  /// Cached pubkey if a wallet is currently unlocked, else null.
  String? get pubkey => _cachedPubkey ?? rust.argosWalletPubkey();

  /// Active network of the unlocked wallet, or the last known network for
  /// onboarding UI hints.
  String? get network => _cachedNetwork ?? rust.argosWalletNetwork();

  /// True if a wallet is currently unlocked in the Rust cache.
  bool get isUnlocked => pubkey != null;

  /// Generate a brand-new wallet on `network` ("mainnet-beta" or "devnet"),
  /// persist it encrypted with `pin`, and return the 24-word mnemonic. The
  /// caller MUST display the mnemonic for backup and force a "I wrote it
  /// down" confirmation before navigating away.
  Future<rust.ArgosWalletInfo> create({
    required String network,
    required String pin,
  }) async {
    final path = await walletPath();
    final info = await rust.argosCreateWallet(
      network: network,
      pin: pin,
      storagePath: path,
    );
    _cachedPubkey = info.pubkeyB58;
    _cachedNetwork = info.network;
    await _secure.write(key: _kStoredPubkey, value: info.pubkeyB58);
    await _secure.write(key: _kStoredNetwork, value: info.network);
    return info;
  }

  /// Restore from a user-typed 12/24-word mnemonic.
  Future<rust.ArgosWalletInfo> restore({
    required String mnemonic,
    required String network,
    required String pin,
  }) async {
    final path = await walletPath();
    final info = await rust.argosRestoreWallet(
      mnemonic: mnemonic,
      network: network,
      pin: pin,
      storagePath: path,
    );
    _cachedPubkey = info.pubkeyB58;
    _cachedNetwork = info.network;
    await _secure.write(key: _kStoredPubkey, value: info.pubkeyB58);
    await _secure.write(key: _kStoredNetwork, value: info.network);
    return info;
  }

  /// Unlock an existing wallet with `pin`. Throws on wrong PIN.
  Future<String> unlock(String pin) async {
    final path = await walletPath();
    final pubkey = await rust.argosUnlockWallet(pin: pin, storagePath: path);
    _cachedPubkey = pubkey;
    _cachedNetwork = rust.argosWalletNetwork();
    return pubkey;
  }

  /// Wipe the Rust-side cache. UI calls this on app-backgrounded.
  Future<void> lock() async {
    await rust.argosLockWallet();
    _cachedPubkey = null;
  }

  /// Permanently delete the on-disk encrypted wallet AND its secure-storage
  /// metadata. Caller MUST confirm with a destructive double-tap dialog.
  Future<void> wipe() async {
    await lock();
    final p = await walletPath();
    final f = File(p);
    if (f.existsSync()) {
      await f.delete();
    }
    await _secure.delete(key: _kStoredPubkey);
    await _secure.delete(key: _kStoredNetwork);
    await _secure.delete(key: _kAutoLockPin);
  }

  /// SOL balance in lamports.
  Future<BigInt> balanceSol() => rust.argosBalanceSol();

  /// SPL token balance for `mint`, raw units.
  Future<BigInt> balanceToken(String mint) =>
      rust.argosBalanceToken(mintB58: mint);

  /// Native SOL send.
  Future<String> sendSol({required String recipient, required BigInt lamports}) =>
      rust.argosSendSol(recipientB58: recipient, lamports: lamports);

  /// SPL token send.
  Future<String> sendToken({
    required String mint,
    required String recipient,
    required BigInt amount,
  }) => rust.argosSendToken(
        mintB58: mint,
        recipientB58: recipient,
        amount: amount,
      );

  /// Jupiter quote — preview a swap.
  Future<rust.ArgosSwapPreview> quoteSwap({
    required String inputMint,
    required String outputMint,
    required BigInt amountIn,
    required int slippageBps,
  }) => rust.argosQuoteSwap(
        inputMintB58: inputMint,
        outputMintB58: outputMint,
        amountIn: amountIn,
        slippageBps: slippageBps,
      );

  /// Execute the cached preview as a swap (output stays in user wallet).
  Future<String> executeSwap() => rust.argosExecuteSwap();

  /// **Auto-Swap-on-Send** — atomic swap+deliver in one tx.
  Future<rust.ArgosSwapAndSendOutcome> swapAndSend(String recipient) =>
      rust.argosSwapAndSend(recipientB58: recipient);

  /// Address validation — returns canonical base58 form or throws.
  String validateAddress(String s) => rust.argosValidateAddress(s: s);

  /// Devnet QA — request a 1 SOL airdrop. Throws on mainnet.
  Future<String> devnetAirdropOneSol() => rust.argosDevnetAirdropOneSol();
}

/// Well-known SPL mints we let the UI surface natively. Keep tiny; expand
/// as we add more first-class assets. Order matters — index 0 is the
/// default "send" pre-selection for SPL transfers.
const argosKnownTokens = [
  ArgosKnownToken(
    symbol: 'USDC',
    name: 'USD Coin',
    mint: 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
    decimals: 6,
  ),
  ArgosKnownToken(
    symbol: 'USDT',
    name: 'Tether',
    mint: 'Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB',
    decimals: 6,
  ),
];

const argosWsolMint = 'So11111111111111111111111111111111111111112';

class ArgosKnownToken {
  final String symbol;
  final String name;
  final String mint;
  final int decimals;
  const ArgosKnownToken({
    required this.symbol,
    required this.name,
    required this.mint,
    required this.decimals,
  });
}
