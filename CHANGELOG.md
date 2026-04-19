# Changelog

All notable changes to PhantomChat are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [2.4.0] — 2026-04-20 — Tier 1 + Tier 2

Top-tier privacy features — everything we previously marked "future work"
on the README roadmap is now real code, on-VPS verified.

### Added — Tier 1

**Sealed Sender (Ed25519 authentication)**

- `keys.rs` — new `PhantomSigningKey` + `verify_ed25519` helper. Ed25519
  identity key separate from the X25519 Envelope crypto.
- `envelope.rs` — `SealedSender { sender_pub, signature }` carried
  *inside* the AEAD-encrypted [`Payload`]. Signs `ratchet_header ||
  encrypted_body`. New `Envelope::new_sealed` /
  `Envelope::new_hybrid_sealed` constructors, and low-level
  `Envelope::seal_classic` / `::seal_hybrid` that take a pre-assembled
  `Payload` for exotic callers.
- `session.rs` — `SessionStore::send_sealed` pairs the plaintext with a
  signature chain; `SessionStore::receive_full` returns a new
  `ReceivedMessage { plaintext, sender: Option<(SealedSender, ok)> }`.
- Relay + man-in-the-middle never learn the sender; only the recipient
  does, and the signature can be cryptographically verified against a
  known identity list.

**Payload padding**

- `Payload::to_bytes` now rounds the serialised length up to the next
  multiple of `PAYLOAD_PAD_BLOCK = 1024` with CSPRNG-filled padding.
  Different-length plaintexts land in the same wire bucket, breaking
  length-correlation attacks.

**Safety Numbers (Signal-style MITM detection)**

- `fingerprint.rs` — `safety_number(addr_a, addr_b)` computes a
  symmetric 60-digit decimal number from two PhantomAddresses using
  5 200 rounds of SHA-512 (the Signal
  `NumericFingerprintGenerator` arithmetic). Twelve 5-digit groups,
  spoken-aloud friendly. Alice and Bob compare it out-of-band — a
  mismatch flags an active MITM.

**X3DH Prekey Bundle**

- `prekey.rs` — `SignedPrekey` (Ed25519-signed rotating X25519 key),
  `OneTimePrekey`, `PrekeyBundle { identity_pub, signed_prekey,
  one_time_prekey }` with wire-level signature-chain verification.
  `PrekeyMaterial::fresh(&identity)` generates a publish-ready bundle
  and keeps the matching secrets on the owner side.
- Ready to be dropped into any transport (Nostr event, NIP-05 HTTP
  endpoint, QR code) for genuine out-of-band handshake.

### Added — Tier 2

**Sender-Keys group chat (pre-MLS)**

- `group.rs` — `PhantomGroup` with Signal's Sender-Keys primitive:
  each member holds a symmetric ratchet (`SenderKeyState`) they
  distribute once per group via the pairwise 1-to-1 channel; subsequent
  sends are O(1) AEAD + O(1) Ed25519 signature. Member removal rotates
  our own chain so post-removal messages stay inaccessible.
- Wire format versioned (`GROUP_VERSION_SENDER_KEYS = 1`) so a future
  MLS (RFC 9420) migration via `openmls` can coexist without a
  flag-day break.

**WASM feature gate (crypto-only core for browser builds)**

- `core/Cargo.toml` — new `net` feature gates libp2p + tokio +
  dandelion + cover_traffic; `ffi` now depends on `net`; a bare
  `cargo check --target wasm32-unknown-unknown --no-default-features
  --features wasm` compiles the crypto core with zero native-runtime
  deps.
- `cfg(target_arch = "wasm32")` pins `getrandom v0.2`'s `js` feature so
  the browser's `crypto.getRandomValues()` backs all RNG.
- Note: `getrandom v0.3` transitives (e.g. through some newer crates)
  currently also need `RUSTFLAGS='--cfg getrandom_backend="wasm_js"'`.
  Documented in README; not a blocker for the feature-gate itself.

### Selftest Phase 3–6

`phantom selftest` grew from 10 messages to **20 checks across 6
phases**: classic envelope, PQXDH, sealed-sender round-trip, safety
number symmetry + format, prekey-bundle signature chain + forgery
rejection, and a 3-member × 2-message group chat. Live on the Hostinger
VPS: **20/20 passed**.

### Tests

`core/tests/sealed_sender_tests.rs` (5): sealed-sender round-trip,
impersonation detection, padding block-alignment, padded-payload
from_bytes round-trip, sealed + hybrid combination. `group.rs` inline
tests (3), `fingerprint.rs` inline tests (3), `prekey.rs` inline tests
(4). Full suite: **64 tests** under
`cargo test --no-default-features --features net`.

---

## [2.3.0] — 2026-04-20 — PQXDH live + Tor live

### Added — Post-Quantum in the message flow

PQXDH (ML-KEM-1024 + X25519) is no longer dormant code — it drives the
envelope encryption key whenever the recipient address carries a PQ
public key.

- `envelope.rs` — new `Envelope::new_hybrid` /
  `Envelope::open_hybrid`. Wire format bumps to version byte `2`; the
  1568-byte ML-KEM ciphertext is appended after the classic payload so
  v1 parsers still decode the common prefix. Hybrid key derivation:
  `HKDF(spend_shared || mlkem_shared, "PhantomChat-v2-HybridEnvelope")`.
- `address.rs` — `PhantomAddress` gains an optional `mlkem_pub` field.
  New `phantomx:` wire prefix with the ML-KEM half base64-encoded:
  `phantomx:<view_hex>:<spend_hex>:<mlkem_b64>`. Classic `phantom:`
  addresses still round-trip untouched.
- `session.rs` — `SessionStore::send` auto-routes to the hybrid path
  when the recipient is hybrid. `receive_hybrid()` variant takes the
  caller's `HybridSecretKey`. Classic `receive()` silently ignores v2
  envelopes so mixed identities can coexist on one node.
- `scanner.rs` — new `scan_envelope_tag_ok()` exposes just the
  view-key phase so `SessionStore` can pick classic-vs-hybrid open
  itself. The existing `scan_envelope()` wrapper remains for v1-only
  callers.
- `cli selftest` — now runs **two** phases: 6 classic messages + 4
  hybrid messages. Live on the Hostinger VPS: 10/10 round-trip.

### Added — Tor runtime

- Tor daemon installed + enabled on the VPS. SOCKS5 listener at
  `127.0.0.1:9050` verified against
  `https://check.torproject.org/api/ip` →
  `{"IsTor":true,"IP":"185.220.101.43"}`.
- `phantom mode stealth` live-verified — switches to MaximumStealth,
  flips CoverTraffic to Aggressive, routes Nostr through SOCKS5.

### Added — Systemd background listener

- `/etc/systemd/system/phantom-listener.service` — runs
  `phantom listen` against `wss://relay.damus.io` on the VPS, restarts
  on failure, appends to `/var/log/phantom-listener.log`. Started after
  `tor.service` so stealth mode has a SOCKS5 listener waiting.

### Tests

`core/tests/hybrid_tests.rs` (7): address wire round-trip, classic vs
hybrid sniff, self-send through PQXDH envelope, classic receive silently
drops v2, foreign hybrid identity rejected, on-wire → parse →
open_hybrid → plaintext intact, classic flow untouched by the extension.

Full suite: **49 / 49 tests passing** under
`cargo test --no-default-features`.

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
