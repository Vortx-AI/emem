//! `memory_contradictions(prefix?, band?, window?, limit?, min_severity?)` —
//! flag every `(cell, band, tslot)` triple where two or more distinct
//! attesters have signed disagreeing observations.
//!
//! emem's facts carry an attester pubkey, a content-addressed CID and
//! a wall-clock `signed_at`. The canonical index is keyed by
//! `(cell, band, tslot)` and is last-write-wins: a second attester
//! writing to the same key silently replaces the first. The
//! `multi_attester_index` sled tree (populated by
//! `MaterializingStorage::put_attestation`) preserves every distinct
//! CID at that key. This primitive scans that tree, hydrates the
//! facts, and computes a per-band severity score over the disagreeing
//! attestations.
//!
//! The contradiction report itself is signed: `cells` carries every
//! unique cell64 cited, `fact_cids` carries every cited fact CID, and
//! the receipt verifies offline against the responder's epoch pubkey
//! via `/v1/verify_receipt`.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_core::{bands::BandRegistry, ErrorCode};
// `bands::DEFAULT` is the process-wide cached registry; we borrow it
// instead of re-parsing the embedded manifest on every call.
use emem_core::bands::DEFAULT as BAND_DEFAULTS;
use emem_fact::{Fact, FactCid, Receipt};
use emem_storage::{Server, StorageError};

use crate::cbor_ops::{as_f64, as_vec_f32, cosine, eq};

/// Default cap on the number of multi-attester keys scanned per call.
/// Override at runtime with `EMEM_CONTRADICTIONS_SCAN_CAP`.
const DEFAULT_SCAN_CAP: usize = 50_000;
/// Default cap on the number of contradictions returned.
const DEFAULT_LIMIT: usize = 100;
/// Hard upper bound on `limit` regardless of caller value.
const MAX_LIMIT: usize = 1_000;
/// Default severity threshold below which a multi-attester key is
/// scanned but not surfaced as a contradiction.
const DEFAULT_MIN_SEVERITY: f32 = 0.1;

/// Request: where to scan, which band, severity threshold.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContradictionsReq {
    /// Bytewise prefix on the cell64 string. `None` scans the whole
    /// multi-attester index up to the scan cap. Examples:
    /// `"defi"` (continent-class), `"defi.zb5"` (region).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_prefix: Option<String>,
    /// Filter by band key. `None` includes all bands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    /// Inclusive `[lo, hi]` filter on the disagreeing attestations'
    /// `signed_at` Unix-seconds value. When set, a contradiction is
    /// surfaced only if every disagreeing attestation falls in that
    /// window — narrow it to recent activity for cheap diffs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_unix_s: Option<[u64; 2]>,
    /// Maximum contradictions to return. Defaults to 100, capped at
    /// 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// `0.0` = report every disagreement, `1.0` = only flagrant. Default
    /// `0.1`. Severity is computed per band kind — see
    /// [`compute_severity`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_severity: Option<f32>,
}

/// One disagreeing attester's contribution to a contradiction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttesterDisagreement {
    /// 32-byte ed25519 pubkey, base32-nopad-lc rendered. Stable
    /// fingerprint of the attester across receipts.
    pub attester_pubkey_b32: String,
    /// Content-addressed CID of the signed fact (base32-nopad-lc).
    pub fact_cid: String,
    /// The disputed value, rendered into JSON. Scalars render as
    /// numbers / strings; vector embeddings render as arrays of f64;
    /// categorical IDs render as integers.
    pub value: serde_json::Value,
    /// Per-fact confidence in [0, 1] as declared by the attester.
    pub confidence: f32,
    /// ISO 8601 wall-clock at signing time.
    pub signed_at: String,
}

/// One contradiction record — a single `(cell, band, tslot)` triple
/// with N ≥ 2 distinct attester opinions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contradiction {
    /// cell64.
    pub cell: String,
    /// Band key (e.g. `"indices.ndvi"`).
    pub band: String,
    /// Time slot. Same scale and origin as the canonical index.
    pub tslot: u64,
    /// One entry per disagreeing attester.
    pub attestations: Vec<AttesterDisagreement>,
    /// Quantitative measure of disagreement in [0, 1]. See
    /// [`compute_severity`] for per-band-kind semantics.
    pub severity: f32,
    /// `"scalar" | "vector" | "categorical" | "mixed"`. Reflects the
    /// shape the severity scorer matched.
    pub kind: String,
}

/// Response — list of contradictions, scan accounting, signed receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionsResp {
    /// Contradictions ordered by descending severity. Empty when the
    /// scanned-key set holds no disagreements above `min_severity`.
    pub contradictions: Vec<Contradiction>,
    /// How many multi-attester keys were actually walked. `0` when
    /// the corpus has no multi-attester entries yet — that is honest
    /// "nothing to contradict" rather than "lookup failed".
    pub corpus_scanned: usize,
    /// True when the hydration budget was spent before every scanned key
    /// could be examined, so `contradictions` is a partial view. A whole-
    /// corpus scan hydrates facts per key straight off sled; pass a `band`
    /// filter (or a tighter `cell_prefix`) for complete, fast coverage.
    #[serde(default)]
    pub scan_truncated: bool,
    /// How long the primitive took to run, milliseconds.
    pub time_taken_ms: u64,
    /// One-paragraph agent-facing summary explaining what to do next.
    pub agent_hint: String,
    /// Signed receipt — `cells` covers every unique cell cited in
    /// `contradictions`, `fact_cids` covers every cited fact CID.
    pub receipt: Receipt,
}

