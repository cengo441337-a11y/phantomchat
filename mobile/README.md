# phantomchat (mobile)

Flutter front-end for PhantomChat. The Rust core is compiled to a native
shared library (via `cargo-ndk` + `flutter_rust_bridge`) and bundled into the
Android APK / iOS app.

## Getting Started

This project is a starting point for a Flutter application.

A few resources to get you started if this is your first Flutter project:

- [Learn Flutter](https://docs.flutter.dev/get-started/learn-flutter)
- [Write your first Flutter app](https://docs.flutter.dev/get-started/codelab)
- [Flutter learning resources](https://docs.flutter.dev/reference/learning-resources)

For help getting started with Flutter development, view the
[online documentation](https://docs.flutter.dev/), which offers tutorials,
samples, guidance on mobile development, and a full API reference.

---

## Production builds + signing

By default `flutter build apk --release` produces a release-mode APK signed
with Flutter's auto-generated **debug-keystore**. That's fine for pilot
sideloading, but it has three big downsides:

1. The debug-keystore is **per-machine and ephemeral** — different from the
   production-keystore. When we eventually switch, every existing install will
   refuse to upgrade (Android rejects upgrades whose signing certificate
   changed) and users will have to **uninstall + reinstall** to get the new
   build.
2. The Play Store **rejects debug-signed uploads**.
3. Browsers and Android show "App von unbekannter Quelle" / "App from unknown
   source" warnings, partly because of debug-signing.

The fix is a persistent **production upload-keystore** that we use for every
release from now on, forever.

### One-time setup: generate the production keystore

Run on the build machine, exactly **once**:

```bash
bash mobile/scripts/generate-release-keystore.sh
```

This:

- Generates `~/.android/phantomchat-release.jks` — RSA 4096, 10000-day
  validity, alias `phantomchat`, distinguished name
  `CN=PhantomChat, OU=DC INFOSEC, O=DC INFOSEC, L=Berlin, ST=Berlin, C=DE`
  (override the city via `--cn-city <city>` or `PHANTOMCHAT_CN_CITY=...`).
- Generates a CSPRNG 32-character alphanumeric password and writes it to
  `~/.android/phantomchat-release.password.txt` (mode `0600`).
- Refuses to overwrite an existing keystore (idempotent re-run is safe).
- Prints a loud warning telling you to back everything up.

> **!!! CRITICAL — READ THIS !!!**
>
> The production keystore is **the only thing on earth that can sign updates
> for the PhantomChat Android app**. If you lose `phantomchat-release.jks`
> *or* you lose the password:
>
> - Every existing PhantomChat install becomes **permanently unupgradeable**.
>   Users have to uninstall and reinstall to receive any new version.
> - The Play Store listing becomes orphaned — you can never publish a new
>   version under the same package id (`de.dcinfosec.phantomchat`) again.
> - There is no recovery. Not from us, not from Google, not from anyone.
>
> Back up **both** files immediately to:
>
> 1. Your password manager (1Password / Bitwarden / KeePassXC) — keystore
>    as a secure attachment, password as a secure note.
> 2. The Hostinger VPS, alongside the Tauri Updater key
>    (`/root/secrets/phantomchat/android/`, root-only perms).
> 3. An offline encrypted USB stored physically separately from your laptop.
>
> Verify each backup by `sha256sum`-ing the file and matching it against the
> local copy.

### Wire the keystore into Gradle

Copy the template, then fill in the real password from the password file:

```bash
cp mobile/android/key.properties.template mobile/android/key.properties
# Then edit mobile/android/key.properties and replace the storePassword /
# keyPassword placeholders with the contents of:
#   ~/.android/phantomchat-release.password.txt
```

`mobile/android/key.properties` is **gitignored** — it never goes near a
commit. The matching `.template` file IS tracked so contributors can see the
expected schema.

`mobile/android/app/build.gradle.kts` automatically picks up `key.properties`
and uses it for `signingConfigs.release`. If `key.properties` is absent, the
build falls back to debug-signing with a warning so contributors without the
production keystore can still produce a local `--release` APK for testing.

### Build a sideloadable APK (release, production-signed)

```bash
bash mobile/scripts/build-android.sh
```

The script logs whether it's using the production or debug keystore. Output
APKs land in `mobile/build/app/outputs/flutter-apk/`.

For a debug-mode APK (no production-signing needed):

```bash
bash mobile/scripts/build-android.sh --debug
```

### Build an Android App Bundle (.aab) for the Play Store

```bash
bash mobile/scripts/build-android-bundle.sh
```

This **requires** `mobile/android/key.properties` to exist — the script
refuses to run otherwise, since debug-signed `.aab`s are useless for upload.
The signed bundle is written to:

```
mobile/build/app/outputs/bundle/release/app-release.aab
```

Upload that file to the Google Play Console (Internal testing → Production).
