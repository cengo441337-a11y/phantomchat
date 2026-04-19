# PhantomChat

![Status](https://img.shields.io/badge/Status-v2.0.0-brightgreen)
![License](https://img.shields.io/badge/License-MIT-blue)
![Crypto](https://img.shields.io/badge/Crypto-PQXDH_%2B_XChaCha20--Poly1305-red)
![PQ](https://img.shields.io/badge/Post--Quantum-ML--KEM--1024-blueviolet)
![Network](https://img.shields.io/badge/Network-libp2p_%2F_Nostr_%2F_Tor-orange)

---

## Der Countdown läuft. Der Unsichtbare ist in der Matrix.

Die meisten Messenger versprechen dir Ende-zu-Ende-Verschlüsselung. Sie sagen dir, dass niemand deine Nachrichten lesen kann. Was sie dir nicht sagen: Sie wissen ganz genau, **wann du online bist, von wo du sendest und mit wem du sprichst.**

WhatsApp, Telegram, Signal — alle speichern Metadaten. Sie kennen deine Telefonnummer. Sie haben zentrale Server, die abgeschaltet, zensiert oder gehackt werden können. Verschlüsselter Inhalt nützt dir wenig, wenn das Muster deiner Kommunikation dich schon längst verraten hat.

**PhantomChat löst das Problem an der Wurzel. Nicht nur der Inhalt ist unsichtbar — die Kommunikation selbst hinterlässt keine Spuren.**

---

## Was PhantomChat anders macht

### Keine Identität. Kein Account. Kein Problem.

Du lädst die App herunter und bist drin. Keine Telefonnummer, keine E-Mail, kein Name, keine SIM-Karte. Deine Identität ist ein kryptografisches Schlüsselpaar — generiert lokal auf deinem Gerät, niemals ein Server berührt es. Du bist ein anonymer Schatten im Netz, und das ist Absicht.

### Post-Quantum gesichert — ab Tag 1

Elliptische Kurven allein sind nicht zukunftssicher. Shor's Algorithmus auf einem Quantencomputer bricht X25519 in Sekundenbruchteilen. PhantomChat nutzt **PQXDH**: eine hybride Schlüsselkapselung aus **ML-KEM-1024** (Kyber, FIPS 203 — der offizielle NIST-Standard) kombiniert mit X25519. Der Session-Key ist `SHA256(x25519_shared || mlkem_shared)` — beide müssen gleichzeitig gebrochen werden. Kein Quantencomputer der nächsten Jahrzehnte schafft das.

Double Ratchet Forward Secrecy ist selbstverständlich. Jede Nachricht rotiert den Key. Frühere Nachrichten bleiben auch bei zukünftiger Kompromittierung sicher.

### Der blinde Postbote — Zero-Metadata via ViewKey-Scanning

Das ist das Kernstück. Wo andere Messenger dem Relay verraten, *für wen* eine Nachricht ist (NIP-04/59-Schwächen leaken Empfänger-Korrelation), geht PhantomChat einen anderen Weg:

**Das Relay weiß niemals, wer der Empfänger einer Nachricht ist.**

Alle Envelopes sehen für das Relay identisch aus — undifferenziertes Rauschen. Der Client läuft lokal einen **Stealth-Scanner** über den gesamten Event-Stream. Mit seinem privaten ViewKey identifiziert er seine eigenen Nachrichten via ECDH + HKDF + HMAC. Das Relay ist strukturell blind gegenüber Sender-Empfänger-Korrelationen — nicht weil wir es bitten, nichts zu loggen, sondern weil es die Information physisch nicht hat.

Das Modell ist direkt vom Monero-Stealth-Address-System inspiriert. Bewährt in der Praxis, mathematisch verifizierbar.

### Dandelion++ — Deine IP existiert nicht

Bevor eine Nachricht im Netzwerk auftaucht, durchläuft sie das **Dandelion++ Protokoll**: In der Stem-Phase wird sie mit Wahrscheinlichkeit p=0,9 an genau einen zufällig gewählten Peer weitergeleitet — ohne Broadcast. Erst nach dem stochastischen Übergang folgt die Fluff-Phase (GossipSub-Broadcast). Der Stem-Peer rotiert alle 10 Minuten.

Ein Netzwerk-Beobachter sieht einen Broadcaster, der mehrere Hops vom wahren Absender entfernt ist. Deine IP ist aus dem Graphen nicht mehr zurückverfolgbar.

### Cover Traffic — Timing-Angriffe ausgehebelt

PhantomChat sendet kontinuierlich Dummy-Envelopes — CSPRNG-befüllt, auf dem Wire von echten Nachrichten nicht zu unterscheiden. Kein Angreifer kann durch Traffic-Timing-Analyse erkennen, wann du wirklich eine Nachricht sendest.

- **Daily Use Mode:** 30–180 Sekunden Zufallsintervall
- **Maximum Stealth Mode:** 5–15 Sekunden — aggressiv, lückenlos

### Der Paranoia-Schalter — Maximum Stealth Mode

Ein Klick in den Einstellungen. Ab diesem Moment:

- libp2p vollständig deaktiviert — kein direktes Peer-Exposure
- Alle Nostr-WebSocket-Verbindungen tunneln durch **SOCKS5** (Tor oder Nym)
- Das Relay sieht nur die Exit-IP des Anonymisierungsnetzes — niemals deine
- Cover Traffic läuft auf Aggressiv-Modus
- Schutz gegen **globale passive Angreifer** — das Bedrohungsmodell des Geheimdienstes

### Unabschaltbar — Echtes Serverless

PhantomChat nutzt kein zentrales AWS-Cluster, keine "DAO-gesteuerten" Netzwerke, deren Dezentralisierung niemand prüfen kann. Der Netzwerk-Stack ist hybrid:

- **libp2p GossipSub** — direktes P2P-Mesh, Kademlia-DHT, selbstheilend
- **Nostr-Relays** — offenes Protokoll (NIP-01), jeder kann einen Relay betreiben
- Fällt ein Node aus, heilt das Netzwerk im Hintergrund selbst

Solange zwei Geräte existieren, lebt das Netzwerk.

### Sybil-Resistance by Math

Jeder Envelope enthält einen **Hashcash Proof-of-Work**. Spam und Sybil-Angriffe kosten Rechenzeit. Keine zentrale Registrierung, kein Captcha — nur Mathematik.

---

## Für alle, die "Trust me bro" nicht als Krypto-Konzept akzeptieren

Ein Hinweis zur Branche: Während Mitbewerber stolz mit Begriffen wie "Individual Adaptive Encryption" und "AES 512" werfen — was jedem, der FIPS 197 gelesen hat, ein müdes Lächeln abringt, da 512-Bit-AES schlicht nicht existiert — setzen wir auf offene, überprüfbare NIST-Standards. Sicherheit entsteht nicht durch das Fantasieren über nicht existierende Schlüssellängen. Sie entsteht durch mathematisch fundierte Protokolle, die jeder prüfen kann.

**PhantomChat liefert Krypto statt Marketing-Esoterik.**

Der gesamte Krypto-Stack ist offen, dokumentiert und verifizierbar:

```
XChaCha20-Poly1305   AEAD Payload-Verschlüsselung
HKDF-SHA256          Schlüsselableitung aus ECDH
X25519               Ephemeral Diffie-Hellman
ML-KEM-1024          Post-Quantum Key Encapsulation (FIPS 203)
HMAC-SHA256          Stealth-Tags für Empfänger-Identifikation
secp256k1 Schnorr    Nostr Event-Signierung (NIP-01)
SQLCipher AES-256    Lokale Datenbankversclüsselung
```

---

## Feature Matrix

| Feature | Status |
|---------|--------|
| XChaCha20-Poly1305 AEAD | ✓ |
| X25519 Ephemeral ECDH + HKDF-SHA256 | ✓ |
| HMAC Stealth-Tags (Monero-Modell) | ✓ |
| ViewKey-basierter Envelope-Scanner | ✓ |
| Post-Quantum PQXDH (ML-KEM-1024 + X25519) | ✓ |
| Double Ratchet Forward Secrecy | ✓ (Envelope-Layer) |
| Dandelion++ IP-Anonymisierung | ✓ |
| libp2p GossipSub P2P Mesh | ✓ |
| Nostr Relay Transport (NIP-01 / Kind 1059) | ✓ |
| StealthNostrRelay (SOCKS5 → TLS → WebSocket) | ✓ |
| Cover Traffic (Light 30–180 s / Aggressive 5–15 s) | ✓ |
| Daily Use / Maximum Stealth Mode | ✓ |
| Hashcash Proof-of-Work | ✓ |
| SQLCipher lokale Verschlüsselung | ✓ |
| Panic Wipe | ✓ |
| Flutter Mobile App (Android / iOS) | ✓ |
| Cyberpunk CLI | ✓ |
| Post-Quantum Hybrid vollständig (ML-KEM / Kyber im Envelope-Flow) | ✓ |
| App-Lock PIN (PBKDF2) + Biometrie + Panic-Wipe | ✓ |
| Core integration-test suite (64 tests) | ✓ |
| CLI selftest (6 phases, 20 checks) | ✓ |
| Tor SOCKS5 Stealth-Routing live | ✓ |
| Systemd Dauer-Listener | ✓ |
| **Sealed Sender** (Ed25519 identity-level message attribution) | ✓ |
| **Payload Padding** (1024-byte blocks, gegen Length-Korrelation) | ✓ |
| **Safety Numbers** (60-Digit Signal-style Fingerprint gegen MITM) | ✓ |
| **X3DH Prekey Bundle** (SignedPrekey + OPK + Bundle-Sig-Chain) | ✓ |
| **Gruppenchat via Sender Keys** (Signal-Stil, Ed25519-signiert) | ✓ |
| **WASM-Feature-Gate** (core crypto ohne libp2p/tokio) | ✓ |
| MLS Gruppen (RFC 9420 via `openmls`) | Deferred |
| Externer Krypto-Audit | Vor Produktion |

---

## Architektur

```
┌──────────────────────────────────────────────────────────┐
│                   Flutter Mobile App                      │
│         Cyberpunk UI · Privacy Settings · Scanner        │
└────────────────────────┬─────────────────────────────────┘
                         │ flutter_rust_bridge (sync + async)
┌────────────────────────▼─────────────────────────────────┐
│                 phantomchat_core  (Rust)                  │
│                                                           │
│  Envelope (XChaCha20 · HKDF · X25519 · ML-KEM-1024)     │
│  ViewKey Scanner · Dandelion++ Router · PoW              │
│  Privacy Config · Cover Traffic Generator                │
│  libp2p GossipSub Network                                │
└────────────┬──────────────────────────┬──────────────────┘
             │                          │
  ┌──────────▼──────────┐   ┌───────────▼──────────────────┐
  │  libp2p GossipSub   │   │     phantomchat_relays        │
  │  + Dandelion++      │   │                               │
  │                     │   │  NostrRelay     (TLS/WS)      │
  │  [Daily Use]        │   │  StealthRelay   (SOCKS5→Tor)  │
  └─────────────────────┘   └──────────────────────────────┘

Daily Use:      libp2p P2P  +  Nostr/TLS  +  Cover Traffic Light
Maximum Stealth: Relay-only  +  Tor/Nym   +  Cover Traffic Aggressive
```

---

## Cyberpunk CLI

```bash
# Keypair generieren
cargo run -p phantomchat_cli -- keygen

# Pairing-QR anzeigen (ASCII, scanbar)
cargo run -p phantomchat_cli -- pair

# Privacy Mode setzen
cargo run -p phantomchat_cli -- mode stealth --proxy 127.0.0.1:9050
cargo run -p phantomchat_cli -- mode stealth --nym
cargo run -p phantomchat_cli -- mode daily

# Nachricht senden (zeigt Dandelion++ Phase)
cargo run -p phantomchat_cli -- send -r <SPEND_PUB_HEX> -m "ghost protocol"

# Lauschen — alle Envelopes scannen, eigene öffnen
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
├── core/              Rust-Kernbibliothek
│   └── src/
│       ├── envelope.rs        Envelope-Format · AEAD · Stealth-Tags
│       ├── keys.rs            Identity / View / Spend / PQXDH-Keys
│       ├── scanner.rs         ViewKey Stealth-Scanner
│       ├── dandelion.rs       Dandelion++ Router
│       ├── cover_traffic.rs   Cover Traffic Generator
│       ├── privacy.rs         PrivacyMode · ProxyConfig
│       ├── network.rs         libp2p GossipSub
│       ├── pow.rs             Hashcash PoW
│       └── api.rs             Flutter-Bridge API (FRB)
├── relays/            Nostr Relay Adapter
│   └── src/
│       ├── lib.rs             NostrRelay · StealthNostrRelay · Factory
│       └── nostr.rs           NIP-01 Event-Typen · Schnorr-Signierung
├── cli/               Cyberpunk Terminal Interface
├── mobile/            Flutter App (Android / iOS)
│   └── lib/
│       ├── services/          privacy_service.dart · ipfs_service.dart
│       └── src/ui/            privacy_settings_view.dart · profile_view.dart
├── docs/              SECURITY.md · PRIVACY.md
├── spec/              SPEC.md Protokollspezifikation
├── infra/             docker-compose.yml (Relay-Infrastruktur)
└── CHANGELOG.md
```

---

## Bedrohungsmodell

PhantomChat verteidigt gegen:

| Angreifer | Abwehr |
|-----------|--------|
| Passiver Global Observer (NSA-Modell) | Maximum Stealth + Tor + Aggressive Cover Traffic |
| Bösartige Relays | ViewKey-Modell — Relay hat keine Empfänger-Information |
| Traffic-Timing-Korrelation | Cover Traffic · Dandelion++ · SOCKS5 |
| Aktiver MITM / Key-Kompromittierung | Double Ratchet · Forward Secrecy · PQXDH |
| Quantencomputer (Shor) | ML-KEM-1024 Hybrid — beide Seiten müssen gleichzeitig brechen |
| Spam / Sybil-Angriffe | Hashcash PoW |
| Gerätekompromittierung | SQLCipher · Panic Wipe · PIN/Biometrie (geplant) |

Vollständige Dokumentation: [docs/SECURITY.md](docs/SECURITY.md)

---

## Sicherheitshinweis

PhantomChat ist ein Forschungs- und Portfolio-Projekt von **DC INFOSEC**. Der Krypto-Code ist nicht extern auditiert. Vor einem produktiven Einsatz in Hochrisiko-Szenarien ist ein unabhängiger kryptografischer Audit erforderlich.

Sicherheitslücken direkt an DC INFOSEC melden — nicht öffentlich.

---

*PhantomChat. Nicht nur verschlüsselt. Unsichtbar.*

---

© 2026 **DC INFOSEC** · [github.com/N0L3X](https://github.com/N0L3X)
