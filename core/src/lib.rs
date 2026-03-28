//! Kernbibliothek für PhantomChat.
//!
//! Diese Bibliothek stellt die grundlegenden Bausteine für den
//! dezentralen Messenger bereit: Schlüsselverwaltung, Double‑Ratchet,
//! Envelope‑Format, Stealth‑Tag‑Generierung und Proof‑of‑Work.  Die
//! aktuelle Implementierung enthält viele Platzhalter und Pseudocode –
//! sie dient vor allem der Veranschaulichung der Architektur und muss
//! durch geprüften Produktionscode ersetzt werden.

pub mod keys;
pub mod envelope;
pub mod pow;
pub mod ratchet;
pub mod util;

pub use keys::{IdentityKey, ViewKey, SpendKey};
pub use envelope::{Envelope, Payload};
pub use pow::{Hashcash};
pub use ratchet::{RatchetState, RatchetError};
