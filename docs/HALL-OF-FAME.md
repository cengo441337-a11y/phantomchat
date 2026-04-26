# PhantomChat Hall of Fame

Anerkennung für Forschende, die PhantomChat sicherer gemacht haben.

Eingetragen wird, wer eine Schwachstelle nach unserer
[Disclosure-Policy](SECURITY.md#responsible-disclosure) gemeldet hat,
in der Frist mit dem Fix-Prozess kooperiert hat, und namentliche
Erwähnung wünscht.

Anonyme Meldungen werden ohne Eintrag akzeptiert — keine Verpflichtung.

---

## Schweregrad-Kategorien

Wir folgen [CVSS 3.1](https://www.first.org/cvss/v3.1/specification-document)
für den Numerical Score und mappen auf vier Stufen:

| Stufe | CVSS-Score | Beispiel |
|-------|------------|----------|
| **Critical** | 9.0 – 10.0 | Krypto-Bypass, Schlüsselextraktion, Auto-Updater-Hijack, RCE im Tauri-Backend |
| **High** | 7.0 – 8.9 | Sealed-Sender-De-Anonymisierung, Sandbox-Escape, persistente XSS mit Plaintext-Leak |
| **Medium** | 4.0 – 6.9 | Metadaten-Leak gegen lokales Netz, Stealth-Tag-Korrelation, IDOR mit limitierter Auswirkung |
| **Low** | 0.1 – 3.9 | Logging-Disclosure, fehlerhafte Cache-Header, Edge-Case-Crashes ohne Datenleck |

Forschende werden in der Tabelle unter ihrem gewünschten Anzeige-Namen
geführt — Klarname, Pseudonym oder Handle, je nach Wunsch. Optional mit
Profil-Link (Twitter/X, GitHub, eigene Webseite).

---

## Tabelle

_Stand: 2026-04-26 — Wave 9 transparency-bundle. Noch keine Einträge._

| Datum | Forscher:in | Profil | Schweregrad | Bereich | CVE / Advisory |
|-------|-------------|--------|-------------|---------|-----------------|
| – | – | – | – | – | – |

<sub>Datum = Tag der Veröffentlichung des Fixes (nicht der Meldung). Meldungs- und
Triage-Daten werden vor der Veröffentlichung nicht offengelegt.</sub>

---

## Wie ein Eintrag dazukommt

1. Schwachstelle an `admin@dc-infosec.de` melden — PGP-verschlüsselt mit dem
   Key in [`keys/security.asc`](../keys/security.asc).
2. Mit unserem Fix-Prozess innerhalb der SLA-Fristen kooperieren.
3. Nach Patch-Release: dem Eintrag in der Tabelle oben zustimmen — wir fragen
   nochmal explizit nach Anzeige-Name + Profil-Link, bevor irgendetwas
   öffentlich wird.

Wenn keine Einigung über die Form der Anerkennung zustandekommt, bleibt der
Eintrag aus, der Fix landet trotzdem.

---

## Zukünftiges bezahltes Bounty

Sobald PhantomChat extern auditiert ist (Status: noch nicht gestartet — siehe
[SECURITY.md → Offene Punkte](SECURITY.md#offene-punkte)) starten wir ein
bezahltes Programm. Bis dahin ist die einzige Vergütung der Eintrag hier.
