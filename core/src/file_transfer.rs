//! File-transfer primitive — chunk, encrypt each chunk with the group's
//! Sender-Keys cipher, serialize everything into a single `.ptf` archive.
//!
//! Wire format (little-endian):
//!
//! ```text
//! MAGIC   "PTF\0"                (4 bytes, ASCII + NUL)
//! VERSION 0x01                   (1 byte)
//! GROUP_ID                       (16 bytes)
//! TOTAL_SIZE                     (8 bytes, u64)  — original plaintext length
//! SHA256                         (32 bytes)      — of original plaintext
//! CHUNK_COUNT                    (4 bytes, u32)
//! FILENAME_LEN                   (2 bytes, u16)
//! FILENAME                       (FILENAME_LEN bytes, UTF-8)
//! [ CHUNK_WIRE_LEN (u32) || CHUNK_WIRE (Sender-Keys envelope) ] × CHUNK_COUNT
//! ```
//!
//! Each CHUNK_WIRE is the output of [`crate::group::PhantomGroup::encrypt`]
//! applied to one plaintext slice. The manifest is *outside* the encrypted
//! region — this is intentional: the group_id + SHA256 must be readable to
//! validate archive integrity before spending CPU on AEAD. An attacker who
//! intercepts the archive still learns only the file name and size class
//! (mitigate with conservative naming + padding at the caller level).

use crate::{group::{GroupError, PhantomGroup}, keys::PhantomSigningKey};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

/// Default chunk size — chosen to keep each encrypted chunk below the
/// envelope padding's 1 KiB block while leaving room for AEAD overhead.
pub const DEFAULT_CHUNK_SIZE: usize = 700;

