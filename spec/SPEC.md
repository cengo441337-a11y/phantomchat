# PhantomChat – Protokollspezifikation (MVP)

Diese Spezifikation beschreibt das Minimal‑Viable‑Product des
dezentralen Messengers **PhantomChat**.  Sie fasst die
Nachrichtenformate, die kryptographischen Bausteine, die Zustandsmaschinen
und den Ablauf des Protokolls zusammen.  Dabei werden bewährte
Kryptoprimitiven wie X25519 für Diffie‑Hellman, HKDF für die
Schlüsselableitung sowie XChaCha20‑Poly1305 als AEAD‑Cipher verwendet.
**Achtung:** Die aktuelle Referenzimplementierung enthält Platzhalter für
die Kryptographie; die hier beschriebenen Algorithmen müssen vor dem
Produktivbetrieb korrekt implementiert oder durch geprüfte Bibliotheken
ersetzt werden.

## 1. Überblick

PhantomChat ist ein asynchroner Messenger, der Nachrichten in
relayanonymisierter Form austauscht.  Jeder Teilnehmer besitzt ein
dreiteiliges Schlüsselpaar: eine *Identity‑Key* (für die Authentisierung der
App), einen *View‑Key* (zur Erkennung eingehender Nachrichten) und einen
*Spend‑Key* (zur Entschlüsselung der Inhalte).  Die Kommunikation baut auf
dem **Double‑Ratchet‑Algorithmus** auf, der für jede Nachricht neue
Sitzungsschlüssel ableitet und so Vorwärts‑ und Rückwärtssicherheit
gewährleistet.  Laut der Signal‑Spezifikation leiten zwei Parteien für
jede Nachricht neue Schlüssel ab; Diffie‑Hellman‑Outputs werden in die
Ableitungen eingemischt, sodass frühere Schlüssel nicht aus späteren
Schlüsseln berechnet werden können【96530739456497†L54-L65】.  Die
Ratchet‑Schlüssel werden in drei KDF‑Ketten verwaltet (Root‑, Sende‑ und
Empfangskette)【96530739456497†L103-L114】.

Zum Versand werden Nachrichten in ein **Envelope**‑Format verpackt.  Das
Envelope enthält nur einen minimalen Satz an Metadaten (Versionsfeld,
Zeitstempel, TTL, Absender‑Ephemeral‑Key, einen HMAC‑basierten Tag zur
Adressierung, einen PoW‑Nonce, den AEAD‑Nonce und die verschlüsselte
Nutzlast).  Die Nutzlast enthält wiederum eine Nachricht‑ID, den
Sender‑Fingerprint, die Ratchet‑Header und den Klartext.  Zur
Relay‑Distribution wird das Envelope als Content eines Nostr‑Events
kodiert (Event‑Kind 4 – verschlüsselte Nachrichten), wobei die Relay‑URL
und andere Felder in den Nostr‑Tags notiert werden.

## 2. Kryptoprimitiven

### 2.1 X25519 und Diffie‑Hellman

Für die Schlüsselaustausche wird die elliptische Kurve Curve25519
verwendet.  **X25519** bezeichnet die Diffie‑Hellman‑Funktion über
Curve25519: Zwei Parteien erzeugen je ein 32‑Byte‑Schlüsselpaar und
vermitteln sich die öffentlichen Schlüssel; daraus kann jeweils ein
gemeinsames Geheimnis abgeleitet werden【408905688727623†L69-L77】.  Das
gemeinsame Geheimnis sollte nicht direkt als Schlüssel genutzt werden,
sondern in eine **Key Derivation Function** (KDF) eingespeist werden, um
zusätzliche Kontextinformationen einzumischen【408905688727623†L75-L97】.  PhantomChat nutzt
hierfür HKDF (siehe unten).

### 2.2 HKDF (HMAC‑based Extract‑and‑Expand KDF)

