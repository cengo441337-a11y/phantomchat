# Changelog

All notable changes to PhantomChat are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [2.0.0] — 2026-04-04

### Added

**Privacy System v2**
- `core/src/privacy.rs` — `PrivacyMode` enum (DailyUse / MaximumStealth), `ProxyConfig` (Tor/Nym), `PrivacyConfig` with `p2p_enabled()` and `proxy_addr()`
- `core/src/dandelion.rs` — Dandelion++ router: Stem phase (p=0.1 transition per hop), Fluff phase (GossipSub broadcast), epoch-based peer rotation every 10 minutes
- `core/src/cover_traffic.rs` — `CoverTrafficGenerator` with Light (30–180 s) and Aggressive (5–15 s) modes; dummy envelopes are CSPRNG-filled and wire-indistinguishable from real traffic
- `core/src/api.rs` — `PRIVACY_CONFIG`, `STEALTH_COVER_TX/RX` static channels; `set_privacy_mode()` / `get_privacy_mode()` with `#[frb(sync)]` annotations; dual bridge tasks for Daily vs Stealth routing

**Post-Quantum Cryptography (PQXDH)**
- `core/src/keys.rs` — `HybridKeyPair` combining ML-KEM-1024 + X25519; `session_secret = SHA256(x25519_shared || mlkem_shared)`
- Dependency: `pqcrypto-mlkem` for ML-KEM-1024 operations

**ViewKey Envelope Scanner**
- `core/src/scanner.rs` — `scan_envelope()`, `scan_batch()`, `ScanResult` enum (Mine / NotMine / Corrupted)
- Uses Monero stealth address model: `ECDH(view_secret, epk)` → HKDF → tag_key → HMAC verify

**Nostr Transport Layer**
- `relays/src/lib.rs` — Full rewrite: `NostrEvent` (NIP-01, Kind 1059 Gift Wrap, Schnorr signature via secp256k1, ephemeral keypair per session), `NostrRelay` (tokio-tungstenite WebSocket), `StealthNostrRelay` (SOCKS5 → TLS → WebSocket), `make_relay()` factory
- `relays/src/nostr.rs` — `PHANTOM_KIND=1984`, `NostrEvent::new_phantom()`, NIP-01 signing
- Maximum Stealth: all Nostr WebSocket connections tunnel through SOCKS5 (Tor `127.0.0.1:9050` or Nym `127.0.0.1:1080`)

**Cyberpunk CLI**
- `cli/src/main.rs` — Full rewrite with neon green / neon magenta ANSI palette matching Flutter theme
- Commands: `keygen`, `pair` (ASCII QR code), `send` (Dandelion++ phase display), `listen` (scan loop), `mode` (Daily/Stealth + proxy config), `relay` (health check), `status`
- `indicatif` spinners, `~/.phantom_config.json` persistence
- Dependencies added: `colored`, `indicatif`, `qrcodegen`, `dirs`, `x25519-dalek`

**Flutter Privacy UI**
- `mobile/lib/src/ui/privacy_settings_view.dart` — Animated mode cards, Tor/Nym chip toggle, SOCKS5 address input, stealth warning box
- `mobile/lib/services/privacy_service.dart` — SharedPreferences persistence, calls FRB-generated `rust.setPrivacyMode()` / `rust.getPrivacyMode()`
- `mobile/lib/src/ui/profile_view.dart` — Privacy tile with live mode indicator, navigation to `PrivacySettingsView`

**Documentation**
- `docs/PRIVACY.md` — Privacy modes architecture, Dandelion++ flow diagram, cover traffic design, StealthNostrRelay connection chain
- `docs/SECURITY.md` — Full threat model table, crypto stack (XChaCha20-Poly1305, HKDF-SHA256, X25519, HMAC-SHA256), feature status matrix
- `spec/SPEC.md` — Sections 7–10: implementation status, Privacy System, Nostr Transport, ViewKey Scanner
- `README.md` — Feature matrix, architecture ASCII diagram, Privacy Modes section, updated CLI commands, workspace structure

### Fixed

- `core/src/envelope.rs` — Struct body corruption (stray `use` statements inside struct from bad merge); full rewrite restoring all 8 fields (`ver`, `ts`, `ttl`, `epk`, `tag`, `pow_nonce`, `nonce`, `ciphertext`) and completing `Envelope::new()` with `Payload` construction before encryption
- `core/src/api.rs` — Cover traffic bridge was unreachable in MaximumStealth (placed after early return); restructured to route cover traffic correctly in both modes
- `relays/src/lib.rs` — `StealthNostrRelay` wrong return type (`tokio_tungstenite::stream::Stream<...>` does not exist); corrected to `WebSocketStream<TlsStream<Socks5Stream<TcpStream>>>`
- `core/src/api.rs` — Missing `#[frb(sync)]` annotations on `set_privacy_mode()` / `get_privacy_mode()` preventing Flutter codegen

### Changed

- `core/src/lib.rs` — Added `pub mod privacy`, `dandelion`, `cover_traffic`, `scanner`, `util`; combined re-exports from all merged branches
- `core/src/network.rs` — Integrated `DandelionRouter`; `ConnectionEstablished/Closed` events update router; `publish_with_phase()` function; `PublishRaw` command handler; `STEM_TOPIC_PREFIX` constant
- `core/src/p2p.rs` — Marked DEPRECATED (not compiled, not in lib.rs)
- `relays/Cargo.toml` — Added `tokio-tungstenite 0.21` (native-tls feature), `tokio-native-tls 0.3`, `native-tls 0.2`, `tokio-socks 0.5`, `secp256k1 0.27`, `sha2`, `hex`, `base64`, `rand`, `tracing`
- `core/Cargo.toml` — Added `tracing = "0.1"`

---

## [1.1.0] — 2026-04-04

### Added

- Flutter app cyberpunk UI overhaul (neon green / magenta palette, Courier monospace, ANSI-style overlays)
- libp2p GossipSub fully decentralized P2P envelope distribution (`feature/libp2p-gossip`)

---

## [1.0.1] — 2026-04-04

### Added

- Flutter app v1.0 — encrypted messenger with initial cyberpunk UI, message list, send flow

### Fixed

- Dependency audit: resolved critical vulnerabilities and build errors
- Android manifest syntax errors; disabled Impeller to fix GPU driver hang on Android 16
- Core bootstrapper: two-stage async startup to avoid blocking main thread

---

## [1.0.0] — 2026-04-02

### Added

- PhantomChat Phase 5 — initial audit baseline
- Double Ratchet crypto (envelope layer), XChaCha20-Poly1305 payload encryption
- Hashcash Proof-of-Work on every envelope (anti-spam / anti-Sybil)
- Stealth tags via HMAC-SHA256 (receiver anonymity from relays)
- SQLCipher local storage (AES-256-CBC, no plaintext key material)
- DC INFOSEC branding and portfolio structure

---

## [0.1.0] — 2026-03-28

### Added

- Initial repository setup
- Core workspace scaffolding (core, relays, cli, mobile)
- Basic key generation and envelope serialization
