# PhantomChat — Windows Build & Code-Signing

How to produce a signed `.msi` (Windows Installer) and `.exe` (NSIS) on Windows.

## TL;DR

PowerShell, prompts for the PFX password so it never lands in shell
history / `ConsoleHost_history.txt`:

```powershell
$env:PHANTOMCHAT_SIGNTOOL = "C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\signtool.exe"
$env:PHANTOMCHAT_PFX_PATH = "E:\phantomchat-pilot-cert.pfx"
$pwd = Read-Host "PFX password" -AsSecureString
$bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($pwd)
$env:PHANTOMCHAT_PFX_PASSWORD = [System.Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
.\scripts\build-windows.cmd
Remove-Item Env:\PHANTOMCHAT_PFX_PASSWORD
```

Output:

- `target\release\phantomchat_desktop.exe` — signed inner exe
- `target\release\bundle\msi\PhantomChat_<ver>_x64_en-US.msi` — signed MSI
- `target\release\bundle\nsis\PhantomChat_<ver>_x64-setup.exe` — signed NSIS
  installer

Tauri's `bundle.windows.signCommand` runs `signtool` at the right moments
(after Tauri patches the exe with bundle-type information), so the inner
exe inside the installer is signed identically to the outer installer.
This is the only build flow that produces a self-consistent SmartScreen
trust path.

## Prerequisites

| Tool                  | Where (Nexus layout)                                                                 |
|-----------------------|--------------------------------------------------------------------------------------|
| Rust (stable, MSVC)   | `D:\rust\.cargo\bin\` (rustup home `D:\rust\.rustup\`)                              |
| Cargo Tauri           | `D:\rust\.cargo\bin\cargo-tauri.exe` (`cargo install tauri-cli --version "^2"` to match CI)  |
| Node.js + npm         | `C:\Program Files\nodejs\` (LTS)                                                     |
| Visual Studio 2019/22 BuildTools | `C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\` (cl.exe + linker) |
| Windows 10/11 SDK     | `C:\Program Files (x86)\Windows Kits\10\` (signtool.exe; SDK 10.0.19041 or newer)    |
| **CMake 4.x**         | `E:\cmake\` (winget install Kitware.CMake --location E:\cmake) — required by `whisper-rs` (Wave 11D) |
| **LLVM / libclang**   | `E:\llvm\` (winget install LLVM.LLVM --location E:\llvm) — `bindgen` needs `libclang.dll`; set `LIBCLANG_PATH=E:\llvm\bin` |
| WiX Toolset 3.14      | Tauri downloads automatically into `%LOCALAPPDATA%\tauri\WixTools314\` (~30 MB)      |
| NSIS                  | Tauri downloads automatically into `%LOCALAPPDATA%\tauri\NSIS\` (~10 MB)             |

Per the Nexus rule, the **project clone + Cargo target dir + Rust toolchain
home** all live on `D:` or `E:`. The auto-downloaded WiX/NSIS caches under
`%LOCALAPPDATA%` are small (~40 MB total) and an exception we accept.

## Code-signing tiers

| Tier        | Cost              | SmartScreen          | Use case                                      |
|-------------|-------------------|-----------------------|-----------------------------------------------|
| Self-signed | 0 €              | "Unknown publisher"  | Dev / pilot / internal demo                   |
| OV cert     | ~€250–400 / year | "Unknown publisher" until reputation built (~3000 installs) | Production, slow rollout                      |
| EV cert     | ~€450–700 / year | Clean install, no warning from day 1 | B2B SaaS, big-customer pilots                 |

Self-signed: fast, free, fine for `n0l3x@nexus → Deniz's laptop` pilot
demos. The customer has to **install the public certificate** into their
**Trusted Root** + **Trusted Publishers** stores once, after which
SmartScreen accepts the installer without warning. Our public cert lives
at `keys/phantomchat-pilot-cert.cer` (committed for reproducibility).

OV / EV: needed for self-service B2B downloads where you can't ask a
customer to install a root cert. EV is the only tier that bypasses
SmartScreen reputation entirely.

## Generating the pilot self-signed certificate

```powershell
$cert = New-SelfSignedCertificate `
    -Type CodeSigning `
    -Subject "CN=DC INFOSEC PhantomChat (Pilot Self-Signed), O=DC INFOSEC, C=DE" `
    -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256 `
    -NotAfter (Get-Date).AddYears(2) `
    -CertStoreLocation Cert:\CurrentUser\My `
    -KeyExportPolicy Exportable

$pwd = Read-Host "Pick a PFX password" -AsSecureString
Export-PfxCertificate -Cert $cert -FilePath "E:\phantomchat-pilot-cert.pfx" -Password $pwd
Export-Certificate    -Cert $cert -FilePath "E:\phantomchat-pilot-cert.cer"
```

