//! emem-CBOR profile — RFC 8949 deterministic encoding plus mandatory
//! emem tags for cells, tslots, vec64, and IPLD CIDs.
//!
//! Two implementations MUST produce byte-identical CBOR for the same fact.

/// CBOR tag for an emem cell ID (u64 packed per spec §3.1).
pub const TAG_EMEM_CELL: u64 = 65000;
/// CBOR tag for a tslot (u64).
pub const TAG_EMEM_TSLOT: u64 = 65001;
/// CBOR tag for a vec64-derived CID (32 bytes).
pub const TAG_EMEM_VEC64: u64 = 65002;
/// CBOR tag 42 (IPLD CID, multibase 'b' base32).
pub const TAG_IPLD_CID: u64 = 42;

/// Encode any `serde::Serialize` value to canonical CBOR bytes.
///
/// `ciborium::ser::into_writer` already emits deterministic encoding when
/// the input traversal is deterministic (which is true for serde-derived
/// structs — fields serialize in declaration order). For freeform maps
/// callers MUST provide pre-sorted keys.
pub fn to_canonical_cbor<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(v, &mut buf)?;
    Ok(buf)
}

/// Compute the BLAKE3 32-byte hash over a canonical CBOR byte string.
pub fn blake3_32(bytes: &[u8]) -> [u8; 32] {
    let h = blake3::hash(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(h.as_bytes());
    out
}

/// Compute base32 (no-pad, lowercase) of the first N bytes of a hash.
pub fn base32_prefix(hash: &[u8; 32], n: usize) -> String {
    use data_encoding::BASE32_NOPAD;
    BASE32_NOPAD.encode(&hash[..n]).to_lowercase()
}
