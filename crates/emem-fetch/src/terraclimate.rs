//! TerraClimate climate-normal connector for emem.dev.
//!
//! TerraClimate is a high-resolution global monthly climatology produced by
//! the University of Idaho Climatology Lab (Abatzoglou et al. 2018,
//! *Scientific Data*; v1.1 release 2025). It interpolates time-varying
//! ERA5 anomalies onto WorldClim's 1/24° (~4 km) climatological surfaces,
//! covering 1958–present at monthly cadence, with derived water-balance
//! variables (AET, PET, runoff, climate water deficit, soil moisture).
//!
//! ## Why this is the right "climate-normal" source for emem
//!
//! Agents asking *"what's the average rainfall here?"* or *"what's the
//! climate normal for this place?"* want a **30-year mean**, not Tuesday's
//! weather. TerraClimate covers every land cell on Earth, is rebuilt
//! annually, and is in the public domain (CC0). The closest live
//! alternative on emem is the `weather.*` family (met.no nowcast) — that
//! tells you what the temperature is *right now*, not what it usually is.
//!
//! ## Data path: THREDDS NCSS
//!
//! TerraClimate is distributed as NetCDF-4. NetCDF/HDF5 doesn't admit a
//! clean COG-style range read for a single point — the chunk index is
//! deep in the file and the chunks themselves are filtered (gzip + shuffle).
//! Instead we use the **NetCDF Subset Service (NCSS)** that THREDDS exposes
//! at port 8080: a REST endpoint that takes lat/lng/time-range and returns
//! a CSV row per timestep. One `wget` per variable, ~360 rows for the
//! 1991–2020 normal window, no client-side NetCDF library needed.
//!
//! Endpoint shape:
//!
//! ```text
//! http://thredds.northwestknowledge.net:8080/thredds/ncss/grid/
//!   agg_terraclimate_<var>_1950_CurrentYear_GLOBE.nc
//!   ?var=<var>&latitude=<lat>&longitude=<lng>
//!   &time_start=1991-01-01T00:00:00Z&time_end=2020-12-01T00:00:00Z
//!   &accept=csv
//! ```
//!
//! NCSS returns the **packed integer** stored in the NetCDF (it does not
//! apply `scale_factor` / `add_offset` for CSV output) — we apply the
//! linear unpacking on our side. The packing parameters are documented in
//! the dataset's variable attributes and we encode them in
//! [`PackedScale`] so the test suite can exercise the unpack math without
//! a network round-trip.
//!
//! ## Normal window
//!
//! We compute the WMO standard 1991–2020 30-year normal. The constant
//! [`NORMAL_WINDOW`] is the single source of truth — bumping it later
//! (e.g. to 2001–2030 once that becomes the new WMO normal in 2031)
//! requires editing exactly one tuple here and the band metadata text in
//! the API crate. We deliberately do *not* use 1981–2010 (the previous WMO
//! normal) because the climate has shifted by ~0.4 °C globally since 1991
//! and the older window understates current normals; we deliberately do
//! not use the full 1958–present record because the pre-1991 baseline is
//! cooler than today's "normal" climate by enough that an agent answering
//! "what's the normal temperature here" off the 1958–present mean would
//! give a value that does not match WMO-published normals.
//!
//! ## Honest defaults
//!
//! - Empty CSV → [`FetchError::Transport`] with a structured "no rows" message.
//!   Caller surfaces this as Absence at the API layer; we never return 0 mm
//!   to mean "TerraClimate has no value here".
//! - Fill-value rows are dropped; if the resulting series is empty, we
//!   return the same structured error as above (a desert cell that exists
//!   in the grid but has all months masked is treated identically to an
//!   off-grid cell).
//! - HTTP non-2xx → [`FetchError::Transport`]. The caller decides whether
//!   to sign Absence or surface the error verbatim.

use std::time::Duration;

use bytes::Bytes;

use crate::FetchError;

