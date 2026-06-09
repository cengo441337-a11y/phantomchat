import 'dart:convert';
import 'dart:io';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:path_provider/path_provider.dart';

import 'app_lock_service.dart';
import '../src/rust/api/wallet.dart' as rust;
import '../src/rust/api/wallet/eth_api.dart' as eth_rust;

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
  static const _kFailedAttempts = 'argos.unlock.failed_attempts';

  /// Persisted count of consecutive wrong-PIN attempts. Survives app
  /// restarts and backgrounding so the 10-try panic-wipe cannot be reset
  /// by an attacker force-stopping the app between attempt batches.
  Future<int> failedAttempts() async {
    final raw = await _secure.read(key: _kFailedAttempts);
    return int.tryParse(raw ?? '0') ?? 0;
  }

  /// Increment + persist the failed-attempt counter. Returns the new value.
  Future<int> incrementFailedAttempts() async {
    final next = (await failedAttempts()) + 1;
    await _secure.write(key: _kFailedAttempts, value: next.toString());
    return next;
  }

  /// Reset the counter (call on every successful unlock).
  Future<void> resetFailedAttempts() async {
    await _secure.delete(key: _kFailedAttempts);
  }

  // ── Biometric unlock (T3-2) ────────────────────────────────────────────
  // The PIN is the Argon2id KDF input, so biometrics alone can't decrypt the
  // keypair. We store the PIN in hardware-backed secure storage and gate its
  // retrieval behind a fresh biometric auth — fingerprint/FaceID then unlocks
  // without typing the PIN. Disabled by default; the user opts in.

  Future<bool> biometricUnlockEnabled() async =>
      (await _secure.read(key: _kAutoLockPin)) != null;

  Future<void> enableBiometricUnlock(String pin) async =>
      _secure.write(key: _kAutoLockPin, value: pin);

  Future<void> disableBiometricUnlock() async =>
      _secure.delete(key: _kAutoLockPin);

  /// Authenticate via biometrics, then unlock the wallet with the stored
  /// PIN. Returns the pubkey on success. Throws on biometric failure or if
  /// biometric-unlock isn't enabled.
  Future<String> unlockWithBiometric() async {
    final ok = await AppLockService.authenticateBiometric(
        reason: 'Argos Wallet entsperren');
    if (!ok) throw 'Biometrie abgebrochen';
    final pin = await _secure.read(key: _kAutoLockPin);
    if (pin == null) throw 'Biometrik-Unlock nicht aktiviert';
    final pk = await unlock(pin);
    await resetFailedAttempts();
    return pk;
  }

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
    // Delete the mnemonic sidecar FIRST (via Rust, same path derivation as
    // persist/load) so the recovery phrase can never survive a wipe — this
    // includes the 10-try panic-wipe. Without this the .mn.enc.json blob
    // stayed on disk and a stolen device kept the phrase recoverable.
    try {
      await eth_rust.argosWipeMnemonicSidecar(storagePath: p);
    } catch (_) {
      // Best-effort; fall through to delete the Solana blob regardless.
    }
    final f = File(p);
    if (f.existsSync()) {
      await f.delete();
    }
    // Belt-and-suspenders: also remove the sidecar Dart-side in case the
    // FFI path ever drifts. Derivation mirrors mnemonic_sidecar_path:
    // strip the last extension, append .mn.enc.json.
    final stem = p.endsWith('.json') ? p.substring(0, p.length - 5) : p;
    final sidecar = File('$stem.mn.enc.json');
    if (sidecar.existsSync()) {
      await sidecar.delete();
    }
    await _secure.delete(key: _kStoredPubkey);
    await _secure.delete(key: _kStoredNetwork);
    await _secure.delete(key: _kAutoLockPin);
    await _secure.delete(key: _kFailedAttempts);
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

  /// Recent Solana transaction history (newest first, up to [limit]).
  Future<List<rust.ArgosTxRow>> recentSignatures({int limit = 25}) =>
      rust.argosRecentSignatures(limit: limit);

  // ── EVM (Ethereum / Base / Polygon) — v1.4.0 ───────────────────────────

  /// EIP-55 address derived for the given EVM network. Re-derives every
  /// call from the cached mnemonic — no on-disk ETH secret.
  Future<String> ethAddress(String network) =>
      eth_rust.argosEthAddress(network: network);

  /// Native balance in wei (decimal string — U256 may overflow Dart's int).
  Future<String> ethBalanceWei(String network) =>
      eth_rust.argosEthBalanceWei(network: network);

  /// ERC-20 balance for [token] in raw base units (decimal string).
  Future<String> ethErc20Balance(String network, String token) =>
      eth_rust.argosEthErc20Balance(network: network, token: token);

  /// Send native ETH/MATIC/etc. `wei` is a decimal-string amount.
  Future<String> ethSendNative({
    required String network,
    required String recipient,
    required String wei,
  }) =>
      eth_rust.argosEthSendNative(
        network: network,
        recipient: recipient,
        wei: wei,
      );

  /// ERC-20 transfer. `amount` is a decimal-string of raw base units.
  Future<String> ethSendErc20({
    required String network,
    required String token,
    required String recipient,
    required String amount,
  }) =>
      eth_rust.argosEthSendErc20(
        network: network,
        token: token,
        recipient: recipient,
        amount: amount,
      );

  /// UI helper: format a wei decimal-string as a 4-decimal ETH string.
  Future<String> ethFormat(String wei) => eth_rust.argosEthFormat(wei: wei);

  /// EIP-55 address validation. Returns the canonical form on success.
  Future<String> ethValidateAddress(String s) => eth_rust.argosEthValidateAddress(s: s);
}

