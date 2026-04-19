# Changelog

All notable changes to PhantomChat are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [2.2.0] — 2026-04-20 — Stufe A: daily-driver

### Added — Real message pipeline

- `core/src/address.rs` — `PhantomAddress` helper (`view_pub + spend_pub`,
  parse/format `phantom:view:spend` wire form).
- `core/src/session.rs` — `SessionStore` combining envelope + scanner +
  ratchet into one `send(address, plaintext) → Envelope` /
  `receive(envelope, view, spend) → Option<Vec<u8>>` pair. Persists to
  JSON so conversations survive CLI restarts.
- `cli`: new `phantom selftest` subcommand exercises a full A↔B exchange
  (including post-rotation traffic) in one process, no relay required.

### Changed — Double Ratchet actually wired up

- `core/src/ratchet.rs` fully rewritten for the Signal-style symmetric
  bootstrap:
  - `initialize_as_sender(initial_shared, recipient_spend_pub)` — picks
    a fresh ratchet secret, seeds root + send chains from
    `ratchet_secret × spend_pub`.
  - `initialize_as_receiver(initial_shared, own_spend_secret,
    peer_ratchet_pub)` — mirrors the sender's DH commutatively, then
    immediately initialises the outbound send chain so the first reply
    can be encrypted.
  - Per-message `encrypt` / `decrypt`, DH-ratchet rotation on incoming
    new peer-ratchet publics.
  - `Serialize` + `Deserialize` + `restore_secret()` so the full state
    round-trips through SessionStore's JSON persistence without losing
    the live DH secret (the 32-byte scalar is persisted alongside but
    never leaks through `Debug`).
- `core/src/api.rs` Flutter bridge:
  - Dead: the AES-GCM-with-phantom_id-as-key demo code.
  - Live: `load_local_identity(view_hex, spend_hex)`,
    `send_secure_message(recipient, _phantom_id, plaintext)` routed
    through SessionStore + network `PublishRaw`,
    `scan_incoming_envelope(wire_bytes) → Option<plaintext>` consumed
    by the listener loop.
- `cli/src/main.rs` — `send` and `listen` now run through
  `SessionStore::send` / `::receive` with `<keyfile>.sessions.json`
  persistence per identity.
- `mobile/lib/services/crypto_service.dart` — annotated `@Deprecated`,
  new code must use the Rust FFI path (`lib/src/rust/api.dart`).

### Tests

Added `core/tests/ratchet_tests.rs` (5) and `core/tests/session_tests.rs`
(5): first-message roundtrip, multi-message chains, bidirectional
exchange with rotation, serde roundtrip mid-conversation, tampered
ciphertext failure, address wire roundtrip, foreign-identity rejection,
and on-disk persistence across process restarts. Together with the
earlier suites: **42 / 42 tests green** under
`cargo test --no-default-features`.

### Verified on VPS

`phantom selftest` on Hostinger Ubuntu — 6 / 6 messages round-tripped
through the full envelope + ratchet stack, including the DH-ratchet
rotation triggered by the first B→A reply.

---

## [2.1.0] — 2026-04-19

### Fixed — Cryptographic correctness

- **Envelope ↔ scanner stealth-tag model unified.** The previous
  implementation derived the tag from `ECDH(eph, spend_pub)` on the sender
  but from `ECDH(view_secret, epk)` on the receiver, using different HKDF
  info strings and different HMAC inputs (16-byte `msg_id` vs 8-byte `ts`).
  No envelope could ever round-trip end-to-end. `Envelope::new` now takes
  **both** `recipient_view_pub` and `recipient_spend_pub`:
  - `view_shared` → `HKDF(info = "PhantomChat-v1-ViewTag")` → HMAC over `epk` → stealth tag
  - `spend_shared` → `HKDF(info = "PhantomChat-v1-Envelope")` → XChaCha20 key
  - Scanner derives the same `tag_key` from `view_secret × epk` and
    constant-time-compares, then `Envelope::open` re-derives the encryption
    key from `spend_shared`. This matches the Monero stealth-address model
    the README advertises.
- **`keys.rs`** — `ViewKey` / `SpendKey` no longer derive `Debug` (prevents
  accidental secret-scalar leakage into logs); replaced deprecated
  `StaticSecret::new(&mut OsRng)` with `::random_from_rng`.
- **`x25519-dalek` features** — added the missing `static_secrets` + `serde`
  features so the crate actually builds.

### Added — Test coverage

Thirty-two integration tests in `core/tests/` — the crate previously had
exactly one `cfg(test)` unit test.