pub const ARCHIVE_MAGIC: [u8; 4] = *b"PTF\0";
pub const ARCHIVE_VERSION: u8 = 1;
pub const MAX_FILENAME_LEN: usize = 1024;
/// Hard cap on archive size accepted by `unpack_into` (8 GiB) — prevents
/// pathological input from exhausting memory.
pub const MAX_ARCHIVE_SIZE: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FileTransferError {
    #[error("archive too short")]
    Truncated,
    #[error("wrong magic — not a PTF archive")]
    BadMagic,
    #[error("unsupported archive version {0}")]
    BadVersion(u8),
    #[error("group_id mismatch: archive targets a different group")]
    GroupMismatch,
    #[error("filename too long ({0} > {max})", max = MAX_FILENAME_LEN)]
    FilenameTooLong(usize),
    #[error("chunk decrypt failed at index {0}: {1:?}")]
    ChunkDecrypt(u32, GroupError),
    #[error("total size mismatch: manifest {claimed}, reassembled {got}")]
    SizeMismatch { claimed: u64, got: u64 },
    #[error("sha256 mismatch — archive corrupted or tampered")]
    HashMismatch,
    #[error("archive exceeds max size {}", MAX_ARCHIVE_SIZE)]
    ArchiveTooLarge,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Pack a plaintext blob into an archive addressed to the given group.
///
/// `filename` is stored verbatim in the manifest header. Callers may pass
/// `""` to omit the original name. `chunk_size` bounds the plaintext slice
/// fed to each `group.encrypt` call — `DEFAULT_CHUNK_SIZE` is a sane default.
pub fn pack(
    group: &mut PhantomGroup,
    signing: &PhantomSigningKey,
    filename: &str,
    plaintext: &[u8],
    chunk_size: usize,
) -> Result<Vec<u8>, FileTransferError> {
    if filename.len() > MAX_FILENAME_LEN {
        return Err(FileTransferError::FilenameTooLong(filename.len()));
    }
    let effective_chunk = chunk_size.max(1);

    let mut sha = Sha256::new();
    sha.update(plaintext);
    let hash: [u8; 32] = sha.finalize().into();

    let mut out: Vec<u8> = Vec::with_capacity(plaintext.len() + 4096);
    out.write_all(&ARCHIVE_MAGIC)?;
    out.push(ARCHIVE_VERSION);
    out.write_all(&group.group_id)?;
    out.write_all(&(plaintext.len() as u64).to_le_bytes())?;
    out.write_all(&hash)?;
    let chunk_count = plaintext.len().div_ceil(effective_chunk);
    out.write_all(&(chunk_count as u32).to_le_bytes())?;
    out.write_all(&(filename.len() as u16).to_le_bytes())?;
    out.write_all(filename.as_bytes())?;

    let mut seen = 0usize;
    while seen < plaintext.len() {
        let end = (seen + effective_chunk).min(plaintext.len());
        let wire = group.encrypt(signing, &plaintext[seen..end]);
        out.write_all(&(wire.len() as u32).to_le_bytes())?;
        out.write_all(&wire)?;
        seen = end;
    }
    // Handle the empty-input edge case — emit zero chunks (chunk_count already 0).
    Ok(out)
}

/// Decoded archive header — returned by [`peek_header`] without any AEAD work.
#[derive(Debug, Clone)]
pub struct ArchiveHeader {
    pub group_id: [u8; 16],
    pub total_size: u64,
    pub sha256: [u8; 32],
    pub chunk_count: u32,
    pub filename: String,
    /// Byte offset after the header — where the first chunk-len prefix begins.
    pub body_offset: usize,
}

pub fn peek_header(archive: &[u8]) -> Result<ArchiveHeader, FileTransferError> {
    if archive.len() as u64 > MAX_ARCHIVE_SIZE {
        return Err(FileTransferError::ArchiveTooLarge);
    }
    // Fixed-size prefix = MAGIC(4) + VER(1) + GID(16) + SIZE(8) + SHA(32) + COUNT(4) + FNLEN(2) = 67
    if archive.len() < 67 {
        return Err(FileTransferError::Truncated);
    }
    if archive[0..4] != ARCHIVE_MAGIC {
        return Err(FileTransferError::BadMagic);
    }
    if archive[4] != ARCHIVE_VERSION {
        return Err(FileTransferError::BadVersion(archive[4]));
    }
    let mut gid = [0u8; 16];
    gid.copy_from_slice(&archive[5..21]);
    let total_size = u64::from_le_bytes(archive[21..29].try_into().unwrap());
    let mut sha = [0u8; 32];
    sha.copy_from_slice(&archive[29..61]);
    let chunk_count = u32::from_le_bytes(archive[61..65].try_into().unwrap());
    let fn_len = u16::from_le_bytes(archive[65..67].try_into().unwrap()) as usize;
    if fn_len > MAX_FILENAME_LEN || archive.len() < 67 + fn_len {
        return Err(FileTransferError::Truncated);
    }
    let filename = std::str::from_utf8(&archive[67..67 + fn_len])
        .map_err(|_| FileTransferError::Truncated)?
        .to_string();
    Ok(ArchiveHeader {
        group_id: gid,
        total_size,
        sha256: sha,
        chunk_count,
        filename,
        body_offset: 67 + fn_len,
    })
}

/// Reassemble + verify an archive into plaintext. The caller must supply a
/// `PhantomGroup` already primed with the sender's distribution (otherwise
/// the first chunk decrypt will return `GroupError::UnknownSender`).
pub fn unpack_into(group: &mut PhantomGroup, archive: &[u8]) -> Result<(ArchiveHeader, Vec<u8>), FileTransferError> {
    let hdr = peek_header(archive)?;
    if hdr.group_id != group.group_id {
        return Err(FileTransferError::GroupMismatch);
    }

    let mut body: Vec<u8> = Vec::with_capacity(hdr.total_size as usize);
    let mut cur = hdr.body_offset;
    for i in 0..hdr.chunk_count {
        if cur + 4 > archive.len() {
            return Err(FileTransferError::Truncated);
        }
        let wire_len = u32::from_le_bytes(archive[cur..cur + 4].try_into().unwrap()) as usize;
        cur += 4;
        if cur + wire_len > archive.len() {
            return Err(FileTransferError::Truncated);
        }
        let plain = group
            .decrypt(&archive[cur..cur + wire_len])
            .map_err(|e| FileTransferError::ChunkDecrypt(i, e))?;
        body.extend_from_slice(&plain);
        cur += wire_len;
    }

    if body.len() as u64 != hdr.total_size {
        return Err(FileTransferError::SizeMismatch {
            claimed: hdr.total_size,
            got: body.len() as u64,
        });
    }
    let mut sha = Sha256::new();
    sha.update(&body);
    let got: [u8; 32] = sha.finalize().into();
    if got != hdr.sha256 {
        return Err(FileTransferError::HashMismatch);
    }
    Ok((hdr, body))
}

/// Streaming `Read` → `Write` variant.
pub fn pack_stream<R: Read, W: Write>(
    group: &mut PhantomGroup,
    signing: &PhantomSigningKey,
    filename: &str,
    input: &mut R,
    output: &mut W,
    chunk_size: usize,
) -> Result<u64, FileTransferError> {
    let mut buf = Vec::new();
    input.read_to_end(&mut buf)?;
    let archive = pack(group, signing, filename, &buf, chunk_size)?;
    output.write_all(&archive)?;
    Ok(archive.len() as u64)
}

pub fn unpack_stream<R: Read, W: Write>(
    group: &mut PhantomGroup,
    input: &mut R,
    output: &mut W,
) -> Result<ArchiveHeader, FileTransferError> {
    let mut archive = Vec::new();
    input.read_to_end(&mut archive)?;
    let (hdr, plain) = unpack_into(group, &archive)?;
    output.write_all(&plain)?;
    Ok(hdr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{address::PhantomAddress, keys::{IdentityKey, SpendKey, ViewKey}};

    fn persona() -> (PhantomAddress, PhantomSigningKey) {
        let id = IdentityKey::generate();
        let view = ViewKey::generate();
        let spend = SpendKey::generate();
        let addr = PhantomAddress::new(view.public, spend.public);
        let signing = PhantomSigningKey::from_bytes(id.private);
        (addr, signing)
    }

    fn make_group_pair(roster: Vec<PhantomAddress>) -> ((PhantomGroup, PhantomSigningKey), (PhantomGroup, PhantomSigningKey)) {
        let (a_addr, a_sig) = persona();
        let (b_addr, b_sig) = persona();
        let roster = {
            let mut r = roster;
            r.push(a_addr.clone());
            r.push(b_addr.clone());
            r
        };
        let mut ga = PhantomGroup::new(roster.clone(), &a_sig);
        let mut gb = PhantomGroup::new(roster, &b_sig);
        gb.group_id = ga.group_id;
        let da = ga.own_distribution(&a_sig);
        let db = gb.own_distribution(&b_sig);
        ga.accept_distribution(db);
        gb.accept_distribution(da);
        ((ga, a_sig), (gb, b_sig))
    }

    #[test]
    fn roundtrip_small() {
        let ((mut ga, sa), (mut gb, _)) = make_group_pair(vec![]);
        let payload = b"hello file";
        let archive = pack(&mut ga, &sa, "greeting.txt", payload, DEFAULT_CHUNK_SIZE).unwrap();
        let (hdr, plain) = unpack_into(&mut gb, &archive).unwrap();
        assert_eq!(hdr.filename, "greeting.txt");
        assert_eq!(hdr.total_size, payload.len() as u64);
        assert_eq!(plain, payload);
    }

    #[test]
    fn roundtrip_multi_chunk() {
        let ((mut ga, sa), (mut gb, _)) = make_group_pair(vec![]);
        let payload: Vec<u8> = (0..10_000u32).map(|i| (i & 0xff) as u8).collect();
        let archive = pack(&mut ga, &sa, "blob.bin", &payload, 500).unwrap();
        let (hdr, plain) = unpack_into(&mut gb, &archive).unwrap();
        assert_eq!(hdr.chunk_count, 20);
        assert_eq!(plain, payload);
    }

    #[test]
    fn empty_payload_ok() {
        let ((mut ga, sa), (mut gb, _)) = make_group_pair(vec![]);
        let archive = pack(&mut ga, &sa, "", b"", DEFAULT_CHUNK_SIZE).unwrap();
        let (hdr, plain) = unpack_into(&mut gb, &archive).unwrap();
        assert_eq!(hdr.chunk_count, 0);
        assert!(plain.is_empty());
    }

    #[test]
    fn tampered_chunk_rejected() {
        let ((mut ga, sa), (mut gb, _)) = make_group_pair(vec![]);
        let payload = b"important";
        let mut archive = pack(&mut ga, &sa, "x", payload, DEFAULT_CHUNK_SIZE).unwrap();
        // Flip one byte inside the first chunk body (past the 4-byte chunk-len prefix).
        let hdr = peek_header(&archive).unwrap();
        let flip = hdr.body_offset + 4 + 32;
        if flip < archive.len() {
            archive[flip] ^= 0xAA;
        }
        assert!(unpack_into(&mut gb, &archive).is_err());
    }

    #[test]
    fn wrong_group_rejected() {
        let ((mut ga, sa), _) = make_group_pair(vec![]);
        let ((_, _), (mut gc, _)) = make_group_pair(vec![]); // different group_id
        let archive = pack(&mut ga, &sa, "", b"secret", DEFAULT_CHUNK_SIZE).unwrap();
        match unpack_into(&mut gc, &archive) {
            Err(FileTransferError::GroupMismatch) => {}
            other => panic!("expected GroupMismatch, got {other:?}"),
        }
    }

    #[test]
    fn bad_magic_rejected() {
        let archive = vec![0u8; 200];
        match peek_header(&archive) {
            Err(FileTransferError::BadMagic) => {}
            other => panic!("expected BadMagic, got {other:?}"),
        }
    }
}
