# PRIVACY.md

PhantomChat ist so konzipiert, dass möglichst wenige Metadaten preisgegeben
werden. Dieser Leitfaden erläutert die Datenschutzprinzipien und beschreibt
das **modulare Privacy-System** der aktuellen Implementierung.

---

## Privacy Modes

PhantomChat bietet zwei operative Modi — wählbar per App-Toggle oder CLI:

### Daily Use (Standard)

```
libp2p ──► Dandelion++ ──► GossipSub Mesh
                │
Nostr Relays ──► TLS WebSocket
                │
Cover Traffic ──► 30–180 s Zufallsintervall
```

- IP gegenüber direkten Peers verschleiert durch Dandelion++-Routing
- Nostr-Relays sehen ausgehende TLS-Verbindungen, aber keinen Klartext
- Leichter Cover Traffic verhindert einfache Timing-Korrelation
- Niedriger Akkuverbrauch, geringe Latenz

### Maximum Stealth

```
App ──► SOCKS5 (Tor | Nym) ──► Nostr Relay ──► WebSocket
         ▲
         libp2p: DEAKTIVIERT
         Cover Traffic: 5–15 s (aggressiv)
```

- libp2p vollständig abgeschaltet — kein direktes Peer-Exposure
- Alle Relay-Verbindungen exklusiv über SOCKS5-Proxy (Tor/Nym)
- Das Relay sieht ausschließlich die Exit-IP des Anonymisierungsnetzes
- Schützt gegen globale passive Angreifer (Traffic-Korrelation, NSA-Level)
- Nutzer akzeptiert bewusst höheren Akkuverbrauch und Latenz

---

## Keine zentralen Identitäten

- Keine Benutzernamen, keine Telefonnummern, keine Account-Registrierung
- Kontakte werden ausschließlich über View- und Spend-Schlüssel (X25519) identifiziert
- Pairing erfolgt Out-of-Band (QR-Code) — kein Verzeichnisserver

## Stealth-Tags (Empfänger-Anonymität)

Jedes Envelope enthält ein **HMAC-SHA256-Tag** das mit einem ephemeren
ECDH-Shared-Secret berechnet wird:

```
EPK ──► ECDH(recipient_spend_pub) ──► HKDF ──► tag_key
tag = HMAC-SHA256(tag_key, msg_id)
```

Relays sehen nur zufällige 32-Byte-Tags — kein Empfänger ableitbar.
Nur der Inhaber des passenden Spend-Keys kann das Tag reproduzieren.

## Dandelion++ (Sender-Anonymität im P2P-Netz)

```
Origin ──► [Stem: 1 Peer] ──► [Stem: 1 Peer] ──► [FLUFF: Broadcast]
                                       ↑
                              10 % Übergangswahrscheinlichkeit pro Hop
                              Stem-Peer rotiert alle 10 Minuten
```

Ein Beobachter sieht nur den Fluff-Phase-Broadcaster, der mehrere Hops
vom wahren Ursprung entfernt ist.

## Cover Traffic

Periodische Dummy-Envelopes aus CSPRNG-Daten sind auf dem Wire
kryptografisch ununterscheidbar von echten Envelopes. Empfänger verwerfen
sie beim HMAC-Scan stillschweigend. Dummies maskieren reale Traffic-Muster
gegen Timing-Analysen.

## Keine Telemetrie (per Default)

PhantomChat sammelt by default keinerlei Diagnosedaten, Nutzungsstatistiken
oder Telemetrie. Alle Logs verbleiben lokal.

**Wave 8J — opt-in Crash-Reporting:** Über *Settings → Diagnose* lässt sich
ein anonymer Crash-Report-Upload aktivieren (Stack-Trace + Build-ID, keine
Nachrichteninhalte, keine Kontaktdaten). Der Upload-Endpunkt ist konfigurierbar
und steht standardmäßig auf `updates.dc-infosec.de`; selbsthostende
Organisationen zeigen ihn auf ihren eigenen Collector
(siehe [`RELAY-SELFHOSTING.md`](RELAY-SELFHOSTING.md)).

## Lokale Datenspeicherung

- **Desktop (Wave 8H):** Identitäts- und Signing-Keys liegen im **OS-Keystore**
  (DPAPI / Keychain / libsecret), nicht mehr in `keys.json`. Memory-Zeroing auf
  allen privaten Schlüsseltypen plus anti-forensisches Pre-Delete-Overwrite
  bei "Wipe All Data".
- **Desktop-Backups (Wave 8C):** Backup-Archiv mit Argon2id-abgeleitetem Schlüssel +
  XChaCha20-Poly1305. Passphrase trägt der Nutzer; ohne sie ist das Archiv
  kryptografisch nutzlos.
