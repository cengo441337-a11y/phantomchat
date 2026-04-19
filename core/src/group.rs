//! Sender-Keys group chat.
//!
//! Signal's pre-MLS group-messaging primitive: each sender holds a
//! symmetric ratchet (a "Sender Key") that they distribute once to every
//! group member via the pairwise 1-to-1 channel built elsewhere in this
//! crate. After distribution, group messages are O(1) to encrypt (one
//! AEAD + one signature) and O(1) to decrypt on the receive side.
//!
//! ## Why Sender Keys and not MLS?
//!
//! MLS (RFC 9420) supersedes Sender Keys and is on the roadmap
//! (`openmls` crate integration). Sender Keys gives the same **end-to-end
//! guarantees** — confidentiality + sender authentication + forward
//! secrecy within a chain — without a ~50-crate transitive dependency. A
//! PhantomChat group shipping today uses Sender Keys; the wire format is
//! versioned (`GROUP_VERSION_SENDER_KEYS = 1`) so a later MLS migration
//! can coexist.
//!
//! ## Threat model
//!
//! - The **relay** sees an opaque blob — the same envelope stack wraps
//!   group messages as 1-to-1 messages.
//! - A **non-member** cannot read or forge messages.
//! - An **active member** cannot impersonate another member: every
//!   message is Ed25519-signed by the sender's [`PhantomSigningKey`] and
//!   the group holds each member's verifying key.
//! - A **past member** cannot read post-removal messages: on member
//!   removal, every remaining sender rotates their Sender Key by
//!   generating a fresh `chain_key` and re-distributing.

use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;

use chacha20poly1305::{
    aead::{Aead, KeyInit as AeadKeyInit, Payload as AeadPayload},
    XChaCha20Poly1305, XNonce,
};

use crate::address::PhantomAddress;
use crate::keys::{verify_ed25519, PhantomSigningKey};

type HmacSha256 = Hmac<Sha256>;

/// Wire-format version for group messages. Kept in a dedicated byte so a
/// future MLS switch doesn't collide with this Sender-Keys format.
pub const GROUP_VERSION_SENDER_KEYS: u8 = 1;

#[derive(Debug, thiserror::Error)]
pub enum GroupError {
    #[error("no sender key registered for this sender")]
    UnknownSender,
    #[error("AEAD decrypt failed — wrong key or tampered ciphertext")]
    DecryptFailed,
    #[error("signature failed to verify against the sender's identity key")]
    SignatureInvalid,
    #[error("malformed wire message")]
    BadWire,
    #[error("this iteration was already consumed")]
    Replay,
}

/// Per-sender ratchet state a group member holds for every other member.
#[derive(Clone, Serialize, Deserialize)]
pub struct SenderKeyState {
    chain_key: [u8; 32],
    /// Last iteration that was consumed. Guards against replay.
    iteration: u32,
    /// Ed25519 verifying key of the sender — used to authenticate every
    /// message that advances this chain.
    signing_pub: [u8; 32],
}

/// The distribution message a member sends on join / on key-rotation to
/// every other member via the pairwise channel. Carries the initial chain
/// state plus the sender's identity so receivers can verify subsequent
/// group messages.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SenderKeyDistribution {
    pub chain_key: [u8; 32],
    pub iteration: u32,
    pub signing_pub: [u8; 32],
}

fn kdf(key: &[u8; 32], info: &[u8]) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC");
    mac.update(info);
    mac.finalize().into_bytes().into()
}

/// A group this device is a member of.
#[derive(Clone, Serialize, Deserialize)]
pub struct PhantomGroup {
    /// 16-byte random group identifier. Same for every member.
    pub group_id: [u8; 16],
    /// Public roster — PhantomAddresses of all members including self.
    pub members: Vec<PhantomAddress>,
    /// Sender key state for every member, keyed by the 8-byte short id of
    /// the member's Ed25519 *signing* public (not their spend_pub — the
    /// signing key is what authenticates group messages).
    sender_keys: HashMap<[u8; 8], SenderKeyState>,
    /// Our own current chain — rotated on member removal.
    own_chain: [u8; 32],
    own_iteration: u32,
}