Thumbprint goes into the build's audit log (`scripts\build-windows.cmd`
echoes the path; the actual cert hash is embedded in the signed binary's
Authenticode block).

## Customer-side: trusting the self-signed cert

Pilot customer ships the `.cer` once + the signed `.msi`. They run
PowerShell **as Administrator**:

```powershell
Import-Certificate -FilePath phantomchat-pilot-cert.cer -CertStoreLocation Cert:\LocalMachine\Root
Import-Certificate -FilePath phantomchat-pilot-cert.cer -CertStoreLocation Cert:\LocalMachine\TrustedPublisher
```

After this the `.msi` installs without "Unknown publisher" warning.

If the customer can't or won't run those commands, that's the queue to
upgrade to OV / EV.

## Verifying a signed artifact

```powershell
Get-AuthenticodeSignature -FilePath PhantomChat_3.0.2_x64_en-US.msi |
    Format-List Status, SignerCertificate, TimeStamperCertificate
```

`Status` should be `Valid` if the cert is in a trusted store, or
`UnknownError` (with status message about untrusted root) if it isn't —
the signature itself is correct in both cases, the difference is only
trust-chain validation.

## How Tauri's signCommand integration works

`desktop/src-tauri/tauri.conf.json` declares:

```json
"bundle": {
  "windows": {
    "signCommand": "cmd /C ..\\..\\scripts\\sign-windows.cmd %1"
  }
}
```

The wrapper `scripts\sign-windows.cmd` is needed because Tauri's
`signCommand` parser is shlex-style and does NOT pipe the command
through `cmd.exe` — meaning `%PHANTOMCHAT_PFX_PATH%` and friends never
get expanded if you put them directly in the JSON. The batch wrapper
expands them via `cmd.exe`'s native env-var resolution, then calls
signtool. Tauri runs this command:

1. Once per **bundle target's patched exe** — Tauri patches
   `phantomchat_desktop.exe` with a per-bundle ID (`msi`, `nsis`, etc.)
   and signs each variant separately. This is critical: a single
   pre-signed exe wouldn't survive the patching.
2. Once on the **outer bundle** itself — `.msi` and the NSIS `.exe`
   installer.

Result: every binary a Windows user touches is signed by the same
certificate, in one `cargo tauri build` invocation. No post-build sign
script needed.

## Building unsigned (dev / CI without a cert)

If `PHANTOMCHAT_PFX_PATH` is unset, signtool will be invoked with a
literal `%PHANTOMCHAT_PFX_PATH%` path that doesn't exist, and will fail.
For pure-dev builds, use:

```cmd
cargo tauri dev      :: dev server, no bundle, no signing
cargo tauri build --no-bundle   :: produces unsigned exe only
```

For "build a bundle but skip signing", temporarily comment out the
`signCommand` line in `tauri.conf.json` — there's no env var to suppress
the signCommand once it's in the config.

## Environment-isolation script (PowerShell wrapper for paranoid users)

If you don't want the PFX password living in your shell history /
ConsoleHost_history.txt, prompt at build time:

```powershell
$pwd = Read-Host "PFX password" -AsSecureString
$bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($pwd)
$env:PHANTOMCHAT_PFX_PASSWORD = [System.Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
.\scripts\build-windows.cmd
Remove-Item Env:\PHANTOMCHAT_PFX_PASSWORD
```

For CI use Azure Key Vault or GitHub Actions encrypted secrets.

## Troubleshooting

**"Multiple certificates were found that meet all the given criteria"** —
signtool found multiple certs in the user store that match its filter.
Either narrow the filter (`/sha1 <thumbprint>`), or use the bash-free
PowerShell invocation. The `signCommand` config above uses `/f <pfx>`
which avoids the cert-store search entirely.

**"Eine Zertifikatkette ... endete jedoch mit einem Stammzertifikat,
das beim Vertrauensanbieter nicht als vertrauenswürdig gilt"** —
PowerShell's `Set-AuthenticodeSignature` is stricter than `signtool` and
refuses to sign with a cert whose root isn't trusted. This is why the
build flow uses `signtool` directly.

**Inner exe is unsigned, outer MSI is signed** — you signed the
artifacts AFTER `cargo tauri build` instead of letting Tauri's
`signCommand` do it during the build. Tauri patches the exe with bundle
metadata after compilation but before bundling, so any pre-build or
post-build signing of the standalone exe is wasted — only the
`signCommand` integration produces a self-consistent signed-everywhere
build.
