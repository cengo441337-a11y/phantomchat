# PRIVACY.md

PhantomChat ist so konzipiert, dass möglichst wenige Metadaten preisgegeben
werden.  Dieser Leitfaden erläutert die Datenschutzprinzipien des
Projekts.

## Keine zentralen Identitäten

* Es gibt keine globalen Benutzernamen oder Telefonnummern.  Kontakte
  werden ausschließlich über View‑ und Spend‑Schlüssel identifiziert.
* Beim Pairing werden lediglich öffentliche Schlüssel und ein
  Fingerprint ausgetauscht; es erfolgt keine Synchronisation über einen
  zentralen Server.

## Keine Telemetrie

* PhantomChat sammelt keinerlei Diagnosedaten, Nutzungsstatistiken oder
  Telemetrie.  Alle Logs verbleiben lokal auf dem Gerät und können vom
  Benutzer gelöscht werden.
* Es existiert keine Kontakt‑Upload‑Funktion; Kontakte werden lokal
  verwaltet und nicht an den Entwickler übertragen.

## Minimale Relaysichtbarkeit

* Durch die Stealth‑Tags erkennen Relays nicht, wer der Empfänger einer
  Nachricht ist.  Tags sind zufällige HMAC‑Werte, die nur vom
  Empfänger rekonstruiert werden können.
* Relays sehen lediglich, dass ein Envelope veröffentlicht wird, aber
  nicht, was dessen Inhalt ist (dank AEAD) und für wen es bestimmt ist.

## Lokale Datenspeicherung

* Schlüssel und Nachrichten werden lokal verschlüsselt gespeichert.  Auf
  Android erfolgt die Verschlüsselung mit Jetpack Security (AES‑GCM) und
  SQLCipher/Room.  Beim CLI‑Client wird ein passwortgeschützter
  SecretStore verwendet.
* Keine Schlüssel werden im Klartext auf der Festplatte abgelegt.

## Optionaler Mixnet‑Layer

* Für Nutzer mit erhöhten Anonymitätsanforderungen kann der Datenverkehr
  optional über ein Mixnet wie Nym oder das Tor‑Netzwerk geleitet
  werden.  Dadurch wird auch die Netzwerkmetadatenebene weiter
  verschleiert.

## Hinweis zur Implementierung

Die in diesem Repository enthaltene Referenzimplementierung ist
unvollständig und ersetzt nicht die Beratung durch erfahrene
Kryptograph*innen.  Alle kryptographischen Platzhalter müssen durch
geprüfte Bibliotheken ersetzt werden, bevor PhantomChat produktiv
eingesetzt wird.