import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:google_fonts/google_fonts.dart';

import '../services/chat_payment.dart';
import '../services/wallet_service.dart';
import '../theme.dart';

/// Renders a `payreq` or `paid` chat-payment payload as a card. For an
/// incoming request the card shows a "Bezahlen" button; for a receipt it
/// links to the block explorer.
class PaymentCard extends StatelessWidget {
  final ChatPayment payment;
  final bool outgoing;

  /// Called when the recipient taps "Bezahlen" on an incoming request.
  final VoidCallback? onPay;

  const PaymentCard({
    super.key,
    required this.payment,
    required this.outgoing,
    this.onPay,
  });

  @override
  Widget build(BuildContext context) {
    final isReq = payment.kind == 'payreq';
    final accent = isReq ? kYellow : kGreen;
    return Align(
      alignment: outgoing ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 4),
        padding: const EdgeInsets.all(14),
        constraints: const BoxConstraints(maxWidth: 280),
        decoration: BoxDecoration(
          color: kBgCard,
          border: Border.all(color: accent.withValues(alpha: 0.6)),
          borderRadius: BorderRadius.circular(10),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(isReq ? Icons.request_page : Icons.check_circle,
                    color: accent, size: 18),
                const SizedBox(width: 8),
                Text(
                  isReq ? 'ZAHLUNGSANFRAGE' : 'BEZAHLT',
                  style: GoogleFonts.orbitron(
                      color: accent,
                      fontSize: 10,
                      letterSpacing: 2,
                      fontWeight: FontWeight.w700),
                ),
              ],
            ),
            const SizedBox(height: 10),
            Row(
              crossAxisAlignment: CrossAxisAlignment.baseline,
              textBaseline: TextBaseline.alphabetic,
              children: [
                Text(payment.amount,
                    style: GoogleFonts.orbitron(
                        color: kWhite,
                        fontSize: 24,
                        fontWeight: FontWeight.w700)),
                const SizedBox(width: 6),
                Text(payment.asset,
                    style: GoogleFonts.orbitron(
                        color: kWhiteDim, fontSize: 13, letterSpacing: 1)),
              ],
            ),
            const SizedBox(height: 4),
            Text(payment.parsedChain?.label ?? payment.chain,
                style: GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 10)),
            if (isReq && !outgoing && onPay != null) ...[
              const SizedBox(height: 12),
              SizedBox(
                width: double.infinity,
                child: ElevatedButton(
                  onPressed: onPay,
                  child: const Text('BEZAHLEN'),
                ),
              ),
            ],
            if (isReq && outgoing) ...[
              const SizedBox(height: 8),
              Text('Wartet auf Zahlung …',
                  style:
                      GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 10)),
            ],
            if (!isReq && payment.explorerUrl() != null) ...[
              const SizedBox(height: 10),
              GestureDetector(
                onTap: () {
                  Clipboard.setData(
                      ClipboardData(text: payment.explorerUrl()!));
                  ScaffoldMessenger.of(context).showSnackBar(SnackBar(
                    content: Text('Explorer-Link kopiert',
                        style: GoogleFonts.spaceMono(color: kCyan)),
                    backgroundColor: kBgCard,
                    duration: const Duration(seconds: 2),
                  ));
                },
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const Icon(Icons.open_in_new, color: kGrayText, size: 13),
                    const SizedBox(width: 6),
                    Text(
                      '${payment.signature!.substring(0, 10)}…',
                      style: GoogleFonts.spaceMono(
                          color: kCyan, fontSize: 10),
                    ),
                  ],
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// Compose a payment request. Returns a [ChatPayment] (kind=payreq) on
/// "Anfordern", or null on cancel. Requires the wallet to be unlocked to
/// resolve the requester's receive address for the chosen chain.
class PaymentRequestSheet extends StatefulWidget {
  const PaymentRequestSheet({super.key});

  @override
  State<PaymentRequestSheet> createState() => _PaymentRequestSheetState();
}

class _PaymentRequestSheetState extends State<PaymentRequestSheet> {
  final _svc = ArgosWalletService.instance;
  ArgosChain _chain = ArgosChain.solanaMainnet;
  late String _asset;
  final _amount = TextEditingController();
  String? _error;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    _asset = _assetsFor(_chain).first;
  }

  @override
  void dispose() {
    _amount.dispose();
    super.dispose();
  }

  List<String> _assetsFor(ArgosChain c) {
    if (c.isSolana) return ['SOL', ...argosKnownTokens.map((t) => t.symbol)];
    final native = c == ArgosChain.polygon ? 'MATIC' : 'ETH';
    return [
      native,
      ...argosEvmKnownTokens.where((t) => t.chain == c).map((t) => t.symbol),
    ];
  }

  Future<void> _request() async {
    final amt = _amount.text.trim().replaceAll(',', '.');
    if (amt.isEmpty || double.tryParse(amt) == null ||
        double.parse(amt) <= 0) {
      setState(() => _error = 'Betrag ungültig.');
      return;
    }
    if (!_svc.isUnlocked) {
      setState(() => _error =
          'Wallet ist gesperrt — im Wallet-Tab entsperren, dann erneut.');
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      // My own receive address for the chosen chain travels in the request.
      final addr = _chain.isSolana
          ? (_svc.pubkey ?? '')
          : await _svc.ethAddress(_chain.backendId);
      if (addr.isEmpty) throw 'Adresse konnte nicht ermittelt werden.';
      final pay = ChatPayment(
        kind: 'payreq',
        chain: _chain.backendId,
        asset: _asset,
        amount: amt,
        address: addr,
      );
      if (!mounted) return;
      Navigator.pop(context, pay);
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = '$e';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final assets = _assetsFor(_chain);
    if (!assets.contains(_asset)) _asset = assets.first;
    return SafeArea(
      child: Padding(
        padding: EdgeInsets.only(
          left: 20,
          right: 20,
          top: 20,
          bottom: 20 + MediaQuery.of(context).viewInsets.bottom,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('ZAHLUNG ANFORDERN',
                style: GoogleFonts.orbitron(
                    color: kCyan,
                    fontSize: 14,
                    letterSpacing: 3,
                    fontWeight: FontWeight.w700)),
            const SizedBox(height: 16),
            // Chain picker
            SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: ArgosChain.values.map((c) {
                  final sel = c == _chain;
                  return Padding(
                    padding: const EdgeInsets.only(right: 6),
                    child: GestureDetector(
                      onTap: () => setState(() {
                        _chain = c;
                        _asset = _assetsFor(c).first;
                      }),
                      child: Container(
                        padding: const EdgeInsets.symmetric(
                            horizontal: 12, vertical: 8),
                        decoration: BoxDecoration(
                          color: sel ? kCyanDim : Colors.transparent,
                          border: Border.all(color: sel ? kCyan : kGray),
                        ),
                        child: Text(c.shortLabel,
                            style: GoogleFonts.orbitron(
                                color: sel ? kCyan : kGrayText,
                                fontSize: 10,
                                letterSpacing: 2,
                                fontWeight: FontWeight.w700)),
                      ),
                    ),
                  );
                }).toList(),
              ),
            ),
            const SizedBox(height: 10),
            // Asset chips
            SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: assets.map((a) {
                  final sel = a == _asset;
                  return Padding(
                    padding: const EdgeInsets.only(right: 6),
                    child: GestureDetector(
                      onTap: () => setState(() => _asset = a),
                      child: Container(
                        padding: const EdgeInsets.symmetric(
                            horizontal: 12, vertical: 8),
                        decoration: BoxDecoration(
                          color: sel ? kCyanDim : Colors.transparent,
                          border: Border.all(color: sel ? kCyan : kGray),
                        ),
                        child: Text(a,
                            style: GoogleFonts.orbitron(
                                color: sel ? kCyan : kGrayText,
                                fontSize: 11,
                                letterSpacing: 1,
                                fontWeight: FontWeight.w700)),
                      ),
                    ),
                  );
                }).toList(),
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _amount,
              keyboardType:
                  const TextInputType.numberWithOptions(decimal: true),
              decoration: InputDecoration(labelText: 'BETRAG · $_asset'),
              style: GoogleFonts.spaceMono(
                  color: kCyan, fontSize: 18, letterSpacing: 2),
            ),
            if (_error != null) ...[
              const SizedBox(height: 10),
              Text(_error!,
                  style:
                      GoogleFonts.spaceMono(color: kMagenta, fontSize: 11)),
            ],
            const SizedBox(height: 16),
            ElevatedButton(
              onPressed: _busy ? null : _request,
              child: _busy
                  ? const SizedBox(
                      width: 18,
                      height: 18,
                      child: CircularProgressIndicator(
                          color: kCyan, strokeWidth: 2))
                  : const Text('ANFORDERN'),
            ),
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: Text('ABBRECHEN',
                  style: GoogleFonts.orbitron(
                      color: kWhiteDim, fontSize: 11, letterSpacing: 2)),
            ),
          ],
        ),
      ),
    );
  }
}