/// Chains a user can pick from in the wallet UI. Solana lives on its own
/// network selector (mainnet-beta / devnet) so it appears twice when
/// onboarding wants to surface both Solana variants — the EVM entries
/// pass straight through to `ethAddress(network)` etc.
enum ArgosChain {
  solanaMainnet,
  solanaDevnet,
  ethereum,
  base,
  polygon;

  String get label => switch (this) {
        ArgosChain.solanaMainnet => 'Solana',
        ArgosChain.solanaDevnet => 'Solana Devnet',
        ArgosChain.ethereum => 'Ethereum',
        ArgosChain.base => 'Base',
        ArgosChain.polygon => 'Polygon',
      };

  String get shortLabel => switch (this) {
        ArgosChain.solanaMainnet => 'SOL',
        ArgosChain.solanaDevnet => 'SOL-D',
        ArgosChain.ethereum => 'ETH',
        ArgosChain.base => 'BASE',
        ArgosChain.polygon => 'POLY',
      };

  bool get isSolana =>
      this == ArgosChain.solanaMainnet || this == ArgosChain.solanaDevnet;

  bool get isEvm => !isSolana;

  /// Backend-facing network identifier (matches the Rust FFI vocabulary).
  String get backendId => switch (this) {
        ArgosChain.solanaMainnet => 'mainnet-beta',
        ArgosChain.solanaDevnet => 'devnet',
        ArgosChain.ethereum => 'ethereum',
        ArgosChain.base => 'base',
        ArgosChain.polygon => 'polygon',
      };
}

/// First-class EVM tokens that get surfaced in the wallet UI per chain.
/// Mints are checksummed Ethereum addresses; chain decides which list applies.
const argosEvmKnownTokens = <ArgosEvmToken>[
  ArgosEvmToken(
    chain: ArgosChain.ethereum,
    symbol: 'USDC',
    name: 'USD Coin',
    address: '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48',
    decimals: 6,
  ),
  ArgosEvmToken(
    chain: ArgosChain.ethereum,
    symbol: 'USDT',
    name: 'Tether',
    address: '0xdAC17F958D2ee523a2206206994597C13D831ec7',
    decimals: 6,
  ),
  ArgosEvmToken(
    chain: ArgosChain.base,
    symbol: 'USDC',
    name: 'USD Coin (Base)',
    address: '0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913',
    decimals: 6,
  ),
  ArgosEvmToken(
    chain: ArgosChain.polygon,
    symbol: 'USDC',
    name: 'USD Coin (Polygon)',
    address: '0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359',
    decimals: 6,
  ),
  ArgosEvmToken(
    chain: ArgosChain.polygon,
    symbol: 'USDT',
    name: 'Tether (Polygon)',
    address: '0xc2132D05D31c914a87C6611C10748AEb04B58e8F',
    decimals: 6,
  ),
];

