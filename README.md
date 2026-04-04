# PhantomChat — Decentralized Privacy Messenger

![Status](https://img.shields.io/badge/Status-Functional-brightgreen)
![License](https://img.shields.io/badge/License-MIT-blue)
![Architecture](https://img.shields.io/badge/Architecture-P2P_%2F_Nostr-orange)
![Security](https://img.shields.io/badge/Crypto-XChaCha20--Poly1305_%2B_X25519-red)
![PQ](https://img.shields.io/badge/PQ--Ready-ML--KEM--1024-blueviolet)

**PhantomChat** ist ein dezentraler, Ende-zu-Ende-verschlüsselter Messenger
mit modularem Privacy-System — entwickelt als Sicherheitsforschungsprojekt
von **DC INFOSEC**.

---

## Features

| Feature | Status |
|---------|--------|
| XChaCha20-Poly1305 AEAD | ✓ |
| X25519 Ephemeral ECDH | ✓ |
| HKDF-SHA256 Schlüsselableitung | ✓ |
| HMAC Stealth-Tags (Monero-Modell) | ✓ |
| ViewKey-basierter Envelope-Scanner | ✓ |
| Hashcash Proof-of-Work (Spam-Schutz) | ✓ |
| libp2p GossipSub P2P Mesh | ✓ |
| Dandelion++ Routing (IP-Anonymität) | ✓ |
| Nostr Relay Transport (NIP-01/NIP-59) | ✓ |
| SOCKS5 Proxy (Tor / Nym) | ✓ |
| Cover Traffic (Light + Aggressive) | ✓ |
| Daily Use / Maximum Stealth Mode | ✓ |
| Post-Quantum Hybrid PQXDH (ML-KEM-1024 + X25519) | ✓ |
| Flutter Mobile App (Android / iOS) | ✓ |
| Cyberpunk CLI | ✓ |
| SQLCipher lokale Verschlüsselung | ✓ |
| Panic Wipe | ✓ |

---

## Architektur

```
┌─────────────────────────────────────────────────────┐
│                  Flutter Mobile App                  │
│         (Cyberpunk UI · Privacy Settings)            │
└──────────────────────┬──────────────────────────────┘
                       │ flutter_rust_bridge
┌──────────────────────▼──────────────────────────────┐
│              phantomchat_core  (Rust)                │
│  Envelope · Keys · Scanner · Dandelion++ · PoW      │
│  Privacy Modes · Cover Traffic · SQLCipher          │
└──────────┬───────────────────────┬──────────────────┘
           │                       │
┌──────────▼──────────┐  ┌─────────▼─────────────────┐
│    libp2p GossipSub  │  │   phantomchat_relays       │
│    + Dandelion++     │  │   NostrRelay (TLS)         │
│    (DailyUse)        │  │   StealthNostrRelay (Tor)  │
└─────────────────────┘  └───────────────────────────┘
```

### Privacy Modes

**Daily Use** — libp2p + Dandelion++ + Nostr/TLS + Light Cover Traffic (30–180 s)

**Maximum Stealth** — Relay-only, alle Verbindungen über SOCKS5 (Tor/Nym) + Aggressiver Cover Traffic (5–15 s)

---

## CLI

```bash
# Keypair generieren
cargo run -p phantomchat_cli -- keygen

# Pairing-QR anzeigen
cargo run -p phantomchat_cli -- pair

# Privacy Mode setzen
cargo run -p phantomchat_cli -- mode stealth --proxy 127.0.0.1:9050
cargo run -p phantomchat_cli -- mode daily

# Nachricht senden
cargo run -p phantomchat_cli -- send -r <SPEND_PUB_HEX> -m "ghost protocol"

# Lauschen (alle Envelopes scannen, eigene öffnen)
cargo run -p phantomchat_cli -- listen

# Relay Health-Check
cargo run -p phantomchat_cli -- relay -u wss://relay.damus.io

# Node Status
cargo run -p phantomchat_cli -- status
```

---

## Workspace

```
phantomchat/
├── core/          Rust-Kernbibliothek (Krypto, Netzwerk, Privacy)
│   └── src/
│       ├── envelope.rs      Envelope-Format + Krypto
│       ├── keys.rs          Identity/View/Spend/PQXDH-Keys
│       ├── scanner.rs       ViewKey-basierter Envelope-Scanner
│       ├── dandelion.rs     Dandelion++ Router
│       ├── cover_traffic.rs Cover Traffic Generator
│       ├── privacy.rs       PrivacyMode + Config
│       ├── network.rs       libp2p GossipSub
│       ├── pow.rs           Hashcash PoW
│       └── api.rs           Flutter-Bridge API
├── relays/        Nostr Relay Adapter
│   └── src/
│       ├── lib.rs           BridgeProvider Trait + Factory
│       └── nostr.rs         NIP-01 Event-Typen
├── cli/           Cyberpunk Terminal Interface
├── mobile/        Flutter App (Android/iOS)
├── docs/          SECURITY.md · PRIVACY.md
├── spec/          SPEC.md Protokollspezifikation
└── infra/         docker-compose.yml (Relay-Infrastruktur)
```

---

## Security

Siehe [docs/SECURITY.md](docs/SECURITY.md) und [docs/PRIVACY.md](docs/PRIVACY.md).

> Dieses Projekt ist ein Forschungs- und Portfolio-Projekt von DC INFOSEC.
> Vor einem produktiven Einsatz ist ein externer kryptografischer Audit erforderlich.

---

© 2026 **DC INFOSEC** · [github.com/N0L3X](https://github.com/N0L3X)
