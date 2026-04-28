//! Onion-routed mixnet.
//!
//! A lighter cousin of Sphinx/Nym: wraps a payload in N nested AEAD layers,
//! one per intermediate hop, so every relay sees only "previous hop → next
//! hop" and never the full route. The payload is already end-to-end
//! encrypted by the higher-level PhantomChat envelope stack; the mixnet
//! layer defeats network-level origin/destination correlation on top of
//! that.
//!
//! ## Packet structure
//!
//! ```text
//! wire = eph_pub_32 || AEAD(k_1, layer_1)
//!
//! layer_i  = 0x01 || next_hop_pub_32 || inner_len:u32 || inner_layer   ← forward
//!          | 0x00 || payload_len:u32  || final_payload                  ← terminal
//! ```
//!
//! A hop drops out of the path by decrypting the outer AEAD with its key
//! (derived from `HKDF(ECDH(own_secret, eph_pub), info = MIX_INFO)`),
//! reading the tag byte, and either forwarding `eph_pub || inner_layer`
//! to `next_hop_pub` or delivering `final_payload` locally.
//!
//! ## What this is not
//!
//! - **Not** Sphinx — no padding, no per-hop blinding factor, no replay
//!   detection. A real mixnet deployment needs all three. This file
//!   gives us a wire-stable first cut plus a test harness that proves
//!   three-hop round-trip works; hardening follows in a dedicated
//!   v2.5-series milestone.
//! - **Not** a routing protocol — pick the hops out-of-band (e.g. from a
//!   Nostr directory of PhantomChat mix nodes).

use chacha20poly1305::{
    aead::{Aead, KeyInit as AeadKeyInit, Payload as AeadPayload},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

/// HKDF info string for per-hop AEAD keys. Bumping this invalidates all
/// in-flight packets (which is fine — mixnet packets are ephemeral).
pub const MIX_INFO: &[u8] = b"PhantomChat-v1-MixnetHop";

/// Tag byte: this hop is the last. Payload follows.
const TAG_FINAL: u8 = 0x00;
/// Tag byte: forward to the next hop.
const TAG_FORWARD: u8 = 0x01;

#[derive(Debug, thiserror::Error)]
pub enum MixnetError {
    #[error("AEAD decrypt failed — wrong key or tampered layer")]
    DecryptFailed,
    #[error("malformed inner layer")]
    BadLayer,
    #[error("wire packet too short to parse")]
    BadWire,
}

/// A single mixnet hop, described by the public key it publishes.
#[derive(Clone, Debug)]
pub struct MixnetHop {
    pub public: PublicKey,
}

impl MixnetHop {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { public: PublicKey::from(bytes) }
    }
}

/// A ready-to-ship onion packet.
#[derive(Clone, Debug)]
pub struct MixnetPacket {
    pub eph_pub: [u8; 32],
    pub layer: Vec<u8>,
}

impl MixnetPacket {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + 4 + self.layer.len());
        out.extend_from_slice(&self.eph_pub);
        out.extend_from_slice(&(self.layer.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.layer);
        out
    }

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 36 { return None; }
        let eph_pub: [u8; 32] = data[0..32].try_into().ok()?;
        let layer_len = u32::from_le_bytes(data[32..36].try_into().ok()?) as usize;
        if data.len() < 36 + layer_len { return None; }
        Some(Self {
            eph_pub,
            layer: data[36..36+layer_len].to_vec(),
        })
    }
}

/// Result of peeling one onion layer.
#[derive(Debug)]
pub enum Peeled {
    /// This node is an intermediate hop — forward `packet` to `next_hop`.
    Forward { next_hop: PublicKey, packet: MixnetPacket },
    /// This node is the terminal recipient.
    Final { payload: Vec<u8> },
}

// ── Key derivation ────────────────────────────────────────────────────────────

fn derive_hop_key_from_dh(dh_shared: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, dh_shared);
    let mut out = [0u8; 32];
    hk.expand(MIX_INFO, &mut out).expect("HKDF");
    out
}

