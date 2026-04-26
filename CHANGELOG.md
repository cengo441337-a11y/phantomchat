# Changelog

All notable changes to PhantomChat are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [3.0.2] — 2026-04-26 — Security audit fixes

### Critical
- AI bridge: `claude_cli_skip_permissions` default flipped from true to false
- AI bridge: Claude CLI history now uses fence-token-delimited turns instead of
  flat-concat, closing a prompt-injection class
- Mobile: silent legacy-crypto-fallback in `chat.dart` replaced with visible error
- Desktop: `save_history` failures now surface as a persistent banner
- Build: GitHub PAT removed from git remote URL

### High
- 9 async hazards eliminated: `SessionStore` save off the reactor, voice-bytes-scan
  in `spawn_blocking`, whisper download via `tokio::fs`, `mutate_message_state`
  via `AsyncMutex`, whisper context cached in `OnceLock`, `search_messages`
  off-thread
- Mobile↔Desktop wire compat: TYPN-1 schema unified, REPL-1/RACT-1/DISA-1 swallow
  handlers added on mobile (no more raw-text rendering)
- `phantomx` mlkem persisted in mobile contacts (no more silent X25519 downgrade)
- `rustls-webpki` bumped to ≥0.103.13 (3 advisories)
- `BindContactModal` silent-failure pattern fixed (mirrors `AddContactModal`)
- `InputBar` restores user text on failed send
- Watchers per-watcher concurrency lock (multi-click no longer fans out)
- Relays save now `restart_listener` so new set takes effect

### Medium
- `MessageStream` virtualization (react-virtuoso) — 1000+ row scrolling smooth
- PBKDF2 600k iters + `compute()` isolate (Mobile PIN-confirm 5–15 s freeze killed)
- APK update: origin-pin + `version_code` check (downgrade-attack closed)
- Mobile crash-report opt-in surface
- Watcher run-now busy gate
- `ConversationHeader` TTL reverts on error
- Linux `install_update` success message clarified

### Cleanup
- Removed orphan `mobile/lib/screens/settings_background.dart`
- Removed dead pub fns (`file_transfer pack/unpack_stream`,
  `storage save_group_message`, `util to_hex`, `relays start_stealth_cover_consumer`)
- Removed empty `pqc = []` core feature
- Dropped 27 unused i18n keys
- `brain.md` + `playbook.md` archived to `docs/archive/`

---

## [3.0.1] — 2026-04-26 — Add-contact mobile↔desktop format compat

### Critical
- Mobile↔Desktop address-format incompatibility fixed — mobile now emits and
  parses the canonical `phantom:<view_hex>:<spend_hex>` form (was emitting
  `phantom:base64-JSON`)
- Both `AddContactModal` silent-failure UIs now surface inline errors

### Build
- Wave 11D STT enabled in MSI (`cmake` + LLVM/libclang on Nexus)
- Mobile build pipeline unstuck: vendored `record_linux` stub, Jetifier on,
  `desugar_jdk_libs` on

---

## [Wave 8 / 9 / 10 / 11 — 2026-04-26 mega-block]

This block summarises the wave-stream that landed on 2026-04-26 between v3.0.0
and v3.0.2. Individual semver entries above pick out the user-visible
release-points; the per-wave breakdown below is the engineering history.

### Wave 7 series — mobile catch-up + desktop UX bundle
- **7A** (`304e628`) mDNS LAN auto-discovery + Join-LAN-Org wizard step
- **7B** (`dbb8d4e`) Flutter app catch-up to v3.0.0 wire protocols
- **7B2** (`d648e46`) Mobile→Desktop send path via pure-Dart Nostr relay client
- **7B3** (`608a5d3`) Android Production-Keystore + Release-Signing pipeline
- **7C** (`858f1db`) pre-seeded MSI templater for org bulk-deploy
- **7D** (`0b72a79`) reply/quote + reactions + disappearing messages

