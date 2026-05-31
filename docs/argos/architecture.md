# Argos — Architektur-Spezifikation v0.1

**Status:** Draft, 2026-05-31
**Autor:** Deniz + Claude Code
**Branch (Vorgänger-Code):** `audit/transactional-ratchet-2026-05-30` (PhantomChat) → Migration nach `argos/rebrand-prep`

---

## 1. Mission

Argos ist ein **non-custodial Krypto-Messenger** mit drei Schutz-Schichten:
1. **End-to-end-verschlüsselte Kommunikation** (PhantomChat-Ratchet-Stack, AGPLv3)
2. **Non-custodial Wallet** mit Send/Receive + Auto-Swap (Solana zuerst, später Multi-Chain)
3. **Pylonyx-Risk-Score** als Pre-Send-Schutz (Closed Source SaaS, Pylonyx-Backend)

Argos warnt. Pylonyx informiert. Niemand gibt Kaufempfehlungen. Geld bleibt beim User.

## 2. System-Layer

```
┌──────────────────────────────────────────────────────────────┐
│ MOBILE APP (Flutter) / DESKTOP APP (Tauri + React)           │
├──────────────────────────────────────────────────────────────┤
│  UI-Layer (open source, AGPLv3)                              │
│  ├── Chat (Ratchet + Stealth + MLS)                          │
│  ├── Wallet (Solana SDK, Ledger-Support)                     │
│  ├── Swap (Jupiter v6 Aggregator UI)                         │
│  └── Pylonyx-Widget (Risk-Score rendering)                   │
├──────────────────────────────────────────────────────────────┤
│  Core-Bridge (Rust via flutter_rust_bridge / Tauri IPC)      │
│  ├── phantomchat_core (Ratchet, Envelope, MLS)               │
│  ├── argos_wallet (Solana keypair, mnemonic, signing)        │
│  └── argos_swap (Jupiter SDK call, atomic-tx-builder)        │
├──────────────────────────────────────────────────────────────┤
│  Network                                                     │
│  ├── relay.dc-infosec.de  (Nostr broadcast, eigener Relay)   │
│  ├── api.mainnet-beta.solana.com  (oder Helius / Triton RPC) │
│  ├── jup.ag/v6  (Swap-Aggregation API)                       │
│  └── argos.dc-infosec.de/api/risk  (Pylonyx Backend)         │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│ ARGOS BACKEND (Hostinger VPS, Closed Source SaaS)            │
├──────────────────────────────────────────────────────────────┤
│  /api/risk         POST {token_addr, target_wallet}          │
│                    → {score, warnings[], honeypot, lp, age}  │
│  /api/health       Smart-Money-Flow + Liquidity-Trends       │
│  /api/abo          Pylonyx-VIP-Subscription-Mgmt             │
│  Pylonyx-Score-Engine  (existing pylonyx-web codebase)       │
└──────────────────────────────────────────────────────────────┘
```

## 3. Wallet-Service (Rust, in core)

### 3.1 Key-Management

```rust
// argos_wallet/src/lib.rs

pub struct ArgosWallet {
    /// 64-byte Solana keypair (32 priv + 32 pub).
    /// Lives in memory only after PIN-decrypt, zeroized on drop.
    keypair: Zeroizing<[u8; 64]>,
    /// Network — mainnet-beta or devnet for tests.
    network: Network,
}

impl ArgosWallet {
    /// Generate fresh wallet from secure RNG. Returns mnemonic for backup.
    pub fn generate(network: Network) -> (Self, Mnemonic) { ... }

    /// Restore from BIP39 mnemonic (12 or 24 words).
    pub fn from_mnemonic(words: &str, network: Network) -> Result<Self> { ... }

    /// Encrypt + persist using Argon2id-derived KEK from user PIN.
    /// File: <app_data>/wallet.enc.json
    pub fn persist_encrypted(&self, pin: &str) -> Result<()> { ... }

    /// Load + decrypt with PIN.
    pub fn load_encrypted(pin: &str) -> Result<Self> { ... }

    pub fn pubkey(&self) -> Pubkey { ... }
    pub fn sign(&self, message: &[u8]) -> Signature { ... }
}
```