/// The WMO standard normal window currently in force (effective 2021-01-01,
/// supersedes 1981–2010). We use these years inclusive, monthly cadence.
pub const NORMAL_WINDOW: (i32, i32) = (1991, 2020);

/// Base URL for the NetCDF Subset Service that fronts TerraClimate
/// aggregations. The dataset name suffix `_1950_CurrentYear_GLOBE.nc` is
/// the THREDDS-side aggregation that spans every published year (1950 →
/// the current calendar year), updated annually as new years are appended.
pub const NCSS_BASE: &str = "http://thredds.northwestknowledge.net:8080/thredds/ncss/grid";

/// Variable names available on the TerraClimate THREDDS aggregation. Map
/// from the emem band suffix to the NetCDF variable, the linear packing
/// (scale + offset that converts stored int → real value), the upstream
/// fill sentinel, the unit, and a short description.
///
/// The packing values come from the actual NetCDF variable attributes
/// served by THREDDS (`scale_factor`, `add_offset`, `_FillValue`); the
/// constants here are kept self-documenting with `///` so a reader doesn't
/// have to roundtrip through OPeNDAP to verify them.
#[derive(Debug, Clone, Copy)]
pub struct VariableSpec {
    /// NetCDF variable short name (`ppt`, `tmax`, `tmin`, `aet`, …).
    pub var: &'static str,
    /// Linear unpacking from the stored integer to the physical value.
    pub packed: PackedScale,
    /// Display unit attached to the resulting fact (`mm`, `degC`, …).
    pub unit: &'static str,
}

/// Linear int → real unpacking pulled from the NetCDF variable attributes.
///
/// `real = stored * scale_factor + add_offset` whenever
/// `stored != fill_value`; otherwise the value is missing.
#[derive(Debug, Clone, Copy)]
pub struct PackedScale {
    /// `scale_factor` attribute. TerraClimate uses 0.01 for tmin/tmax
    /// (1/100 °C resolution) and 0.1 for ppt/aet (1/10 mm resolution).
    pub scale: f64,
    /// `add_offset` attribute. Zero for water-balance variables, -99.0 for
    /// tmin/tmax (so the int range covers -99..+228 °C).
    pub offset: f64,
    /// `_FillValue` / `missing_value` sentinel for masked cells.
    pub fill: i64,
}

impl PackedScale {
    /// Apply the linear unpacking from an exact stored integer.
    /// Returns `None` for the fill sentinel.
    pub fn unpack(&self, stored: i64) -> Option<f64> {
        if stored == self.fill {
            None
        } else {
            Some(stored as f64 * self.scale + self.offset)
        }
    }

    /// Apply the linear unpacking from a float that was *formatted* to look
    /// like a float in the CSV but is in fact the packed sample (NCSS emits
    /// the stored short/int as `1483.0`). Round to nearest before checking
    /// the fill sentinel so float-formatting noise on integer values does
    /// not silently keep a fill row in the series.
    pub fn unpack_real(&self, stored: f64) -> Option<f64> {
        // NaN / non-finite were dropped by the caller, but defend in
        // depth: never apply the linear transform to a non-finite value.
        if !stored.is_finite() {
            return None;
        }
        // Snap to the packed integer the CSV writer rounded from. The fill
        // sentinel is integer-valued in the source NetCDF; comparing as
        // i64 after `round()` makes fill detection robust to whether NCSS
        // formats it as `-32768` or `-32768.0`.
        let snapped = stored.round() as i64;
        if snapped == self.fill {
            return None;
        }
        Some(stored * self.scale + self.offset)
    }
}

/// Variable spec for TerraClimate monthly mean precipitation (mm/month).
pub const PPT: VariableSpec = VariableSpec {
    var: "ppt",
    packed: PackedScale {
        scale: 0.1,
        offset: 0.0,
        fill: -2_147_483_648,
    },
    unit: "mm",
};

