# SECURITY.md

## Bedrohungsmodell

PhantomChat geht von einem starken Angreifer aus:

| Angreifer | Fähigkeiten |
|-----------|-------------|
| Passiver Global Observer | Sieht alle Verbindungsmetadaten (Timing, Größe, IPs) netzwerkweit |
| Bösartige Relays | Manipulation, selektives Speichern, Metadaten-Leakage |
| Aktiver MITM | Verbindungsabfang, Replay-Angriffe, Schlüsselkompromittierung |
| Gerätekompromittierung | Physischer Zugriff, Malware — Schlüssel könnten ausgelesen werden |

---

## Abwehrmaßnahmen

### Ende-zu-Ende-Verschlüsselung

Nachrichten werden mit **Double-Ratchet** (Signal-Protokoll-Prinzip)
verschlüsselt. Pro Nachricht werden neue Schlüssel abgeleitet — frühere
Nachrichten sind aus späteren Schlüsseln nicht rekonstruierbar.

**Envelope-Krypto-Stack:**
```
XChaCha20-Poly1305  (AEAD Payload-Verschlüsselung)
HKDF-SHA256         (Schlüsselableitung aus ECDH)
X25519              (Ephemeral Diffie-Hellman)
HMAC-SHA256         (Stealth-Tag für Empfänger-Identifikation)
```

### Dandelion++ (IP-Ursprung-Anonymität)

Bevor ein Envelope per GossipSub gebroadcastet wird, läuft es durch
eine **Stem-Phase**: Weitergabe an genau einen zufällig gewählten Peer.
Erst nach dem stochastischen Übergang (p=0,1 pro Hop) folgt der Broadcast.
Der Stem-Peer rotiert alle 10 Minuten.

Gegen einen Netzwerk-Beobachter: der sichtbare Broadcaster ist mehrere
Hops vom wahren Sender entfernt.

### Cover Traffic

Periodic dummy envelopes — CSPRNG-befüllte Nachrichten, die auf dem Wire
nicht von echten Envelopes zu unterscheiden sind — maskieren reale
Traffic-Muster gegen Timing-Korrelationsangriffe.

- **Daily Use:** 30–180 s Zufallsintervall
- **Maximum Stealth:** 5–15 s Zufallsintervall

### Maximum Stealth Mode (gegen globale passive Angreifer)

Bei aktiviertem Stealth-Modus:
- libp2p vollständig deaktiviert (kein direktes Peer-Exposure)
- Alle Nostr-WebSocket-Verbindungen über SOCKS5 (Tor oder Nym)
- Das Relay sieht die Exit-IP des Anonymisierungsnetzes, nicht die App-IP
- Aggressiver Cover Traffic

Schützt gegen Traffic-Korrelation durch globale Beobachter.

### Stealth-Tags (Empfänger-Anonymität)

Relays und Netzwerk-Beobachter können den Empfänger eines Envelopes
nicht bestimmen. Tags sind 32-Byte-HMAC-Werte die nur mit dem privaten
Spend-Key reproduzierbar sind.

### Hashcash Proof-of-Work

Jedes Envelope enthält einen PoW-Nonce. Sendern wird ein minimaler
Rechenaufwand auferlegt — Sybil- und Spam-Angriffe werden teurer.

### Lokale Verschlüsselung

SQLCipher (AES-256-CBC) sichert Nachrichten und Schlüssel auf dem Gerät.
Kein Schlüsselmaterial im Klartext.

---

## Offene Punkte

| Feature | Status |
|---------|--------|
| Post-Quanten-Hybrid (ML-KEM/Kyber) | Dependency vorhanden (`pqcrypto-mlkem`), nicht vollständig integriert |
| Double-Ratchet vollständig | Envelope-Layer fertig, Ratchet-State-Verwaltung im Aufbau |
| Gerätekompromittierung | App-Lock mit PIN/Biometrie geplant, Panic-Wipe implementiert |
| Extern auditierter Krypto-Code | Erforderlich vor Produktion |

---

## Responsible Disclosure

Sicherheitslücken bitte direkt an DC INFOSEC melden — nicht öffentlich.