### 3.2 Storage-Format

```json
// wallet.enc.json
{
  "version": 1,
  "kdf": "argon2id",
  "kdf_params": { "m_cost": 65536, "t_cost": 3, "p_cost": 4 },
  "salt": "<32 bytes base64>",
  "nonce": "<24 bytes base64>",
  "ciphertext": "<XChaCha20-Poly1305 of {keypair, network}>"
}
```

PIN-Brute-Force-Schutz: Argon2id mit OWASP-2023-Parametern, plus File-Lock auf 10 falsche PINs (existing PhantomChat-PIN-Logic wiederverwenden).

### 3.3 Hardware-Wallet (Phase 2)

Ledger-Anbindung via `solana-ledger-wallet` Rust-Crate. Erlaubt `sign_with_ledger()` statt `sign()`. Treasury-Wallet **MUSS** Hardware-only sein.

## 4. Send-Flow (klassisch, kein Swap)

```
User in Chat → "Senden" → SendSheet
    ↓
[Adresse: Empfänger-Solana-Address oder Phantom-ID (Chat-Contact)]
[Token: dropdown SOL / USDC / USDT / SPL-Token-Picker]
[Betrag]
    ↓
Pre-Send-Check parallel:
  - Pylonyx-Risk(Empfänger-Wallet) → blacklist?
  - Pylonyx-Risk(Token-Mint) → honeypot? rug?
  - Native-Balance reicht für Tx-Fee (~0.000005 SOL)?
    ↓
┌──────────────────────────────────┐
│ Senden:                          │
│   100 USDC an Alice              │
│   Gebühr: ~0.0001 SOL ($0.02)   │
│                                  │
│ 🛡 Pylonyx: Adresse clean       │
│                                  │
│ [ Senden → ]                     │
└──────────────────────────────────┘
    ↓
solana-client: build_transaction → sign → send_raw_transaction
    ↓
Chat-Nachricht erweitert mit crypto_attachment Block (siehe §6)
```

## 5. Swap-Flow (Jupiter v6)

### 5.1 Quote

```rust
// argos_swap/src/lib.rs

pub async fn quote(
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount_in: u64,
    slippage_bps: u16,  // 50 = 0.5%
    user_pubkey: Pubkey,
    treasury_referral: Pubkey,  // Argos-Fee-Wallet
) -> Result<JupiterQuote> {
    let url = format!(
        "https://quote-api.jup.ag/v6/quote?\
         inputMint={}&outputMint={}&amount={}&slippageBps={}&\
         platformFeeBps=50&\
         feeAccount={}",
        input_mint, output_mint, amount_in, slippage_bps,
        treasury_referral_token_account
    );
    reqwest::get(url).await?.json().await
}
```

**Fee-Modell:** `platformFeeBps=50` → 0,5%. Jupiter führt die Fee atomic im selben Tx-Block aus, geht direkt an unsere Treasury (über das Associated-Token-Account auf Treasury-Wallet).

### 5.2 Swap-Transaction

```rust
pub async fn swap(
    quote: &JupiterQuote,
    wallet: &ArgosWallet,
) -> Result<Signature> {
    // 1. Jupiter swap-instruction holen
    let swap_resp = reqwest::Client::new()
        .post("https://quote-api.jup.ag/v6/swap")
        .json(&SwapRequest {
            quote_response: quote.clone(),
            user_public_key: wallet.pubkey(),
            wrap_and_unwrap_sol: true,
            ...
        })
        .send().await?.json::<SwapResponse>().await?;

    // 2. Deserialize + sign
    let mut tx = bincode::deserialize::<VersionedTransaction>(
        &base64::decode(swap_resp.swap_transaction)?
    )?;
    tx.sign(&[&wallet.keypair_arc()], tx.message.recent_blockhash);

    // 3. Send + confirm
    let sig = rpc_client.send_and_confirm_transaction(&tx).await?;
    Ok(sig)
}
```

