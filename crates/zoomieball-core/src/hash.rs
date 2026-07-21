//! Canonical FNV-1a hash helpers for replay witnesses.

/// FNV-1a 32-bit offset basis used for one canonical physics body.
pub const PHYSICS_OFFSET_BASIS: u32 = 0x811c_9dc5;
const PHYSICS_PRIME: u32 = 0x0100_0193;
/// FNV-1a 64-bit offset basis.
pub const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// Hash ten canonical physics words and bind them to one stable body ID.
#[must_use]
pub fn physics_body_hash(words: [u32; 10], body_id: u32) -> u32 {
    let mut hash = words.into_iter().fold(PHYSICS_OFFSET_BASIS, |hash, word| {
        (hash ^ word).wrapping_mul(PHYSICS_PRIME)
    });
    hash ^= body_id;
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7feb_352d);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846c_a68b);
    hash ^ (hash >> 16)
}

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
