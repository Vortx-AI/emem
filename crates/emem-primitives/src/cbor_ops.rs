//! Helpers for evaluating ops and similarity over `ciborium::Value`.

/// Coerce a CBOR value to f64. Integers and floats round-trip; everything
/// else returns None.
pub fn as_f64(v: &ciborium::Value) -> Option<f64> {
    match v {
        ciborium::Value::Integer(i) => i128::from(*i).try_into().ok().map(|x: i64| x as f64),
        ciborium::Value::Float(f) => Some(*f),
        _ => None,
    }
}

/// Coerce a CBOR value to a Vec<f32> if it is an array of numbers; else None.
pub fn as_vec_f32(v: &ciborium::Value) -> Option<Vec<f32>> {
    match v {
        ciborium::Value::Array(a) => {
            let mut out = Vec::with_capacity(a.len());
            for x in a {
                let f = as_f64(x)?;
                out.push(f as f32);
            }
            Some(out)
        }
        _ => None,
    }
}

/// Equality over the subset of CBOR types the protocol actually compares:
/// numbers, strings, bools, byte strings.
pub fn eq(a: &ciborium::Value, b: &ciborium::Value) -> bool {
    if let (Some(fa), Some(fb)) = (as_f64(a), as_f64(b)) {
        return fa == fb;
    }
    match (a, b) {
        (ciborium::Value::Text(sa), ciborium::Value::Text(sb)) => sa == sb,
        (ciborium::Value::Bool(ba), ciborium::Value::Bool(bb)) => ba == bb,
        (ciborium::Value::Bytes(ba), ciborium::Value::Bytes(bb)) => ba == bb,
        _ => false,
    }
}

/// Numeric ordering. Returns `None` when either side is not numeric *or*
/// either side is NaN — NaN comparisons are undefined, not "false", and
/// the caller should treat undefined as a distinct outcome (e.g. a
/// verify verdict of "incomparable" rather than "claim does not hold").
pub fn lt(a: &ciborium::Value, b: &ciborium::Value) -> Option<bool> {
    match (as_f64(a), as_f64(b)) {
        (Some(x), Some(y)) if !x.is_nan() && !y.is_nan() => Some(x < y),
        _ => None,
    }
}

/// Cosine similarity between two equal-length f32 vectors. Returns 0.0
/// when either vector is the zero vector. Length is taken as the min so
/// callers can compare slices of different lengths safely.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mut dot: f64 = 0.0;
    let mut na: f64 = 0.0;
    let mut nb: f64 = 0.0;
    for i in 0..n {
        let x = a[i] as f64;
        let y = b[i] as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())) as f32
}

/// Wrap an f32 into a CBOR Float for response values.
pub fn f32_to_cbor(x: f32) -> ciborium::Value {
    ciborium::Value::Float(x as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_orthogonal_is_zero() {
        let a = [1.0_f32, 0.0];
        let b = [0.0_f32, 1.0];
        assert!((cosine(&a, &b)).abs() < 1e-6);
    }
    #[test]
    fn cosine_identical_is_one() {
        let a = vec![0.3_f32, 0.4, 0.5];
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-6);
    }
}