### 5.3 Auto-Swap-on-Send (Killer-Feature)

```rust
/// Atomic: swap input_mint → output_mint, dann transfer output_mint an recipient,
/// alles in einer einzigen Solana-Transaction (oder als versionedTx mit lookup-tables).
pub async fn swap_and_send(
    wallet: &ArgosWallet,
    input_mint: Pubkey,     // hat user
    output_mint: Pubkey,    // will user senden
    amount_out_desired: u64,// will user senden
    recipient: Pubkey,
    treasury_referral: Pubkey,
) -> Result<Signature> {
    // 1. ExactOut quote ("ich will GENAU X output, sag mir wie viel input")
    let quote = jupiter::quote_exact_out(...).await?;

    // 2. Swap-instructions holen (output landet in user-wallet)
    let swap_instructions = jupiter::swap_instructions(...).await?;

    // 3. Transfer-instruction anhängen (output → recipient)
    let transfer_ix = spl_token::instruction::transfer(
        &spl_token::id(),
        &wallet.token_account_for(output_mint),
        &recipient_token_account,
        &wallet.pubkey(),
        &[], amount_out_desired,
    )?;

    // 4. Single VersionedTransaction bauen mit allen instructions
    let tx = build_versioned_tx(
        &wallet,
        [
            swap_instructions.compute_budget_instructions,
            swap_instructions.setup_instructions,
            swap_instructions.swap_instruction,
            vec![transfer_ix],     // ← atomic anhängen
            swap_instructions.cleanup_instruction,
        ].concat()
    )?;

    rpc.send_and_confirm_transaction(&tx).await
}
```

**UX:** User schreibt "ich schick dir 50 USDC", hat aber nur SOL. App zeigt:
> 🔄 Auto-Swap aktiv: 0,32 SOL → 50 USDC via Jupiter
> Sender-App-Fee: 0,5% (0,25 USDC)
> 🛡 Pylonyx: USDC-Mint clean, Recipient clean
> [ Senden → ]

Eine Tx, eine Bestätigung, fertig. Empfänger sieht nur "+50 USDC".

## 6. Chat-Envelope-Erweiterung

PhantomChats Wire-Format wird um optionalen `crypto_attachment` Block ergänzt:

```text
Payload (inside outer envelope, after AEAD-decrypt):
┌──────────────────────────────────────────────────────────────┐
│ ratchet_header  (60 B)                                       │
│ encrypted_body  (var)        ← plaintext message text       │
│ sender_attribution  (opt 97 B SealedSender)                  │
│ NEW: crypto_attachment  (opt, length-prefixed)               │
│   ├── kind:  u8  (0=none, 1=tx-receipt, 2=intent, 3=request) │
│   ├── chain: u8  (1=Solana, 2=Eth, 3=Lightning)              │
│   ├── tx_sig: 64 B  (Solana signature, for receipt)          │
│   ├── token_mint: 32 B  (Solana SPL mint, all-zero for SOL)  │
│   ├── amount:  u64  (raw token amount, no decimals)          │
│   ├── memo:    var  (UTF-8, max 256 B)                       │
│   └── pylonyx_score: opt 4 B  (risk score at time of send)   │
│ padding  (random, pads to PAYLOAD_PAD_BLOCK)                 │
└──────────────────────────────────────────────────────────────┘
```

- `kind=1` (receipt): "Ich habe dir 50 USDC geschickt, sig=…" — append zu echter Tx
- `kind=2` (intent): "Lass uns 50 USDC tauschen" — Chat-Negotiation, niemand sendet noch
- `kind=3` (request): "Kannst du mir 50 USDC schicken?" — generates payment request link

