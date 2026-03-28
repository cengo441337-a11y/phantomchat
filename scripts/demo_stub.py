#!/usr/bin/env python3
"""
demo_stub.py – Ein einfaches Testharness für PhantomChat

Dieses Skript implementiert einen stark vereinfachten Nachrichtenfluss
zwischen zwei Clients (Alice und Bob) mit lokalen Schlüsselpaaren,
Stealth‑Tags und Proof‑of‑Work.  Es dient ausschließlich der
Veranschaulichung des Protokolls.  Es werden **keine** sicheren
Kryptoprimitiven verwendet.  Die in der Spezifikation geforderten
Algorithmen (X25519, HKDF, XChaCha20‑Poly1305) werden hier durch
platzhalterhafte Hash‑ und XOR‑Operationen ersetzt.  Verwenden Sie
dieses Skript nicht für produktive Zwecke!

Funktionsweise:

1. Alice und Bob erzeugen je ein zufälliges View‑ und Spend‑Keypair (32 Byte).
2. Alice erstellt eine Nachricht an Bob, berechnet einen gemeinsamen
   Schlüssel (Simulation von ECDH per SHA‑256 über secret||pub), leitet
   `enc_key` und `tag_key` mittels HKDF ab und verschlüsselt die
   Nutzlast per XOR.
3. Alice berechnet ein HMAC‑Tag über die Nachricht‑ID und führt eine
   einfache Hashcash‑Proof‑of‑Work aus.
4. Das erzeugte Envelope wird an eine In‑Memory‑Relay weitergegeben.
5. Bob scannt die eingehenden Envelopes, prüft die Tags und entschlüsselt
   die Nachricht.

Zum Ausführen:

```sh
python3 demo_stub.py
```
"""

import os
import json
import time
import hmac
import hashlib
import base64
from typing import List, Callable


def random_bytes(n: int) -> bytes:
    return os.urandom(n)


def hkdf_sha256(ikm: bytes, info: bytes, length: int = 64) -> bytes:
    """Sehr vereinfachte HKDF‑Implementierung (ohne Salt)."""
    prk = hmac.new(b"\x00" * 32, ikm, hashlib.sha256).digest()
    okm = b""
    prev = b""
    counter = 1
    while len(okm) < length:
        prev = hmac.new(prk, prev + info + bytes([counter]), hashlib.sha256).digest()
        okm += prev
        counter += 1
    return okm[:length]


def xor_encrypt(key: bytes, data: bytes) -> bytes:
    return bytes([b ^ key[i % len(key)] for i, b in enumerate(data)])


def count_leading_zero_bits(data: bytes) -> int:
    count = 0
    for byte in data:
        if byte == 0:
            count += 8
        else:
            count += bin(byte)[2:].zfill(8).find("1")
            break
    return count


class Envelope:
    def __init__(self, epk: bytes, tag: bytes, pow_nonce: int,
                 nonce: bytes, ciphertext: bytes, mac: bytes, ts: int, ttl: int, ver: int = 1):
        self.ver = ver
        self.ts = ts
        self.ttl = ttl
        self.epk = epk
        self.tag = tag
        self.pow_nonce = pow_nonce
        self.nonce = nonce
        self.ciphertext = ciphertext
        self.mac = mac

    def serialize(self) -> bytes:
        parts = [
            self.ver.to_bytes(1, 'little'),
            self.ts.to_bytes(8, 'little'),
            self.ttl.to_bytes(4, 'little'),
            self.epk,
            len(self.tag).to_bytes(4, 'little'), self.tag,
            self.pow_nonce.to_bytes(8, 'little'),
            self.nonce,
            len(self.ciphertext).to_bytes(4, 'little'), self.ciphertext,
            self.mac,
        ]
        return b"".join(parts)


class InMemoryRelay:
    def __init__(self):
        self.queue: List[Envelope] = []
        self.subscribers: List[Callable[[Envelope], None]] = []

    def publish(self, env: Envelope):
        self.queue.append(env)
        for cb in self.subscribers:
            cb(env)

    def subscribe(self, cb: Callable[[Envelope], None]):
        self.subscribers.append(cb)