class ArgosEvmToken {
  final ArgosChain chain;
  final String symbol;
  final String name;
  final String address;
  final int decimals;
  const ArgosEvmToken({
    required this.chain,
    required this.symbol,
    required this.name,
    required this.address,
    required this.decimals,
  });
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

/// Parse a user-entered decimal amount string into raw base units using
/// EXACT BigInt math. `double` has a 53-bit mantissa (~15-16 significant
/// digits) and silently loses precision / overflows for high-decimal tokens
/// (USDC 6, SOL 9, ETH 18). Going through `double` could send a slightly
/// different amount than the user typed. This stays integer the whole way.
///
/// Truncates any digits beyond `decimals` (never rounds up) so the user can
/// never accidentally send MORE than they entered. Returns null on invalid
/// input or a non-positive amount.
BigInt? decimalToBaseUnits(String input, int decimals) {
  var t = input.trim().replaceAll(',', '.');
  if (t.isEmpty || t == '.') return null;
  if (!RegExp(r'^\d*\.?\d*$').hasMatch(t)) return null;
  final dot = t.indexOf('.');
  String wholePart;
  String fracPart;
  if (dot < 0) {
    wholePart = t;
    fracPart = '';
  } else {
    wholePart = t.substring(0, dot);
    fracPart = t.substring(dot + 1);
  }
  if (wholePart.isEmpty) wholePart = '0';
  if (fracPart.length > decimals) {
    fracPart = fracPart.substring(0, decimals);
  } else {
    fracPart = fracPart.padRight(decimals, '0');
  }
  final combined = wholePart + fracPart;
  final v = BigInt.tryParse(combined.isEmpty ? '0' : combined);
  if (v == null || v <= BigInt.zero) return null;
  return v;
}

/// A Solana NFT as returned by the Argos NFT proxy (Helius DAS server-side).
class ArgosNft {
  final String id;
  final String name;
  final String? image;
  final String? collection;
  const ArgosNft({
    required this.id,
    required this.name,
    this.image,
    this.collection,
  });
  factory ArgosNft.fromJson(Map<String, dynamic> j) => ArgosNft(
        id: j['id'] as String? ?? '',
        name: j['name'] as String? ?? 'Unnamed',
        image: j['image'] as String?,
        collection: j['collection'] as String?,
      );
}

/// Fetches the Solana NFTs owned by [owner] via the Argos backend proxy
/// (which holds the Helius key server-side). Best-effort: returns [] on error.
Future<List<ArgosNft>> fetchArgosNfts(String owner) async {
  const base = 'https://pylonyx-dev.dc-infosec.de/api/argos/nfts';
  try {
    final client = HttpClient()..connectionTimeout = const Duration(seconds: 10);
    final req = await client
        .getUrl(Uri.parse('$base?owner=${Uri.encodeComponent(owner)}'))
        .timeout(const Duration(seconds: 10));
    final res = await req.close().timeout(const Duration(seconds: 12));
    if (res.statusCode != 200) return [];
    final body = await res.transform(const Utf8Decoder()).join();
    client.close(force: true);
    final json = jsonDecode(body) as Map<String, dynamic>;
    final list = (json['nfts'] as List?) ?? const [];
    return list
        .map((e) => ArgosNft.fromJson(e as Map<String, dynamic>))
        .toList();
  } catch (_) {
    return [];
  }
}