/// Scan the multi-attester index, score every multi-attester key, and
/// return the contradictions that meet `min_severity`.
pub async fn memory_contradictions(
    req: &ContradictionsReq,
    srv: &Server,
) -> Result<ContradictionsResp, StorageError> {
    let started = Instant::now();
    let scan_cap = std::env::var("EMEM_CONTRADICTIONS_SCAN_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SCAN_CAP);
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let min_severity = req
        .min_severity
        .unwrap_or(DEFAULT_MIN_SEVERITY)
        .clamp(0.0, 1.0);
    // Wall-clock budget for the per-key fact hydration below. Kept well
    // under the HTTP dispatch cap so an unfiltered whole-corpus scan
    // returns a partial-but-signed result instead of hanging the request.
    let time_budget_ms: u128 = std::env::var("EMEM_CONTRADICTIONS_TIME_BUDGET_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8_000);
    let mut scan_truncated = false;

    let storage = srv.storage.as_ref();
    let multi = storage
        .scan_multi_attester(req.cell_prefix.as_deref(), scan_cap)
        .await?;
    let corpus_scanned = multi.len();

    let bands: &BandRegistry = &BAND_DEFAULTS;

    let mut contradictions: Vec<Contradiction> = Vec::new();
    for (key, cids) in &multi {
        if let Some(ref b) = req.band {
            if key.band != *b {
                continue;
            }
        }
        if cids.len() < 2 {
            continue;
        }
        // Bounded hydration. `get_facts_many` reads sled per key, so an
        // unfiltered whole-corpus scan can fan out to tens of thousands of
        // reads and blow the dispatch cap (the origin of the broad-scan
        // 500). Once the budget is spent, stop and report the result as
        // partial rather than hanging. A `band` filter skips non-matching
        // keys above, so filtered scans effectively never reach this.
        if started.elapsed().as_millis() > time_budget_ms {
            scan_truncated = true;
            break;
        }
        // Hydrate facts. A multi-attester key might point at CIDs the
        // hot cache has since lost (eviction policy — Hot tier is 30
        // days). Drop any None so the scorer only sees live facts. We
        // still require ≥ 2 live facts to call this a contradiction.
        let facts: Vec<Fact> = storage
            .get_facts_many(cids)
            .await?
            .into_iter()
            .flatten()
            .collect();
        if facts.len() < 2 {
            continue;
        }

        // Discard non-Primary variants — Absence facts at the same
        // key trivially "agree" (both assert no value); Derivative
        // facts have no canonical key and shouldn't be here at all.
        let primaries: Vec<&emem_fact::PrimaryFact> = facts
            .iter()
            .filter_map(|f| match f {
                Fact::Primary(p) => Some(p),
                _ => None,
            })
            .collect();
        if primaries.len() < 2 {
            continue;
        }

        // Distinct attesters check — if the same attester re-signed
        // the same triple with two different values (rare; a content
        // mistake, not a disagreement), we still flag it. But two
        // attesters with identical pubkeys and identical values are
        // not a contradiction even if their CIDs differ (would only
        // happen on a CBOR canonicalisation drift, which is itself
        // worth surfacing — but as `CanonicalEncodingDivergence`,
        // not a contradiction).
        let attester_set: BTreeSet<[u8; 32]> = primaries.iter().map(|p| p.signer.0).collect();
        // Window filter: every attestation must fall in [lo, hi]
        // (interpreted as Unix-seconds parsed from `signed_at`).
        if let Some([lo, hi]) = req.window_unix_s {
            let all_in_window = primaries.iter().all(|p| {
                iso8601_to_unix_s(&p.signed_at)
                    .map(|t| t >= lo && t <= hi)
                    .unwrap_or(false)
            });
            if !all_in_window {
                continue;
            }
        }

        let (severity, kind) = compute_severity(&primaries, bands, &key.band);
        // Perfect agreement is never a contradiction — drop it even at
        // `min_severity=0.0` so an agent scanning for "anything
        // disagreed" doesn't get back a wall of identical values
        // signed by two attesters.
        if severity == 0.0 {
            continue;
        }
        if attester_set.len() < 2 {
            // Only one attester involved — same signer re-attested
            // with two different values. That's a content mistake,
            // not a multi-attester contradiction. Drop unless the
            // caller asked for severity=0 (impossible — already
            // filtered above). Future versions may surface these as a
            // separate "same-attester drift" record.
            continue;
        }
        if severity < min_severity {
            continue;
        }

        let attestations: Vec<AttesterDisagreement> = primaries
            .iter()
            .zip(
                facts
                    .iter()
                    .filter(|f| matches!(f, Fact::Primary(_)))
                    .zip(cids.iter()),
            )
            .map(|(p, (_, cid))| AttesterDisagreement {
                attester_pubkey_b32: data_encoding::BASE32_NOPAD
                    .encode(&p.signer.0)
                    .to_lowercase(),
                fact_cid: cid.as_str().to_string(),
                value: cbor_to_json(&p.value),
                confidence: p.confidence,
                signed_at: p.signed_at.clone(),
            })
            .collect();

        contradictions.push(Contradiction {
            cell: key.cell.clone(),
            band: key.band.clone(),
            tslot: key.tslot,
            attestations,
            severity,
            kind,
        });
    }

    // Highest severity first; ties broken by cell+band for determinism.
    contradictions.sort_by(|a, b| {
        b.severity
            .partial_cmp(&a.severity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cell.cmp(&b.cell))
            .then_with(|| a.band.cmp(&b.band))
            .then_with(|| a.tslot.cmp(&b.tslot))
    });
    contradictions.truncate(limit);

    // Build receipt citations: every unique cell + every cited CID.
    let mut cells_set: BTreeSet<String> = BTreeSet::new();
    let mut fact_cids: Vec<FactCid> = Vec::new();
    for c in &contradictions {
        cells_set.insert(c.cell.clone());
        for a in &c.attestations {
            fact_cids.push(FactCid::new(&a.fact_cid));
        }
    }
    let cells: Vec<String> = cells_set.into_iter().collect();

    let elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;

    let agent_hint = build_agent_hint(
        &contradictions,
        corpus_scanned,
        scan_cap,
        req.cell_prefix.as_deref(),
        req.band.as_deref(),
        scan_truncated,
    );

    let receipt = srv.sign_receipt(
        "emem.memory_contradictions",
        cells,
        fact_cids,
        true,
        started,
        None,
    );

    Ok(ContradictionsResp {
        contradictions,
        corpus_scanned,
        scan_truncated,
        time_taken_ms: elapsed_ms,
        agent_hint,
        receipt,
    })
}

/// Compute the disagreement severity in [0, 1] over a slice of
/// `PrimaryFact` opinions at the same `(cell, band, tslot)`.
///
/// Branches by value shape:
///
/// - Scalar (all values coerce to `f64`): `severity = (max - min) /
///   band_range`, clamped to [0, 1]. The band's documented range is
///   read from the registry; when the band has no `value_range`
///   declared we fall back to the value spread itself (giving 1.0
///   when there's any disagreement, 0.0 when they all agree).
/// - Vector (all values coerce to `Vec<f32>`): severity is `1 - mean`
///   of the off-diagonal cosine-similarity matrix. Identical
///   embeddings score 0.0; orthogonal score 1.0; anti-correlated score
///   near 1.0 (cosine ∈ [-1, 1]).
/// - Categorical (all values are integers / strings that round-trip
///   through `eq`): mode-disagreement, `severity = 1 -
///   max_class_share`. 4 attesters say `50` and 1 says `10` →
///   `1 - 4/5 = 0.2`.
/// - Mixed shapes: severity = 1.0, kind = `"mixed"`. Flagged for
///   human review.
pub fn compute_severity(
    primaries: &[&emem_fact::PrimaryFact],
    bands: &BandRegistry,
    band_key: &str,
) -> (f32, String) {
    if primaries.len() < 2 {
        return (0.0, "scalar".into());
    }
    let values: Vec<&ciborium::Value> = primaries.iter().map(|p| &p.value).collect();

    // Try numeric scalar first.
    let scalars: Option<Vec<f64>> = values.iter().map(|v| as_f64(v)).collect();
    if let Some(nums) = scalars {
        let (lo, hi) = nums
            .iter()
            .copied()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                (lo.min(x), hi.max(x))
            });
        let spread = (hi - lo).max(0.0);
        // First-pass: see if the band looks categorical (small integer
        // domain like ESA WorldCover class IDs). When EVERY value is a
        // non-negative integer and the band registry's value_range
        // either spans a small enum or is absent, route through the
        // mode-disagreement scorer — `(max - min)/range` would
        // silently flatten a 50/10/95 disagreement to a small number,
        // which is the wrong shape for a discrete class.
        let all_int = nums.iter().all(|x| x.fract() == 0.0 && *x >= 0.0);
        if all_int && looks_categorical(band_key) {
            return (mode_disagreement(&values), "categorical".into());
        }
        let band_range = band_range_for(bands, band_key).unwrap_or(spread.max(1.0));
        if band_range <= 0.0 {
            return (if spread > 0.0 { 1.0 } else { 0.0 }, "scalar".into());
        }
        let s = (spread / band_range).clamp(0.0, 1.0) as f32;
        return (s, "scalar".into());
    }

    // Vector path — every value is an Array of numbers.
    let vectors: Option<Vec<Vec<f32>>> = values.iter().map(|v| as_vec_f32(v)).collect();
    if let Some(vecs) = vectors {
        if vecs.iter().all(|v| !v.is_empty()) {
            // Pairwise cosine on the (i, j) upper triangle; severity is
            // `1 - mean(cosine)`. Clamp to [0, 1] (anti-correlated
            // vectors can push raw severity past 1.0).
            let mut sum: f64 = 0.0;
            let mut count: usize = 0;
            for i in 0..vecs.len() {
                for j in (i + 1)..vecs.len() {
                    sum += cosine(&vecs[i], &vecs[j]) as f64;
                    count += 1;
                }
            }
            if count == 0 {
                return (0.0, "vector".into());
            }
            let mean_cos = sum / count as f64;
            let sev = (1.0 - mean_cos).clamp(0.0, 1.0) as f32;
            return (sev, "vector".into());
        }
    }

    // Categorical / text fallback.
    if values.iter().all(|v| {
        matches!(
            v,
            ciborium::Value::Text(_) | ciborium::Value::Integer(_) | ciborium::Value::Bytes(_)
        )
    }) {
        return (mode_disagreement(&values), "categorical".into());
    }

    // Mixed shapes — surface as flagrant.
    (1.0, "mixed".into())
}