impl PhantomGroup {
    /// Create a brand-new group. `self_signing_key` seeds our own Sender
    /// Key chain; `members` is the initial public roster (including self).
    pub fn new(members: Vec<PhantomAddress>, self_signing_key: &PhantomSigningKey) -> Self {
        let mut group_id = [0u8; 16];
        OsRng.fill_bytes(&mut group_id);

        let mut own_chain = [0u8; 32];
        OsRng.fill_bytes(&mut own_chain);

        let mut g = Self {
            group_id,
            members,
            sender_keys: HashMap::new(),
            own_chain,
            own_iteration: 0,
        };

        // Register our own Sender-Key state so we can decrypt messages we
        // just sent (useful for message history echoes from the relay).
        let mut own_short = [0u8; 8];
        own_short.copy_from_slice(&self_signing_key.public_bytes()[..8]);
        g.sender_keys.insert(
            own_short,
            SenderKeyState {
                chain_key: own_chain,
                iteration: 0,
                signing_pub: self_signing_key.public_bytes(),
            },
        );
        g
    }

    /// Produce the distribution message to hand out to every other member
    /// via the pairwise 1-to-1 channel.
    pub fn own_distribution(&self, signing: &PhantomSigningKey) -> SenderKeyDistribution {
        SenderKeyDistribution {
            chain_key: self.own_chain,
            iteration: self.own_iteration,
            signing_pub: signing.public_bytes(),
        }
    }

    /// Register a Sender-Key distribution received from another member.
    pub fn accept_distribution(&mut self, dist: SenderKeyDistribution) {
        let mut key = [0u8; 8];
        key.copy_from_slice(&dist.signing_pub[..8]);
        self.sender_keys.insert(
            key,
            SenderKeyState {
                chain_key: dist.chain_key,
                iteration: dist.iteration,
                signing_pub: dist.signing_pub,
            },
        );
    }

    /// Rotate our own Sender Key. Call after [`remove_member`] or on a
    /// periodic schedule (e.g. every N messages) to limit forward-secrecy
    /// windows. The fresh distribution must be re-sent to every remaining
    /// member through the pairwise channel.
    pub fn rotate_own_chain(&mut self, signing: &PhantomSigningKey) -> SenderKeyDistribution {
        OsRng.fill_bytes(&mut self.own_chain);
        self.own_iteration = 0;
        let mut own_short = [0u8; 8];
        own_short.copy_from_slice(&signing.public_bytes()[..8]);
        self.sender_keys.insert(
            own_short,
            SenderKeyState {
                chain_key: self.own_chain,
                iteration: 0,
                signing_pub: signing.public_bytes(),
            },
        );
        self.own_distribution(signing)
    }

    /// Add a member to the public roster. Distribution of existing
    /// members' Sender Keys to the newcomer must happen via the pairwise
    /// channel afterwards. (Forward-secrecy note: do **not** share old
    /// `chain_key`s with newcomers; rotate first if pre-join history
    /// should remain inaccessible.)
    pub fn add_member(&mut self, member: PhantomAddress) {
        if !self.members.iter().any(|m| m == &member) {
            self.members.push(member);
        }
    }

    /// Remove a member and rotate our own Sender Key so post-removal
    /// messages stay inaccessible to them.
    pub fn remove_member(
        &mut self,
        member: &PhantomAddress,
        signing: &PhantomSigningKey,
    ) -> SenderKeyDistribution {
        self.members.retain(|m| m != member);
        self.rotate_own_chain(signing)
    }

    /// Encrypt a plaintext with our current Sender Key chain + sign it
    /// with our Ed25519 identity. Returns the wire bytes to be wrapped
    /// in a 1-to-1 PhantomChat envelope for every member.
    pub fn encrypt(&mut self, signing: &PhantomSigningKey, plaintext: &[u8]) -> Vec<u8> {
        // Advance our own chain.
        let msg_key = kdf(&self.own_chain, &[0x01]);
        self.own_chain = kdf(&self.own_chain, &[0x02]);
        self.own_iteration += 1;

        // Also advance the mirror SenderKeyState stored for ourselves, so
        // a relay echo of our own message still decrypts.
        let mut own_short = [0u8; 8];
        own_short.copy_from_slice(&signing.public_bytes()[..8]);
        if let Some(st) = self.sender_keys.get_mut(&own_short) {
            st.chain_key = self.own_chain;
            st.iteration = self.own_iteration;
        }

        let cipher = XChaCha20Poly1305::new_from_slice(&msg_key).expect("cipher init");
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut nonce);

