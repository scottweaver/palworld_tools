//! The outer `.sav` container: a 12-byte header — uncompressed
//! length, compressed length, three magic bytes, compression type —
//! followed by the compressed GVAS stream. Two eras exist: `PlZ`
//! (zlib, pre-0.6) and `PlM` (Oodle, standard since Palworld 0.6);
//! Xbox saves wrap either in a `CNK` prefix. All decompression
//! happens here, so the GVAS layer only ever sees plain bytes.

use std::io::Read;

use flate2::read::ZlibDecoder;
use oozextract::Extractor;

#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error("not a Palworld save container (no PlZ/PlM magic)")]
    NotASave,
    #[error("unsupported compression: magic {magic:?}, save type {save_type:#x}")]
    Unsupported { magic: [u8; 3], save_type: u8 },
    #[error("zlib decompression failed: {0}")]
    Zlib(#[from] std::io::Error),
    #[error("oodle decompression failed: {0:?}")]
    Oodle(oozextract::OozError),
    #[error("decompressed length {actual} does not match header claim {expected}")]
    LengthMismatch { expected: usize, actual: usize },
}

/// Whether `bytes` start with a recognizable save container header.
#[must_use]
pub(crate) fn has_save_magic(bytes: &[u8]) -> bool {
    bytes.len() > 12 && matches!(&bytes[8..11], b"PlZ" | b"PlM" | b"CNK")
}

/// Strips the container: validates the header and returns the
/// decompressed GVAS bytes.
pub(crate) fn decompress(bytes: &[u8]) -> Result<Vec<u8>, ContainerError> {
    let (header_at, body_at) = if bytes.len() >= 24 && &bytes[8..11] == b"CNK" {
        (12, 24)
    } else {
        (0, 12)
    };
    if bytes.len() <= body_at {
        return Err(ContainerError::NotASave);
    }

    let uncompressed_len = read_u32(bytes, header_at) as usize;
    let magic: [u8; 3] = bytes[header_at + 8..header_at + 11]
        .try_into()
        .expect("slice of length 3");
    let save_type = bytes[header_at + 11];
    let body = &bytes[body_at..];

    let decompressed = match (&magic, save_type) {
        (b"PlZ", 0x31) => zlib(body)?,
        (b"PlZ", 0x32) => zlib(&zlib(body)?)?,
        (b"PlM", 0x31) => oodle(body, uncompressed_len)?,
        (b"PlZ" | b"PlM", other) => {
            return Err(ContainerError::Unsupported {
                magic,
                save_type: other,
            });
        }
        _ => return Err(ContainerError::NotASave),
    };

    if decompressed.len() != uncompressed_len {
        return Err(ContainerError::LengthMismatch {
            expected: uncompressed_len,
            actual: decompressed.len(),
        });
    }
    Ok(decompressed)
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("slice of length 4"))
}

fn zlib(body: &[u8]) -> Result<Vec<u8>, ContainerError> {
    let mut out = Vec::new();
    ZlibDecoder::new(body).read_to_end(&mut out)?;
    Ok(out)
}

fn oodle(body: &[u8], uncompressed_len: usize) -> Result<Vec<u8>, ContainerError> {
    let mut out = vec![0u8; uncompressed_len];
    Extractor::new()
        .read_from_slice(body, &mut out)
        .map_err(ContainerError::Oodle)?;
    Ok(out)
}