/// Mode-share disagreement: 1 - (size_of_largest_equivalence_class /
/// total). Two attesters agreeing on class 50 and three on class 10
/// gives `1 - 3/5 = 0.4`.
fn mode_disagreement(values: &[&ciborium::Value]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    // O(n²) equivalence over the protocol's `eq` (numbers, strings,
    // bools, byte strings). With n ≤ a few attesters per key in
    // practice this is fine.
    let mut counts: Vec<usize> = vec![0; values.len()];
    for (i, vi) in values.iter().enumerate() {
        for vj in values.iter() {
            if eq(vi, vj) {
                counts[i] += 1;
            }
        }
    }
    let max = counts.iter().copied().max().unwrap_or(1);
    let share = max as f32 / values.len() as f32;
    (1.0 - share).clamp(0.0, 1.0)
}

/// Look up a documented scalar range `[min, max]` for `band_key` in
/// the band registry. Returns `Some(max - min)` for both 1-D scalar
/// bands and per-dimension scalar suffixes (e.g.
/// `weather.temperature_2m_mean`). Falls back to `None` when the band
/// is multi-dim with no top-level range — the caller can substitute
/// the observed spread.
fn band_range_for(bands: &BandRegistry, band_key: &str) -> Option<f64> {
    if let Some(b) = bands.lookup(band_key) {
        if let Some(range) = parse_value_range(&b.value_range) {
            return Some(range);
        }
    }
    // Dotted suffix path — `indices.ndvi`, `copdem30m.elevation_mean`.
    // The dotted scalar is itself not a registry band today; surface
    // the per-dimension range when the suffix matches a
    // dimensions[].name on a registered family band.
    if let Some((family, suffix)) = band_key.split_once('.') {
        if let Some(b) = bands.lookup(family) {
            if let Some(dim) = b.dimensions.iter().find(|d| d.name == suffix) {
                if let Some(range) = parse_value_range(&dim.value_range) {
                    return Some(range);
                }
            }
        }
    }
    None
}