### Wave 8 series — desktop polish + mobile hardening + infra
- **8A** (`00acb99`) Mobile APK polish + Android security hardening
- **8B** (`db9a38a`) Android Foreground Service for persistent relay listening
- **8C** (`1d7feaf`) encrypted backup/restore (Argon2id + XChaCha20-Poly1305)
- **8D** (`1cdcb88`) theme system — Cyberpunk Dark + Soft Light + Corporate
- **8E** (`398b6f2`) window-state persistence with multi-monitor awareness
- **8F** (`8aa4670`) markdown + link auto-detect + @-mentions in MLS groups
- **8G** (`56dd679`) image-inline-preview + Pin/Star/Archive
- **8H** (`6421c48`) OS-keystore-backed key storage + memory-zeroing +
  anti-forensic shred
- **8I** (`82fed11`) CI/CD GitHub Actions + Reproducible-Builds + Fuzz harnesses
- **8J** (`873c13d`) self-hosted-relay docs + opt-in crash-reporting

### Wave 9 — transparency bundle (`2d95cf2`)
- Disclosure policy + PGP key (`keys/security.asc`)
- `docs/HALL-OF-FAME.md` template
- `.well-known/security.txt` (RFC 9116, PGP-signed)

### Wave 10 — signed Windows build pipeline
- **10** (`8918ea5`) Wave 10 base — MSI + NSIS signing
- (`bfe29b2`, `3949e35`, `b399e19`) `signCommand` wrapper iteration:
  bare `signtool` + PATH prepend → `.cmd` wrapper → `cmd /C` invocation +
  correct relative path
- (`86a07b8`) Pilot self-signed cert shipped as `keys/phantomchat-pilot-cert.cer`
- Wrapper script: `scripts/sign-windows.cmd`
  reads `PHANTOMCHAT_PFX_PATH` + `PHANTOMCHAT_PFX_PASSWORD` env vars and signs
  via `signtool` with SHA-256 + RFC 3161 timestamp

### Wave 11 — AI Bridge series (`docs/AI-BRIDGE.md` is canonical)
- **11A** (`c502a11`) Home-LLM Bridge — AI as virtual PhantomChat contact
- **11B** (`43828d1`) voice messages (mobile record + desktop playback)
- **11C** (`10bf022`) tool-using AI bridge + `docs/AI-BRIDGE.md` published
- **11D** (`dac9deb`) voice → whisper.cpp STT → LLM (closes the voice-control loop)
- **11E** + **11G** (`80fa6fe`) proactive watchers (cron) + mobile in-app APK
  auto-update
- **11F** (`a7acf45`) per-contact routing in AI Bridge

### Post-wave-11 stabilisation (between 3.0.0 → 3.0.1 → 3.0.2)
- (`3246d1f`) watchers startup panic — use `tauri::async_runtime::spawn` (no
  tokio reactor in `setup()`)
- (`b9c1a00`) purge startup panic — same pattern
- (`5bda2b5`) Mobile PIN-Confirm silent hang — busy-state + `try/catch` + spinner
- (`8febc15` / `dfa0a7e`) v3.0.1 — add-contact mobile↔desktop format compat
- (`f49b9a7`) v3.0.2 build path — APK pipeline 4-fix bundle

---

## [3.0.0] — 2026-04-25 — Tauri Desktop + B2B-ready stack

Major surface expansion. PhantomChat is now a shippable B2B internal messenger,
not just a research crypto stack. Feature parity with mid-tier commercial messengers
(read receipts, typing indicators, search, audit, i18n, system tray, auto-updater)
plus the security primitives nobody else has (PQXDH + MLS + multi-relay + Tor mode +
sealed-sender attribution).

### Added — Tauri 2 Desktop App (`desktop/`)

New workspace member `desktop/src-tauri` (`phantomchat_desktop` crate) plus React +
Vite + TypeScript + Tailwind frontend. Uses `phantomchat_core` directly — no FFI.

- **Onboarding** — 5-step wizard (welcome → identity gen/restore → relays
  → share QR → done) with persistent marker; `is_onboarded` /
  `mark_onboarded` Tauri commands.
- **1:1 sealed-sender messaging** — full attribution UX:
  - `✓` sent / `✓✓` delivered / `✓✓` (cyber-cyan) read per outgoing row
  - IntersectionObserver auto-mark-read on viewport visibility (60% threshold)
  - bind-button workflow for unbound (`?<8hex>`) senders →
    `bind_last_unbound_sender(contact_label)` writes signing_pub onto contact
  - tampered (`sig_ok=false`) rows show red tint + ⚠ + glitch text effect
