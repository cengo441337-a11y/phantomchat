# PhantomChat — Mobile (Flutter / Android)

The mobile client for [PhantomChat](../README.md). Up-to-date wire-protocol parity
with the v3.0.2 Desktop release: **MLS group chat (RFC 9420)**, **sealed-sender
attribution**, **read-receipts + typing indicators**, **voice messages** (Wave 11B),
**in-app APK auto-update** (Wave 11G), plus stub / system-message handling for
**file-transfer** so a v3 Desktop user sending a file no longer renders garbage
on the phone.

iOS is **out of scope** for this release — see the bottom of this file.

---

## Voice messages (Wave 11B)

The mobile client can record short voice clips and send them as encrypted
messages on the same E2E + relay path as text. Workflow:

1. **Record** — long-press the microphone button in the input bar. The
   client uses the platform-native recorder (`record` package on Android)
   to capture an Opus-encoded WebM (or AAC fallback on older devices).
2. **Send** — release the button. The client builds a `VOICE-1:` framed
   payload (header bytes + raw codec bytes), wraps it in a sealed
   envelope, and publishes via the existing relay path. There is no
   separate audio-CDN; the bytes ride inside the same envelope as a text
   message would.
3. **Playback** — incoming `VOICE-1:` messages render as a play-button
   row in the chat. The audio bytes are saved (read-only) under
   `<app_documents>/voice/<msg_id>.<ext>` and decoded by the platform
   audio player; they are **never re-uploaded**.

If the desktop side has STT + AI Bridge enabled (Wave 11D), an inbound
voice message is also transcribed in-process via whisper.cpp and the
text is fed to the configured LLM provider — but the audio itself stays
local on the desktop. See [`docs/AI-BRIDGE.md`](../docs/AI-BRIDGE.md).

## In-app APK auto-update (Wave 11G)

The Android client polls a signed update manifest URL on startup (and
manually via *Settings → About → Check for updates*). When a newer
version is detected, an **update banner** appears at the top of the
chat list with a "Download" action.

- Manifest URL is configurable in *Settings → Updates → Manifest URL*
  (default: `https://updates.dc-infosec.de/phantomchat-android/manifest.json`).
- The downloader pins the **HTTPS origin** to the configured manifest
  host — the manifest cannot redirect the APK download to a third
  party.
- Downgrade is rejected — the new APK's `version_code` must be strictly
  greater than the installed one. Closes the obvious downgrade-attack
  vector.
- Install uses the standard Android `ACTION_INSTALL_PACKAGE` intent;
  user confirms in the system dialog.

Self-hosted orgs publish to their own manifest URL (template in
[`scripts/publish-android-update-manifest.sh`](../scripts/publish-android-update-manifest.sh)
and walkthrough in [`docs/RELAY-SELFHOSTING.md`](../docs/RELAY-SELFHOSTING.md)).

## Layout

```
mobile/
├── lib/                       # Dart / Flutter source
│   ├── main.dart              # bootstrap + RustLib.init
│   ├── screens/
│   │   ├── home.dart          # contact list + Channels button
│   │   ├── chat.dart          # 1:1 chat (now v3 sealed-sender)
│   │   └── channels.dart      # NEW — MLS group chat tab
│   ├── services/
│   │   ├── relay_service.dart # NEW — prefix-dispatch (MLS / FILE / RCPT / TYPN)
│   │   └── contact_directory.dart  # NEW — v3 contact book w/ signing_pub_hex
│   └── src/rust/              # auto-generated FRB Dart bindings
├── rust/                      # NEW — wrapper crate exposing v3 APIs
│   ├── Cargo.toml             # standalone (not a workspace member)
│   └── src/api.rs             # send_sealed_v3 / receive_full_v3 / mls_*
├── android/                   # Android Gradle project
├── scripts/
│   └── build-android.sh       # NEW — cross-compile + Gradle helper
└── flutter_rust_bridge.yaml   # points at ./rust (NOT ../core)
```

The wrapper crate (`mobile/rust/`) intentionally pulls
`phantomchat_core` with `default-features = false, features = ["net"]` so we
don't link a duplicate `flutter_rust_bridge` runtime (the desktop build pulls
the `ffi` feature; the cdylib symbols would collide).

