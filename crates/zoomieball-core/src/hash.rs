//! Canonical FNV-1a hash helpers for replay witnesses.

/// FNV-1a 64-bit offset basis.
pub const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// Fold one `u64` as eight little-endian bytes.
#[must_use]
pub fn fold_u64(hash: u64, value: u64) -> u64 {
    value.to_le_bytes().into_iter().fold(hash, |state, byte| {
        (state ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

/// Fold one `i32` as its exact two's-complement word.
#[must_use]
pub fn fold_i32(hash: u64, value: i32) -> u64 {
    value.to_le_bytes().into_iter().fold(hash, |state, byte| {
        (state ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

/// Fold a byte slice.
#[must_use]
pub fn fold_bytes(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().copied().fold(hash, |state, byte| {
        (state ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}
