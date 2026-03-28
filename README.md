# 🛡️ PhantomChat — Decentralized Privacy Messenger

![Branding](https://img.shields.io/badge/Status-Functional-brightgreen)
![License](https://img.shields.io/badge/License-MIT-blue)
![Architecture](https://img.shields.io/badge/Architecture-P2P%20%2F%20Decentralized-orange)
![Security](https://img.shields.io/badge/Security-Double%20Ratchet-red)

**PhantomChat** is a decentralized, end-to-end encrypted messaging bridge designed for maximum privacy and resilience. It implements a robust cryptographic stack including a functional **Double Ratchet** protocol for perfect forward secrecy.

---

## 🚀 Key Features

- **🔐 End-to-End Encryption**: Powered by **XChaCha20-Poly1305** and **X25519** key exchange.
- **🛡️ Double Ratchet Protocol**: Perfect forward secrecy through continuous key updates for every message.
- **👻 Decentralized Identity**: No central server, no registration, no metadata logging.
- **⚙️ Rust Core**: High-performance, memory-safe cryptographic implementation.

---

## 🛠️ Technical Stack

- **Core**: Rust 1.75+
- **Cryptography**: `x25519-dalek`, `chacha20poly1305`, `sha2`
- **Networking**: P2P Simulation (Nostr Relay Support in dev)
- **Serialization**: `serde`, `serde_json`

---

## 📖 Setup & Usage (CLI)

### 🔑 Keygen
Generate your cryptographic identity:
```bash
cargo run --package phantomchat_cli -- keygen --out keys.json
```

### 📡 Listen
Start listening for messages:
```bash
cargo run --package phantomchat_cli -- listen --file keys.json
```

### ✉️ Send
Send a message securely:
```bash
cargo run --package phantomchat_cli -- send --file keys.json --recipient-spend-pub <HEX_PUBKEY> --message "Hello from the shadows"
```

---

## 🛡️ Security Disclaimer
*This project is a cryptographic demonstration for educational and portfolio purposes. While it implements real Double Ratchet mechanisms, it has not been audited by a professional security firm.*

---

## 🇩🇪 Deutsch: Kurzbeschreibung
PhantomChat ist ein dezentraler Messenger mit Fokus auf maximale Anonymität. Er nutzt das Double-Ratchet-Verfahren für Forward Secrecy und basiert auf dem performanten Rust-Stack. **Entwickelt von DC INFOSEC.**

---

© 2026 **DC INFOSEC** | [N0L3X GitHub](https://github.com/N0L3X)