## 7. Pylonyx Pre-Send Risk Check

### 7.1 API-Contract

```http
POST https://argos.dc-infosec.de/api/risk
Content-Type: application/json
X-Argos-Client-Version: 1.0.0
X-Argos-Tier: free | vip

{
  "checks": [
    { "type": "token", "mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" },
    { "type": "wallet", "address": "9WzD...Alice" }
  ]
}
```

Response:
```json
{
  "results": [
    {
      "type": "token",
      "mint": "EPjFWdd5...",
      "score": 5,               // 0=clean ... 100=avoid
      "warnings": [],
      "metadata": { "name": "USDC", "trusted": true }
    },
    {
      "type": "wallet",
      "address": "9WzD...",
      "score": 12,
      "warnings": ["wallet_age_days: 7"],
      "metadata": {}
    }
  ],
  "tier_used": "free",
  "rate_limit_remaining": 89
}
```

### 7.2 Rate-Limiting

- Free: 100 checks/Tag/Device — basic Score (no pre-launch data)
- Pylonyx-VIP (79€/Monat): 10.000 checks/Tag, pre-launch tokens, smart-money-flow
- Argos-App nutzt VIP-Tier wenn User Pylonyx-Abo hat (Auth via OAuth2)

### 7.3 Client-Side-Rendering

```dart
// mobile/lib/widgets/pylonyx_risk_card.dart
class PylonyxRiskCard extends StatelessWidget {
  final List<RiskResult> results;

  @override
  Widget build(BuildContext ctx) {
    final maxScore = results.map((r) => r.score).reduce(max);
    final color = maxScore > 50 ? kMagenta : maxScore > 20 ? kAmber : kGreen;
    final icon = maxScore > 50 ? Icons.warning : Icons.shield;
    // Render: Score-Badge + Warnings-List + "Trotzdem senden"-Button bei high-risk
  }
}
```

Wichtig: Das Pylonyx-Backend gibt nur Daten zurück — die UI-Logik (Schwellwerte, Farben, Disclaimer) ist Argos-Client-side. Pylonyx bleibt "Information", App rendert visuell.

## 8. Treasury / Fee-Flow

```
User klickt Swap
        ↓
Jupiter v6 Quote API
  └── platformFeeBps: 50    (0,5%)
  └── feeAccount: <Treasury-USDC-ATA>
        ↓
User signiert Tx
        ↓
Solana RPC: send_transaction
        ↓
On-chain:
  Source-Token-Account → Jupiter-Router
    ├── 99,5%  → Output-Mint → User-Receiver-ATA
    └── 0,5%   → Treasury-ATA (Argos-UG-Wallet)
        ↓
Buchhaltung (cron daily):
  - Read Treasury-Token-Balances via Helius
  - Konvertiere zu EUR via CoinGecko historical price
  - Schreibe in argos-bookkeeping DB (Pylonyx-VPS, Postgres)
  - Monthly CSV → Steuerberater
```

**Treasury-Wallet-Schema:**
- 1× **Cold Treasury** (Ledger Hardware, niemals online) → empfängt Fees direkt
- 1× **Hot Treasury** (operational, max 100 SOL Wert) → für gelegentliche Konvertierung in EUR via OTC

## 9. Migration-Phasen

| Phase | Wochen | Inhalt | Risk |
|---|---|---|---|
| **0: Naming + Legal** | 1 | DPMA-Check, Domain, GbR-/UG-Vorbereitung | Markenkonflikt → Plan-B-Name |
| **1: Spec-Lock + Rebrand-PR** | 1 | docs/spec/, App-Icon, Branding-Strings, README | breite Code-Touches |
| **2: Wallet MVP** | 2 | Generate, send/receive SOL+USDC, encrypted persist, restore | Schlüssel-Sicherheit |
| **3: Pylonyx-API** | 1 | /api/risk endpoint + score-engine wrap | Performance |
| **4: Swap** | 2 | Jupiter integration, platform fee, ExactOut quote | edge-cases bei tiefer Liquidität |
| **5: Auto-Swap-on-Send** | 1 | atomic versioned-tx builder | Tx-Größen-Limit Solana 1232 B |
| **6: UG-Switch** | 0,5 | Treasury-Adresse rotation, App-Update push | breaking change für laufende Tx-receipts |
| **7: Public Launch** | 1 | Play Store + App Store Submission, Privacy-Policy, AGB | Apple Review Round-Trip |

