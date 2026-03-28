//! Hilfsfunktionen (Hex‑Kodierung, Zufallsgeneratoren etc.).

use sha2::{Digest, Sha256};

/// Berechnet den SHA‑256‑Digest über die gegebenen Bytes und gibt ihn als
/// Vektor zurück.
pub fn sha256(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Wandelt ein Bytearray in einen hexkodierten String um.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Zählt die Anzahl der führenden Nullbits in einem Bytearray.
pub fn leading_zero_bits(bytes: &[u8]) -> u32 {
    let mut count = 0;
    for b in bytes {
        if *b == 0 {
            count += 8;
        } else {
            count += b.leading_zeros();
            break;
        }
    }
    count
}