/// Variable spec for TerraClimate monthly maximum temperature (°C).
pub const TMAX: VariableSpec = VariableSpec {
    var: "tmax",
    packed: PackedScale {
        scale: 0.01,
        offset: -99.0,
        fill: -32_768,
    },
    unit: "degC",
};

/// Variable spec for TerraClimate monthly minimum temperature (°C).
pub const TMIN: VariableSpec = VariableSpec {
    var: "tmin",
    packed: PackedScale {
        scale: 0.01,
        offset: -99.0,
        fill: -32_768,
    },
    unit: "degC",
};

/// Variable spec for TerraClimate monthly actual evapotranspiration
/// (mm/month). Annual AET is the sum across the 12 calendar months of the
/// monthly normal.
pub const AET: VariableSpec = VariableSpec {
    var: "aet",
    packed: PackedScale {
        scale: 0.1,
        offset: 0.0,
        fill: -32_768,
    },
    unit: "mm",
};

/// Build the NCSS URL for a single variable + point + year range. Pure
/// function — exposed so the unit tests can pin the URL shape without a
/// network round-trip and so the responder can include it in the receipt.
///
/// `start_year` and `end_year` are both inclusive; we ask for `Jan 1 of
/// start_year` to `Dec 1 of end_year` (TerraClimate's monthly time
/// coordinate is the first day of each month).
pub fn ncss_url(spec: &VariableSpec, lat: f64, lng: f64, start_year: i32, end_year: i32) -> String {
    format!(
        "{base}/agg_terraclimate_{var}_1950_CurrentYear_GLOBE.nc?var={var}&latitude={lat:.4}&longitude={lng:.4}&time_start={sy:04}-01-01T00:00:00Z&time_end={ey:04}-12-01T00:00:00Z&accept=csv",
        base = NCSS_BASE,
        var = spec.var,
        lat = lat,
        lng = lng,
        sy = start_year,
        ey = end_year,
    )
}

/// Parse the NCSS CSV body into `(year, month, real_value)` rows.
///
/// NCSS CSV header line is fixed:
///   `time,latitude[unit="..."],longitude[unit="..."],<var>[unit="..."]`
/// Subsequent lines look like one of
///   `2020-01-01T00:00:00Z,1.35,103.82,1483`        (int form)
///   `2020-01-01T00:00:00Z,1.35,103.82,1483.0`      (float form, integer-valued)
///
/// **NCSS always returns the packed sample**, never the unpacked physical
/// value, regardless of whether the server formats it as `1483` or
/// `1483.0`. We always apply [`PackedScale::unpack_real`] to convert the
/// stored sample to physical units. Empirically THREDDS 4.6 emits the
/// float form (`1483.0`) for every TerraClimate variable I have measured;
/// older deployments and the int form are both tolerated.
///
/// Fill values are detected against the documented sentinel and the row
/// is dropped — we never fabricate a 0 to mean "no data".
///
/// Returns an empty vec on a header-only response (off-grid point).
pub fn parse_ncss_csv(body: &str, packed: &PackedScale) -> Result<Vec<TerraRow>, FetchError> {
    let mut out = Vec::new();
    let mut lines = body.lines();
    // Header — must exist, must contain `time` as the first column. We
    // don't validate every column name because the variable-specific tail
    // (e.g. `ppt[unit="mm"]`) drifts between server upgrades.
    let header = lines
        .next()
        .ok_or_else(|| FetchError::Transport("terraclimate: empty CSV (no header line)".into()))?;
    if !header.starts_with("time,") {
        return Err(FetchError::Transport(format!(
            "terraclimate: unexpected CSV header: {}",
            header.chars().take(120).collect::<String>()
        )));
    }
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut cols = line.split(',');
        let time_s = match cols.next() {
            Some(s) => s,
            None => continue,
        };
        // Skip lat, lng (we sent them; round-trip back unchanged).
        let _lat = cols.next();
        let _lng = cols.next();
        let value_s = match cols.next() {
            Some(s) => s.trim(),
            None => continue,
        };
        let (year, month) = match parse_iso_date_ym(time_s) {
            Some(ym) => ym,
            None => continue,
        };
        // Parse the value column to f64; both `1483` and `1483.0` parse
        // cleanly. NCSS always emits the packed sample so we always
        // unpack — never do "if integer, unpack; if float, pass through"
        // because float-form `1483.0` is still the packed sample, not
        // 1483 mm of rain.
        let stored_f = match value_s.parse::<f64>() {
            Ok(f) if f.is_finite() => f,
            _ => continue,
        };
        let Some(real) = packed.unpack_real(stored_f) else {
            // fill sentinel
            continue;
        };
        out.push(TerraRow {
            year,
            month,
            value: real,
        });
    }
    Ok(out)
}