- **Mobile:** PBKDF2 (600k iters) für PIN-abgeleiteten DB-Key + Biometrie-Quick-
  Unlock + Panic-Wipe nach 10 Fehlversuchen.
- **CLI / Legacy Mobile:** SQLCipher (AES-256-CBC) für die ältere
  Mobile-Storage-Schicht (Migrationspfad existiert).

Kein Schlüsselmaterial liegt im Klartext auf dem Gerät.

---

## Wave 11 — KI-Bridge / Voice / Watchers / On-device STT

Wave 11 fügt PhantomChat eine optionale **AI-Bridge** hinzu, die ein lokales oder
cloud-gehostetes LLM als virtuellen Kontakt einbindet. Die Privacy-Eigenschaften
sind je nach Provider-Wahl unterschiedlich — die Architektur ist transparent:

### Datenfluss zum LLM

```
Allow-listed Kontakt sendet Klartext
        │
        ▼
Desktop entschlüsselt (Sealed-Sender + Double-Ratchet)
        │
        ▼
ai_bridge::complete(provider) — Klartext oder Transkript
        │
        ├── ClaudeCli   ──► Subprozess `claude` lokal — bleibt im Prozess /
        │                    OAuth-Tokens in `~/.claude/`
        ├── Ollama      ──► HTTP `localhost:11434` — verlässt das Gerät NICHT
        ├── ClaudeApi   ──► HTTPS `api.anthropic.com` — Anthropic sieht Klartext
        └── OpenAiCompat──► HTTPS Provider-Endpoint — Provider sieht Klartext
```

**Einordnung:** mit `ClaudeCli` oder `Ollama` verlässt der Klartext das Heim-Gerät
nicht. Mit `ClaudeApi` / `OpenAiCompat` bekommt der Provider die Botschaft im
Klartext zu sehen — gleiche Vertrauensbasis wie Direkt-Kontakt mit dem Provider.
Vollständiges Threat-Model: [`docs/AI-BRIDGE.md`](AI-BRIDGE.md) § Security model.

### Voice-Messages (Wave 11B)

Eingehende Sprachnachrichten landen unter
`<app_data>/voice/<msg_id>.<ext>` als die ursprünglich gesendeten Bytes (Opus
oder AAC). Sie werden **nie wieder hochgeladen** — die Datei ist read-only nach
Empfang, der Player lädt sie ausschließlich lokal.

### On-device STT — whisper.cpp (Wave 11D)

Wenn STT aktiviert ist, läuft folgende Pipeline **vollständig in-process** auf
dem Desktop:

```
voice/<msg_id>.<ext>
   ▼
audio decode (symphonia + ffi-frei)
   ▼
16 kHz f32 PCM resample (in RAM, nie auf Disk)
   ▼
whisper.cpp inference (lokales Modell aus <app_data>/whisper/)
   ▼
Transkript (Text)
   ▼
ai_bridge::complete(...) wie eine getippte Nachricht
```

Die Audio-Bytes verlassen das Gerät nicht — selbst mit einem Cloud-LLM
konfiguriert sieht der Provider nur den **Text**, nie das Audio. Whisper-Modelle
werden einmalig von Hugging Face heruntergeladen und liegen lokal vor.

### Watcher / Shell-Exec (Wave 11E)

Watchers sind Cron-/Intervall-getriggerte Shell-Kommandos, die unter dem
**Bridge-Prozess-User** laufen und ihre Stdout an einen vordefinierten
allow-listed-Kontakt schicken.

- Stdout wird auf 8000 Zeichen gekürzt, bevor sie versendet wird.
- Jeder Watcher-Lifecycle-Event (`watcher_added` / `watcher_updated` /
  `watcher_removed` / `watcher_fired` / `watcher_failed`) landet im
  `audit.log` unter Kategorie `ai_bridge` — Compliance-Audit-fähig.
- Per-Watcher-Concurrency-Lock (3.0.2) verhindert, dass eine zweite Instanz
  startet, während die erste noch läuft.
- 5 min wall-clock-Timeout pro Befehl gegen runaway-Prozesse.

Wer den Watcher konfigurieren kann, kann beliebigen Code als Bridge-User
ausführen — selbe Vertrauensstufe wie ein Shell-Login. Daher: nur
allow-listed Targets, Audit-Log lesen, keine fremden Configs importieren.

---

## Hinweis zur Implementierung

Diese Referenzimplementierung ist ein Forschungs- und Portfolio-Projekt.
Vor einem produktiven Einsatz sind externe kryptografische Audits
erforderlich.
