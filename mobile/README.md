# PhantomChat — Mobile (Flutter / Android)

The mobile client for [PhantomChat](../README.md). This wave (7B-mobile-catchup) brings the Flutter
app up to wire-protocol parity with the v3.0.0 Desktop release: **MLS group chat (RFC 9420)**,
**sealed-sender attribution**, **read-receipts + typing indicators**, plus stub / system-message
handling for **file-transfer** so a v3 Desktop user sending a file no longer renders garbage on
the phone.

iOS is **out of scope for this wave** — see the bottom of this file.

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

| Prefix | Direction | Mobile behaviour (this wave) |
|--------|-----------|------------------------------|
| (none) | RX        | sealed-sender 1:1, attribution surfaced via `RelayEvent.kind == "message"` |
| `MLS-WLC2` | RX | `mlsDirectoryInsert(meta) → mlsJoinViaWelcome(welcome)`, emits `mls_joined` |
| `MLS-WLC1` | RX | legacy v1 fallback (placeholder inviter), emits `mls_joined` |
| `MLS-APP1` | RX | `mlsDecrypt`, emits `mls_message` / `mls_epoch` |
| `FILE1:01` | RX | system message ("file received: …, switch to Desktop"). **Write-to-storage deferred to wave 7B-followup.** |
| `RCPT-1:`  | RX | emits `receipt` event (msg_id + delivered/read) |
| `TYPN-1:`  | RX | emits `typing` event (sender_pub + ttl) |
| `MLS-APP1` | TX | wrapped + ciphertext returned by `mlsEncrypt`; transport hookup pending |
| `RCPT-1:` / `TYPN-1:` | TX | encoder helpers in `relay_service.dart`; UI wiring pending |

## iOS — out of scope (deferred)

iOS support requires:

- macOS host (Xcode is macOS-only)
- Xcode 15.x + iOS 17 SDK
- Apple Developer account (~ \$99 / year) for signing + TestFlight
- Manual `phantomchat_mobile` cross-compile to `aarch64-apple-ios` +
  `aarch64-apple-ios-sim` + `x86_64-apple-ios` and lipo into a single
  `libphantomchat_mobile.a` checked into `ios/Frameworks/`.
- `mobile/ios/Podfile` regeneration via `pod install`.

Targeting **wave 7B-followup-2** when a Mac mini or rented CI runner is
available.