- **MLS RFC 9420 group chat** with **automatic relay transport** — no manual
  base64 paste:
  - new wire prefixes: `MLS-WLC2:` (welcome with embedded inviter directory
    bootstrap meta) + `MLS-APP1:` (app message) — wrapped inside sealed
    envelopes, fanned out to every known group member's PhantomAddress
  - directory persisted to `mls_directory.json`; rehydrates on app start
  - file-backed openmls storage (`mls_state.bin` + `mls_meta.json`):
    groups survive app restart
  - lifecycle commands: `mls_create_group`, `mls_add_member`, `mls_join_via_welcome`,
    `mls_encrypt`, `mls_decrypt`, `mls_status`, `mls_list_members`,
    `mls_leave_group`, `mls_remove_member`
  - auto-init on first incoming Welcome (no need for explicit `mls_init`)
- **Multi-Relay subscription** with reliability guarantees:
  - `MultiRelay` BridgeProvider wraps N underlying relays (default:
    Damus + nos.lol + snort.social)
  - SHA256-LRU dedupe (4096 envelopes, HashSet + VecDeque ring) so the
    handler fires exactly once per unique envelope across all relays
  - per-underlying-relay auto-reconnect inside `NostrRelay` with
    jitterized exponential backoff (`base = 2^min(attempt,6)` capped at
    60s, plus 0–5s jitter, attempt counter resets on successful connect)
  - new `ConnectionEvent` enum (`Connecting`/`Connected`/`Disconnected`/
    `Reconnecting`) emitted via aggregate state-channel up to the
    frontend StatusFooter pill
  - new `BridgeProvider::subscribe_with_state` trait method (default
    impl wraps existing `subscribe` for backwards compat)
- **Tor / Maximum Stealth privacy mode** toggle in Settings — persists
  to `privacy.json`, `restart_listener` Tauri command re-spawns subscriber
  with new mode without app restart, SOCKS5 proxy address configurable
