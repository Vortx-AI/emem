//! cid64 — token-economical short form of a 32-byte fact CID.
//!
//! Used in inline text (token-economical channel). Full CIDs MUST be used
//! in canonical CBOR.

use data_encoding::BASE32_NOPAD;

/// Encode a 32-byte CID as a 13-char base32 short form.
pub fn to_cid64(cid: &[u8; 32]) -> String {
    BASE32_NOPAD.encode(&cid[..8]).to_lowercase()
}

/// Decode a cid64 string back to its 8-byte prefix.
/// Note: cid64 is a *prefix*; full collision-resistance requires the full CID.
pub fn from_cid64(s: &str) -> Result<[u8; 8], Cid64Error> {
    let bytes = BASE32_NOPAD.decode(s.to_uppercase().as_bytes())
        .map_err(|_| Cid64Error::BadBase32)?;
    if bytes.len() != 8 {
        return Err(Cid64Error::WrongLength(bytes.len()));
    }
    let mut out = [0u8; 8];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// cid64 errors.
#[derive(Debug, thiserror::Error)]
pub enum Cid64Error {
    /// String wasn't valid base32 nopad.
    #[error("invalid base32")]
    BadBase32,
    /// Decoded length was not 8 bytes.
    #[error("expected 8 bytes after decoding, got {0}")]
    WrongLength(usize),
}
