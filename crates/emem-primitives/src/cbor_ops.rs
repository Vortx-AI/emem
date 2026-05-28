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

/// Strict RFC 3339 / ISO 8601 validator used for `as_of_signed_at`.
///
/// We do not need to convert to a calendrical type — every comparison
/// the bi-temporal filter performs is lexicographic against another
/// RFC 3339 string (`fact.signed_at`). What we DO need is to reject
/// malformed input with a clear error so an agent passing the wrong
/// format gets a 400, not a silent always-empty result.
///
/// Accepted shape: `YYYY-MM-DDTHH:MM:SS(.fff)?(Z|±HH:MM)`. The check
/// is deliberately permissive on the fractional-second precision so a
/// caller that pastes e.g. `2026-05-21T00:00:00.123456Z` from another
/// system still wins.
pub fn parse_rfc3339_strict(s: &str) -> Result<(), String> {
    // Bare minimum: `YYYY-MM-DDTHH:MM:SSZ` is 20 chars; with offset it's
    // 25; with sub-second it can be longer. Anything shorter than 20 is
    // structurally impossible to satisfy.
    if s.len() < 20 {
        return Err(format!(
            "as_of_signed_at must be RFC 3339 (e.g. `2026-05-21T00:00:00Z`); got `{s}` (too short)"
        ));
    }
    let b = s.as_bytes();
    // YYYY-MM-DDT
    let is_d = |i: usize| b[i].is_ascii_digit();
    if !(is_d(0)
        && is_d(1)
        && is_d(2)
        && is_d(3)
        && b[4] == b'-'
        && is_d(5)
        && is_d(6)
        && b[7] == b'-'
        && is_d(8)
        && is_d(9)
        && (b[10] == b'T' || b[10] == b't' || b[10] == b' ')
        && is_d(11)
        && is_d(12)
        && b[13] == b':'
        && is_d(14)
        && is_d(15)
        && b[16] == b':'
        && is_d(17)
        && is_d(18))
    {
        return Err(format!(
            "as_of_signed_at must be RFC 3339 (e.g. `2026-05-21T00:00:00Z`); got `{s}` (bad date/time skeleton)"
        ));
    }
    // Validate calendar / clock ranges to reject obvious garbage like
    // `2026-13-32T25:99:99Z`. Bi-temporal filtering relies on the input
    // being lexically order-comparable; a syntactically valid but
    // calendrically impossible value would lex-sort to a misleading
    // spot.
    let parse_n = |start: usize, len: usize| -> u32 {
        let mut v = 0u32;
        for i in 0..len {
            v = v * 10 + (b[start + i] - b'0') as u32;
        }
        v
    };
    let month = parse_n(5, 2);
    let day = parse_n(8, 2);
    let hour = parse_n(11, 2);
    let min = parse_n(14, 2);
    let sec = parse_n(17, 2);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || min > 59 || sec > 60
    // RFC 3339 allows leap seconds (60).
    {
        return Err(format!(
            "as_of_signed_at must be RFC 3339 (e.g. `2026-05-21T00:00:00Z`); got `{s}` (out-of-range component)"
        ));
    }
    // Optional fractional seconds: `.ddd…`.
    let mut i = 19usize;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == frac_start {
            return Err(format!(
                "as_of_signed_at must be RFC 3339; got `{s}` (empty fractional seconds after `.`)"
            ));
        }
    }
    // Required timezone: `Z` or `±HH:MM`.
    if i >= b.len() {
        return Err(format!(
            "as_of_signed_at must end with a timezone (`Z` or `±HH:MM`); got `{s}`"
        ));
    }
    match b[i] {
        b'Z' | b'z' => {
            if i + 1 != b.len() {
                return Err(format!(
                    "as_of_signed_at has trailing chars after `Z`; got `{s}`"
                ));
            }
        }
        b'+' | b'-' => {
            if i + 6 != b.len()
                || !is_d(i + 1)
                || !is_d(i + 2)
                || b[i + 3] != b':'
                || !is_d(i + 4)
                || !is_d(i + 5)
            {
                return Err(format!(
                    "as_of_signed_at timezone offset must be `±HH:MM`; got `{s}`"
                ));
            }
            let tz_h = parse_n(i + 1, 2);
            let tz_m = parse_n(i + 4, 2);
            if tz_h > 14 || tz_m > 59 {
                return Err(format!(
                    "as_of_signed_at timezone offset out of range; got `{s}`"
                ));
            }
        }
        _ => {
            return Err(format!(
                "as_of_signed_at must end with `Z` or `±HH:MM`; got `{s}`"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod rfc3339_tests {
    use super::parse_rfc3339_strict;

    #[test]
    fn parses_z_suffixed() {
        assert!(parse_rfc3339_strict("2026-05-21T00:00:00Z").is_ok());
        assert!(parse_rfc3339_strict("2026-05-21T00:00:00.123Z").is_ok());
        assert!(parse_rfc3339_strict("2026-05-21T00:00:00.123456Z").is_ok());
    }

    #[test]
    fn parses_offset_suffix() {
        assert!(parse_rfc3339_strict("2026-05-21T00:00:00+05:30").is_ok());
        assert!(parse_rfc3339_strict("2026-05-21T00:00:00-08:00").is_ok());
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse_rfc3339_strict("2026-05-21").is_err());
        assert!(parse_rfc3339_strict("2026-13-21T00:00:00Z").is_err());
        assert!(parse_rfc3339_strict("2026-05-21T25:00:00Z").is_err());
        assert!(parse_rfc3339_strict("2026-05-21T00:00:00").is_err());
        assert!(parse_rfc3339_strict("yesterday").is_err());
    }
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
