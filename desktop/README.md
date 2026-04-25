# PhantomChat Desktop

## What this is

Tauri 2 desktop messenger built on top of the `phantomchat_core` Rust crypto
crate with a React + Tailwind frontend. Provides end-to-end encrypted 1:1
chat (sealed-sender envelopes over a Double Ratchet), MLS group chat
(RFC 9420), multi-relay redundancy with auto-reconnect and dedupe, native
system tray, and OS notifications. The same backend feature subset as the
CLI minus the Flutter FFI bridge and SQLCipher storage.

## Quick start (development)

```sh
cd desktop
npm install
npm run tauri dev
```

Tauri prerequisites must be installed first — see
<https://tauri.app/start/prerequisites/>. The most common gotcha on Linux
is missing system libs:

```sh
# Debian / Ubuntu
sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev \
    libssl-dev libayatana-appindicator3-dev librsvg2-dev \
    build-essential pkg-config
```

On Windows, WebView2 is preinstalled on Win10/11 — no extra runtime
download needed. macOS needs Xcode Command Line Tools.

## Build

```sh
npm run tauri build
```

Output lands in `src-tauri/target/release/bundle/...` (deb/AppImage on
Linux, .msi/.exe on Windows, .app/.dmg on macOS).

For a Windows MSI cross-built or built natively:

```sh
npm run tauri build -- --target x86_64-pc-windows-msvc
```

This needs MSVC + WiX on the host. **NB:** when building on the Windows
dev box (codename Nexus), all toolchains and the project clone MUST live
on `D:\` or `E:\` — never on `C:\`. The `C:\` SSD is small and reserved
for the OS; `cargo` and `node_modules` will fill it in minutes.

## First-launch flow

The first run drops the user into a 5-step onboarding wizard (Welcome →
Identity (generate or restore) → Relays → Share address → Done). The
generated identity is persisted to the platform's standard app-data dir:

| Platform | Location                                                       |
| -------- | -------------------------------------------------------------- |
| Linux    | `~/.local/share/de.dc-infosec.phantomchat/`                    |
| macOS    | `~/Library/Application Support/de.dc-infosec.phantomchat/`     |
| Windows  | `%APPDATA%\de.dc-infosec.phantomchat\`                         |

Files written there: `keys.json`, `contacts.json`, `sessions.json`,
`messages.json`, `relays.json`, `me.json`, `mls_directory.json`, plus the
`mls_state/` subdir for OpenMLS group state.

## Relay configuration

Default relay set written to `relays.json` on first launch:

- `wss://relay.damus.io`
- `wss://nos.lol`
- `wss://relay.snort.social`

Edit any time from `Settings → Relays`. Save fans out across all listed
relays in parallel — incoming envelopes are deduped by ID, outgoing
envelopes are published to every relay (best-effort, errors logged).

## MLS groups

Open the `Channels` tab. Create a group, then invite peers by pasting:

1. Their MLS Key Package (hex bytes, exported via `mls_publish_key_package`).
2. Their PhantomChat address (`phantom:view:spend`).
3. Their Ed25519 signing-pub (hex), so sealed-sender attribution shows
   their human label instead of `?<8hex>`.

The Welcome envelope auto-flies via the configured relay set. Subsequent
group messages broadcast to all members through the same fan-out path.

## File transfer

Paperclip button in the input bar. Single-shot upload, max **5 MiB** per
file, encrypted as a regular sealed envelope with an `application/octet-stream`
payload tag. Received files are saved to `~/Downloads/PhantomChat/` (or
the OS equivalent) and a notification is fired.

## System tray

A single tray icon with tooltip `PhantomChat`. Single left-click toggles
the main window between shown / hidden. Right-click opens a menu with
`Show / Hide`, a status line (current connection state), and `Quit`.
Closing the main window via the X hides it instead of exiting — quit
through the tray menu.

## Tor mode

`Settings → Privacy → Maximum Stealth` flips all relay connections to
SOCKS5. Default proxy address is `127.0.0.1:9050` (matches the system
`tor` package on Debian/Arch). Override the address in the same panel
if you're running Tor on a non-default port or remote host.

## Architecture

```
Frontend (React + Tailwind, Vite)
        ↕ Tauri IPC (invoke / emit)
Backend (lib.rs)
        ↕ phantomchat_core (sealed envelope, MLS, file_transfer)
        ↕ phantomchat_relays (multi-relay, dedupe, auto-reconnect)
        ↕ Nostr WebSocket (or SOCKS5/Tor)
```

## Project layout

```
desktop/
├── index.html              Vite entry HTML
├── package.json            npm scripts + JS deps (React, Tailwind, Tauri API)
├── tailwind.config.js      Cyberpunk palette (neon-green, neon-magenta, …)
├── vite.config.ts          Vite + Tauri dev-server glue
├── src/                    React frontend
│   ├── main.tsx            React root
│   ├── App.tsx             Top-level layout (panes, tabs, modals)
│   ├── styles.css          Tailwind layer + global .neon-* utilities
│   ├── types.ts            Shared TS types mirroring Rust wire structs
│   └── components/         All UI panes / modals
│       ├── AddContactModal.tsx     Contact-add form (label + address)
│       ├── AddressQR.tsx           QR-code render of phantom: address
│       ├── BindContactModal.tsx    Bind unbound sender → known contact
│       ├── ChannelsPane.tsx        MLS group list + create/invite UI
│       ├── ContactsPane.tsx        1:1 contact list + selection
│       ├── IdentityGate.tsx        Splash before identity exists
│       ├── InputBar.tsx            Message composer + file attach
│       ├── MessageStream.tsx       Scrollback + bind-prompt CTA
│       ├── OnboardingWizard.tsx    5-step first-launch flow
│       ├── SettingsPanel.tsx       Identity / relays / about / wipe
│       └── StatusFooter.tsx        Connection status + gear button
└── src-tauri/              Rust backend (Tauri 2)
    ├── Cargo.toml          Backend deps (tauri, phantomchat_core / _relays, qrcodegen, …)
    ├── tauri.conf.json     App identifier, window config, bundle metadata
    ├── build.rs            Tauri build-script glue
    ├── capabilities/       Tauri 2 capability files (tray, notif, dialog)
    ├── icons/              App + tray icons (binary — do not Read)
    └── src/
        ├── main.rs         Thin binary entry point → lib::run()
        └── lib.rs          All Tauri commands, listener loop, persistence
```

## Troubleshooting

- **WebView2 missing on old Win**: PhantomChat uses Tauri 2, which needs
  WebView2. Win10/11 ship it; Win8.1 / older needs the Evergreen
  installer from Microsoft.
- **Linux build fails with `libwebkit2gtk-4.1` not found**: install the
  system deps from the *Quick start* section above. On older distros the
  package is `libwebkit2gtk-4.0-dev`; pin Tauri to a matching version.
- **MLS state corruption** after a crash mid-commit: delete
  `mls_state.bin` (or the entire `mls_state/` dir) in the app-data
  directory to reset all group memberships. You'll need to be re-invited
  to any groups you were in.
- **"identity already exists; refusing to overwrite"** during onboarding:
  a previous run wrote a `keys.json`. Either restore from that keyfile
  via the Restore flow, or wipe the app-data dir to start fresh.
- **Tray icon missing on Linux**: install `libayatana-appindicator3-dev`
  and a tray-capable desktop (GNOME needs the AppIndicator extension).
