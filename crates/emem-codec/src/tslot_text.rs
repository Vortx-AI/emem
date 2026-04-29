//! Token-economical text rendering of `Tslot`.
//!
//! Format: `t.<base32-of-varint-tslot>`. Two tokens under cl100k.

use data_encoding::BASE32_NOPAD;
use emem_core::Tslot;

/// Render a tslot as `"t.<base32>"`.
pub fn to_tslot_text(t: Tslot) -> String {
    let mut buf = [0u8; 10];
    let n = encode_varint(t.0, &mut buf);
    format!("t.{}", BASE32_NOPAD.encode(&buf[..n]).to_lowercase())
}

/// Parse a tslot from `"t.<base32>"`.
pub fn from_tslot_text(s: &str) -> Result<Tslot, TslotTextError> {
    let body = s.strip_prefix("t.").ok_or(TslotTextError::MissingPrefix)?;
    let bytes = BASE32_NOPAD
        .decode(body.to_uppercase().as_bytes())
        .map_err(|_| TslotTextError::BadBase32)?;
    let v = decode_varint(&bytes).ok_or(TslotTextError::BadVarint)?;
    Ok(Tslot(v))
}

/// Tslot text errors.
#[derive(Debug, thiserror::Error)]
pub enum TslotTextError {
    /// String did not start with `t.`.
    #[error("missing 't.' prefix")]
    MissingPrefix,
    /// Body was not valid base32.
    #[error("bad base32 body")]
    BadBase32,
    /// Decoded bytes were not a valid LEB128 varint.
    #[error("bad varint")]
    BadVarint,
}

fn encode_varint(mut v: u64, out: &mut [u8; 10]) -> usize {
    let mut i = 0;
    while v >= 0x80 {
        out[i] = (v as u8) | 0x80;
        v >>= 7;
        i += 1;
    }
    out[i] = v as u8;
    i + 1
}

fn decode_varint(buf: &[u8]) -> Option<u64> {
    let mut v: u64 = 0;
    let mut shift = 0;
    for &b in buf {
        v |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for t in [0u64, 1, 26, 1024, 1_000_000, u64::MAX / 2] {
            let s = to_tslot_text(Tslot(t));
            assert_eq!(from_tslot_text(&s).unwrap(), Tslot(t), "{s}");
        }
    }
}
