# phantom-build-org-msi

PhantomChat **enterprise MSI templater** — bakes per-org bootstrap data
into a deploy artifact so an IT-Admin can push PhantomChat to N PCs via
SCCM / Intune / Group Policy and have every install auto-enroll into the
same organization directory without showing the onboarding wizard.

This is the Wave 7C complement to the Wave 7A **mDNS LAN auto-discovery**:

| Scenario                                  | Best fit                       |
| ----------------------------------------- | ------------------------------ |
| Office LAN, all on the same subnet        | Wave 7A mDNS (zero config)     |
| Remote workforce / corporate fleet push   | Wave 7C pre-seeded MSI         |
| Hybrid (some onsite, some VPN, some WFH)  | Both — the bootstrap stamps a LAN code so onsite hosts also auto-join the mDNS group |

## What gets baked in

The templater emits a `bootstrap.json` describing the org directory + a
PowerShell installer script. The desktop app reads `bootstrap.json` on
first launch (see `desktop/src-tauri/src/lib.rs::try_apply_bootstrap`) and:

* Skips the onboarding wizard entirely.
* Generates a fresh per-install identity (no UI prompt).
* Pre-populates `contacts.json` with every directory entry except self.
* Pre-populates `relays.json` with the org's relay set.
* Auto-calls `lan_org_join(<code>)` if `auto_join_lan_org_code` is set.
* Persists `branding.display_name` to `me.json` and re-titles the window.

The schema is versioned (`schema_version: 1`) so future incompatible
field changes can be detected + refused cleanly.

## Usage

```bash
phantom-build-org-msi \
  --org-name "Kanzlei Schmidt & Partner" \
  --org-id   kanzlei-schmidt-2026-04 \
  --members  members.csv \
  --relays   relays.txt \
  --base-msi PhantomChat_0.1.0_x64_en-US.msi \
  --out      PhantomChat-Kanzlei-0.1.0.msi \
  --branding-display-name "Kanzlei-Chat" \
  --branding-primary-color "#00FF9F"
```

This writes three artifacts next to `--out`:

* `<out>.bootstrap.json` — the per-org pre-seed
* `<out>.deploy.ps1`     — silent installer + bootstrap-stage script
* `<out>.README.txt`     — admin-facing quick-start

### `members.csv` schema

CSV with header row, comma-separated, UTF-8:

```
label,address,signing_pub_hex
alice@kanzlei-schmidt.de,phantom:abc...:def...,1122aabb...
bob@kanzlei-schmidt.de,phantom:111...:222...,3344ccdd...
```

All three columns are required for every row. The `label` is what other
org members will see in their contact lists; the `address` is the
canonical `phantom:view:spend` form the CLI also produces; the
`signing_pub_hex` is the Ed25519 sealed-sender pubkey from the user's
keyfile (`signing_public` field in `keys.json`).

### `relays.txt` format

One `wss://` URL per line. Blank lines and `#`-comments are ignored.

```
# Public relays
wss://relay.damus.io
wss://nos.lol
# Org-private relay
wss://strfry.kanzlei-schmidt.de
```

## Deployment recipes

### SCCM (Microsoft Endpoint Configuration Manager)

1. Drop `<out>.bootstrap.json`, `<out>.deploy.ps1`, and the upstream
   `PhantomChat_<ver>_x64_en-US.msi` into a single source directory on
   the SCCM content share.
2. Create a new **Application** → Script Installer.
3. Install command: `powershell.exe -ExecutionPolicy Bypass -File "<out>.deploy.ps1"`
4. Detection method: file presence of
   `%APPDATA%\de.dc-infosec.phantomchat\bootstrap.json`.
5. Deploy to the target collection. SCCM runs the script in SYSTEM
   context — the per-user `%APPDATA%` path resolves at PhantomChat first
   launch, not at MSI install time, so this works for every user profile
   created on the host afterwards.

### Intune (Microsoft Endpoint Manager)

1. Wrap `<out>.deploy.ps1` and the MSI into a `.intunewin` package via
   the IntuneWinAppUtil.
2. Add as a new **Win32 app**.
3. Install command: `powershell.exe -ExecutionPolicy Bypass -File "<out>.deploy.ps1"`
4. Uninstall command: standard `msiexec /x {ProductCode} /qn`.
5. Detection rule: file `%APPDATA%\de.dc-infosec.phantomchat\bootstrap.json`.
6. Assign to the target Azure AD group.

### Group Policy (legacy, AD-joined PCs)

1. Place the bundle on a UNC share readable by Domain Computers.
2. Create a Computer Configuration → Preferences → Scheduled Task
   that runs `<out>.deploy.ps1` once at next boot.
3. Alternatively use a Computer-Configuration-Startup-Script GPO
   pointing at `<out>.deploy.ps1`.

### Manual (single host, smoke test)

```powershell
# As local admin
.\PhantomChat-Kanzlei-0.1.0.msi.deploy.ps1
```

Then launch PhantomChat — the wizard is skipped and the org directory
is visible immediately.

## Limitations (Wave 7C scope)

* **One MSI per org**, not per user. The directory enumerates the whole
  org but the install doesn't know **which** entry it is. Each install
  generates a fresh per-host identity and relies on (a) mDNS auto-
  discovery on the office LAN and (b) every other org member having the
  new identity added through their own contact-add flow for it to be
  reachable from remote.
* **No true MSI re-bundling.** WiX is a Windows-only toolchain and we're
  shipping a Linux-side tool; the deploy.ps1 fallback achieves the same
  B2B outcome (one-click admin push → all PCs auto-enrolled) without
  forcing every contributor onto the WiX SDK.
* **`org_secret` is staged but inert.** Generated as 32 CSPRNG bytes and
  persisted into `bootstrap.json`, but not yet wired into any send/
  receive path. Reserved for a follow-up wave that signs org-internal
  directory updates.
* **No anti-replay / TOFU validation** of `bootstrap.json` on the
  desktop side beyond `schema_version == 1`. An attacker with write
  access to `%APPDATA%\de.dc-infosec.phantomchat\bootstrap.json` before
  first launch could pre-seed a malicious directory. Mitigation:
  bootstrap is only consumed BEFORE the onboarded marker exists, so
  post-install tampering is a no-op.

## Wave 7C-followup ideas

* True per-user MSI re-bundling (each user gets their own MSI with
  their identity already baked in → real Zero-Touch).
* Cross-platform WiX shim so the templater emits a true `.msi` instead
  of a deploy.ps1 fallback.
* Sign the bootstrap with the `org_secret` so the desktop app can
  detect tampering before applying.
