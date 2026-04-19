#[cfg(feature = "ffi")]
mod frb_generated; /* AUTO INJECTED BY flutter_rust_bridge. This line may not be accurate, and you can change it according to your needs. */
#[cfg(feature = "ffi")]
pub mod api;
#[cfg(feature = "ffi")]
pub mod network;
#[cfg(feature = "ffi")]
pub mod storage;

pub mod address;
pub mod envelope;
pub mod fingerprint;
pub mod group;
pub mod keys;
pub mod mixnet;
pub mod mls;
pub mod pow;
pub mod prekey;
pub mod privacy;
pub mod psi;
pub mod ratchet;
pub mod scanner;
pub mod session;
pub mod util;

#[cfg(feature = "wasm")]
pub mod wasm;

// ── Native-runtime modules ───────────────────────────────────────────────────
// These pull tokio + libp2p and therefore only compile on hosts with a real
// OS. WASM / embedded builds (`--no-default-features`, optional `wasm`) still
// get the full Envelope + Session + PQXDH + SealedSender + prekey + safety
// number stack — just no peer discovery or traffic generation.
#[cfg(feature = "net")]
pub mod cover_traffic;
#[cfg(feature = "net")]
pub mod dandelion;

pub use address::PhantomAddress;
pub use envelope::{Envelope, Payload, SealedSender};
pub use fingerprint::safety_number;
pub use group::{GroupError, PhantomGroup, SenderKeyDistribution};
pub use mixnet::{pack_onion, peel_onion, MixnetError, MixnetHop, MixnetPacket, Peeled};
pub use psi::{PsiClient, PsiError, PsiServer};
pub use keys::{
    verify_ed25519, HybridKeyPair, HybridPublicKey, HybridSecretKey, IdentityKey,
    PhantomSigningKey, SpendKey, ViewKey,
};
pub use pow::Hashcash;
pub use prekey::{OneTimePrekey, PrekeyBundle, PrekeyMaterial, SignedPrekey};
pub use ratchet::{RatchetError, RatchetState};
pub use scanner::{scan_batch, scan_envelope, ScanResult};
pub use session::{ReceivedMessage, SessionError, SessionStore};