/// Pay an incoming request. Sends the funds via ArgosWalletService and
/// returns a [ChatPayment] (kind=paid) with the tx signature on success,
/// or null on cancel/failure.
class PaySheet extends StatefulWidget {
  final ChatPayment request;
  const PaySheet({super.key, required this.request});

  @override
  State<PaySheet> createState() => _PaySheetState();
}

class _PaySheetState extends State<PaySheet> {
  final _svc = ArgosWalletService.instance;
  bool _busy = false;
  String? _error;

  int _decimalsFor(ArgosChain chain, String asset) {
    final isNative =
        asset == 'SOL' || asset == 'ETH' || asset == 'MATIC';
    if (isNative) return chain.isSolana ? 9 : 18;
    if (chain.isSolana) {
      return argosKnownTokens.firstWhere((t) => t.symbol == asset).decimals;
    }
    return argosEvmKnownTokens
        .firstWhere((t) => t.chain == chain && t.symbol == asset)
        .decimals;
  }

  Future<void> _pay() async {
    final req = widget.request;
    final chain = req.parsedChain;
    if (chain == null) {
      setState(() => _error = 'Unbekannte Chain.');
      return;
    }
    if (!_svc.isUnlocked) {
      setState(() => _error =
          'Wallet gesperrt — im Wallet-Tab entsperren, dann erneut.');
      return;
    }
    final base =
        decimalToBaseUnits(req.amount, _decimalsFor(chain, req.asset));
    if (base == null) {
      setState(() => _error = 'Betrag ungültig.');
      return;
    }
    final addr = req.address;
    if (addr == null || addr.isEmpty) {
      setState(() => _error = 'Empfängeradresse fehlt in der Anfrage.');
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      String sig;
      final isNative =
          req.asset == 'SOL' || req.asset == 'ETH' || req.asset == 'MATIC';
      if (chain.isSolana) {
        if (isNative) {
          sig = await _svc.sendSol(recipient: addr, lamports: base);
        } else {
          final tok =
              argosKnownTokens.firstWhere((t) => t.symbol == req.asset);
          sig = await _svc.sendToken(
              mint: tok.mint, recipient: addr, amount: base);
        }
      } else {
        if (isNative) {
          sig = await _svc.ethSendNative(
              network: chain.backendId,
              recipient: addr,
              wei: base.toString());
        } else {
          final tok = argosEvmKnownTokens
              .firstWhere((t) => t.chain == chain && t.symbol == req.asset);
          sig = await _svc.ethSendErc20(
              network: chain.backendId,
              token: tok.address,
              recipient: addr,
              amount: base.toString());
        }
      }
      final receipt = ChatPayment(
        kind: 'paid',
        chain: req.chain,
        asset: req.asset,
        amount: req.amount,
        signature: sig,
      );
      if (!mounted) return;
      Navigator.pop(context, receipt);
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = '$e';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final req = widget.request;
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('ZAHLUNG BESTÄTIGEN',
                style: GoogleFonts.orbitron(
                    color: kCyan,
                    fontSize: 14,
                    letterSpacing: 3,
                    fontWeight: FontWeight.w700)),
            const SizedBox(height: 16),
            Row(
              crossAxisAlignment: CrossAxisAlignment.baseline,
              textBaseline: TextBaseline.alphabetic,
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Text(req.amount,
                    style: GoogleFonts.orbitron(
                        color: kCyan,
                        fontSize: 32,
                        fontWeight: FontWeight.w700)),
                const SizedBox(width: 8),
                Text(req.asset,
                    style: GoogleFonts.orbitron(
                        color: kWhiteDim, fontSize: 14, letterSpacing: 2)),
              ],
            ),
            const SizedBox(height: 4),
            Text('auf ${req.parsedChain?.label ?? req.chain}',
                style: GoogleFonts.spaceMono(color: kWhiteDim, fontSize: 11)),
            const SizedBox(height: 8),
            Text(
              '→ ${req.address != null && req.address!.length > 12 ? "${req.address!.substring(0, 6)}…${req.address!.substring(req.address!.length - 6)}" : req.address ?? "?"}',
              style: GoogleFonts.spaceMono(color: kCyan, fontSize: 12),
            ),
            if (_error != null) ...[
              const SizedBox(height: 12),
              Text(_error!,
                  textAlign: TextAlign.center,
                  style:
                      GoogleFonts.spaceMono(color: kMagenta, fontSize: 11)),
            ],
            const SizedBox(height: 18),
            ElevatedButton(
              onPressed: _busy ? null : _pay,
              child: _busy
                  ? const SizedBox(
                      width: 18,
                      height: 18,
                      child: CircularProgressIndicator(
                          color: kCyan, strokeWidth: 2))
                  : Text('${req.amount} ${req.asset} SENDEN'),
            ),
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: Text('ABBRECHEN',
                  style: GoogleFonts.orbitron(
                      color: kWhiteDim, fontSize: 11, letterSpacing: 2)),
            ),
          ],
        ),
      ),
    );
  }
}