HKDF ist eine standardisierte KDF nach RFC 5869, die das sogenannte
„extract‑then‑expand“‑Paradigma anwendet: Zunächst wird aus einem
gegebenen Schlüsselmaterial mittels HMAC ein pseudorandomer Schlüssel *K*
extrahiert, der anschließend in eine oder mehrere Unterschlüssel erweitert
wird【41338857693594†L105-L124】.  Dieser Ansatz sorgt dafür, dass auch bei
teilweise vorhersehbarem Input (z.&nbsp;B. Diffie‑Hellman‑Werte) sichere
Schlüssel entstehen.  In PhantomChat wird HKDF mit SHA‑256 eingesetzt.

### 2.3 XChaCha20‑Poly1305

Als Authenticated Encryption with Associated Data (AEAD) kommt
**XChaCha20‑Poly1305** zum Einsatz.  Gegenüber dem klassischen
ChaCha20‑Poly1305 erlaubt XChaCha20 einen erweiterten Nonce von 192 Bit,
wodurch die Wahrscheinlichkeit von Nonce‑Kollisionen bei lang laufenden
Verbindungen deutlich sinkt【788252068004131†L53-L63】.  Bei XChaCha20 wird der
Schlüssel zusammen mit einem Teil des Nonce über die Funktion HChaCha20 zu
einem Subkey verarbeitet; dieser Subkey dient dann als Schlüssel für
ChaCha20, wobei der verbleibende Nonce‑Anteil für die Generierung des
Keystreams genutzt wird【788252068004131†L60-L63】.  Die AEAD‑Konstruktion
verkettet den ChaCha‑Keystream mit dem Poly1305‑MAC, sodass neben der
Vertraulichkeit auch die Integrität der Nachricht gewährleistet wird.

### 2.4 Hashcash

Um Spam zu vermeiden, müssen Sender einen Proof‑of‑Work berechnen.  Das
verwendete Verfahren lehnt sich an **Hashcash** an: Der Sender findet
eine Nonce, sodass die Hashfunktion über ausgewählte Headerfelder (Version,
Zeitstempel, Ephemeral‑Key, Tag) plus Nonce eine vorgegebene Anzahl führender
Nullbits ergibt.  Hashcash ist ein kryptographisches
Proof‑of‑Work‑System, bei dem der Sender wiederholt Zufallswerte
ausprobiert, bis der Hash mit einer bestimmten Anzahl von Nullbits
beginnt【43054307062348†L142-L172】.  Der Empfänger und die Relays können die
Gültigkeit dieser Arbeit effizient verifizieren.

## 3. Nachrichtenformat

### 3.1 Envelope

Alle an Relays gesendeten Nachrichten werden in ein binäres Envelope
verpackt.  Das Format ist wie folgt (alle Felder sind in
Little‑Endian‑Bytefolge kodiert):

| Feld       | Typ     | Beschreibung |
|-----------|--------|--------------|
| `ver`     | `u8`    | Protokollversion (derzeit 1) |
| `ts`      | `u64`   | UNIX‑Zeitstempel in Millisekunden |
| `ttl`     | `u32`   | Gültigkeitsdauer in Sekunden; nach Ablauf kann das Relay löschen |
| `epk`     | `[32]`  | Ephemerer öffentlicher X25519‑Schlüssel des Senders |
| `tag`     | `[16]` oder `[32]` | HMAC‑basiertes Tag zur Empfängeridentifikation |
| `pow_nonce` | `u64` | Nonce für das Proof‑of‑Work |
| `nonce`   | `[24]`  | Nonce für XChaCha20 |
| `ciphertext` | `[..]` | Verschlüsselte Nutzlast (AEAD‑Ciphertext) |
| `mac`     | `[16]`  | Authentikationscode von XChaCha20‑Poly1305 |

Die Feldlängen orientieren sich an den Spezifikationen von X25519
(32 Bytes), XChaCha20 (24 Byte Nonce) und Poly1305 (16 Byte Tag).  Die
Kryptographie in der Referenzimplementierung ist als Platzhalter
realisiert; im Produktionscode muss die AEAD‑Verschlüsselung mit den
abgeleiteten `enc_key` erfolgen.

### 3.2 Payload (Klartext)

Die AEAD‑klar verschlüsselte Nutzlast besteht aus folgenden Elementen:

| Feld            | Typ        | Beschreibung |
|----------------|-----------|--------------|
| `msg_id`       | `u128`     | Zufällige Nachricht‑ID zur Deduplizierung |
| `sender_fp`    | `u32`      | Fingerprint des Senders (z.&nbsp;B. SAS‑Words) |
| `ratchet_header` | variable  | Header der Double‑Ratchet, enthält z.&nbsp;B. den aktuellen Ratchet‑Public‑Key, Kettenpositionen usw. |
| `body`         | variable   | Anwendungspayload (Textnachricht) |

Die Ratchet‑Header dienen zum Synchronisieren der KDF‑Ketten.  Der
`sender_fp` wird bei der Pairing‑Prozedur erzeugt und lässt sich vom
Empfänger zur Verifikation des Schlüsseltauschs verwenden.

### 3.3 Tag‑Generierung

Damit ein Empfänger seine Nachrichten zwischen allen publizierten
Events erkennen kann, wird ein Stealth‑Tag berechnet.  Der Sender
ermittelt zunächst einen gemeinsamen Schlüssel `K = ECDH(epk, spend_pub)`
mittels X25519.  Daraus werden via HKDF ein Verschlüsselungsschlüssel
`enc_key` und ein Tag‑Schlüssel `tag_key` abgeleitet.  Anschließend wird
über die Nachricht‑ID eine HMAC gebildet: `tag = HMAC(tag_key, msg_id)`.
Da der `tag_key` aus dem spend‑Key des Empfängers abgeleitet wird, kann
nur dieser den HMAC rekonstruieren und somit erkennen, ob ein Envelope
für ihn bestimmt ist.  Drittparteien sehen nur einen zufälligen Wert.

## 4. Protokollablauf

### 4.1 Pairing und Schlüsselaustausch

Zwei Geräte führen zunächst einen Pairing‑Vorgang durch.  Dabei
übertragen sie sich gegenseitig ihre öffentlichen View‑ und Spend‑Keys
(`view_pub`, `spend_pub`) sowie einen human‑readable Fingerprint (SAS‑Words).
Dies kann per QR‑Code erfolgen.  Anschließend werden die Ratchet‑Zustände
initialisiert und es wird ein gemeinsamer Ratchet‑Root‑Key mittels HKDF
aus ECDH(view_pub, spend_pub) abgeleitet.

### 4.2 Nachricht senden

1. Der Sender wählt einen neuen ephemeren Keypair `(epk, esk)`, berechnet
   den gemeinsamen Schlüssel `K = ECDH(esk, spend_pub_recv)` und leitet
   daraus `enc_key` und `tag_key` ab.
2. Aus dem `tag_key` und einer zufällig gewählten `msg_id` wird ein
   HMAC‑Tag gebildet.
3. Die Double‑Ratchet‑Engine erzeugt anhand des aktuellen
   Sende‑Zustands ein Ratchet‑Header und einen Message‑Key.
4. Die Klartext‑Payload wird serialisiert und mit XChaCha20‑Poly1305
   unter Verwendung des `enc_key` und eines zufälligen Nonce
   verschlüsselt.  Der AEAD‑Tag wird im Envelope gespeichert.
5. Ein Hashcash‑Nonce wird gesucht, sodass der SHA‑256‑Hash der Felder
   (`ver`, `ts`, `epk`, `tag`, `pow_nonce`) eine konfigurierbare Anzahl
   führender Nullbits besitzt.  Hashcash ist so aufgebaut, dass der Sender
   durch wiederholtes Ausprobieren nach einem gültigen Nonce sucht【43054307062348†L142-L172】.
6. Das Envelope wird serialisiert und an mehrere Relays parallel
   übermittelt.  Die Relay‑Schicht verwaltet Health‑Scores, führt
   Deduplizierung durch und implementiert Backoff‑Strategien.

### 4.3 Nachricht empfangen

1. Der Empfänger abonniert mindestens drei Relays und liest eingehende
   Envelopes.
2. Für jedes Envelope wird das HMAC‑Tag mithilfe des eigenen
   `spend_priv` neu berechnet.  Stimmt der Tag, wird das Envelope als
   eigen identifiziert; andernfalls wird es verworfen.