def simulate_ecdh(secret: bytes, peer_pub: bytes) -> bytes:
    """Simuliert ECDH durch SHA‑256(secret || peer_pub)."""
    return hashlib.sha256(secret + peer_pub).digest()


def generate_pow(header: bytes, difficulty_bits: int) -> int:
    nonce = 0
    while True:
        h = hashlib.sha256(header + nonce.to_bytes(8, 'little')).digest()
        if count_leading_zero_bits(h) >= difficulty_bits:
            return nonce
        nonce += 1


def main():
    # Schlüsselgenerierung
    alice_view_priv = random_bytes(32)
    alice_view_pub = random_bytes(32)
    alice_spend_priv = random_bytes(32)
    alice_spend_pub = random_bytes(32)

    bob_view_priv = random_bytes(32)
    bob_view_pub = random_bytes(32)
    bob_spend_priv = random_bytes(32)
    bob_spend_pub = random_bytes(32)

    # Relay
    relay = InMemoryRelay()

    # Bob abonniert Relay
    def bob_handler(env: Envelope):
        # Reconstruct shared secret using Bob's spend_priv and env.epk
        K = simulate_ecdh(bob_spend_priv, env.epk)
        okm = hkdf_sha256(K, b"pc.enc|pc.tag")
        enc_key = okm[:32]
        tag_key = okm[32:64]
        # Prüfe Tag
        # msg_id muss aus Nutzlast gelesen werden; hier nicht möglich
        # -> wir entschlüsseln direkt
        pt = xor_encrypt(enc_key, env.ciphertext)
        # Deserialize payload
        msg_id = int.from_bytes(pt[0:16], 'little')
        sender_fp = int.from_bytes(pt[16:20], 'little')
        rh_len = int.from_bytes(pt[20:24], 'little')
        idx = 24
        ratchet_header = pt[idx:idx+rh_len]
        idx += rh_len
        body_len = int.from_bytes(pt[idx:idx+4], 'little')
        idx += 4
        body = pt[idx:idx+body_len]
        print(f"Bob hat Nachricht empfangen: {body.decode()} (msg_id={msg_id})")

    relay.subscribe(bob_handler)

    # Alice sendet Nachricht an Bob
    msg_id = int.from_bytes(random_bytes(16), 'little')
    sender_fp = 0
    ratchet_header = b""
    body = b"Hallo Bob!"  # Nachricht
    # ECDH
    K = simulate_ecdh(alice_spend_priv, bob_spend_pub)
    okm = hkdf_sha256(K, b"pc.enc|pc.tag")
    enc_key = okm[:32]
    tag_key = okm[32:64]
    # Tag
    tag = hmac.new(tag_key, msg_id.to_bytes(16, 'little'), hashlib.sha256).digest()
    # Payload
    payload = (
        msg_id.to_bytes(16, 'little') +
        sender_fp.to_bytes(4, 'little') +
        len(ratchet_header).to_bytes(4, 'little') +
        ratchet_header +
        len(body).to_bytes(4, 'little') +
        body
    )
    # Encryption (XOR)
    nonce = random_bytes(24)
    ciphertext = xor_encrypt(enc_key, payload)
    mac = b""  # kein MAC in dieser Demo
    # Header für PoW (ver, ts, ttl, epk, tag)
    ver = (1).to_bytes(1, 'little')
    ts = int(time.time() * 1000).to_bytes(8, 'little')
    ttl = (60).to_bytes(4, 'little')
    epk = random_bytes(32)
    header = ver + ts + ttl + epk + tag
    pow_nonce = generate_pow(header, difficulty_bits=12)
    env = Envelope(epk, tag, pow_nonce, nonce, ciphertext, mac, int(time.time()*1000), 60)
    relay.publish(env)
    # Warten damit Bob die Nachricht empfängt
    time.sleep(1)


if __name__ == '__main__':
    main()