- **File transfer 1:1** — paperclip button + drag-drop in InputBar; magic
  prefix `FILE1:01` + ULEB128(meta_len) + JSON manifest + raw bytes
  wrapped in sealed envelope; receiver sha256-verifies, basename-sanitizes
  (rejects `..`/`/`/`\`/null), auto-renames on collision, writes to
  `~/Downloads/PhantomChat/`, fires native notification + emits
  `file_received` event; 5 MiB cap per file (single-envelope MVP, chunking
  in 3.1)
- **Read Receipts + Typing Indicators** — new wire prefixes `RCPT-1:` and
  `TYPN-1:`, both wrapped in sealed envelopes (no metadata leaked to relay):
  - `mark_read(msg_id, contact_label)` Tauri command; receiver auto-emits
    a `delivered` receipt on every successful 1:1 decode
  - `typing_ping(contact_label)` Tauri command, leading-edge 1.5s throttled
  - `msg_id` = first 16 hex of `SHA256("v1|" || hex(plaintext))` —
    plaintext-only so sender + receiver compute byte-identical IDs
- **System Tray** (Tauri 2 `TrayIconBuilder`) — Show/Hide/Status/Quit menu,
  single-click toggles main window, close-button hides instead of exits
- **Native Notifications** (`tauri-plugin-notification`) — focus-aware
  (only fires when `is_focused() == false || is_visible() == false`),
  click-to-restore, separate titles for 1:1 / MLS / file events
- **Settings Panel** — Identity (with QR via `qrcodegen` SVG, copy address,
  display name), Privacy (Tor toggle + SOCKS5 config), Relays (editable URL
  list with per-row connection state), About (version + update check),
  Audit Log (filterable table + Export-Path button), Danger Zone
  (two-step DELETE confirm → `wipe_all_data` removes app_data_dir + exits)
- **Audit Log** — JSONL append-only at `audit.log`, ISO27001/ISMS-friendly:
  identity_created/restored, contact_added/bound, mls_created/added/left/
  removed, relay_changed, privacy_changed, data_wiped/onboarded —
  categorical metadata only (never key material, never message bodies)
- **i18n DE + EN** via `react-i18next` + `i18next-browser-languagedetector`,
  ~230 namespaced keys (`settings.identity.title` etc.), localStorage
  persistence, Auto/English/Deutsch toggle in Settings → Identity → Language;
  formal "Sie" throughout German strings
- **Auto-Updater** (`tauri-plugin-updater`) — Ed25519-signed releases,
  endpoint `https://updates.dc-infosec.de/phantomchat/{{target}}/{{current_version}}`,
  startup auto-check + manual "Check for updates" button + passive top-banner
  on available update
- **Message Search** (Ctrl+F / Cmd+F) — `search_messages(query, sender_filter, limit)`
  Tauri command scans messages.json, debounced 200ms, magenta substring
  highlights, sender-filter dropdown, click-result scrolls main MessageStream
  + `pc-search-pulse` 1.5s animation on the row
- **1:1 message persistence** — `messages.json` JSONL with file rows
  (`kind: "text" | "file"` + optional `file_meta`, `direction`, `sender_pub_hex`);
  debounced auto-save 500ms after every message; hydrated on mount
- **Connection-status pill** — live state from `connection` events
  (connected breathing pulse / disconnected red blink / connecting yellow pulse)
- **Cyberpunk visual polish** — CRT scanlines + grid background with 60s drift,
  pane-focus glow, glitch-text effect on tampered messages (every ~6s, 0.3s
  burst), slide-in animations on new messages, modal glass effect with 8px
  backdrop-blur, Orbitron display font for headers, blinking cursor in input
- **Graceful subscriber shutdown** — `tokio::oneshot` channel + `select!`,
  3s timeout fallback to `JoinHandle::abort`, explicit `drop(relay)` ensures
  clean WebSocket close before respawn

### Added — Cyberpunk TUI (`cli/src/tui.rs`, `phantom chat`)

- `ratatui` + `crossterm` three-pane chat (contacts left, message stream
  center, input bottom)
- Sealed-sender attribution + bind-keybinding (`b`)
- Auto-upgrade for legacy keyfiles (adds `signing_private` / `signing_public`)
- Same SessionStore + relay code path as headless `send` / `listen`
- Cyberpunk palette matching the Tauri Desktop and CLI banner

### Changed — Core (`core/src/mls.rs`)

- New public accessors on `PhantomMlsMember`: `provider()`, `signer()`,
  `credential_with_key()` — enable safe `MlsGroup::load(provider, &gid)`
  per-call pattern (replacing the prior `unsafe { mem::transmute }` workaround)
- New `PhantomMlsGroup::from_parts(member, group)` constructor
- New module-level `pub fn load_group(member, &group_id) -> Result<MlsGroup, MlsError>`
- New `pub fn group_id_bytes(group)` helper
- Re-exports `pub use openmls::group::{GroupId, MlsGroup}` so consumers
  don't need an openmls direct dep
- Custom file-backed `StorageProvider` wrapping the upstream
  `MemoryStorage` — `persist()` snapshots the entire HashMap atomically
  to `mls_state.bin` (bincode), `new_with_storage_dir` rehydrates on startup
- Two new tests: `file_backed_member_round_trips_storage_across_restarts`,
  `state_blob_roundtrips_arbitrary_pairs` — both green (6/6 MLS tests pass)

### Changed — Relays (`relays/src/lib.rs`)

- `MultiRelay` BridgeProvider — fan-out publish (succeed-if-any), dedupe-LRU
  subscribe, `id() == "multi:N"`
- `make_multi_relay(urls, stealth, proxy)` factory; single-URL passthrough
  optimization
- `NostrRelay::subscribe` rewritten to use new auto-reconnect loop with
  exponential backoff (StealthNostrRelay deliberately untouched per scope)
- New `ConnectionEvent` enum + `StateHandler` type alias + default-impl
  `subscribe_with_state` trait method on `BridgeProvider`

### Changed — CLI (`cli/src/main.rs`)

- New `phantom chat` subcommand opens TUI
- `cmd_keygen` now also generates + persists Ed25519 signing keypair
  (`signing_private` b64, `signing_public` hex) for sealed-sender attribution
- Cleaned 21 build warnings → 0 (deprecated `base64::encode` migrations,
  unused-import deletes, dead-code annotations)

### Documentation

- `desktop/README.md` (179 lines) — quickstart, build, OS-specific app-data
  paths, troubleshooting, ASCII architecture diagram
- This README updated with B2B sales positioning + new feature matrix rows

### Build / Distribution

- Tauri Windows build verified end-to-end on Win11 25H2:
  - MSVC + WiX MSI bundling (Visual Studio Build Tools 2022 on `D:\BuildTools`,
    Rust toolchain on `D:\rust`, repo on `D:\phantomchat`)
  - Ed25519 release signing via `minisign 0.12` on Hostinger VPS
  - Update server: nginx vhost on `updates.dc-infosec.de` serving Tauri
    updater protocol JSON manifests + signed `.msi` + `.minisig`
  - Companion CLI binary cross-built (`phantom.exe`, 7.11 MiB)

### Tests / Quality

- Selftest still 30/30 across 9 phases
- MLS unit tests: 6/6 pass (4 original + 2 new for file-backed storage)
- `phantomchat_cli` build: 0 warnings
- `phantomchat_relays` build: 0 warnings
- `phantomchat_desktop` cargo check: clean
- Frontend bundle: 303 KB JS / 27 KB CSS (gzip: 90 / 6 KB)

### Sales positioning (decided 2026-04-25)

PhantomChat now markets as **internal company messenger replacing email** for
German SMEs and law firms with hard confidentiality obligations
(`Anwaltsgeheimnis` § 203 StGB). Pricing model: bundled with DC INFOSEC
pentest engagements (cross-sell), self-hosted flat-license tier, and
optional managed hosting tier.

---

## [2.6.0] — 2026-04-20 — MLS (RFC 9420) live

### Added — Real MLS group messaging via openmls 0.8

Replaces the v2.4 roadmap stub with a working integration.

- `core/src/mls.rs` — `PhantomMlsMember` + `PhantomMlsGroup<'_>` wrapping
  `openmls::MlsGroup`. Pins ciphersuite
  `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519` so the MLS layer reuses
  the same X25519 + Ed25519 primitives the rest of PhantomChat already
  has. Uses `OpenMlsRustCrypto` as the persistent storage + crypto
  provider; the signing key is `openmls_basic_credential::SignatureKeyPair`.
- Public API:
  - `PhantomMlsMember::new(identity)` — bootstrap a local member.
  - `publish_key_package()` → serialised bytes another member invites us with.
  - `create_group()` → `PhantomMlsGroup` holding the fresh MlsGroup.
  - `PhantomMlsGroup::add_member(bytes)` → `(commit_bytes, welcome_bytes)`;
    automatically calls `merge_pending_commit` so our epoch view advances.
  - `join_via_welcome(welcome_bytes)` — joiner-side, uses
    `StagedWelcome::new_from_welcome(..., into_group(...))` as required
    by openmls 0.6+.
  - `encrypt(plaintext)` / `decrypt(wire)` — application messages.
    `decrypt` transparently merges staged commits from other members so
    the group stays in sync across epoch changes.
- Wire version byte `GROUP_VERSION_MLS = 2` reserved (Sender-Keys stays
  `1`) — receivers can dispatch by format.
- **4 tests** (`cargo test --features mls mls::`): two-member end-to-end
  flow with Welcome + application message, bidirectional messaging
  wellformedness, malformed-welcome rejection, byte-exact payload
  round-trip (including non-ASCII bytes).

### Selftest: 8 → 9 phases, 30 checks

Phase 9 drives the full MLS pipeline in one process: two members,
seven steps (two init, publish_key_package, create_group, add_member,
encrypt, join_via_welcome, decrypt + byte-compare).  Live on Hostinger
VPS: **30/30 passed.**

### Deps (`mls` feature only — zero impact on classic builds)

```toml
openmls                  = "0.8"   # 0.8.1 — the post-audit release
openmls_rust_crypto      = "0.5"   # crypto + storage provider
openmls_traits           = "0.5"
openmls_basic_credential = "0.5"   # SignatureKeyPair lives here in 0.5+
tls_codec                = "0.4"   # features = ["derive", "serde", "mls"]
```

The `mls` feature is gated entirely behind `#[cfg(feature = "mls")]` so
cargo builds without it never pull the ~50 transitive crates
(`hpke-rs`, `tls_codec`, p256/p384, `rustls`-ish machinery).

### Fixed

- `core/src/mixnet.rs` test — borrow-order issue (`pkt.layer.len()`
  called inside `pkt.layer[..]` subscript) surfaced by the newer rustc
  on the VPS. Extracted to a local.
- `cli/Cargo.toml` — CLI now depends on `phantomchat_core` with
  `features = ["net", "mls"]` so `phantom selftest` can demonstrate the
  full Tier-2 stack.

---

## [2.5.0] — 2026-04-20 — Tier 2 fertig

### Added — Onion-routed mixnet

- `mixnet.rs` — Sphinx-style layered AEAD mixnet. N-hop route, one
  X25519 ephemeral shared across all hops; each hop peels its layer via
  `ECDH(own_secret, eph_pub) → HKDF → XChaCha20-Poly1305` and either
  forwards (`TAG_FORWARD`) or delivers (`TAG_FINAL`).
- `MixnetHop`, `MixnetPacket` (with serde-free wire serialisation),
  `pack_onion()`, `peel_onion() → Peeled::{Forward, Final}`.
- **5 tests**: single-hop delivery, 3-hop peel-chain, wrong-key refusal,
  AEAD-tamper detection, wire serialisation round-trip.
- Hops pick themselves out of a public Nostr directory (future work);
  this module is the transport primitive.

### Added — Private Set Intersection (contact discovery)

- `psi.rs` — DDH-PSI over Ristretto255 (`curve25519-dalek`). Three-round
  protocol: Alice sends `H(a)^α`, Bob returns `{H(a)^(αβ)}` + his own
  blinded set `H(b)^β`, Alice re-blinds and intersects. Each side
  learns only the intersection — the non-matching half of their set
  stays hidden under the DDH assumption.
- `PsiClient::new(local_set)`, `PsiServer::new(directory)`, stateless
  `blinded_query` / `double_blind` / `blinded_directory` / `intersect`.
- Domain-separated hash-to-Ristretto so PSI points can't collide with
  any other PhantomChat subprotocol.
- **5 tests**: exact-intersection recovery, empty-intersection privacy,
  all-match (self-intersection), arity mismatch rejection, fresh
  scalars on every session (no cross-run membership leakage).

### Added — WebAssembly bindings

- `wasm.rs` — `wasm-bindgen`-annotated entry points guarded by the
  `wasm` Cargo feature. Stateless surface: `wasm_generate_address`,
  `wasm_safety_number`, `wasm_address_parse_ok`,
  `wasm_prekey_bundle_verify`, `wasm_pack_onion`, `wasm_peel_onion`.
- Enables a browser-side PhantomChat client that hands session state
  to IndexedDB and calls these crypto primitives per message.
- Build recipe documented in the module header; pins `getrandom v0.2`
  `js` feature via `[target.'cfg(target_arch = "wasm32")']`.

### Added — MLS integration plan

- `mls.rs` — intentional stub + roadmap. `GROUP_VERSION_MLS = 2`
  reserved so future TreeKEM-based groups coexist with the shipping
  Sender-Keys format without a flag day. The `openmls` v0.6 dep and
  ciphersuite bridge is a separate commit (see module docs for the
  full rationale — pulling `rustls` + ~50 transitive crates is
  non-trivial and best done in a dedicated session).

### Selftest: 6 → 8 phases, 23 checks

`phantom selftest` now runs Phases 7 (onion mixnet — 3-hop peel +
wrong-key refusal) and 8 (PSI — 2 shared of 3, 0 non-shared leaked).
Live on the Hostinger VPS: **23/23 passed**.

### Deps

- `curve25519-dalek = 4.1` with `rand_core` + `digest` features (for
  PSI's Ristretto hash-to-point).
- `wasm-bindgen = 0.2` + `serde-wasm-bindgen = 0.6` (optional, `wasm`
  feature only).

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