/// Parse a `value_range` JSON expression of the form `[lo, hi]` into
/// `hi - lo`. `None` for missing / malformed / zero-width entries.
fn parse_value_range(v: &Option<serde_json::Value>) -> Option<f64> {
    let arr = v.as_ref()?.as_array()?;
    if arr.len() != 2 {
        return None;
    }
    let lo = arr[0].as_f64()?;
    let hi = arr[1].as_f64()?;
    let r = hi - lo;
    if r > 0.0 {
        Some(r)
    } else {
        None
    }
}

/// Heuristic: bands whose values are class IDs rather than continuous
/// scalars. Hard-coded for the categorical bands ship today; new
/// categorical bands should append here.
fn looks_categorical(band_key: &str) -> bool {
    matches!(
        band_key,
        "esa_worldcover.lc_2021" | "landcover" | "surface_water.transition_class" | "s2.scl"
    )
}

/// Render a CBOR value as JSON for the wire response. Mirrors the
/// subset of the protocol's `eq` semantics: numbers, strings, bools,
/// byte strings, arrays of any of the above. Unknown variants
/// degrade to `null` rather than failing — the rest of the
/// contradiction record (CIDs, attester pubkeys, severity) is the
/// load-bearing payload.
fn cbor_to_json(v: &ciborium::Value) -> serde_json::Value {
    match v {
        ciborium::Value::Integer(i) => {
            let n: i128 = (*i).into();
            if let Ok(n64) = i64::try_from(n) {
                serde_json::Value::Number(n64.into())
            } else {
                serde_json::Value::String(n.to_string())
            }
        }
        ciborium::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        ciborium::Value::Text(s) => serde_json::Value::String(s.clone()),
        ciborium::Value::Bool(b) => serde_json::Value::Bool(*b),
        ciborium::Value::Null => serde_json::Value::Null,
        ciborium::Value::Bytes(b) => {
            serde_json::Value::String(data_encoding::BASE32_NOPAD.encode(b).to_lowercase())
        }
        ciborium::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(cbor_to_json).collect())
        }
        ciborium::Value::Map(m) => {
            let mut out = serde_json::Map::new();
            for (k, vv) in m {
                let key = match k {
                    ciborium::Value::Text(s) => s.clone(),
                    ciborium::Value::Integer(i) => i128::from(*i).to_string(),
                    _ => continue,
                };
                out.insert(key, cbor_to_json(vv));
            }
            serde_json::Value::Object(out)
        }
        _ => serde_json::Value::Null,
    }
}