- `envelope_tests.rs` (10) — round-trip correctness, foreign-ViewKey
  rejection, two-key-split validation (wrong ViewKey ⇒ NotMine even with
  correct SpendKey), mismatched-SpendKey ⇒ Corrupted, wire serialisation
  round-trip, truncated-bytes graceful failure, tag/ciphertext tampering
  breaks decryption, dummy-envelope wire validity vs scanner rejection,
  per-dummy entropy check.
- `scanner_tests.rs` (3) — batch scanning returns only matching payloads,
  PoW verifier accepts at-or-below difficulty and rejects dummies.
- `pow_tests.rs` (5) — compute/verify symmetry, wrong-nonce rejection,
  difficulty-zero shortcut, difficulty-ladder behaviour, input-dependent
  nonce uniqueness.
- `keys_tests.rs` (7) — PQXDH round-trip (sender and receiver derive
  identical 32-byte session key), two independent encapsulations differ,
  `HybridPublicKey` 1600-byte wire round-trip, short-input rejection,
  View/Spend independence, `IdentityKey` size + uniqueness, X25519 ECDH
  commutativity.
- `dandelion_tests.rs` (6) — empty-router falls back to Fluff, peer-update
  selects a stem, stem-removal triggers rotation, `force_rotate` on empty
  router is safe, first-peer-add initialises stem, statistical stem/fluff
  distribution (FLUFF_PROB = 0.1, tolerance 5–20 %).

All green: `cargo test --no-default-features` → **33 passed, 0 failed**.

### Added — Flutter app-lock

- `services/app_lock_service.dart` — PBKDF2-HMAC-SHA256 (100 000 iterations,
  16-byte CSPRNG salt) PIN derivation backed by `FlutterSecureStorage`;
  biometric quick-unlock via `local_auth`; configurable auto-lock timeout
  (default 60 s inactivity); **panic-wipe after 10 consecutive wrong PINs**
  that erases identity, contacts, messages, preferences, and the SQLCipher
  DB password.
- `screens/lock_screen.dart` — cyberpunk PIN-Pad UI, unlock + setup-mode,
  biometric button, attempts-remaining warning.
- `widgets/app_lock_gate.dart` — `WidgetsBindingObserver` gate that
  re-checks the lock state on lifecycle resume and forces setup for any
  existing identity that has no PIN configured yet (migration path for
  pre-2.1 installs).
- `services/storage_service.dart` — `StorageService.wipe()` added, used by
  the panic-wipe pipeline.
- `screens/onboarding.dart` — identity-creation flow now hands off to a
  mandatory PIN setup before the home screen becomes reachable.
- `main.dart` — wraps the app in `AppLockGate`.

### Fixed — Build / workspace plumbing

- `core/Cargo.toml` — new `ffi` feature (default on) gates
  `flutter_rust_bridge` + `rusqlite` (SQLCipher) so pure-crypto tests run
  with `cargo test --no-default-features` on hosts without OpenSSL dev
  headers.
- `core/src/lib.rs` — `api`, `storage`, `network`, and `frb_generated`
  modules moved behind `#[cfg(feature = "ffi")]`.
- `cli/Cargo.toml`, `relays/Cargo.toml` — depend on core with
  `default-features = false`; relays gains its own `ffi` feature that
  reactivates `start_stealth_cover_consumer`.
- `relays/src/lib.rs` / `nostr.rs` — API upgrades for newer crate
  versions: `Keypair` → `KeyPair`, `Message::from_digest` →
  `Message::from_slice`, added `use futures::SinkExt`, `BridgeProvider`
  made dyn-compatible by replacing generic `subscribe<F>` with
  `subscribe(Box<dyn Fn(Envelope) + Send + Sync + 'static>)`, JSON macro
  `[] as Vec<Vec<String>>` rewritten with a typed binding.
- `cli/src/main.rs` — recipient address now parsed as
  `view_pub_hex:spend_pub_hex` (matches the `phantom pair` QR payload);
  `listen` re-wired onto `scan_envelope`/`ScanResult` instead of brute-
  forcing every envelope with the SpendKey; borrow-checker temporaries
  lifted into `let` bindings; format-string arity corrected.

### Changed

- `Envelope::new` signature — now `(view_pub, spend_pub, msg_id, …)`
  instead of `(spend_pub, msg_id, …)`. All callers updated.
- Scanner HKDF info label: `"PhantomChat-v1-Tag"` → `"PhantomChat-v1-ViewTag"`
  (matches `envelope.rs`).

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
