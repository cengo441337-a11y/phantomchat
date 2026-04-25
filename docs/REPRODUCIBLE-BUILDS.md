# Reproducible Builds

This document explains how to rebuild PhantomChat artifacts from source on
your own machine and verify, byte-for-byte, that they match the official
releases published at <https://updates.dc-infosec.de/download/> and on the
project's GitHub Releases page.

The goal is **falsifiability**: a security-conscious customer should not need
to trust DC INFOSEC's build infrastructure. They should be able to clone the
public Git tag, run a documented build, and confirm the SHA-256 of every
artifact matches the checksums file we published alongside the release.

> Reproducibility is best-effort on Windows MSI today — see the
> [Caveats](#caveats) section. CLI binaries (Linux/macOS) and the Android
> APK reproduce cleanly with `SOURCE_DATE_EPOCH` pinned.

---

## Pinned toolchain

The official build pins exact tool versions. To match our checksums you must
match these versions exactly.

| Component         | Version                                                                 | Source of truth                  |
|-------------------|-------------------------------------------------------------------------|----------------------------------|
| Rust              | `stable` (whatever `dtolnay/rust-toolchain@stable` resolves on tag day) | `rust-toolchain.toml` if present, otherwise the CI run log |
| Node.js           | `20.x`                                                                  | `desktop/.nvmrc`                 |
| Flutter           | stable channel pinned to commit `cc0734ac71`                            | `mobile/flutter-version`         |
| Android NDK       | r26 (`26.3.11579264`)                                                   | `.github/workflows/release.yml`  |
| WiX Toolset       | latest from chocolatey at tag day                                       | `.github/workflows/release.yml`  |
| Tauri CLI         | `^2`                                                                    | `desktop/package.json`           |

To force-pin tools locally:

```bash
# Rust — match the toolchain CI used (look at the Release workflow log)
rustup install stable
rustup default stable

# Node — via nvm
nvm install
nvm use

# Flutter — exact commit
git clone https://github.com/flutter/flutter.git ~/flutter-pinned
( cd ~/flutter-pinned && git checkout cc0734ac71 )
export PATH="$HOME/flutter-pinned/bin:$PATH"
flutter --version
```

---

## Build steps

### CLI (Linux x86_64 / macOS / Linux aarch64)

```bash
git clone --depth 1 --branch v<X.Y.Z> https://github.com/cengo441337-a11y/phantomchat.git
cd phantomchat

# Pin embedded timestamps to the tag's commit time (deterministic).
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)

cargo build --release -p phantomchat_cli --target x86_64-unknown-linux-gnu

sha256sum target/x86_64-unknown-linux-gnu/release/phantomchat_cli
```

For other targets, swap `--target` (`aarch64-unknown-linux-gnu`,
`x86_64-apple-darwin`, `aarch64-apple-darwin`).

### Android APK (Linux build host)

```bash
# Same SOURCE_DATE_EPOCH pin
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)

# One-shot build script (cross-compile Rust core + Flutter APK)
bash scripts/build-android.sh release apk

# APKs land in:
ls mobile/build/app/outputs/flutter-apk/*.apk
sha256sum mobile/build/app/outputs/flutter-apk/*.apk
```

### Windows MSI (Windows build host or VM)

```powershell
# In an elevated PowerShell with WiX + Node + Rust installed:
$env:SOURCE_DATE_EPOCH = (git log -1 --format=%ct)

cd desktop
npm ci
npm run build
cargo tauri build --bundles msi

Get-FileHash -Algorithm SHA256 src-tauri/target/release/bundle/msi/*.msi
```

---

## Verification

Each release publishes a `SHA256SUMS.txt` file alongside the artifacts.
We additionally auto-commit `docs/checksums-v<X.Y.Z>.txt` so you can verify
checksums via the Git history without trusting the GitHub Releases UI.

The fast path is the companion script:

```bash
# Verify a published version end-to-end (downloads, hashes, compares)
bash scripts/verify-release.sh v3.0.0
```

The script will:

1. Resolve the GitHub Release for the given tag.
2. Download `SHA256SUMS.txt` from that Release.
3. Download every listed artifact from <https://updates.dc-infosec.de/download/>
   (falling back to GitHub Release assets if the CDN is unreachable).
4. Compute local SHA-256 hashes.
5. Compare line-by-line against the published checksums.
6. Print `OK` or `FAIL` with a clear summary; exit code reflects the result.

Manual verification (if you don't trust the script either — fair):

```bash
curl -fLO https://github.com/cengo441337-a11y/phantomchat/releases/download/v3.0.0/SHA256SUMS.txt
curl -fLO https://updates.dc-infosec.de/download/phantomchat-3.0.0.msi
sha256sum -c SHA256SUMS.txt --ignore-missing
```

---

## Caveats

Even with every tool pinned, two known sources of nondeterminism remain
on Windows:

- **WiX CAB headers embed the build timestamp.** Workaround: post-process
  the MSI with a CAB header strip (we ship `scripts/strip-msi-timestamps.ps1`
  for this — to be added in a future PR). Until then, MSI bytes diverge
  even though the contained payload reproduces.
- **PE timestamps in compiled DLLs.** Mitigated by `SOURCE_DATE_EPOCH`
  on Rust 1.78+ for most cases; cross-language linker stages (MSVC) may
  still embed a local clock.

Other caveats that apply across platforms:

- **Build-host paths** can leak into debug info. We strip with
  `--release` + `strip` in CI but the source-of-truth is a `cargo build`
  output, not a stripped binary. Use `--config 'profile.release.strip="symbols"'`
  if you want symbol-stripped reproducibility.
- **Cargo registry timestamps.** Cargo records the *registry-fetched*
  Cargo.lock as input but does not record fetch timestamps in the binary.
  Should not affect output bytes.
- **Platform-specific deps may differ.** OpenSSL / SQLCipher are dynamic
  on some hosts and statically vendored on others. Match the CI host
  (Ubuntu 22.04 LTS) for closest match.
- **Flutter codegen.** `flutter_rust_bridge_codegen` is regenerated on
  every build. Pin to `2.12.0` exactly (in `pubspec.yaml`) — newer codegen
  versions emit different field ordering.

---

## What CI commits per release

The release workflow auto-attaches:

- The artifact bundle (`*.msi`, `*.apk`, `phantomchat_cli-<triple>`).
- `SHA256SUMS.txt` covering every artifact.

A future enhancement (tracked separately) will also auto-commit
`docs/checksums-v<X.Y.Z>.txt` to the repo on each tag, so the checksums
gain a second, Git-attested provenance trail.