## Prerequisites

- **Flutter** ≥ 3.41 — <https://docs.flutter.dev/get-started/install>
- **Rust** stable + the four Android targets:

  ```bash
  rustup target add aarch64-linux-android \
                    armv7-linux-androideabi \
                    x86_64-linux-android \
                    i686-linux-android
  ```

- **cargo-ndk** — `cargo install cargo-ndk`
- **Android SDK + NDK** — install via Android Studio; the script reads
  `ANDROID_NDK_ROOT` (or `NDK_HOME`). Tested with NDK 26.x.
- **JDK 17** — required by the Flutter Gradle plugin (set via `JAVA_HOME`).

## Building an APK

```bash
mobile/scripts/build-android.sh release apk
```

The script:

1. Cross-compiles `phantomchat_mobile` to all four ABIs via `cargo ndk`.
2. Stages the resulting `libphantomchat_mobile.so` files into
   `mobile/android/app/src/main/jniLibs/<abi>/` (this directory is
   git-ignored — it's regenerated on every build).
3. Runs `flutter build apk --release --split-per-abi`.

Outputs land under `mobile/build/app/outputs/apk/release/`.

For an AAB suitable for Google Play:

```bash
mobile/scripts/build-android.sh release appbundle
```

For a quick debug-signed APK (faster, but bigger):

```bash
mobile/scripts/build-android.sh debug
```

## Regenerating the FRB bindings

After editing `mobile/rust/src/api.rs`:

```bash
cd mobile && flutter_rust_bridge_codegen generate
```

This rewrites:

- `mobile/lib/src/rust/api.dart` (Dart facade)
- `mobile/lib/src/rust/frb_generated.{dart,io.dart,web.dart}`
- `mobile/rust/src/frb_generated.rs` (Rust glue)

Don't edit any of the above by hand — the codegen will overwrite them.

## v3 wire-format support matrix

| Prefix | Direction | Mobile behaviour |
|--------|-----------|------------------|
| (none) | RX        | sealed-sender 1:1, attribution surfaced via `RelayEvent.kind == "message"` |
| `MLS-WLC2` | RX | `mlsDirectoryInsert(meta) → mlsJoinViaWelcome(welcome)`, emits `mls_joined` |
| `MLS-WLC1` | RX | legacy v1 fallback (placeholder inviter), emits `mls_joined` |
| `MLS-APP1` | RX | `mlsDecrypt`, emits `mls_message` / `mls_epoch` |
| `FILE1:01` | RX | system message ("file received: …, switch to Desktop"). **Write-to-storage deferred — desktop-only feature.** |
| `RCPT-1:`  | RX | emits `receipt` event (msg_id + delivered/read) |
| `TYPN-1:`  | RX | emits `typing` event (sender_pub + ttl) — schema unified with desktop in 3.0.2 |
| `REPL-1:` / `RACT-1:` / `DISA-1:` | RX | swallow handlers (3.0.2) — no longer rendered as raw text |
| `VOICE-1:` | RX | save bytes to `<app_documents>/voice/<msg_id>.<ext>`, render play-button row |
| `MLS-APP1` | TX | wrapped + ciphertext returned by `mlsEncrypt`; transport hookup live |
| `RCPT-1:` / `TYPN-1:` | TX | encoder helpers in `relay_service.dart`; UI wired |
| `VOICE-1:` | TX | long-press mic → record (Opus/AAC) → wrap → send via existing relay path |

## iOS — out of scope (deferred)

iOS support requires:

- macOS host (Xcode is macOS-only)
- Xcode 15.x + iOS 17 SDK
- Apple Developer account (~ \$99 / year) for signing + TestFlight
- Manual `phantomchat_mobile` cross-compile to `aarch64-apple-ios` +
  `aarch64-apple-ios-sim` + `x86_64-apple-ios` and lipo into a single
  `libphantomchat_mobile.a` checked into `ios/Frameworks/`.
- `mobile/ios/Podfile` regeneration via `pod install`.

Future iOS support is gated on Mac hardware + Apple Developer cost —
see the decision matrix in [`docs/WINDOWS-BUILD.md`](../docs/WINDOWS-BUILD.md)
(the same doc covers the Mac-mini-vs-rented-CI tradeoff).
