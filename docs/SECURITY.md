# SECURITY.md

## Bedrohungsmodell

PhantomChat geht davon aus, dass ein Angreifer folgende Fähigkeiten besitzt:

* **Passiver Global Observer** – Ein globaler Netzbetreiber kann sämtliche
  eingehende und ausgehende Nachrichten einsehen (Timing, Größe,
  Verbindungsdaten) und versucht, Beziehungen abzuleiten.
* **Bösartige Relays** – Einzelne oder mehrere Relays versuchen, Nachrichten
  zu manipulieren, selektiv zu speichern oder Metadaten zu leaken.
* **Aktiver MITM** – Ein Angreifer kann Verbindungen abfangen, Relay‑
  Antworten manipulieren, Replay‑Angriffe durchführen oder versuchen,
  Schlüsselmaterial zu kompromittieren.
* **Gerätekompromittierung** – Der Endnutzer verliert die Kontrolle über
  sein Gerät (Malware, physischer Zugriff).  Schlüssel und Ratchet‑State
  könnten ausgelesen werden.

## Abwehrmaßnahmen

* **End‑zu‑End‑Verschlüsselung** – Alle Nachrichten werden mit
  Double‑Ratchet verschlüsselt.  Neue Schlüssel werden pro Nachricht
  abgeleitet, sodass frühere Nachrichten nicht aus späteren Schlüsseln
  berechnet werden können【96530739456497†L54-L65】.
* **Stealth‑Tags** – Empfängeridentitäten werden durch HMAC‑basierte Tags
  verschleiert.  Nur der Empfänger kann erkennen, ob ein Envelope für ihn
  bestimmt ist; Relays sehen nur zufällige Tags.
* **Mehrwege‑Transport** – Nachrichten werden parallel über mehrere
  Relays gesendet.  Eine Policy‑Engine bewertet die Health (Latenz,
  Fehlerrate) und wählt dynamisch die besten Relays aus.  Dies reduziert
  das Risiko von Relay‑Ausfällen und korreliertem Traffic.
* **Spam‑Schutz** – Hashcash sorgt dafür, dass das Versenden von
  Nachrichten einen minimalen Arbeitsaufwand erfordert.  Angreifer müssen
  für jede Nachricht Rechenzeit aufbringen, um einen gültigen Hash mit
  genügend führenden Nullbits zu finden【43054307062348†L142-L172】.
* **Lokale Verschlüsselung** – Schlüsselmaterial und Ratchet‑State werden
  lokal verschlüsselt gespeichert (Android Keystore mit AES‑GCM,
  Room/SQLCipher).  Beim CLI‑Client wird ein passwortgestützter
  SecretStore genutzt (Argon2id + libsodium sealed boxes), derzeit als
  Platzhalter hinterlegt.
* **Forward/Backward Secrecy** – Durch den Double‑Ratchet gehen beim
  kompromittierten Ratchet‑Key nur wenige Nachrichten verloren.  Die
  nächste Diffie‑Hellman‑Runde ersetzt den kompromittierten Schlüssel
  durch einen neuen【96530739456497†L139-L156】.

## Offene Punkte

* **Post‑Quanten‑Hybrid** – Optional kann der Key‑Exchange um einen Kyber
  Key Encapsulation Mechanism erweitert werden, um einen hybriden
  Pre‑Quanten‑Schutz zu realisieren.  Dies ist nicht im MVP enthalten.
* **Mixnet‑Integration** – Der Einsatz von Nym oder Tor als Transport
  über einen Mixnet‑Layer erfordert weitere Anpassungen (z.&nbsp;B.
  Circuit‑Management, Cover‑Traffic).  Es handelt sich um einen
  „Hard‑Mode“‑Schalter.
* **Gerätekompromittierung** – PhantomChat kann den Verlust eines
  kompromittierten Geräts nicht verhindern.  Anwender sollten ihre
  Geräte mit PIN/Biometrie schützen und regelmäßige
  Betriebssystem‑Updates durchführen.  Eine „App‑Lock“‑Funktion mit
  zusätzlichem Passwort ist vorgesehen.