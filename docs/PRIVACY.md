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

## Keine Telemetrie

PhantomChat sammelt keinerlei Diagnosedaten, Nutzungsstatistiken oder
Telemetrie. Alle Logs verbleiben lokal.

## Lokale Datenspeicherung

Schlüssel und Nachrichten werden mit SQLCipher (AES-256) verschlüsselt
gespeichert. Kein Schlüsselmaterial liegt im Klartext auf dem Gerät.

## Hinweis zur Implementierung

Diese Referenzimplementierung ist ein Forschungs- und Portfolio-Projekt.
Vor einem produktiven Einsatz sind externe kryptografische Audits
erforderlich.