/// One unpacked monthly observation from a TerraClimate point query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerraRow {
    /// Calendar year of the observation, e.g. 1991.
    pub year: i32,
    /// Calendar month, 1..=12.
    pub month: u8,
    /// Real-units value (e.g. mm/month for ppt; °C for tmax/tmin).
    pub value: f64,
}

/// `2020-01-01T00:00:00Z` → `Some((2020, 1))`. Returns `None` for
/// anything that isn't an ISO-8601 instant we recognise.
fn parse_iso_date_ym(s: &str) -> Option<(i32, u8)> {
    if s.len() < 10 {
        return None;
    }
    let year: i32 = s.get(0..4)?.parse().ok()?;
    if s.as_bytes().get(4) != Some(&b'-') {
        return None;
    }
    let month: u8 = s.get(5..7)?.parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some((year, month))
}

/// Compute the **annual mean of monthly totals** across the supplied rows.
///
/// For precipitation and AET this is the canonical "annual normal in mm" —
/// for each year inside `window`, sum the 12 monthly values, then mean
/// across years. Years with fewer than 12 valid months are skipped (we do
/// not extrapolate; a partial year would understate the annual total).
///
/// Returns `Err` if no full year survives, with a structured message the
/// caller can surface as a band-Absence reason.
pub fn annual_total_normal(rows: &[TerraRow], window: (i32, i32)) -> Result<f64, FetchError> {
    let (sy, ey) = window;
    let mut yearly_totals: Vec<f64> = Vec::new();
    for y in sy..=ey {
        let mut months = [false; 12];
        let mut total = 0.0f64;
        for r in rows {
            if r.year == y && (1..=12).contains(&r.month) {
                let idx = (r.month - 1) as usize;
                if !months[idx] {
                    months[idx] = true;
                    total += r.value;
                }
            }
        }
        if months.iter().all(|&b| b) {
            yearly_totals.push(total);
        }
    }
    if yearly_totals.is_empty() {
        return Err(FetchError::Transport(format!(
            "terraclimate: no complete year in {sy}..={ey} (got {} rows total)",
            rows.len()
        )));
    }
    let mean = yearly_totals.iter().sum::<f64>() / yearly_totals.len() as f64;
    Ok(mean)
}

/// Compute the **mean of monthly values** across the supplied rows in the
/// window. Used for temperature normals (mean monthly °C).
///
/// Returns `Err` if no row falls in the window.
pub fn monthly_mean_normal(rows: &[TerraRow], window: (i32, i32)) -> Result<f64, FetchError> {
    let (sy, ey) = window;
    let in_win: Vec<f64> = rows
        .iter()
        .filter(|r| r.year >= sy && r.year <= ey)
        .map(|r| r.value)
        .collect();
    if in_win.is_empty() {
        return Err(FetchError::Transport(format!(
            "terraclimate: no row in {sy}..={ey} window (got {} rows total)",
            rows.len()
        )));
    }
    Ok(in_win.iter().sum::<f64>() / in_win.len() as f64)
}