**Total: 10 Wochen** auf MVP-Public-Launch von heute aus.

## 10. App-Store-Compliance-Risiken

### Apple (App Store Review)

| Guideline | Risiko | Mitigation |
|---|---|---|
| 3.1.5(b) Crypto in-app | **mid** | Wallet ist non-custodial, **kein** in-App-Token-Verkauf. Fee ist on-chain. ✓ |
| 5.2.1 Trademark | **low** | Argos-Marke gecheckt vor Submission |
| 4.0 General | **low** | Keine deceptive UI, klare Disclosures |
| Export Compliance (ECCN 5D002) | **mid** | Encryption Compliance Form ausfüllen, "Mass Market"-Ausnahme greift |

### Google Play

| Policy | Risiko | Mitigation |
|---|---|---|
| Financial Services | **low** | Non-custodial wallet sind explizit erlaubt |
| Crypto-Trading-App-Section | **mid** | Geo-Targeting für DE/EU nötig, USA komplex |
| Personal Loans | **n/a** | Argos macht keine Kredite |

### EU / MiCA

| Service | CASP-Pflicht? | Begründung |
|---|---|---|
| Non-custodial Wallet UI | **Nein** | Recital 22 — Software-Bereitstellung, kein "Verwahren" |
| Jupiter-Swap-UI mit Referral | **Nein** | Argos ist nur User-Interface, Jupiter (Singapur) ist der Aggregator |
| Pylonyx-Information | **Nein** | Reine Marktdaten, keine Empfehlung |
| Custodial Treasury für Fees | **Custody der Argos-UG selbst**, nicht User-Custody — keine CASP |

## 11. Offene Architektur-Fragen

- **Multi-Chain**: Lightning (BTC) als Phase 8? Base/Ethereum als Phase 9?
- **Mnemonic-Backup-UX**: Schriftliche 24-Wort-Phrase ist zu komplex für Mainstream. Encrypted Cloud-Backup mit User-Passphrase optional anbieten?
- **Multi-Device**: Wallet auf mehreren Geräten synchronisieren = entweder shared secret (kompliziert) oder gleiches mnemonic (einfacher, aber Sicherheits-Tradeoff)
- **Spam-Token-Filter**: Jeder Solana-Wallet empfängt 100+ Dust-Airdrops/Monat. UI-Filter standardmäßig auf Pylonyx-Top-1000-Whitelist
- **NFTs**: Phase 10 oder gar nicht? NFTs sind reines UI, kein neuer Code-Path

## Anhang A: Tech-Stack-Decisions

- **Solana SDK**: `solana-sdk 2.x` + `solana-client` (Rust). Mobile via flutter_rust_bridge.
- **Mnemonic**: `bip39` crate (12/24-word, English wordlist).
- **Hashing**: `argon2` (OWASP 2023 params), `chacha20poly1305`.
- **Jupiter**: REST-API direkt (kein offizielles Rust-SDK, aber stable JSON-API).
- **RPC-Endpoint**: Erst Helius free-tier (10 M req/Monat), bei Skalierung Triton oder eigener Solana-Validator.
- **Pylonyx-Backend-Erweiterung**: Bestehender Next.js-Stack auf VPS, neuer Endpoint `/api/argos/risk` + Internal-Auth zwischen App und Pylonyx-Score-Engine.
