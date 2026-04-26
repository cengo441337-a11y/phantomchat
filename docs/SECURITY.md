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

- **Desktop (Wave 8H + 8C):** OS-Keystore (DPAPI / Keychain / libsecret) für
  Identitäts- und Signing-Keys. Plaintext-`keys.json` ist auf neuen Installationen
  Geschichte. Backups via Argon2id + XChaCha20-Poly1305 mit Passphrase
  (`docs/RELAY-SELFHOSTING.md` enthält das Restore-Verfahren).
- **Mobile:** PBKDF2 (600k iters) + Biometrie + Panic-Wipe nach 10 Fehlversuchen.
- **CLI / Legacy:** SQLCipher (AES-256-CBC) für die ältere Mobile-Storage-Schicht.

Kein Schlüsselmaterial im Klartext auf dem Gerät.

---

## Offene Punkte

| Feature | Status |
|---------|--------|
| PQXDH (ML-KEM-1024) Hybrid | **Geliefert in 2.3.0** — auditiert: offen |
| Double-Ratchet vollständig | **Geliefert in 2.4.0** |
| App-Lock (PIN / Biometrie) | **Geliefert** — Wave 8H Desktop + Mobile PIN-Flow |
| Externer Krypto-Audit | **Offen** — Wave 9 Transparency-Bundle (RFC 9116 + Hall of Fame) ist der no-budget-Ersatz, bis Audit-Budget steht |

---

## Responsible Disclosure

PhantomChat hat (noch) **kein bezahltes Bug-Bounty-Programm** — der Krypto-Stack
ist nicht extern auditiert und das Produkt ist Pre-Audit. Trotzdem nehmen wir
Schwachstellenmeldungen ernst und antworten zeitnah.

### Kontakt

| Kanal | Adresse |
|-------|---------|
| **E-Mail** (bevorzugt) | `admin@dc-infosec.de` |
| **PGP-Key** | [`keys/security.asc`](../keys/security.asc) |
| **Fingerprint** | `0F8D A258 1B8A 1428 9F0F  2FD7 EF08 6D82 9914 A0E3` |
| **Expiry** | 2028-04-25 (Ed25519 / Curve25519) |
| **security.txt** | [`/.well-known/security.txt`](../.well-known/security.txt) (RFC 9116) |

Bitte kein öffentliches GitHub-Issue für Sicherheitsmeldungen — `admin@dc-infosec.de`
mit verschlüsseltem Inhalt unter Verwendung des oben verlinkten PGP-Keys.

### Service-Level

| Phase | Frist |
|-------|-------|
| Empfangsbestätigung | ≤ 72 h |
| Triage + Schweregrad | ≤ 7 Tage |
| Fix-Plan + Eta | ≤ 14 Tage nach Triage |
| Koordinierte Veröffentlichung | nach Patch-Release; Standard-Embargo 90 Tage |

Wenn die Frist nicht eingehalten werden kann, schreiben wir vor Ablauf
mit Begründung + neuer Schätzung.

### Scope

**In Scope:**
- Krypto-Implementierungen in `core/` (Envelope, Ratchet, Sealed-Sender, MLS, Stealth-Tags)
- Wire-Format-Schwächen (`MLS-WLC2`, `MLS-APP1`, `FILE1:01`, `RCPT-1:`, `TYPN-1:`, `RACT-1:`, `REPL-1:`, `DISA-1:`, `VOICE-1:`)
- Tauri-Desktop-Backend (`desktop/src-tauri`) — IPC/command-handler-Bugs, sandbox-Escapes, file-system-Zugriffe
- Auto-Updater (`updates.dc-infosec.de`) — Signaturprüfung, Downgrade-Angriffe
- Cover-Traffic / Dandelion++ / Stealth-Mode — Statistical Disclosure / Timing-Korrelation
- Reproducible-Build-Pipeline — Supply-Chain (CI/CD, Release-Signaturen, Checksum-Veröffentlichung)
- Frontend (React + Tailwind in `desktop/src/`) — XSS, prototype-pollution, sensitive-data-in-DOM
- **AI-Bridge Angriffsfläche** (Wave 11) — Provider-Credential-Leakage, Watcher-Confused-Deputy, Allow-list-Bypass, whisper.cpp-Parser-Bugs (siehe [`docs/AI-BRIDGE.md`](AI-BRIDGE.md) § Security model)

**Out of Scope** (bekannt oder akzeptiert):
- DoS gegen Public-Relays (Hashcash mildert, eliminiert nicht)
- Social-Engineering, Phishing, physischer Zutritt
- Abhängigkeiten von Drittanbietern (`tauri`, `openmls`, `libp2p`, `whisper.cpp`) — bitte direkt beim Upstream melden, wir tracken aber CVE-Updates
- Allow-listed-Watcher-Befehle die als Bridge-Prozessuser ausgeführt werden — das ist by-design (siehe AI-BRIDGE.md § Watcher security model). Findings müssen einen Kontroll-Bypass über das Allow-list/Audit-Gate hinaus zeigen.

### Safe Harbor

Wir verpflichten uns, **keine rechtlichen Schritte** gegen Forschende zu unternehmen, die:

- in gutem Glauben Schwachstellen suchen und ausschließlich gegen eigene Test-Installationen oder die offizielle Test-Domain testen;
- keine Daten anderer Nutzer:innen abgreifen, exfiltrieren, modifizieren oder löschen;
- Schwachstellen vertraulich an `admin@dc-infosec.de` melden und uns vor öffentlicher Disclosure die SLA-Frist einräumen;
- nicht über Reproduktion + Validierung hinaus aktiv sind (kein Pivoting, kein Halten von Zugriff).

Diese Zusage gilt nach deutschem Recht im Rahmen von § 202c StGB / § 303a StGB. Forschende
außerhalb der EU bleiben für ihre lokale Rechtslage selbst verantwortlich.

### Anerkennung

Erfolgreiche, im Scope erbrachte Meldungen werden — sofern gewünscht — in der
[Hall of Fame](HALL-OF-FAME.md) namentlich aufgeführt. Bei kritischen Findings
sind Anerkennung im Release-Changelog + Co-Author-Eintrag im Fix-Commit Standard.

Ein bezahltes Bounty-Programm ist nach erfolgtem externen Audit geplant.