/// Inner HTTP plumbing: GET the NCSS endpoint, return the response body
/// as bytes. Bounded by our shared materializer timeout.
async fn ncss_get(url: &str, timeout: Duration) -> Result<Bytes, FetchError> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!(
            "emem.dev/",
            env!("CARGO_PKG_VERSION"),
            " (avijeet@vortx.ai)"
        ))
        .build()
        .map_err(|e| FetchError::Transport(format!("terraclimate client build: {e}")))?;
    let resp = client
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|e| FetchError::Transport(format!("terraclimate https: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(FetchError::Transport(format!(
            "terraclimate ncss status {} for {url}",
            status.as_u16()
        )));
    }
    resp.bytes()
        .await
        .map_err(|e| FetchError::Transport(format!("terraclimate body: {e}")))
}

/// One `(variable, normal_value)` pair as understood by an emem fact.
#[derive(Debug, Clone)]
pub struct NormalSample {
    /// NetCDF variable short name (`ppt`, `tmax`, `tmin`, `aet`).
    pub variable: &'static str,
    /// Real-units normal value (mm/year for ppt+aet, °C for tmax+tmin).
    pub value: f64,
    /// Unit attached to the fact (`mm`, `degC`).
    pub unit: &'static str,
    /// URL the responder hit upstream — surfaced in the fact's `Source.id`
    /// so a verifier can replay the same NCSS query.
    pub url: String,
    /// Number of complete years (for annual totals) or monthly samples
    /// (for monthly means) that contributed to the normal. Surfaced in
    /// the fact's `confidence` calibration if a caller wants it.
    pub n_samples: usize,
}

/// What kind of normal to compute from the monthly series.
///
/// - `AnnualTotal`: sum the 12 monthly values for each year inside the
///   window, then mean across years (precip, AET — gives mm/year).
/// - `MonthlyMean`: arithmetic mean of every monthly value inside the
///   window (temperature normals — gives °C).
#[derive(Debug, Clone, Copy)]
pub enum NormalKind {
    /// Annual-total normal: mean across years of the per-year sum of
    /// 12 monthly values. Yields the canonical "mm/year" climatology.
    AnnualTotal,
    /// Monthly-mean normal: arithmetic mean of every monthly value in
    /// the window. Yields the canonical "mean monthly °C" climatology.
    MonthlyMean,
}

/// Fetch the 1991–2020 (or `window`-specified) climate normal for one
/// TerraClimate variable at the requested point. Returns the unpacked
/// real-units value plus the URL hit + sample count.
///
/// This is the function the API-layer materializer wires to a fact.
pub async fn fetch_terraclimate_normal(
    spec: &VariableSpec,
    lat: f64,
    lng: f64,
    window: (i32, i32),
    kind: NormalKind,
    timeout: Duration,
) -> Result<NormalSample, FetchError> {
    let url = ncss_url(spec, lat, lng, window.0, window.1);
    let body = ncss_get(&url, timeout).await?;
    let body_str = std::str::from_utf8(&body)
        .map_err(|e| FetchError::Transport(format!("terraclimate body utf8: {e}")))?;
    let rows = parse_ncss_csv(body_str, &spec.packed)?;
    let (value, n) = match kind {
        NormalKind::AnnualTotal => {
            let v = annual_total_normal(&rows, window)?;
            // Number of complete years inside the window.
            let mut n_years = 0usize;
            for y in window.0..=window.1 {
                let mut months = [false; 12];
                for r in &rows {
                    if r.year == y && (1..=12).contains(&r.month) {
                        months[(r.month - 1) as usize] = true;
                    }
                }
                if months.iter().all(|&b| b) {
                    n_years += 1;
                }
            }
            (v, n_years)
        }
        NormalKind::MonthlyMean => {
            let v = monthly_mean_normal(&rows, window)?;
            let n = rows
                .iter()
                .filter(|r| r.year >= window.0 && r.year <= window.1)
                .count();
            (v, n)
        }
    };
    Ok(NormalSample {
        variable: spec.var,
        value,
        unit: spec.unit,
        url,
        n_samples: n,
    })
}