3. Die Double‑Ratchet‑Engine verarbeitet den Ratchet‑Header und leitet
   den passenden Message‑Key ab.  Die Nutzlast wird mit XChaCha20‑Poly1305
   entschlüsselt und verifiziert.  Durch den Double‑Ratchet werden für
   jede Nachricht neue Schlüssel generiert; Diffie‑Hellman‑Outputs
   fließen in den Root‑Key ein, wodurch spätere Schlüssel nicht aus
   früheren abgeleitet werden können【96530739456497†L54-L65】.
4. Nach erfolgreicher Verarbeitung sendet der Empfänger ein quittiertes
   ACK über die gleichen Relays.  Relays dürfen das Envelope nach
   erfolgreichem ACK und Ablauf der TTL löschen.

## 5. Zustandsmaschinen

### 5.1 Double‑Ratchet‑Zustände

Jeder Teilnehmer hält folgende Zustände:

* `root_key`: Schlüssel der Wurzelkette.
* `send_chain_key`, `recv_chain_key`: Ketten für laufende Nachrichten in
  jede Richtung.  Die Schlüssel werden fortlaufend mit HKDF aktualisiert
  (symmetric‑key‑ratchet)【96530739456497†L119-L129】.
* `ratchet_key`: Aktuelles DH‑Keypair für die nächste
  Diffie‑Hellman‑Runde【96530739456497†L139-L156】.
* `msg_keys_skipped`: Puffer für übersprungene Nachrichten.

Beim Empfang eines neuen Ratchet‑Public‑Keys des Gegenübers wird ein
DH‑Ratchet‑Schritt durchgeführt: es wird ein neues Ratchet‑Keypair
generiert und ein Diffie‑Hellman‑Output berechnet, der in den
`root_key` eingespeist wird.  Dadurch werden neue Sende‑ und
Empfangsketten erzeugt【96530739456497†L139-L156】.

### 5.2 Outbox/In‑Flight‑Zustände

Die lokale Datenbank führt eine Zustandsmaschine über gesendete
Nachrichten: `PENDING → PUBLISHED → DELIVERED → ACKED`.  Nicht
abgeschlossene Nachrichten werden erneut gesendet; nach Ablauf der TTL
werden sie verworfen.  Identische Envelopes werden dedupliziert.

## 6. Relays und Transport

PhantomChat nutzt Nostr‑kompatible Relays als Transportebene.  Laut
NIP‑01 besteht ein Nostr‑Event aus den Feldern `id`, `pubkey`,
`created_at`, `kind`, `tags`, `content` und `sig`【615806819960107†L28-L55】.  Das
`id`‑Feld ist der SHA‑256‑Hash der serialisierten Event‑Daten【615806819960107†L58-L70】.  Tags
dienen der flexiblen Indizierung; der erste Eintrag eines Tag‑Arrays
definiert den Schlüssel, der zweite den Wert【615806819960107†L124-L127】.

Im MVP wird für PhantomChat das Event‑Kind `30001` verwendet (Freie
Anwendungsereignisse im Bereich `30000 ≤ kind < 40000` werden von
Relays nicht gespeichert).  Die Nutzlast des Events (Feld
`content`) enthält das Base64‑kodierte Envelope.  Zusätzlich wird ein
`p`‑Tag mit dem Empfänger‑View‑Public‑Key gesetzt, damit Relays die
Event‑Auswahl optimieren können.  Clients abonnieren mehrere Relays mit
einem Filter `{"#p": [<view_pub_hex>]}` und empfangen so nur ihre
Nachrichten.

## 7. Anmerkungen

* Der hier vorgestellte Prototyp bildet die Architektur nach und
  illustriert den Ablauf, ersetzt aber **nicht** eine gründliche
  kryptographische Implementierung.
* Für produktionsreife Builds müssen geprüfte Bibliotheken (z.&nbsp;B.
  libsodium für X25519 und XChaCha20‑Poly1305) verwendet werden.  Die
  Ableitung der Schlüssel, die Ratchet‑Logik und die AEAD‑Verschlüsselung
  sollten ausgetauscht werden.
* Die Pairing‑Prozedur, die QR‑Übertragung und der Mixnet‑Layer sind
  skizziert, aber nicht implementiert.
