//! MLS (Messaging Layer Security, RFC 9420) — stub + integration plan.
//!
//! ## Status
//!
//! **Not yet implemented.** [`group::PhantomGroup`](crate::group) is the
//! currently-shipping group primitive (Signal's Sender-Keys scheme). MLS
//! is the successor and is wire-format-versioned so a migration can
//! coexist without breaking clients.
//!
//! ## Why MLS is next
//!
//! Sender Keys is O(n) key-exchange work on every membership change —
//! each added member receives every other member's Sender Key over the
//! pairwise channel. MLS amortises this via a **TreeKEM** structure:
//! key exchange is O(log n), so 1 000-member groups are practical.
//! MLS also has Double-Ratchet-style forward secrecy *between epochs*,
//! which Sender Keys lacks (we only get it within a chain).
//!
//! ## Integration plan
//!
//! 1. Add `openmls` v0.6 + `openmls_rust_crypto` as optional deps
//!    behind a new `mls` Cargo feature (so the classic builds aren't
//!    dragged through ~50 transitive crates).
//! 2. Wrap `openmls::MlsGroup` in a `PhantomMlsGroup` struct that:
//!    - holds the caller's `PhantomSigningKey` as the MLS "Identity"
//!      credential
//!    - bridges MLS's `KeyPackage` onto our [`PrekeyBundle`] — the
//!      X3DH signed-prekey becomes the MLS leaf key
//!    - wires MLS Welcome / Commit / Application messages through the
//!      pairwise [`SessionStore`] channel
//! 3. Add a wire version byte `GROUP_VERSION_MLS = 2` to the group
//!    message format so receivers can pick Sender Keys vs. MLS at
//!    decode time.
//! 4. Provide a migration helper that rebuilds a Sender-Keys group as
//!    an MLS group via a one-shot "epoch 0" Welcome round.
//!
//! ## Why it is deferred to a dedicated commit
//!
//! - `openmls` pulls a full rustls stack and a ciphersuite registry
//!   that collides name-wise with our `chacha20poly1305` / `sha2` /
//!   `ed25519-dalek` deps (double-registration of the MLS ciphersuite
//!   IDs). Resolving that is the first 50 % of the work.
//! - The MLS Credentials model and our [`SealedSender`] identities
//!   need a mapping decision: do we use MLS-native credentials, or do
//!   we bridge through a custom `PhantomCredential` type? Both options
//!   trade off directory-server semantics — worth a design
//!   conversation, not a quick commit.
//!
//! Tracking: this file stays as the canonical landing spot for the
//! integration so anyone searching `grep -r MLS` finds the plan.

/// Wire version byte reserved for the future MLS group message format.
/// Current group messages use [`crate::group::GROUP_VERSION_SENDER_KEYS`]
/// (= 1); MLS will be 2.
pub const GROUP_VERSION_MLS: u8 = 2;

/// Placeholder. The real type will hold an `openmls::MlsGroup` plus a
/// mapping from PhantomAddresses to MLS leaf indices.
pub struct PhantomMlsGroup {
    _reserved: (),
}

impl PhantomMlsGroup {
    /// Will construct a new MLS group with the caller as the sole
    /// member, returning a KeyPackage to share.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self { _reserved: () }
    }
}