/// Fetch the **mean annual temperature normal** by averaging tmin and tmax
/// independently and returning their per-month mean. Two upstream calls.
///
/// `T_mean = (T_min + T_max) / 2` is the standard meteorological
/// convention for daily/monthly mean temperature in the absence of a
/// continuously-sampled mean (Linacre 1992, *Climate Data and Resources*).
/// TerraClimate ships tmin and tmax but not tmean, so this is how every
/// downstream user computes mean temperature from TerraClimate.
pub async fn fetch_terraclimate_tmean_normal(
    lat: f64,
    lng: f64,
    window: (i32, i32),
    timeout: Duration,
) -> Result<TmeanSample, FetchError> {
    let tmin = fetch_terraclimate_normal(&TMIN, lat, lng, window, NormalKind::MonthlyMean, timeout)
        .await?;
    let tmax = fetch_terraclimate_normal(&TMAX, lat, lng, window, NormalKind::MonthlyMean, timeout)
        .await?;
    Ok(TmeanSample {
        value: (tmin.value + tmax.value) / 2.0,
        unit: "degC",
        tmin_url: tmin.url,
        tmax_url: tmax.url,
        n_samples: tmin.n_samples.min(tmax.n_samples),
    })
}

/// Result of [`fetch_terraclimate_tmean_normal`]. Carries both upstream URLs
/// so the responder can attribute both halves of the derivation in the
/// signed fact.
#[derive(Debug, Clone)]
pub struct TmeanSample {
    /// Mean monthly temperature in the unpacking unit (always `degC`).
    pub value: f64,
    /// Unit attached to the fact.
    pub unit: &'static str,
    /// Upstream URL for the tmin half of the derivation.
    pub tmin_url: String,
    /// Upstream URL for the tmax half of the derivation.
    pub tmax_url: String,
    /// Min of (`tmin.n_samples`, `tmax.n_samples`) — the conservative
    /// number of monthly observations that contributed.
    pub n_samples: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ncss_url` MUST be byte-stable across builds — receipts pin the
    /// URL the responder hit, so a silent format change would break
    /// verifier replay. This test pins both the fixed query parameters
    /// (variable, time window, accept=csv) and the lat/lng formatting
    /// (4 fractional digits, matches our cell64 → lat/lng precision).
    #[test]
    fn ncss_url_is_stable() {
        let u = ncss_url(&PPT, 1.3521, 103.8198, 1991, 2020);
        assert_eq!(
            u,
            "http://thredds.northwestknowledge.net:8080/thredds/ncss/grid/agg_terraclimate_ppt_1950_CurrentYear_GLOBE.nc?var=ppt&latitude=1.3521&longitude=103.8198&time_start=1991-01-01T00:00:00Z&time_end=2020-12-01T00:00:00Z&accept=csv"
        );
    }

    /// Different variables route to different aggregation files. Cover
    /// each of the three sub-bands we materialize so a future rename
    /// (e.g. THREDDS dropping the `_GLOBE` suffix) gets caught here.
    #[test]
    fn ncss_url_per_variable() {
        for (spec, marker) in [
            (PPT, "agg_terraclimate_ppt_"),
            (TMAX, "agg_terraclimate_tmax_"),
            (TMIN, "agg_terraclimate_tmin_"),
            (AET, "agg_terraclimate_aet_"),
        ] {
            let u = ncss_url(&spec, 0.0, 0.0, 1991, 2020);
            assert!(u.contains(marker), "variable {} not in URL: {u}", spec.var);
            assert!(
                u.contains(&format!("var={}", spec.var)),
                "URL missing var=: {u}"
            );
        }
    }

    /// `PackedScale::unpack` for each of the four variables we wire,
    /// using documented values from the THREDDS dataset attributes.
    /// These constants are pinned because shifting a packing parameter
    /// would silently change every signed fact (e.g. an off-by-one in
    /// `add_offset` makes Reykjavik look 99 °C colder than it is).
    #[test]
    fn unpack_matches_documented_packing() {
        // ppt: stored=1483 → 148.3 mm (Singapore Jan)
        assert!((PPT.packed.unpack(1483).unwrap() - 148.3).abs() < 1e-9);
        // tmax: stored=10250 → 10250 * 0.01 - 99.0 = 3.5 degC (Reykjavik Jan)
        assert!((TMAX.packed.unpack(10250).unwrap() - 3.5).abs() < 1e-9);
        // tmin: same packing as tmax. stored=9800 → -1.0 degC.
        assert!((TMIN.packed.unpack(9800).unwrap() - -1.0).abs() < 1e-9);
        // aet: stored=290 → 29.0 mm (low desert AET).
        assert!((AET.packed.unpack(290).unwrap() - 29.0).abs() < 1e-9);
        // Fill values unpack to None.
        assert!(PPT.packed.unpack(-2_147_483_648).is_none());
        assert!(TMAX.packed.unpack(-32_768).is_none());
        assert!(AET.packed.unpack(-32_768).is_none());
    }

    /// CSV round-trip: a NCSS-shaped reply parses to the expected
    /// `TerraRow` series, with fill rows dropped silently. Covers BOTH
    /// the integer form (older THREDDS deployments) and the float form
    /// (current 4.6 emits `1483.0` for an int-stored sample). Also pins
    /// fill detection in the float form (`-2147483648.0` → drop).
    #[test]
    fn parse_ncss_csv_smoke() {
        let body = concat!(
            "time,latitude[unit=\"degrees_north\"],longitude[unit=\"degrees_east\"],ppt[unit=\"mm\"]\n",
            // Integer form (legacy THREDDS): 2400 (stored) → 240.0 mm.
            "1991-01-01T00:00:00Z,1.35,103.82,2400\n",
            // Float form (current 4.6): 1500.0 (stored) → 150.0 mm.
            "1991-02-01T00:00:00Z,1.35,103.82,1500.0\n",
            // Fill row (integer form): drop.
            "1991-03-01T00:00:00Z,1.35,103.82,-2147483648\n",
            // Fill row (float form): drop too.
            "1991-04-01T00:00:00Z,1.35,103.82,-2147483648.0\n",
            "1992-01-01T00:00:00Z,1.35,103.82,2600.0\n",
        );
        let rows = parse_ncss_csv(body, &PPT.packed).expect("parse ok");
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            TerraRow {
                year: 1991,
                month: 1,
                value: 240.0
            }
        );
        assert_eq!(
            rows[1],
            TerraRow {
                year: 1991,
                month: 2,
                value: 150.0
            }
        );
        // 1991-03 + 1991-04 dropped (fill, both forms).
        assert_eq!(
            rows[2],
            TerraRow {
                year: 1992,
                month: 1,
                value: 260.0
            }
        );
    }

    /// Header-only response (NCSS sometimes returns just the header for
    /// off-grid points) must surface as an error, not a silent zero.
    #[test]
    fn parse_ncss_header_only_yields_empty() {
        let body = "time,latitude[unit=\"degrees_north\"],longitude[unit=\"degrees_east\"],ppt[unit=\"mm\"]\n";
        let rows = parse_ncss_csv(body, &PPT.packed).expect("parse ok");
        assert!(rows.is_empty());
        // Then `annual_total_normal` MUST refuse to fabricate a value.
        let err = annual_total_normal(&rows, NORMAL_WINDOW).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no complete year"), "{msg}");
    }

    /// `annual_total_normal` averages per-year sums across the window.
    /// We construct two complete years (12 months each) and one partial
    /// year that MUST be dropped.
    #[test]
    fn annual_total_normal_skips_partial_years() {
        let mut rows = Vec::new();
        // Year 1991: 12 × 100 mm → annual total 1200.
        for m in 1u8..=12 {
            rows.push(TerraRow {
                year: 1991,
                month: m,
                value: 100.0,
            });
        }
        // Year 1992: 12 × 200 mm → annual total 2400.
        for m in 1u8..=12 {
            rows.push(TerraRow {
                year: 1992,
                month: m,
                value: 200.0,
            });
        }
        // Year 1993: only Jan–Mar (partial). Must be dropped.
        for m in 1u8..=3 {
            rows.push(TerraRow {
                year: 1993,
                month: m,
                value: 999_999.0,
            });
        }
        let v = annual_total_normal(&rows, (1991, 1993)).unwrap();
        // (1200 + 2400) / 2 = 1800 — the partial year is excluded.
        assert!((v - 1800.0).abs() < 1e-9, "got {v}");
    }

    /// `monthly_mean_normal` is a flat arithmetic mean across the window.
    #[test]
    fn monthly_mean_normal_is_arithmetic_mean() {
        let rows = vec![
            TerraRow {
                year: 1991,
                month: 1,
                value: 0.0,
            },
            TerraRow {
                year: 1991,
                month: 2,
                value: 10.0,
            },
            TerraRow {
                year: 1991,
                month: 3,
                value: 20.0,
            },
            TerraRow {
                year: 1992,
                month: 1,
                value: 30.0,
            },
            // Outside the window — must be ignored.
            TerraRow {
                year: 1990,
                month: 1,
                value: 1_000_000.0,
            },
            TerraRow {
                year: 2021,
                month: 1,
                value: 1_000_000.0,
            },
        ];
        let v = monthly_mean_normal(&rows, (1991, 2020)).unwrap();
        // (0 + 10 + 20 + 30) / 4 = 15.0
        assert!((v - 15.0).abs() < 1e-9, "got {v}");
    }

    /// Float-format input still gets unpacked — pin the parser's
    /// documented "always unpack" contract. NCSS in current production
    /// emits the packed integer (e.g. `1483`); a float like `148.3`
    /// is treated as the same packed sample (real = stored * 0.1 =
    /// 14.83), not as a pre-unpacked value. If a future NCSS server
    /// pre-applies scale_factor we'll need a deliberate parser
    /// change — this test is the regression sentinel for that day.
    #[test]
    fn parse_ncss_csv_float_values_are_unpacked_too() {
        let body = concat!(
            "time,latitude[unit=\"degrees_north\"],longitude[unit=\"degrees_east\"],ppt[unit=\"mm\"]\n",
            "1991-01-01T00:00:00Z,1.35,103.82,148.3\n",
        );
        let rows = parse_ncss_csv(body, &PPT.packed).expect("parse ok");
        assert_eq!(rows.len(), 1);
        // 148.3 (packed) * 0.1 (scale_factor) = 14.83 (real mm).
        assert!(
            (rows[0].value - 14.83).abs() < 1e-9,
            "got {}",
            rows[0].value
        );
    }

    /// Live integration smoke: hits THREDDS NCSS for Singapore precip
    /// across the 1991-2020 normal window. Marked `#[ignore]` so it
    /// runs only with `cargo test -- --ignored` — keeps CI offline-clean
    /// while letting the operator verify wire-level correctness.
    ///
    /// Singapore (1.35°N, 103.82°E) annual normal precip is ≈2300 mm
    /// (well-documented climatology); we accept anything in 1500..3500
    /// mm/year so this stays robust to THREDDS's nearest-cell snap.
    #[tokio::test]
    #[ignore]
    async fn live_singapore_annual_precip() {
        let s = fetch_terraclimate_normal(
            &PPT,
            1.35,
            103.82,
            NORMAL_WINDOW,
            NormalKind::AnnualTotal,
            Duration::from_secs(60),
        )
        .await
        .expect("upstream NCSS available");
        assert!(
            (1500.0..=3500.0).contains(&s.value),
            "Singapore annual precip out of plausible range: {} mm",
            s.value
        );
        assert_eq!(s.unit, "mm");
        assert!(
            s.n_samples >= 25,
            "expected ≥25 complete years, got {}",
            s.n_samples
        );
    }
}
