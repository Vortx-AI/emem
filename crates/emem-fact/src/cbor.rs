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
pub fn to_canonical_cbor<T: serde::Serialize>(
    v: &T,
) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(v, &mut buf)?;
    Ok(buf)
}

/// Map an IEEE-754 double to its canonical representative so that
/// content-addressing is a function of the numeric *value*, not a
/// transient bit pattern:
///
/// - every NaN (any sign, any payload, quiet or signalling) collapses to
///   the one canonical quiet NaN, which `ciborium` emits as the two-byte
///   `f9 7e00` per RFC 8949 §4.2.2;
/// - negative zero collapses to positive zero (they compare equal but
///   encode as `f9 8000` vs `f9 0000`).
///
/// Every other value is returned unchanged. `ciborium` already emits the
/// shortest round-tripping form for finite values, so those are already
/// value-canonical; this closes only the two degenerate cases where the
/// encoder would otherwise preserve a distinguishable bit pattern (B8).
pub fn canonical_float(f: f64) -> f64 {
    if f.is_nan() {
        // `f64::NAN` is the canonical quiet NaN (0x7ff8_0000_0000_0000).
        f64::NAN
    } else if f == 0.0 {
        // `-0.0 == 0.0` is true, so this folds negative zero to +0.0.
        0.0
    } else {
        f
    }
}

/// Recursively canonicalize every float embedded in a `ciborium::Value`
/// (see [`canonical_float`]). Walks arrays, maps (keys and values), and
/// tag payloads. Idempotent — canonical input is returned unchanged.
pub fn canonicalize_value(v: &mut ciborium::Value) {
    match v {
        ciborium::Value::Float(f) => *f = canonical_float(*f),
        ciborium::Value::Array(items) => {
            for item in items.iter_mut() {
                canonicalize_value(item);
            }
        }
        ciborium::Value::Map(entries) => {
            for (k, val) in entries.iter_mut() {
                canonicalize_value(k);
                canonicalize_value(val);
            }
        }
        ciborium::Value::Tag(_, inner) => canonicalize_value(inner),
        _ => {}
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(v: &ciborium::Value) -> Vec<u8> {
        to_canonical_cbor(v).unwrap()
    }

    #[test]
    fn every_nan_payload_encodes_to_the_canonical_two_byte_nan() {
        // The canonical quiet NaN is the two-byte f16 `f9 7e00` (RFC 8949
        // §4.2.2). A non-standard payload NaN otherwise stays a distinct
        // 9-byte f64 — the B8 hole. After canonicalization both collapse.
        let canonical = [0xf9, 0x7e, 0x00];
        for bits in [
            0x7ff8_0000_0000_0000u64, // canonical quiet NaN
            0x7ff8_0000_0000_0001,    // quiet NaN, non-zero payload
            0xfff8_0000_0000_0000,    // sign-bit-set NaN
            0x7ff0_0000_0000_0001,    // signalling NaN
        ] {
            let raw = f64::from_bits(bits);
            assert!(raw.is_nan());
            let mut v = ciborium::Value::Float(raw);
            canonicalize_value(&mut v);
            assert_eq!(
                enc(&v),
                canonical,
                "NaN bits {bits:#018x} must canonicalize to f9 7e00"
            );
        }
    }

    #[test]
    fn negative_zero_folds_to_positive_zero() {
        let mut neg = ciborium::Value::Float(-0.0);
        canonicalize_value(&mut neg);
        assert_eq!(enc(&neg), enc(&ciborium::Value::Float(0.0)));
        assert_eq!(enc(&neg), [0xf9, 0x00, 0x00]);
    }

    #[test]
    fn finite_values_are_untouched() {
        // Canonicalization must be a no-op on ordinary measurements, from
        // either an f32 or f64 origin — content addresses of real facts do
        // not move.
        for x in [918.0f64, -273.15, 0.7421, 1e-9, 6.022e23, f64::from(0.1f32)] {
            let before = enc(&ciborium::Value::Float(x));
            let mut v = ciborium::Value::Float(x);
            canonicalize_value(&mut v);
            assert_eq!(enc(&v), before, "finite value {x} must be unchanged");
        }
    }

    #[test]
    fn canonicalization_recurses_and_is_idempotent() {
        let mut v = ciborium::Value::Array(vec![
            ciborium::Value::Float(-0.0),
            ciborium::Value::Map(vec![(
                ciborium::Value::Text("k".into()),
                ciborium::Value::Float(f64::from_bits(0x7ff8_0000_0000_0001)),
            )]),
            ciborium::Value::Integer(5.into()),
        ]);
        canonicalize_value(&mut v);
        let once = enc(&v);
        canonicalize_value(&mut v);
        assert_eq!(enc(&v), once, "canonicalize_value must be idempotent");
    }
}