fn aead_seal(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    // 24-byte nonce prepended to the ciphertext; the hop-key differs per
    // hop so a static nonce is acceptable in principle, but we randomise
    // for robustness anyway.
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new_from_slice(key).expect("cipher");
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), AeadPayload { msg: plaintext, aad: b"" })
        .expect("encrypt");
    let mut out = Vec::with_capacity(24 + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

fn aead_open(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, MixnetError> {
    if blob.len() < 24 { return Err(MixnetError::DecryptFailed); }
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| MixnetError::DecryptFailed)?;
    cipher
        .decrypt(XNonce::from_slice(&blob[..24]), AeadPayload { msg: &blob[24..], aad: b"" })
        .map_err(|_| MixnetError::DecryptFailed)
}

// ── Packet construction ──────────────────────────────────────────────────────

/// Wrap `final_payload` in `hops.len()` AEAD layers. `hops[0]` is the
/// first relay the packet touches; the last element of `hops` is where
/// the packet is delivered (so its key decrypts the `TAG_FINAL` layer).
///
/// Empty `hops` is not allowed (the caller would be the delivery target).
pub fn pack_onion(hops: &[MixnetHop], final_payload: &[u8]) -> MixnetPacket {
    assert!(!hops.is_empty(), "mixnet route must have at least one hop");

    let eph_secret = StaticSecret::random_from_rng(OsRng);
    let eph_pub = PublicKey::from(&eph_secret);

    // Build innermost (delivery) layer.
    let mut current = Vec::with_capacity(1 + 4 + final_payload.len());
    current.push(TAG_FINAL);
    current.extend_from_slice(&(final_payload.len() as u32).to_le_bytes());
    current.extend_from_slice(final_payload);

    // Now wrap from last hop outward. After each iteration, `current`
    // contains the plaintext that this hop's AEAD will seal; the
    // enclosing hop then adds its own AEAD + forwarding header around it.
    for (i, hop) in hops.iter().enumerate().rev() {
        let dh = eph_secret.diffie_hellman(&hop.public);
        let key = derive_hop_key_from_dh(dh.as_bytes());
        let sealed = aead_seal(&key, &current);

        if i == 0 {
            // Outermost layer — `sealed` is what the first hop decrypts
            // directly from the wire.
            return MixnetPacket { eph_pub: *eph_pub.as_bytes(), layer: sealed };
        }

        // Prepend a forwarding header so the PREVIOUS hop (i-1) knows
        // where to send this hop's bytes.
        let next_hop_pub = *hop.public.as_bytes();
        let mut wrapped = Vec::with_capacity(1 + 32 + 4 + sealed.len());
        wrapped.push(TAG_FORWARD);
        wrapped.extend_from_slice(&next_hop_pub);
        wrapped.extend_from_slice(&(sealed.len() as u32).to_le_bytes());
        wrapped.extend_from_slice(&sealed);
        current = wrapped;
    }

    unreachable!("loop returns on i == 0");
}

/// Peel one layer with `my_secret`. Returns either a forwarding
/// instruction or the final plaintext.
pub fn peel_onion(packet: &MixnetPacket, my_secret: &StaticSecret) -> Result<Peeled, MixnetError> {
    let eph_pub = PublicKey::from(packet.eph_pub);
    let dh = my_secret.diffie_hellman(&eph_pub);
    let key = derive_hop_key_from_dh(dh.as_bytes());

    let inner = aead_open(&key, &packet.layer)?;
    if inner.is_empty() { return Err(MixnetError::BadLayer); }

    match inner[0] {
        TAG_FINAL => {
            if inner.len() < 1 + 4 { return Err(MixnetError::BadLayer); }
            let plen = u32::from_le_bytes(inner[1..5].try_into().unwrap()) as usize;
            if inner.len() < 1 + 4 + plen { return Err(MixnetError::BadLayer); }
            Ok(Peeled::Final { payload: inner[5..5+plen].to_vec() })
        }
        TAG_FORWARD => {
            if inner.len() < 1 + 32 + 4 { return Err(MixnetError::BadLayer); }
            let mut next_hop_bytes = [0u8; 32];
            next_hop_bytes.copy_from_slice(&inner[1..33]);
            let inner_len = u32::from_le_bytes(inner[33..37].try_into().unwrap()) as usize;
            if inner.len() < 1 + 32 + 4 + inner_len { return Err(MixnetError::BadLayer); }
            let next_layer = inner[37..37+inner_len].to_vec();
            Ok(Peeled::Forward {
                next_hop: PublicKey::from(next_hop_bytes),
                packet: MixnetPacket { eph_pub: packet.eph_pub, layer: next_layer },
            })
        }
        _ => Err(MixnetError::BadLayer),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct HopKeypair {
        secret: StaticSecret,
        hop: MixnetHop,
    }

    fn random_hop() -> HopKeypair {
        let secret = StaticSecret::random_from_rng(&mut OsRng);
        let public = PublicKey::from(&secret);
        HopKeypair { secret, hop: MixnetHop { public } }
    }

    #[test]
    fn single_hop_delivers_payload_directly() {
        let bob = random_hop();
        let packet = pack_onion(&[bob.hop.clone()], b"hello bob");
        match peel_onion(&packet, &bob.secret).unwrap() {
            Peeled::Final { payload } => assert_eq!(payload, b"hello bob"),
            Peeled::Forward { .. } => panic!("single-hop must deliver terminally"),
        }
    }

    #[test]
    fn three_hop_route_peels_layer_by_layer() {
        let h1 = random_hop();
        let h2 = random_hop();
        let h3 = random_hop();

        let packet = pack_onion(
            &[h1.hop.clone(), h2.hop.clone(), h3.hop.clone()],
            b"deep-onion",
        );

        // Hop 1 peels — must forward to hop 2.
        let peeled1 = peel_onion(&packet, &h1.secret).unwrap();
        let (next_pub_1, pkt1) = match peeled1 {
            Peeled::Forward { next_hop, packet } => (next_hop, packet),
            _ => panic!("hop1 must forward"),
        };
        assert_eq!(next_pub_1.as_bytes(), h2.hop.public.as_bytes());

        // Hop 2 peels — must forward to hop 3.
        let peeled2 = peel_onion(&pkt1, &h2.secret).unwrap();
        let (next_pub_2, pkt2) = match peeled2 {
            Peeled::Forward { next_hop, packet } => (next_hop, packet),
            _ => panic!("hop2 must forward"),
        };
        assert_eq!(next_pub_2.as_bytes(), h3.hop.public.as_bytes());

        // Hop 3 peels — delivers.
        match peel_onion(&pkt2, &h3.secret).unwrap() {
            Peeled::Final { payload } => assert_eq!(payload, b"deep-onion"),
            Peeled::Forward { .. } => panic!("hop3 must deliver"),
        }
    }

    #[test]
    fn peeling_with_wrong_key_fails() {
        let real = random_hop();
        let impostor = random_hop();
        let pkt = pack_onion(&[real.hop.clone()], b"x");
        assert!(peel_onion(&pkt, &impostor.secret).is_err());
    }

    #[test]
    fn tampered_layer_fails() {
        let h1 = random_hop();
        let mut pkt = pack_onion(&[h1.hop.clone()], b"fragile");
        // Flip a byte inside the sealed layer (not the nonce) — AEAD MAC breaks.
        let idx = pkt.layer.len() - 4;
        pkt.layer[idx] ^= 0xFF;
        assert!(peel_onion(&pkt, &h1.secret).is_err());
    }

    #[test]
    fn wire_roundtrip() {
        let h1 = random_hop();
        let h2 = random_hop();
        let pkt = pack_onion(&[h1.hop.clone(), h2.hop.clone()], b"over-the-wire");

        let bytes = pkt.to_bytes();
        let restored = MixnetPacket::from_bytes(&bytes).expect("parse");
        assert_eq!(restored.eph_pub, pkt.eph_pub);
        assert_eq!(restored.layer, pkt.layer);

        // And the restored packet still peels correctly.
        let p1 = peel_onion(&restored, &h1.secret).unwrap();
        match p1 {
            Peeled::Forward { packet, .. } => {
                match peel_onion(&packet, &h2.secret).unwrap() {
                    Peeled::Final { payload } => assert_eq!(payload, b"over-the-wire"),
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }
}