/// Parse an ISO-8601 timestamp like `2026-05-28T13:55:07Z` to Unix
/// seconds. We accept only the canonical RFC 3339 `Z` form that the
/// rest of the protocol emits via `iso8601_now()`; offsets and
/// fractional seconds are not used by attesters.
fn iso8601_to_unix_s(s: &str) -> Option<u64> {
    // 1234-12-29T23:59:59Z
    if s.len() < 20 || !s.ends_with('Z') {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let mo: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    let hh: u32 = s.get(11..13)?.parse().ok()?;
    let mm: u32 = s.get(14..16)?.parse().ok()?;
    let ss: u32 = s.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || hh >= 24 || mm >= 60 || ss >= 60 {
        return None;
    }
    // Howard Hinnant days-from-civil; inverse of the same algorithm
    // used in `server::iso8601_from_unix`.
    let m = mo as i64;
    let y_civil = if m <= 2 { y - 1 } else { y };
    let era = if y_civil >= 0 { y_civil } else { y_civil - 399 } / 400;
    let yoe = y_civil - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hh as i64 * 3_600 + mm as i64 * 60 + ss as i64;
    u64::try_from(secs).ok()
}

/// Compose the agent-facing hint paragraph. Tells the caller exactly
/// what was scanned and what to do with the result.
fn build_agent_hint(
    contradictions: &[Contradiction],
    corpus_scanned: usize,
    scan_cap: usize,
    cell_prefix: Option<&str>,
    band: Option<&str>,
    truncated: bool,
) -> String {
    let trunc = if truncated {
        " NOTE: the hydration budget was spent before every key was examined, so this is a PARTIAL view; pass a `band` filter (or a tighter `cell_prefix`) for complete, fast coverage."
    } else {
        ""
    };
    let mut filter = String::new();
    if let Some(p) = cell_prefix {
        filter.push_str(&format!(" cell_prefix='{p}'"));
    }
    if let Some(b) = band {
        filter.push_str(&format!(" band='{b}'"));
    }
    if filter.is_empty() {
        filter.push_str(" (whole corpus)");
    }
    if contradictions.is_empty() {
        if corpus_scanned == 0 {
            return format!(
                "No multi-attester observations exist yet{filter}. Either the corpus is single-attester (every key has one signer) or no second attester has ever overlapped on the same (cell, band, tslot). This is honest 'no contradictions known', not 'lookup failed'."
            );
        }
        return format!(
            "Scanned {corpus_scanned} multi-attester keys{filter} (cap {scan_cap}); zero met the severity threshold. Tune `min_severity` down (e.g. 0.0) to see borderline disagreements; raise it (e.g. 0.5) for flagrant only.{trunc}"
        );
    }
    let n = contradictions.len();
    let top = &contradictions[0];
    format!(
        "Found {n} contradiction(s) above the severity floor; scanned {corpus_scanned} multi-attester keys{filter} (cap {scan_cap}). Highest severity: {sev:.3} at cell={cell} band={band} tslot={tslot} kind={kind} with {att_n} disagreeing attester(s). Next steps: (a) call POST /v1/diff with two of these fact_cids to compare them numerically, (b) call POST /v1/verify_receipt on any cited fact_cid to confirm the signature offline, (c) call GET /v1/facts/<fact_cid> to fetch the full signed payload of any disputed observation.{trunc}",
        n = n,
        corpus_scanned = corpus_scanned,
        scan_cap = scan_cap,
        sev = top.severity,
        cell = top.cell,
        band = top.band,
        tslot = top.tslot,
        kind = top.kind,
        att_n = top.attestations.len(),
        filter = filter,
    )
}

/// Convenience wrapper for callers (REST handlers) that already
/// validated the request shape.
pub fn validate_request(req: &ContradictionsReq) -> Result<(), StorageError> {
    if let Some(w) = req.window_unix_s {
        if w[0] > w[1] {
            return Err(StorageError::Protocol {
                code: ErrorCode::InvalidArgument,
                message: format!(
                    "memory_contradictions: window_unix_s must be [lo, hi] with lo <= hi; got [{}, {}]",
                    w[0], w[1]
                ),
            });
        }
    }
    if let Some(s) = req.min_severity {
        if !(0.0..=1.0).contains(&s) {
            return Err(StorageError::Protocol {
                code: ErrorCode::InvalidArgument,
                message: format!("memory_contradictions: min_severity must be in [0, 1]; got {s}"),
            });
        }
    }
    // band_metadata lookup is best-effort so we don't fail on unknown
    // bands here — agents may scan over experimental bands.
    let _ = HashMap::<String, ()>::new();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use async_trait::async_trait;
    use ciborium::Value as CborValue;

    use emem_cache::CanonicalKey;
    use emem_core::AttesterKey;
    use emem_fact::{
        Attestation, Derivation, Fact, FactCid, PrimaryFact, RegistryCid, SchemaCid, Source,
    };
    use emem_storage::server::{ManifestCids, ResponderIdentity};
    use emem_storage::{Server, Storage, StorageError};

    /// Minimal `Storage` impl with a hand-rolled multi-attester index
    /// for tests. Lets us exercise the contradictions primitive
    /// without bringing up the full `MaterializingStorage`.
    struct MockMultiStorage {
        cid_to_fact: std::sync::Mutex<std::collections::HashMap<String, Fact>>,
        multi: std::sync::Mutex<Vec<(CanonicalKey, Vec<FactCid>)>>,
    }

    impl MockMultiStorage {
        fn new() -> Self {
            Self {
                cid_to_fact: std::sync::Mutex::new(Default::default()),
                multi: std::sync::Mutex::new(Vec::new()),
            }
        }

        #[allow(clippy::too_many_arguments)]
        fn insert_attestation(
            &self,
            cell: &str,
            band: &str,
            tslot: u64,
            signer: [u8; 32],
            value: CborValue,
            confidence: f32,
            signed_at: &str,
        ) -> FactCid {
            let cid_str = format!(
                "test-cid-{}-{}-{}-{}-{}",
                cell,
                band,
                tslot,
                hex(&signer[..4]),
                signed_at
            );
            let cid = FactCid::new(&cid_str);
            let fact = Fact::Primary(PrimaryFact {
                cell: cell.into(),
                band: band.into(),
                tslot,
                value,
                unit: None,
                confidence,
                uncertainty: None,
                sources: vec![Source {
                    scheme: "test".into(),
                    id: cid_str.clone(),
                    cid: None,
                    hash: None,
                    captured_at: None,
                    url: None,
                }],
                derivation: Derivation {
                    fn_key: "test@1".into(),
                    args: None,
                },
                privacy_class: "public".into(),
                schema_cid: SchemaCid::new("test"),
                signer: AttesterKey(signer),
                signed_at: signed_at.into(),
                served_via: None,
            });
            self.cid_to_fact
                .lock()
                .unwrap()
                .insert(cid_str.clone(), fact);
            let mut multi = self.multi.lock().unwrap();
            let key = CanonicalKey {
                cell: cell.into(),
                band: band.into(),
                tslot,
            };
            if let Some(slot) = multi.iter_mut().find(|(k, _)| k == &key) {
                if !slot.1.iter().any(|c| c.as_str() == cid_str) {
                    slot.1.push(cid.clone());
                }
            } else {
                multi.push((key, vec![cid.clone()]));
            }
            cid
        }
    }

    fn hex(b: &[u8]) -> String {
        let mut s = String::new();
        for x in b {
            s.push_str(&format!("{:02x}", x));
        }
        s
    }

    #[async_trait]
    impl Storage for MockMultiStorage {
        async fn lookup_canonical_many(
            &self,
            _keys: &[CanonicalKey],
        ) -> Result<Vec<Option<FactCid>>, StorageError> {
            unimplemented!("not used")
        }

        async fn get_facts_many(
            &self,
            cids: &[FactCid],
        ) -> Result<Vec<Option<Fact>>, StorageError> {
            let map = self.cid_to_fact.lock().unwrap();
            Ok(cids.iter().map(|c| map.get(c.as_str()).cloned()).collect())
        }

        async fn put_attestation(&self, _att: &Attestation) -> Result<Vec<FactCid>, StorageError> {
            unimplemented!("not used")
        }

        async fn materialize_many(
            &self,
            _keys: &[CanonicalKey],
        ) -> Result<Vec<FactCid>, StorageError> {
            unimplemented!("not used")
        }

        async fn scan_cell(
            &self,
            _cell: &str,
            _tslot: Option<u64>,
        ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
            Ok(Vec::new())
        }

        async fn iter_index(
            &self,
            _limit: Option<usize>,
        ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
            Ok(Vec::new())
        }

        async fn scan_multi_attester(
            &self,
            cell_prefix: Option<&str>,
            limit: usize,
        ) -> Result<Vec<(CanonicalKey, Vec<FactCid>)>, StorageError> {
            let mut out = Vec::new();
            for (k, v) in self.multi.lock().unwrap().iter() {
                if let Some(p) = cell_prefix {
                    if !k.cell.starts_with(p) {
                        continue;
                    }
                }
                if v.len() < 2 {
                    continue;
                }
                out.push((k.clone(), v.clone()));
                if out.len() >= limit {
                    break;
                }
            }
            Ok(out)
        }
    }

    fn test_server(storage: Arc<MockMultiStorage>) -> Server {
        Server {
            storage,
            identity: ResponderIdentity::fresh(),
            manifests: ManifestCids {
                registry_cid: RegistryCid::new("test-registry"),
                schema_cid: SchemaCid::new("test-schema"),
                bands_cid: "test-bands".into(),
                sources_cid: "test-sources".into(),
            },
            started_at_unix_s: 0,
        }
    }

    fn signer(b: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    /// Two attesters write disagreeing NDVI at the same (cell, band,
    /// tslot). One contradiction MUST surface with both CIDs cited.
    #[tokio::test]
    async fn ndvi_scalar_disagreement_surfaces() {
        let storage = Arc::new(MockMultiStorage::new());
        // Attester A says NDVI = 0.85 (dense vegetation)
        let cid_a = storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "indices.ndvi",
            12,
            signer(1),
            CborValue::Float(0.85),
            1.0,
            "2026-05-01T12:00:00Z",
        );
        // Attester B says NDVI = 0.10 (sparse / bare) at the SAME key
        let cid_b = storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "indices.ndvi",
            12,
            signer(2),
            CborValue::Float(0.10),
            0.9,
            "2026-05-02T12:00:00Z",
        );
        let srv = test_server(storage);
        let resp = memory_contradictions(
            &ContradictionsReq {
                min_severity: Some(0.0),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("memory_contradictions ok");
        assert_eq!(resp.contradictions.len(), 1, "exactly one contradiction");
        let c = &resp.contradictions[0];
        assert_eq!(c.band, "indices.ndvi");
        assert_eq!(c.attestations.len(), 2);
        // NDVI documented range is [-1, 1] in the registry; spread
        // 0.75 / 2.0 = 0.375 → severity should land in that ballpark.
        assert!(
            c.severity > 0.2 && c.severity < 0.6,
            "severity {} should reflect 0.75-spread / 2.0 NDVI range",
            c.severity
        );
        assert_eq!(c.kind, "scalar");
        let cids: Vec<&str> = c.attestations.iter().map(|a| a.fact_cid.as_str()).collect();
        assert!(cids.contains(&cid_a.as_str()));
        assert!(cids.contains(&cid_b.as_str()));
        // Receipt must cite both CIDs.
        assert_eq!(resp.receipt.fact_cids.len(), 2);
    }

    /// Two attesters write IDENTICAL NDVI — no contradiction.
    #[tokio::test]
    async fn ndvi_agreement_yields_zero_contradictions() {
        let storage = Arc::new(MockMultiStorage::new());
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "indices.ndvi",
            12,
            signer(1),
            CborValue::Float(0.42),
            1.0,
            "2026-05-01T12:00:00Z",
        );
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "indices.ndvi",
            12,
            signer(2),
            CborValue::Float(0.42),
            1.0,
            "2026-05-02T12:00:00Z",
        );
        let srv = test_server(storage);
        let resp = memory_contradictions(
            &ContradictionsReq {
                min_severity: Some(0.0),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        assert!(
            resp.contradictions.is_empty(),
            "agreeing NDVI must not flag a contradiction; got {:?}",
            resp.contradictions
        );
    }

    /// Two attesters disagree on a 128-D vector embedding —
    /// severity uses cosine.
    #[tokio::test]
    async fn vector_disagreement_uses_cosine_severity() {
        let storage = Arc::new(MockMultiStorage::new());
        let mk_vec =
            |x: f32| CborValue::Array(vec![CborValue::Float(x as f64), CborValue::Float(0.0)]);
        // Two near-orthogonal vectors at the same key.
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "geotessera",
            0,
            signer(1),
            mk_vec(1.0),
            1.0,
            "2026-05-01T12:00:00Z",
        );
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "geotessera",
            0,
            signer(2),
            // [0, 1] is orthogonal to [1, 0]; cosine = 0, severity = 1.
            CborValue::Array(vec![CborValue::Float(0.0), CborValue::Float(1.0)]),
            1.0,
            "2026-05-02T12:00:00Z",
        );
        let srv = test_server(storage);
        let resp = memory_contradictions(
            &ContradictionsReq {
                min_severity: Some(0.0),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        assert_eq!(resp.contradictions.len(), 1);
        let c = &resp.contradictions[0];
        assert_eq!(c.kind, "vector");
        // Orthogonal vectors → cosine ≈ 0 → severity ≈ 1.0.
        assert!(
            (c.severity - 1.0).abs() < 0.05,
            "orthogonal vectors → severity ≈ 1.0; got {}",
            c.severity
        );
    }

    /// Three attesters disagree on the WorldCover land-cover class:
    /// 2 say class 50 (built-up), 1 says class 10 (tree). Severity is
    /// 1 - max_share = 1 - 2/3 ≈ 0.333.
    #[tokio::test]
    async fn categorical_disagreement_uses_mode_share() {
        let storage = Arc::new(MockMultiStorage::new());
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "esa_worldcover.lc_2021",
            0,
            signer(1),
            CborValue::Integer(50i32.into()),
            1.0,
            "2026-05-01T12:00:00Z",
        );
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "esa_worldcover.lc_2021",
            0,
            signer(2),
            CborValue::Integer(50i32.into()),
            1.0,
            "2026-05-01T13:00:00Z",
        );
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "esa_worldcover.lc_2021",
            0,
            signer(3),
            CborValue::Integer(10i32.into()),
            1.0,
            "2026-05-01T14:00:00Z",
        );
        let srv = test_server(storage);
        let resp = memory_contradictions(
            &ContradictionsReq {
                min_severity: Some(0.0),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        assert_eq!(resp.contradictions.len(), 1);
        let c = &resp.contradictions[0];
        assert_eq!(c.kind, "categorical");
        // 1 - 2/3 ≈ 0.333
        assert!(
            (c.severity - 0.333).abs() < 0.01,
            "categorical severity should be 1 - mode_share ≈ 0.333; got {}",
            c.severity
        );
    }

    /// `cell_prefix` filter prunes the scan: a contradiction in a
    /// different cell must NOT surface when the prefix targets a
    /// disjoint region.
    #[tokio::test]
    async fn cell_prefix_filter_prunes_scan() {
        let storage = Arc::new(MockMultiStorage::new());
        // Contradiction in cell A
        storage.insert_attestation(
            "alfa.zb000.aaaa.aaaa",
            "indices.ndvi",
            0,
            signer(1),
            CborValue::Float(0.8),
            1.0,
            "2026-05-01T12:00:00Z",
        );
        storage.insert_attestation(
            "alfa.zb000.aaaa.aaaa",
            "indices.ndvi",
            0,
            signer(2),
            CborValue::Float(0.1),
            1.0,
            "2026-05-02T12:00:00Z",
        );
        // Contradiction in cell B
        storage.insert_attestation(
            "bravo.zb000.aaaa.aaaa",
            "indices.ndvi",
            0,
            signer(3),
            CborValue::Float(0.9),
            1.0,
            "2026-05-01T12:00:00Z",
        );
        storage.insert_attestation(
            "bravo.zb000.aaaa.aaaa",
            "indices.ndvi",
            0,
            signer(4),
            CborValue::Float(0.0),
            1.0,
            "2026-05-02T12:00:00Z",
        );
        let srv = test_server(storage);
        let resp = memory_contradictions(
            &ContradictionsReq {
                cell_prefix: Some("alfa".into()),
                min_severity: Some(0.0),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        assert_eq!(resp.contradictions.len(), 1, "only alfa.* should surface");
        assert!(resp.contradictions[0].cell.starts_with("alfa"));
    }

    /// Receipt MUST verify offline against the responder pubkey
    /// embedded in it. Reproduces `/v1/verify_receipt`'s signature
    /// check inline.
    #[tokio::test]
    async fn receipt_verifies_offline() {
        let storage = Arc::new(MockMultiStorage::new());
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "indices.ndvi",
            12,
            signer(1),
            CborValue::Float(0.85),
            1.0,
            "2026-05-01T12:00:00Z",
        );
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "indices.ndvi",
            12,
            signer(2),
            CborValue::Float(0.10),
            1.0,
            "2026-05-02T12:00:00Z",
        );
        let srv = test_server(storage);
        let resp = memory_contradictions(
            &ContradictionsReq {
                min_severity: Some(0.0),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        let r = &resp.receipt;
        // Rebuild the v1 preimage exactly as an offline verifier does
        // (domain-separated, tagged, length-prefixed segments).
        // Dispatch on the receipt's OWN version, which is what an offline
        // verifier does. Pinning v1 here made a deliberate preimage bump
        // look like a signing regression, while testing nothing about
        // whether the signature is correct.
        assert!(r.preimage_version >= emem_attest::PREIMAGE_V1);
        let manifest_hex_opt = if r.source_versions.is_empty() {
            None
        } else {
            let mut buf = Vec::with_capacity(128);
            let _ = ciborium::into_writer(&r.source_versions, &mut buf);
            Some(data_encoding::HEXLOWER.encode(blake3::hash(&buf).as_bytes()))
        };
        let msg = if r.preimage_version >= emem_attest::PREIMAGE_V2 {
            // v2 binds the inclusion proof (or its declared absence) into
            // the signature, so the rebuild has to include it or the
            // digest will not match what was signed.
            let merkle_hex = data_encoding::HEXLOWER.encode(&emem_attest::merkle_binding_v2(
                r.merkle_proof
                    .as_ref()
                    .map(|p| (&p.root, p.leaf_index, p.path.as_slice(), p.version)),
            ));
            emem_attest::receipt_preimage_v2(
                &r.request_id,
                &r.served_at,
                None,
                None,
                None,
                manifest_hex_opt.as_deref(),
                None,
                &r.primitive,
                r.cells.iter().map(|s| s.as_str()),
                r.fact_cids.iter().map(|c| c.as_str()),
                &merkle_hex,
            )
        } else {
            emem_attest::receipt_preimage_v1(
                &r.request_id,
                &r.served_at,
                None,
                None,
                None,
                manifest_hex_opt.as_deref(),
                None,
                &r.primitive,
                r.cells.iter().map(|s| s.as_str()),
                r.fact_cids.iter().map(|c| c.as_str()),
            )
        };
        let pk = ed25519_dalek::VerifyingKey::from_bytes(&r.responder.0).expect("pubkey");
        let sig = ed25519_dalek::Signature::from_bytes(&r.signature.0);
        pk.verify_strict(&msg, &sig)
            .expect("receipt signature must verify against responder pubkey");
    }

    /// Empty corpus path — no multi-attester entries at all. Returns
    /// honestly with `corpus_scanned: 0` and an agent hint.
    #[tokio::test]
    async fn empty_corpus_returns_zero_with_honest_hint() {
        let storage = Arc::new(MockMultiStorage::new());
        let srv = test_server(storage);
        let resp = memory_contradictions(&ContradictionsReq::default(), &srv)
            .await
            .expect("ok");
        assert!(resp.contradictions.is_empty());
        assert_eq!(resp.corpus_scanned, 0);
        assert!(resp.agent_hint.contains("No multi-attester"));
    }

    /// `min_severity` threshold gates results: a 1%-spread NDVI
    /// disagreement should NOT surface at `min_severity = 0.5`.
    #[tokio::test]
    async fn min_severity_threshold_gates_results() {
        let storage = Arc::new(MockMultiStorage::new());
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "indices.ndvi",
            12,
            signer(1),
            CborValue::Float(0.50),
            1.0,
            "2026-05-01T12:00:00Z",
        );
        storage.insert_attestation(
            "damO.zb000.xUti.zde78",
            "indices.ndvi",
            12,
            signer(2),
            CborValue::Float(0.51),
            1.0,
            "2026-05-02T12:00:00Z",
        );
        let srv = test_server(storage);
        // High threshold: should drop the borderline disagreement.
        let resp_high = memory_contradictions(
            &ContradictionsReq {
                min_severity: Some(0.5),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        assert!(resp_high.contradictions.is_empty());
        // Floor: surfaces it.
        let resp_low = memory_contradictions(
            &ContradictionsReq {
                min_severity: Some(0.0),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        assert_eq!(resp_low.contradictions.len(), 1);
    }

    /// `validate_request` rejects malformed windows.
    #[test]
    fn validate_rejects_inverted_window() {
        let r = ContradictionsReq {
            window_unix_s: Some([100, 50]),
            ..Default::default()
        };
        assert!(validate_request(&r).is_err());
    }

    /// ISO-8601 round-trip with the protocol's own time formatter.
    #[test]
    fn iso8601_parses_known_unix() {
        assert_eq!(iso8601_to_unix_s("1970-01-01T00:00:00Z"), Some(0));
        // 2026-01-01T00:00:00Z = 1_767_225_600
        assert_eq!(
            iso8601_to_unix_s("2026-01-01T00:00:00Z"),
            Some(1_767_225_600)
        );
        assert_eq!(iso8601_to_unix_s(""), None);
        assert_eq!(iso8601_to_unix_s("2026-13-45T99:99:99Z"), None);
    }
}