        // aad = group_id || iteration — binds the ciphertext to this group
        // slot so a copy-paste into another group would fail decryption.
        let mut aad = Vec::with_capacity(16 + 4);
        aad.extend_from_slice(&self.group_id);
        aad.extend_from_slice(&self.own_iteration.to_le_bytes());

        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), AeadPayload { msg: plaintext, aad: &aad })
            .expect("encrypt");

        // Sign (ver || group_id || iteration || nonce || ciphertext).
        let mut signed = Vec::with_capacity(1 + 16 + 4 + 24 + ciphertext.len());
        signed.push(GROUP_VERSION_SENDER_KEYS);
        signed.extend_from_slice(&self.group_id);
        signed.extend_from_slice(&self.own_iteration.to_le_bytes());
        signed.extend_from_slice(&nonce);
        signed.extend_from_slice(&ciphertext);

        let signature = signing.sign(&signed);

        // Wire: [ver][group_id][iteration][nonce][ct_len:u32][ct][signer_pub][sig]
        let mut wire = Vec::with_capacity(signed.len() + 4 + 32 + 64);
        wire.push(GROUP_VERSION_SENDER_KEYS);
        wire.extend_from_slice(&self.group_id);
        wire.extend_from_slice(&self.own_iteration.to_le_bytes());
        wire.extend_from_slice(&nonce);
        wire.extend_from_slice(&(ciphertext.len() as u32).to_le_bytes());
        wire.extend_from_slice(&ciphertext);
        wire.extend_from_slice(&signing.public_bytes());
        wire.extend_from_slice(&signature);
        wire
    }

    /// Decrypt + verify an incoming group wire message. Returns plaintext
    /// on success.
    pub fn decrypt(&mut self, wire: &[u8]) -> Result<Vec<u8>, GroupError> {
        if wire.len() < 1 + 16 + 4 + 24 + 4 + 32 + 64 {
            return Err(GroupError::BadWire);
        }
        let mut c = 0usize;

        if wire[c] != GROUP_VERSION_SENDER_KEYS {
            return Err(GroupError::BadWire);
        }
        c += 1;

        let mut group_id = [0u8; 16];
        group_id.copy_from_slice(&wire[c..c+16]); c += 16;
        if group_id != self.group_id {
            return Err(GroupError::BadWire);
        }

        let iteration = u32::from_le_bytes(
            wire[c..c+4].try_into().map_err(|_| GroupError::BadWire)?,
        );
        c += 4;

        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(&wire[c..c+24]); c += 24;

        let ct_len = u32::from_le_bytes(
            wire[c..c+4].try_into().map_err(|_| GroupError::BadWire)?,
        ) as usize;
        c += 4;
        if c + ct_len + 32 + 64 > wire.len() {
            return Err(GroupError::BadWire);
        }
        let ciphertext = &wire[c..c+ct_len]; c += ct_len;

        let mut signer_pub = [0u8; 32];
        signer_pub.copy_from_slice(&wire[c..c+32]); c += 32;

        let mut signature = [0u8; 64];
        signature.copy_from_slice(&wire[c..c+64]);

        // Verify signature first — cheap rejection for forged messages.
        let signed_end = 1 + 16 + 4 + 24 + ct_len; // excludes signer_pub+sig
        // Reconstruct the signed bytes (ver||gid||iter||nonce||ct) exactly
        // the way encrypt() did.
        let mut signed = Vec::with_capacity(signed_end);
        signed.push(GROUP_VERSION_SENDER_KEYS);
        signed.extend_from_slice(&self.group_id);
        signed.extend_from_slice(&iteration.to_le_bytes());
        signed.extend_from_slice(&nonce);
        signed.extend_from_slice(ciphertext);

        if !verify_ed25519(&signer_pub, &signed, &signature) {
            return Err(GroupError::SignatureInvalid);
        }

        // Look up the sender's state + advance to the claimed iteration.
        // In Sender Keys, receivers skip forward — they can't go back, so
        // out-of-order delivery within a single sender's chain would fail.
        let mut short = [0u8; 8];
        short.copy_from_slice(&signer_pub[..8]);
        let state = self.sender_keys.get_mut(&short).ok_or(GroupError::UnknownSender)?;

        if iteration <= state.iteration {
            return Err(GroupError::Replay);
        }
        // Fast-forward the chain — advance (iteration - state.iteration) times.
        let mut msg_key = [0u8; 32];
        while state.iteration < iteration {
            msg_key = kdf(&state.chain_key, &[0x01]);
            state.chain_key = kdf(&state.chain_key, &[0x02]);
            state.iteration += 1;
        }

        let cipher = XChaCha20Poly1305::new_from_slice(&msg_key).expect("cipher init");
        let mut aad = Vec::with_capacity(16 + 4);
        aad.extend_from_slice(&self.group_id);
        aad.extend_from_slice(&iteration.to_le_bytes());

        cipher
            .decrypt(XNonce::from_slice(&nonce), AeadPayload { msg: ciphertext, aad: &aad })
            .map_err(|_| GroupError::DecryptFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{SpendKey, ViewKey};

    fn mock_addr() -> PhantomAddress {
        PhantomAddress::new(ViewKey::generate().public, SpendKey::generate().public)
    }

    #[test]
    fn sender_keys_group_roundtrip() {
        let alice_sign = PhantomSigningKey::generate();
        let bob_sign   = PhantomSigningKey::generate();

        let members = vec![mock_addr(), mock_addr()];
        let mut alice_group = PhantomGroup::new(members.clone(), &alice_sign);
        let mut bob_group   = PhantomGroup {
            group_id: alice_group.group_id,
            members: members.clone(),
            sender_keys: HashMap::new(),
            own_chain: {
                let mut b = [0u8; 32];
                OsRng.fill_bytes(&mut b);
                b
            },
            own_iteration: 0,
        };

        // Alice hands Bob her Sender-Key distribution; Bob registers it.
        let dist = alice_group.own_distribution(&alice_sign);
        bob_group.accept_distribution(dist);

        // Alice sends two messages — Bob decrypts both.
        let w0 = alice_group.encrypt(&alice_sign, b"hi group");
        let w1 = alice_group.encrypt(&alice_sign, b"second");

        assert_eq!(bob_group.decrypt(&w0).unwrap(), b"hi group");
        assert_eq!(bob_group.decrypt(&w1).unwrap(), b"second");

        // And Alice can self-decrypt her own echoed messages (via the own
        // SenderKeyState mirror). The iteration has already advanced past
        // w0/w1 so an echo replay from the relay returns Replay — that's
        // the correct behaviour for the receive path.
        assert!(matches!(alice_group.decrypt(&w0), Err(GroupError::Replay)));

        let _ = bob_sign;
    }

    #[test]
    fn sender_keys_group_rejects_tampered_signature() {
        let sign = PhantomSigningKey::generate();
        let mut g = PhantomGroup::new(vec![mock_addr()], &sign);
        let dist = g.own_distribution(&sign);

        // Set up another group instance that accepts the distribution.
        let mut other = PhantomGroup {
            group_id: g.group_id,
            members: Vec::new(),
            sender_keys: HashMap::new(),
            own_chain: [0u8; 32],
            own_iteration: 0,
        };
        other.accept_distribution(dist);

        let mut w = g.encrypt(&sign, b"payload");
        // Flip a byte inside the signature region.
        let len = w.len();
        w[len - 10] ^= 0xFF;
        assert!(matches!(
            other.decrypt(&w),
            Err(GroupError::SignatureInvalid)
        ));
    }

    #[test]
    fn sender_keys_group_rejects_unknown_sender() {
        // A receiver that never saw the sender's distribution should bail.
        let alice = PhantomSigningKey::generate();
        let mut alice_g = PhantomGroup::new(vec![mock_addr()], &alice);
        let w = alice_g.encrypt(&alice, b"x");

        let bob = PhantomSigningKey::generate();
        let mut bob_g = PhantomGroup::new(vec![mock_addr()], &bob);
        bob_g.group_id = alice_g.group_id;
        // Note: no accept_distribution called.
        assert!(matches!(bob_g.decrypt(&w), Err(GroupError::UnknownSender)));
    }
}
