//! MODIS bands from the Planetary Computer's COGs, with ORNL DAAC's subset
//! service as the fallback.
//!
//! ORNL's service took 16 to 90 s for one request at one point in the week
//! of 2026-11-14. The Planetary Computer serves the same LP DAAC granules
//! (the item id carries the production time ORNL reports as `proc_date`) as
//! COGs, and one composite is a tile search plus two pixel reads.
//!
//! [`BANDS`] is the one table both readers take a band from: the product,
//! the value and QA layers, the scale, the unit, the composite length, the
//! ORNL window and both reader stamps. So a fact read here means what the
//! ORNL fact meant. The window is the same, and so is the pick (the valid
//! composite nearest the target, or the newest with no target, by
//! `DatedPick::rank`). The fill values, the QA gate and the confidence it
//! gives are the same functions. Unit, scale and tempo do not change. The
//! source and the reader stamp change, because the fact names where its
//! bytes came from.
//!
//! The catalogue has holes: composites LP DAAC produced and ORNL serves that
//! the Planetary Computer does not hold. On 2026-11-15 every tile checked
//! lacked every 8-day and 16-day composite from 2025-07-04 to 2025-11-17,
//! and MCD64A1 lacked 2023-09, 2024-07, 2025-05 and 2025-08 to 2025-10. A
//! missing composite that ORNL could hold sends the read to ORNL, so a hole
//! never moves the pick to another composite. A missing composite too
//! recent to have been produced is skipped instead, because no source has
//! it yet.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use emem_core::tslot::{Tempo, Tslot};
use emem_fact::{Derivation, Fact, FactCid, PrimaryFact, SchemaCid, Source};
use emem_fetch::modis_pc::{self, ModisPcItem};

use crate::{AppState, DatedPick};

const DAY: i64 = 86_400;
/// Source scheme of a fact read from the Planetary Computer's COGs.
const SCHEME: &str = "mpc_modis_cog";
/// How long the Planetary Computer read may take before ORNL is asked.
const PC_BUDGET: Duration = Duration::from_secs(12);
/// Items one tile search returns at most. The widest search (nine months
/// of MCD64A1, or 72 days of an 8-day collection that also holds the Aqua
/// and combined products) is under 40.
const SEARCH_LIMIT: usize = 100;

/// How a product cuts time into composites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Composite {
    /// Calendar months (MCD64A1).
    Month,
    /// `n` days from 1 January of each year: days 1, 9, 17, ... for 8-day
    /// products, 1, 17, 33, ... for 16-day ones. The last composite of a
    /// year stops at 31 December, so it is short.
    Days(i64),
}

/// Which ORNL reader in lib.rs serves the band, and so what the Planetary
/// Computer read does where that reader differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OrnlReader {
    /// `materialize_ornl_subset_band`: nothing valid in the window is a
    /// signed Absence.
    Subset,
    /// `materialize_modis_ndvi_ornl`: nothing valid is an error, and a
    /// composite this node already signed is not signed again.
    Ndvi,
}

/// One MODIS band, as both readers take it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ModisBand {
    pub(crate) band: &'static str,
    /// The LP DAAC product: ORNL's product name, the prefix of a granule
    /// id, and the key of `ornl_value`, `ornl_qc_gate` and
    /// `ornl_qc_confidence`.
    pub(crate) product: &'static str,
    /// The value layer: ORNL's band name and the COG asset key, which are
    /// the same.
    pub(crate) variable: &'static str,
    /// The QA layer, named the same way.
    pub(crate) qa: &'static str,
    /// Multiplies the raw value. ORNL states a scale with each answer and
    /// its reader prefers that; on 2026-11-15 ORNL stated these.
    pub(crate) scale: f64,
    pub(crate) unit: Option<&'static str>,
    pub(crate) composite: Composite,
    pub(crate) tempo: Tempo,
    /// The window either side of a target, in days.
    pub(crate) half_window_days: i64,
    /// With no target, how many days back the window reaches. ORNL refuses
    /// more than ten composites in one subset, so the subset bands keep to
    /// nine.
    pub(crate) newest_days: i64,
    /// Days after a composite ends within which LP DAAC may not have
    /// produced it yet. Measured on the Planetary Computer's item ids on
    /// 2026-11-15: 8-day products 2 to 7 days (95th percentile 6),
    /// MOD13Q1 1 to 20 (95th percentile 11), MCD64A1 41 to 43.
    pub(crate) produced_within_days: i64,
    pub(crate) ornl: OrnlReader,
    pub(crate) ornl_fn_key: &'static str,
    /// The Planetary Computer collection and the reader stamp of a fact read
    /// there. `None`: the collection is not there, so ORNL reads it.
    pub(crate) pc: Option<(&'static str, &'static str)>,
}

/// Every MODIS band either reader serves.
pub(crate) static BANDS: [ModisBand; 7] = [
    ModisBand {
        band: "modis.lst_day_8day",
        product: "MOD11A2",
        variable: "LST_Day_1km",
        qa: "QC_Day",
        scale: 0.02,
        unit: Some("K"),
        composite: Composite::Days(8),
        tempo: Tempo::Composite8Day,
        half_window_days: 8,
        newest_days: 9 * 8,
        produced_within_days: 10,
        ornl: OrnlReader::Subset,
        ornl_fn_key: "modis_ornl_mod11a2_lstday@1",
        pc: Some(("modis-11A2-061", "modis_pc_mod11a2_lstday@1")),
    },
    ModisBand {
        band: "modis.lst_night_8day",
        product: "MOD11A2",
        variable: "LST_Night_1km",
        qa: "QC_Night",
        scale: 0.02,
        unit: Some("K"),
        composite: Composite::Days(8),
        tempo: Tempo::Composite8Day,
        half_window_days: 8,
        newest_days: 9 * 8,
        produced_within_days: 10,
        ornl: OrnlReader::Subset,
        ornl_fn_key: "modis_ornl_mod11a2_lstnight@1",
        pc: Some(("modis-11A2-061", "modis_pc_mod11a2_lstnight@1")),
    },
    // ET_QC_500m: bits 0-1 MODLAND_QC (Mu 2011 §2.3). MOD16A2 is not on the
    // Planetary Computer.
    ModisBand {
        band: "modis.et_8day",
        product: "MOD16A2",
        variable: "ET_500m",
        qa: "ET_QC_500m",
        scale: 0.1,
        unit: Some("kg/m^2"),
        composite: Composite::Days(8),
        tempo: Tempo::Composite8Day,
        half_window_days: 8,
        newest_days: 9 * 8,
        produced_within_days: 10,
        ornl: OrnlReader::Subset,
        ornl_fn_key: "modis_ornl_mod16a2_et@1",
        pc: None,
    },
    // Psn_QC_500m: bits 0-1 MODLAND_QC, bits 5-7 CloudState.
    ModisBand {
        band: "modis.gpp_8day",
        product: "MOD17A2H",
        variable: "Gpp_500m",
        qa: "Psn_QC_500m",
        scale: 1e-4,
        unit: Some("kg C/m^2"),
        composite: Composite::Days(8),
        tempo: Tempo::Composite8Day,
        half_window_days: 8,
        newest_days: 9 * 8,
        produced_within_days: 10,
        ornl: OrnlReader::Subset,
        ornl_fn_key: "modis_ornl_mod17a2h_gpp@1",
        pc: Some(("modis-17A2H-061", "modis_pc_mod17a2h_gpp@1")),
    },
    // FparLai_QC: bits 0-1 MODLAND_QC, bits 5-7 CloudState.
    ModisBand {
        band: "modis.lai_8day",
        product: "MOD15A2H",
        variable: "Lai_500m",
        qa: "FparLai_QC",
        scale: 0.1,
        unit: Some("m^2/m^2"),
        composite: Composite::Days(8),
        tempo: Tempo::Composite8Day,
        half_window_days: 8,
        newest_days: 9 * 8,
        produced_within_days: 10,
        ornl: OrnlReader::Subset,
        ornl_fn_key: "modis_ornl_mod15a2h_lai@1",
        pc: Some(("modis-15A2H-061", "modis_pc_mod15a2h_lai@1")),
    },
    // QA bit 0 is the land mask: it gates out non-land Burn_Date values
    // that upstream marks apart from the -1 unmapped fill.
    ModisBand {
        band: "modis.burned_area_monthly",
        product: "MCD64A1",
        variable: "Burn_Date",
        qa: "QA",
        scale: 1.0,
        unit: Some("doy"),
        composite: Composite::Month,
        tempo: Tempo::Medium,
        half_window_days: 32,
        newest_days: 9 * 31,
        produced_within_days: 45,
        ornl: OrnlReader::Subset,
        ornl_fn_key: "modis_ornl_mcd64a1_burndate@1",
        pc: Some(("modis-64A1-061", "modis_pc_mcd64a1_burndate@1")),
    },
    // pixel_reliability: 0 good, 1 marginal, 2 snow or ice, 3 cloudy.
    ModisBand {
        band: "modis.ndvi_mean",
        product: "MOD13Q1",
        variable: "250m_16_days_NDVI",
        qa: "250m_16_days_pixel_reliability",
        scale: 1e-4,
        unit: None,
        composite: Composite::Days(16),
        tempo: Tempo::Composite16Day,
        half_window_days: 32,
        newest_days: 90,
        produced_within_days: 16,
        ornl: OrnlReader::Ndvi,
        ornl_fn_key: "modis_ornl_subset@1",
        pc: Some(("modis-13Q1-061", "modis_pc_mod13q1_ndvi@1")),
    },
];

/// The table row of `band`.
pub(crate) fn spec(band: &str) -> Option<&'static ModisBand> {
    BANDS.iter().find(|b| b.band == band)
}

impl ModisBand {
    /// The `[lo, hi]` (unix seconds) the read covers for `pick`: the window
    /// either side of a target, clipped to now and at least a day; a slot
    /// and its margin; or the lookback with no target.
    pub(crate) fn window(&self, pick: DatedPick, now: i64) -> (i64, i64) {
        match pick {
            DatedPick::Newest => (now - self.newest_days * DAY, now),
            DatedPick::Nearest(t) => {
                let lo = (t - self.half_window_days * DAY).max(0);
                let hi = (t + self.half_window_days * DAY).min(now);
                (lo, hi.max(lo + DAY))
            }
            DatedPick::Slot { .. } => pick.window(self.half_window_days, now),
        }
    }

    /// Days past the window the tile search reaches, to see whether the
    /// catalogue holds anything newer than a composite it lacks. More than
    /// `produced_within_days` plus one composite, so a missing composite
    /// that counts as recent has its successors in the search.
    fn lookahead_days(&self) -> i64 {
        match self.composite {
            Composite::Month => 92,
            Composite::Days(n) => 3 * n,
        }
    }
}

fn date_of(unix: i64) -> String {
    let (y, m, d) = crate::civil_from_days(unix.div_euclid(DAY));
    format!("{y:04}-{m:02}-{d:02}")
}

fn date_unix(s: &str) -> Option<i64> {
    let y: i32 = s.get(0..4)?.parse().ok()?;
    let m: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(crate::days_from_civil(y, m, d) * DAY)
}

fn next_month(month: i64) -> i64 {
    let (y, m, _) = crate::civil_from_days(month.div_euclid(DAY));
    let (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    crate::days_from_civil(y, m, 1) * DAY
}

fn jan_1(year: i32) -> i64 {
    crate::days_from_civil(year, 1, 1) * DAY
}

impl Composite {
    /// First days (unix) of the composites whose first day falls in
    /// `[lo, hi]` by calendar day: the composites ORNL's subset returns for
    /// `startDate = day(lo)`, `endDate = day(hi)`.
    fn starts_in(self, lo: i64, hi: i64) -> Vec<i64> {
        let (first, last) = (lo.div_euclid(DAY) * DAY, hi.div_euclid(DAY) * DAY);
        let mut out = Vec::new();
        match self {
            Composite::Month => {
                let (y, m, d) = crate::civil_from_days(first.div_euclid(DAY));
                let mut month = crate::days_from_civil(y, m, 1) * DAY;
                if d != 1 {
                    month = next_month(month);
                }
                while month <= last {
                    out.push(month);
                    month = next_month(month);
                }
            }
            Composite::Days(n) => {
                let (y0, _, _) = crate::civil_from_days(first.div_euclid(DAY));
                let (y1, _, _) = crate::civil_from_days(last.div_euclid(DAY));
                for y in y0..=y1 {
                    let mut start = jan_1(y);
                    while start < jan_1(y + 1) {
                        if (first..=last).contains(&start) {
                            out.push(start);
                        }
                        start += n * DAY;
                    }
                }
            }
        }
        out
    }

    /// The first day after the composite that starts on `start`.
    fn end(self, start: i64) -> i64 {
        match self {
            Composite::Month => next_month(start),
            Composite::Days(n) => {
                let (y, _, _) = crate::civil_from_days(start.div_euclid(DAY));
                (start + n * DAY).min(jan_1(y + 1))
            }
        }
    }

    /// How a composite is named in a reason: `2023-08` or `2024-07-11`.
    fn label(self, start: i64) -> String {
        let d = date_of(start);
        match self {
            Composite::Month => d[..7].to_string(),
            Composite::Days(_) => d,
        }
    }
}

/// The window and the pick the ORNL reader would use.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Window {
    lo: i64,
    hi: i64,
    pick: DatedPick,
    now: i64,
}

impl Window {
    fn new(b: &ModisBand, pick: DatedPick, now: i64) -> Self {
        let (lo, hi) = b.window(pick, now);
        Window { lo, hi, pick, now }
    }

    /// The instant the read is about: an Absence is anchored there.
    fn target(&self) -> Option<i64> {
        match self.pick {
            DatedPick::Newest => None,
            DatedPick::Nearest(t) => Some(t),
            DatedPick::Slot { start, end, .. } => Some((start + end) / 2),
        }
    }
}

/// Raw value and QA at the point in one item, and the pixel they are in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pixel {
    value: i64,
    qa: i64,
    col: i64,
    row: i64,
}

/// Where the walk gets its items and pixels: the Planetary Computer, or a
/// fake in tests.
trait CompositeSource {
    /// `h30v10`, the tile the point is in.
    fn tile(&self) -> String;
    /// The tile's items whose period intersects `[from, to]` (unix seconds).
    async fn items(&self, from: i64, to: i64) -> Result<Vec<ModisPcItem>, String>;
    /// The pixel holding the point in `item`.
    async fn read(&self, item: &ModisPcItem) -> Result<Pixel, String>;
}

struct PcSource {
    band: &'static ModisBand,
    collection: &'static str,
    client: reqwest::Client,
    x: f64,
    y: f64,
    h: u32,
    v: u32,
}

impl CompositeSource for PcSource {
    fn tile(&self) -> String {
        format!("h{:02}v{:02}", self.h, self.v)
    }

    async fn items(&self, from: i64, to: i64) -> Result<Vec<ModisPcItem>, String> {
        let datetime = format!(
            "{}/{}",
            crate::iso8601_utc(from.max(0) as u64),
            crate::iso8601_utc(to.max(0) as u64)
        );
        modis_pc::search_tile(
            &self.client,
            self.collection,
            self.h,
            self.v,
            &datetime,
            SEARCH_LIMIT,
        )
        .await
    }

    async fn read(&self, item: &ModisPcItem) -> Result<Pixel, String> {
        let asset = |name: &str| {
            item.assets
                .get(name)
                .ok_or_else(|| format!("item {} has no {name} asset", item.id))
        };
        let (value_href, qa_href) = (asset(self.band.variable)?, asset(self.band.qa)?);
        let tile = (self.h, self.v);
        let (value, qa) = tokio::join!(
            modis_pc::read_point(&self.client, value_href, tile, self.x, self.y),
            modis_pc::read_point(&self.client, qa_href, tile, self.x, self.y),
        );
        let (value, qa) = (value?, qa?);
        if (value.col, value.row) != (qa.col, qa.row) {
            return Err(format!(
                "{} and {} put the point in different pixels",
                self.band.variable, self.band.qa
            ));
        }
        if !value.raw.is_finite() || !qa.raw.is_finite() {
            return Err(format!("non-finite pixel in {}", item.id));
        }
        Ok(Pixel {
            value: value.raw as i64,
            qa: qa.raw as i64,
            col: value.col,
            row: value.row,
        })
    }
}

/// What the Planetary Computer can say for the window.
#[derive(Debug, Clone, PartialEq)]
enum Pick {
    /// The composite the ORNL reader would pick, read.
    Found {
        item: ModisPcItem,
        start: i64,
        px: Pixel,
        value: f64,
    },
    /// Every composite in the window was read, or is not produced yet, and
    /// none is a valid observation: what the ORNL reader says when its
    /// subset holds nothing valid.
    Nothing {
        nodata: usize,
        rejected_for_qa: usize,
        not_yet: Vec<String>,
        searched: String,
    },
}

/// Walk the window's composites in the ORNL reader's order and stop at the
/// first valid one, so the pick is the one ORNL would make. `Err` means the
/// Planetary Computer cannot answer for this window and ORNL should be
/// asked: the search failed, a read failed, or a composite that ranks
/// before the pick is missing and old enough that ORNL may hold it.
async fn pick<S: CompositeSource>(b: &ModisBand, src: &S, w: Window) -> Result<Pick, String> {
    let collection = b.pc.map_or("", |(c, _)| c);
    // Lower rank first; the earlier of two at one rank, as ORNL's
    // chronological scan with a strict `<` keeps the first.
    let mut ranked: Vec<((u8, i64), i64)> = b
        .composite
        .starts_in(w.lo, w.hi)
        .into_iter()
        .filter_map(|s| Some((w.pick.rank(s)?, s)))
        .collect();
    ranked.sort();
    let to = (w.hi + b.lookahead_days() * DAY).min(w.now).max(w.hi);
    // A collection can hold the Aqua and combined products beside Terra's
    // (MYD11A2 and MOD11A2 in modis-11A2-061): only the product's own count.
    let prefix = format!("{}.", b.product);
    let mut by_start: BTreeMap<i64, ModisPcItem> = BTreeMap::new();
    for item in src.items(w.lo, to).await? {
        if !item.id.starts_with(&prefix) {
            continue;
        }
        let Some(start) = date_unix(&item.start_date) else {
            continue;
        };
        // Two granules for one composite: the later production, whose id
        // ends in the later production time.
        if by_start.get(&start).is_none_or(|prev| prev.id < item.id) {
            by_start.insert(start, item);
        }
    }
    let newest = by_start.keys().next_back().copied();
    let (mut nodata, mut rejected_for_qa, mut not_yet) = (0, 0, Vec::new());
    for (_, start) in ranked {
        let Some(item) = by_start.get(&start) else {
            let recent = b.composite.end(start) > w.now - b.produced_within_days * DAY;
            if recent && newest.is_none_or(|n| n < start) {
                not_yet.push(b.composite.label(start));
                continue;
            }
            return Err(format!(
                "no {collection} item for {} at {}",
                b.composite.label(start),
                src.tile()
            ));
        };
        let px = src.read(item).await?;
        let Some(value) = crate::ornl_value(b.product, px.value, b.scale) else {
            nodata += 1;
            continue;
        };
        if !crate::ornl_qc_gate(b.product, px.qa) {
            rejected_for_qa += 1;
            continue;
        }
        return Ok(Pick::Found {
            item: item.clone(),
            start,
            px,
            value,
        });
    }
    Ok(Pick::Nothing {
        nodata,
        rejected_for_qa,
        not_yet,
        searched: format!(
            "{}?collections={collection}&modis:tile={}&datetime={}/{}",
            emem_fetch::stac::STAC_MPC_V1,
            src.tile(),
            date_of(w.lo),
            date_of(to)
        ),
    })
}

/// Sign what the Planetary Computer said: the composite's value, or what
/// the ORNL reader does when nothing is valid (an Absence at the tslot it
/// would anchor one to, or its error).
#[allow(clippy::too_many_arguments)]
async fn sign(
    b: &ModisBand,
    cell64: &str,
    s: &AppState,
    tile: &str,
    lat: f64,
    lng: f64,
    w: Window,
    p: Pick,
) -> Result<FactCid, String> {
    let fn_key = b.pc.map_or("", |(_, key)| key);
    match p {
        Pick::Found {
            item,
            start,
            px,
            value,
        } => {
            let href = |name: &str| item.assets.get(name).cloned().unwrap_or_default();
            let cal_date = date_of(start);
            let tslot = Tslot::from_unix(start, b.tempo).0;
            if b.ornl == OrnlReader::Ndvi && !crate::force_resign() {
                if let Some(cid) =
                    crate::existing_same_acquisition(s, cell64, b.band, tslot, &cal_date).await
                {
                    return Ok(cid);
                }
            }
            let signed_at = crate::chrono_iso8601_utc();
            let fact = Fact::Primary(PrimaryFact {
                cell: cell64.to_string(),
                band: b.band.to_string(),
                tslot,
                value: ciborium::Value::Float(value),
                unit: b.unit.map(str::to_string),
                confidence: crate::ornl_qc_confidence(b.product, px.qa),
                uncertainty: None,
                sources: vec![Source {
                    scheme: SCHEME.into(),
                    // Unsigned asset URLs: a SAS token expires within the
                    // hour, and anyone can mint a new one.
                    id: format!("{} ; {}", href(b.variable), href(b.qa)),
                    cid: None,
                    hash: None,
                    captured_at: Some(cal_date.clone()),
                    url: None,
                }],
                derivation: Derivation {
                    fn_key: fn_key.into(),
                    // The ORNL subset reader's seven arguments, in its order
                    // (lat, lng, product, variable, composite date, scale,
                    // QA value), then the item and the pixel read.
                    args: Some(ciborium::Value::Array(vec![
                        ciborium::Value::Float(lat),
                        ciborium::Value::Float(lng),
                        ciborium::Value::Text(b.product.into()),
                        ciborium::Value::Text(b.variable.into()),
                        ciborium::Value::Text(cal_date),
                        ciborium::Value::Float(b.scale),
                        ciborium::Value::Integer(px.qa.into()),
                        ciborium::Value::Text(item.id.clone()),
                        ciborium::Value::Text(tile.to_string()),
                        ciborium::Value::Integer(px.col.into()),
                        ciborium::Value::Integer(px.row.into()),
                    ])),
                },
                privacy_class: "public".into(),
                schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
                signer: s.identity.pubkey,
                signed_at: signed_at.clone(),
                served_via: None,
            });
            crate::sign_and_persist(s, fact, &signed_at).await
        }
        Pick::Nothing {
            nodata,
            rejected_for_qa,
            not_yet,
            searched,
        } => {
            let mut where_ = format!(" on the Planetary Computer, tile {tile}");
            if !not_yet.is_empty() {
                where_.push_str(&format!(" (not produced yet: {})", not_yet.join(", ")));
            }
            if b.ornl == OrnlReader::Ndvi {
                return Err(format!(
                    "{}{where_}",
                    crate::ndvi_none_valid((w.lo, w.hi), w.target().is_some(), rejected_for_qa)
                ));
            }
            let mut reason = match w.target() {
                Some(t) => format!(
                    "no valid {} {} observation in ±{}d around unix={t}",
                    b.product, b.variable, b.half_window_days
                ),
                None => format!(
                    "no valid {} {} observation from {} to {}",
                    b.product,
                    b.variable,
                    date_of(w.lo),
                    date_of(w.hi)
                ),
            };
            reason.push_str(&where_);
            if nodata > 0 {
                reason.push_str(&format!(" ({nodata} composite(s) hold a fill value)"));
            }
            if rejected_for_qa > 0 {
                reason.push_str(&format!(
                    " ({rejected_for_qa} composite(s) rejected by {} QC gate)",
                    b.product
                ));
            }
            let signed_at = crate::chrono_iso8601_utc();
            let tslot = Tslot::from_unix(w.target().unwrap_or(w.now), b.tempo).0;
            crate::sign_band_absence(
                cell64, s, b.band, tslot, SCHEME, &searched, &signed_at, &reason,
            )
            .await
        }
    }
}

/// Read the composite from the Planetary Computer and sign it, or ask
/// `ornl` when the Planetary Computer cannot answer within `budget`. A
/// signing error is returned as is: ORNL would hit the same store.
#[allow(clippy::too_many_arguments)]
async fn pc_then<S, F, Fut>(
    b: &ModisBand,
    cell64: &str,
    s: &AppState,
    src: &S,
    lat: f64,
    lng: f64,
    w: Window,
    budget: Duration,
    ornl: F,
) -> Result<FactCid, String>
where
    S: CompositeSource,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<FactCid, String>>,
{
    let picked = match tokio::time::timeout(budget, pick(b, src, w)).await {
        Ok(r) => r,
        Err(_) => Err(format!("no answer in {} ms", budget.as_millis())),
    };
    match picked {
        Ok(p) => sign(b, cell64, s, &src.tile(), lat, lng, w, p).await,
        Err(why) => {
            tracing::info!(
                target: "emem::materialize",
                materialize_cell = %cell64,
                materialize_band = b.band,
                pc_miss = %why,
                "planetary computer cannot answer; asking ORNL"
            );
            ornl()
                .await
                .map_err(|e| format!("{e} (after the Planetary Computer: {why})"))
        }
    }
}

/// `b` at a point: the Planetary Computer's COGs first, `ornl` when they
/// cannot answer or the band is not there.
pub(crate) async fn pc_then_ornl<F, Fut>(
    b: &'static ModisBand,
    cell64: &str,
    s: &AppState,
    lat: f64,
    lng: f64,
    pick: DatedPick,
    ornl: F,
) -> Result<FactCid, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<FactCid, String>>,
{
    let Some((collection, _)) = b.pc else {
        return ornl().await;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (x, y) = modis_pc::sinusoidal_xy(lat, lng);
    let Some((h, v)) = modis_pc::tile_of(x, y) else {
        return ornl().await;
    };
    let src = PcSource {
        band: b,
        collection,
        client: crate::s2_http_client(),
        x,
        y,
        h,
        v,
    };
    let w = Window::new(b, pick, now);
    pc_then(b, cell64, s, &src, lat, lng, w, PC_BUDGET, ornl).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    fn day(y: i32, m: u32, d: u32) -> i64 {
        crate::days_from_civil(y, m, d) * DAY
    }

    fn band(name: &str) -> &'static ModisBand {
        spec(name).expect("in the table")
    }

    fn burn() -> &'static ModisBand {
        band("modis.burned_area_monthly")
    }

    fn lst() -> &'static ModisBand {
        band("modis.lst_day_8day")
    }

    fn ndvi() -> &'static ModisBand {
        band("modis.ndvi_mean")
    }

    /// An item of `product` starting on `start` (`YYYY-MM-DD`), produced at
    /// `prod`.
    fn item_of(b: &ModisBand, product: &str, start: &str, prod: &str) -> ModisPcItem {
        let base = format!(
            "https://modiseuwest.blob.core.windows.net/modis-061-cogs/{product}/30/10/{start}/x"
        );
        ModisPcItem {
            id: format!("{product}.A{start}.h30v10.061.{prod}"),
            start_date: start.to_string(),
            h: 30,
            v: 10,
            assets: BTreeMap::from([
                (b.variable.to_string(), format!("{base}_{}.tif", b.variable)),
                (b.qa.to_string(), format!("{base}_{}.tif", b.qa)),
            ]),
        }
    }

    /// Items and pixels by composite; a composite with no pixel entry fails
    /// to read.
    struct Fake {
        items: Result<Vec<ModisPcItem>, String>,
        pixels: BTreeMap<String, (i64, i64)>,
        read_log: Mutex<Vec<String>>,
        stall: Option<Duration>,
    }

    impl Fake {
        fn new(b: &ModisBand, items: &[(&str, Option<(i64, i64)>)]) -> Self {
            Fake {
                items: Ok(items
                    .iter()
                    .map(|(m, _)| item_of(b, b.product, m, "2024001000000"))
                    .collect()),
                pixels: items
                    .iter()
                    .filter_map(|(m, p)| p.map(|p| (m.to_string(), p)))
                    .collect(),
                read_log: Mutex::new(Vec::new()),
                stall: None,
            }
        }
        fn reads(&self) -> Vec<String> {
            self.read_log.lock().unwrap().clone()
        }
    }

    impl CompositeSource for Fake {
        fn tile(&self) -> String {
            "h30v10".into()
        }
        async fn items(&self, _from: i64, _to: i64) -> Result<Vec<ModisPcItem>, String> {
            if let Some(d) = self.stall {
                tokio::time::sleep(d).await;
            }
            self.items.clone()
        }
        async fn read(&self, item: &ModisPcItem) -> Result<Pixel, String> {
            self.read_log.lock().unwrap().push(item.start_date.clone());
            let (value, qa) = self
                .pixels
                .get(&item.start_date)
                .copied()
                .ok_or_else(|| format!("read {}: status 503", item.id))?;
            Ok(Pixel {
                value,
                qa,
                col: 2246,
                row: 600,
            })
        }
    }

    /// A target in mid-August 2023, a long time before `now`.
    fn aug_2023() -> Window {
        Window::new(
            burn(),
            DatedPick::Nearest(day(2023, 8, 14)),
            day(2026, 11, 14),
        )
    }

    fn now() -> i64 {
        day(2026, 11, 14)
    }

    #[test]
    fn every_band_has_one_row_and_pc_names_its_own_stamp() {
        for (i, b) in BANDS.iter().enumerate() {
            assert!(
                BANDS[i + 1..].iter().all(|o| o.band != b.band),
                "{} twice",
                b.band
            );
            if let Some((collection, fn_key)) = b.pc {
                assert!(collection.starts_with("modis-"), "{collection}");
                assert!(fn_key.starts_with("modis_pc_"), "{fn_key}");
                assert_ne!(fn_key, b.ornl_fn_key);
            }
        }
        assert!(
            spec("modis.et_8day").unwrap().pc.is_none(),
            "MOD16A2 is ORNL's"
        );
        assert!(spec("modis.nope").is_none());
    }

    #[test]
    fn months_in_take_whole_days() {
        let m = Composite::Month;
        assert_eq!(
            m.starts_in(day(2023, 7, 13), day(2023, 9, 15)),
            vec![day(2023, 8, 1), day(2023, 9, 1)]
        );
        // A month whose first day is the window's first day is in it, at
        // any hour of that day, as ORNL's startDate takes it.
        assert_eq!(
            m.starts_in(day(2023, 8, 1) + 3600 * 13, day(2023, 8, 2)),
            vec![day(2023, 8, 1)]
        );
        assert_eq!(
            m.starts_in(day(2023, 12, 2), day(2024, 2, 1)),
            vec![day(2024, 1, 1), day(2024, 2, 1)]
        );
        assert!(m.starts_in(day(2023, 8, 2), day(2023, 8, 30)).is_empty());
    }

    /// 8-day composites start on days 1, 9, 17, ... of each year, so the
    /// last one starts on day 361 and the next year starts again on day 1.
    #[test]
    fn eight_day_composites_restart_each_year() {
        let c = Composite::Days(8);
        // ORNL returned A2024193 and A2024201 for startDate=A2024186,
        // endDate=A2024201 (2026-11-15): A2024185 starts before the window.
        assert_eq!(
            c.starts_in(day(2024, 7, 4) + 3600 * 5, day(2024, 7, 19)),
            vec![day(2024, 7, 11), day(2024, 7, 19)]
        );
        // Day 361 of 2024 (a leap year) is 26 December; 2025 starts on 1
        // January, not on 3 January.
        assert_eq!(
            c.starts_in(day(2024, 12, 20), day(2025, 1, 12)),
            vec![day(2024, 12, 26), day(2025, 1, 1), day(2025, 1, 9)]
        );
        // The last composite of a year is short: it stops at 31 December.
        assert_eq!(c.end(day(2024, 12, 26)), day(2025, 1, 1));
        assert_eq!(c.end(day(2023, 12, 27)), day(2024, 1, 1));
        assert_eq!(c.end(day(2024, 7, 11)), day(2024, 7, 19));
        assert_eq!(c.label(day(2024, 7, 11)), "2024-07-11");
    }

    /// 16-day composites start on days 1, 17, 33, ...: the last of a year
    /// on day 353, 13 or 14 days long.
    #[test]
    fn sixteen_day_composites_restart_each_year() {
        let c = Composite::Days(16);
        assert_eq!(
            c.starts_in(day(2024, 6, 9), day(2024, 7, 11)),
            vec![day(2024, 6, 9), day(2024, 6, 25), day(2024, 7, 11)]
        );
        assert_eq!(
            c.starts_in(day(2023, 12, 1), day(2024, 1, 20)),
            vec![
                day(2023, 12, 3),
                day(2023, 12, 19),
                day(2024, 1, 1),
                day(2024, 1, 17)
            ]
        );
        assert_eq!(c.end(day(2023, 12, 19)), day(2024, 1, 1));
    }

    #[test]
    fn rank_matches_ornl_priority() {
        let w = Window::new(burn(), DatedPick::Nearest(day(2023, 8, 25)), now());
        let mut m: Vec<_> = [day(2023, 8, 1), day(2023, 9, 1)]
            .into_iter()
            .map(|s| (w.pick.rank(s).unwrap(), s))
            .collect();
        m.sort();
        assert_eq!(m[0].1, day(2023, 9, 1));
        // Equal distance: the earlier month, as ORNL's chronological scan
        // with a strict `<` keeps the first.
        let mid = (day(2023, 8, 1) + day(2023, 9, 1)) / 2;
        let w = Window::new(burn(), DatedPick::Nearest(mid), now());
        let mut m: Vec<_> = [day(2023, 9, 1), day(2023, 8, 1)]
            .into_iter()
            .map(|s| (w.pick.rank(s).unwrap(), s))
            .collect();
        m.sort();
        assert_eq!(m[0].1, day(2023, 8, 1));
    }

    #[test]
    fn window_is_the_ornl_window() {
        let w = aug_2023();
        assert_eq!(w.lo, day(2023, 8, 14) - 32 * DAY);
        assert_eq!(w.hi, day(2023, 8, 14) + 32 * DAY);
        let w = Window::new(burn(), DatedPick::Newest, now());
        assert_eq!((w.lo, w.hi), (now() - 9 * 31 * DAY, now()));
        let w = Window::new(lst(), DatedPick::Newest, now());
        assert_eq!((w.lo, w.hi), (now() - 72 * DAY, now()));
        let w = Window::new(lst(), DatedPick::Nearest(now() - 3 * DAY), now());
        assert_eq!((w.lo, w.hi), (now() - 11 * DAY, now()));
        // NDVI keeps its own: 90 days back, ±32 around a target.
        let w = Window::new(ndvi(), DatedPick::Newest, now());
        assert_eq!((w.lo, w.hi), (now() - 90 * DAY, now()));
        let t = day(2024, 7, 1);
        let w = Window::new(ndvi(), DatedPick::Nearest(t), now());
        assert_eq!((w.lo, w.hi), (t - 32 * DAY, t + 32 * DAY));
    }

    #[tokio::test]
    async fn picks_the_month_nearest_the_target() {
        let src = Fake::new(
            burn(),
            &[
                ("2023-07-01", Some((190, 3))),
                ("2023-08-01", Some((230, 3))),
                ("2023-09-01", Some((250, 3))),
            ],
        );
        let p = pick(burn(), &src, aug_2023()).await.unwrap();
        let Pick::Found {
            start, px, item, ..
        } = p
        else {
            panic!("expected a pick, got {p:?}")
        };
        assert_eq!(start, day(2023, 8, 1));
        assert_eq!(px.value, 230);
        assert!(item.id.contains("2023-08-01"));
        assert_eq!(src.reads(), vec!["2023-08-01"], "one read, no more");
    }

    #[tokio::test]
    async fn unburned_land_is_a_valid_zero() {
        let src = Fake::new(
            burn(),
            &[("2023-08-01", Some((0, 3))), ("2023-09-01", Some((250, 3)))],
        );
        let Pick::Found { px, value, .. } = pick(burn(), &src, aug_2023()).await.unwrap() else {
            panic!("expected a pick")
        };
        assert_eq!((px.value, value), (0, 0.0));
    }

    /// -1 (unmapped) and -2 (water) are not days of year: the next month
    /// in the ORNL order is read instead, as the ORNL reader skips them.
    #[tokio::test]
    async fn fill_values_pass_to_the_next_month() {
        let src = Fake::new(
            burn(),
            &[("2023-08-01", Some((-1, 3))), ("2023-09-01", Some((-2, 2)))],
        );
        let p = pick(burn(), &src, aug_2023()).await.unwrap();
        assert!(
            matches!(
                p,
                Pick::Nothing {
                    nodata: 2,
                    rejected_for_qa: 0,
                    ..
                }
            ),
            "{p:?}"
        );
        let src = Fake::new(
            burn(),
            &[
                ("2023-08-01", Some((-1, 3))),
                ("2023-09-01", Some((250, 3))),
            ],
        );
        let Pick::Found { start, .. } = pick(burn(), &src, aug_2023()).await.unwrap() else {
            panic!("expected a pick")
        };
        assert_eq!(start, day(2023, 9, 1));
    }

    /// QA bit 0 clear is a non-land cell: rejected even with a burn date,
    /// the gate the ORNL reader applies.
    #[tokio::test]
    async fn qa_land_bit_gates_the_value() {
        let src = Fake::new(
            burn(),
            &[("2023-08-01", Some((230, 2))), ("2023-09-01", Some((0, 3)))],
        );
        let Pick::Found { start, px, .. } = pick(burn(), &src, aug_2023()).await.unwrap() else {
            panic!("expected a pick")
        };
        assert_eq!((start, px.value), (day(2023, 9, 1), 0));
        let src = Fake::new(
            burn(),
            &[
                ("2023-08-01", Some((230, 0))),
                ("2023-09-01", Some((231, 2))),
            ],
        );
        let p = pick(burn(), &src, aug_2023()).await.unwrap();
        assert!(
            matches!(
                p,
                Pick::Nothing {
                    nodata: 0,
                    rejected_for_qa: 2,
                    ..
                }
            ),
            "{p:?}"
        );
    }

    #[tokio::test]
    async fn search_failure_is_a_miss() {
        let mut src = Fake::new(burn(), &[]);
        src.items = Err("stac status 503 Service Unavailable".into());
        let e = pick(burn(), &src, aug_2023()).await.unwrap_err();
        assert!(e.contains("503"), "{e}");
    }

    #[tokio::test]
    async fn read_failure_is_a_miss() {
        let src = Fake::new(
            burn(),
            &[("2023-08-01", None), ("2023-09-01", Some((250, 3)))],
        );
        let e = pick(burn(), &src, aug_2023()).await.unwrap_err();
        assert!(e.contains("503"), "{e}");
    }

    #[tokio::test]
    async fn no_item_for_an_old_month_is_a_miss() {
        let src = Fake::new(burn(), &[]);
        let e = pick(burn(), &src, aug_2023()).await.unwrap_err();
        assert!(e.contains("no modis-64A1-061 item for 2023-08"), "{e}");
    }

    /// A hole that ranks before a month the catalogue holds: ORNL may hold
    /// the hole, so the read goes there rather than moving the pick.
    #[tokio::test]
    async fn a_hole_never_moves_the_pick() {
        let src = Fake::new(
            burn(),
            &[
                ("2023-07-01", Some((190, 3))),
                ("2023-09-01", Some((250, 3))),
            ],
        );
        let w = Window::new(burn(), DatedPick::Nearest(day(2023, 8, 10)), now());
        let e = pick(burn(), &src, w).await.unwrap_err();
        assert!(e.contains("2023-08"), "{e}");
        assert!(src.reads().is_empty(), "nothing read past the hole");
    }

    /// The newest months are not produced yet, by anyone: skipped, and the
    /// newest month the catalogue holds is the answer.
    #[tokio::test]
    async fn recent_months_not_yet_produced_are_skipped() {
        let src = Fake::new(
            burn(),
            &[("2026-08-01", Some((0, 3))), ("2026-09-01", Some((262, 3)))],
        );
        let w = Window::new(burn(), DatedPick::Newest, now());
        let Pick::Found { start, .. } = pick(burn(), &src, w).await.unwrap() else {
            panic!("expected a pick")
        };
        assert_eq!(start, day(2026, 9, 1));
        assert_eq!(src.reads(), vec!["2026-09-01"]);
        // A target in a month not produced yet: an Absence from here, as
        // ORNL has no composite to give either.
        let src = Fake::new(burn(), &[("2026-09-01", Some((262, 3)))]);
        let w = Window::new(burn(), DatedPick::Nearest(day(2026, 11, 1)), now());
        let p = pick(burn(), &src, w).await.unwrap();
        let Pick::Nothing { not_yet, .. } = p else {
            panic!("expected nothing, got {p:?}")
        };
        assert_eq!(not_yet, vec!["2026-11", "2026-10"]);
    }

    /// A recent month missing while a newer one is held is a hole, not a
    /// month still being produced.
    #[tokio::test]
    async fn a_recent_hole_behind_a_newer_month_is_a_miss() {
        let src = Fake::new(
            burn(),
            &[("2026-08-01", Some((0, 3))), ("2026-10-01", Some((0, 3)))],
        );
        let w = Window::new(burn(), DatedPick::Nearest(day(2026, 9, 3)), now());
        let e = pick(burn(), &src, w).await.unwrap_err();
        assert!(e.contains("2026-09"), "{e}");
    }

    /// An old month the catalogue lacks is never taken as "not produced
    /// yet", even when it is past the newest item: the catalogue may have
    /// stopped. September 2026 ended 45 days before `now`, so it still
    /// counts as recent; August is the first month ORNL may hold.
    #[tokio::test]
    async fn a_stalled_catalogue_is_a_miss() {
        let src = Fake::new(burn(), &[("2026-05-01", Some((0, 3)))]);
        let w = Window::new(burn(), DatedPick::Newest, now());
        let e = pick(burn(), &src, w).await.unwrap_err();
        assert!(e.contains("no modis-64A1-061 item for 2026-08"), "{e}");
        assert!(src.reads().is_empty());
    }

    #[tokio::test]
    async fn the_later_granule_of_a_month_wins() {
        let mut src = Fake::new(burn(), &[("2023-08-01", Some((230, 3)))]);
        src.items = Ok(vec![
            item_of(burn(), "MCD64A1", "2023-08-01", "2023284120835"),
            item_of(burn(), "MCD64A1", "2023-08-01", "2021307215246"),
        ]);
        let Pick::Found { item, .. } = pick(burn(), &src, aug_2023()).await.unwrap() else {
            panic!("expected a pick")
        };
        assert!(item.id.ends_with("2023284120835"));
    }

    /// LST with no target: the newest 8-day composite whose QC passes. The
    /// two newest are not produced yet (ended under 10 days ago), a cloudy
    /// one is rejected by QC bits 0-1, and 0 is MOD11A2's fill.
    #[tokio::test]
    async fn lst_takes_the_newest_composite_that_passes_qc() {
        let src = Fake::new(
            lst(),
            &[
                ("2026-10-08", Some((15100, 0))),
                ("2026-10-16", Some((14900, 0b10))),
                ("2026-10-24", Some((0, 0))),
            ],
        );
        let w = Window::new(lst(), DatedPick::Newest, now());
        let p = pick(lst(), &src, w).await.unwrap();
        let Pick::Found {
            start, value, px, ..
        } = p
        else {
            panic!("expected a pick, got {p:?}")
        };
        assert_eq!(start, day(2026, 10, 8));
        assert_eq!(value, 15100.0 * 0.02);
        assert_eq!(px.qa, 0);
        assert_eq!(src.reads(), vec!["2026-10-24", "2026-10-16", "2026-10-08"]);
    }

    /// 2026-11-01 ended on 2026-11-08, six days before `now`: not produced
    /// yet. 2026-10-24 ended 14 days before: ORNL may hold it, so its
    /// absence from the catalogue is a miss.
    #[tokio::test]
    async fn an_eight_day_composite_ten_days_past_its_end_is_a_hole() {
        let src = Fake::new(lst(), &[("2026-10-16", Some((15100, 0)))]);
        let w = Window::new(lst(), DatedPick::Newest, now());
        let e = pick(lst(), &src, w).await.unwrap_err();
        assert!(e.contains("no modis-11A2-061 item for 2026-10-24"), "{e}");
        let src = Fake::new(
            lst(),
            &[
                ("2026-10-16", Some((15100, 0))),
                ("2026-10-24", Some((15200, 0))),
            ],
        );
        let p = pick(lst(), &src, w).await.unwrap();
        let Pick::Found { start, .. } = p else {
            panic!("expected a pick, got {p:?}")
        };
        assert_eq!(start, day(2026, 10, 24));
    }

    /// modis-11A2-061 holds MYD11A2 (Aqua) beside MOD11A2 (Terra); ORNL's
    /// MOD11A2 is Terra's alone.
    #[tokio::test]
    async fn another_platform_in_the_collection_is_ignored() {
        let mut src = Fake::new(lst(), &[("2024-07-11", Some((15147, 0)))]);
        src.items = Ok(vec![
            item_of(lst(), "MYD11A2", "2024-07-11", "2024202043251"),
            item_of(lst(), "MOD11A2", "2024-07-11", "2024202041819"),
            item_of(lst(), "MYD11A2", "2024-07-03", "2024195055526"),
        ]);
        let w = Window::new(lst(), DatedPick::Nearest(day(2024, 7, 11)), now());
        let Pick::Found { item, .. } = pick(lst(), &src, w).await.unwrap() else {
            panic!("expected a pick")
        };
        assert!(item.id.starts_with("MOD11A2."), "{}", item.id);
        // Only Aqua holds 2024-07-03, which ranks second: a hole for Terra.
        let mut src = Fake::new(lst(), &[]);
        src.items = Ok(vec![item_of(
            lst(),
            "MYD11A2",
            "2024-07-03",
            "2024195055526",
        )]);
        let w = Window::new(lst(), DatedPick::Nearest(day(2024, 7, 4)), now());
        let e = pick(lst(), &src, w).await.unwrap_err();
        assert!(e.contains("2024-07-03"), "{e}");
    }

    /// NDVI keeps the MOD13Q1 reader's rules: -3000 is the fill, a value
    /// outside [-0.2, 1.0] is not NDVI, and pixel_reliability 2 (snow) and
    /// 3 (cloud) fail the gate.
    #[tokio::test]
    async fn ndvi_keeps_the_mod13q1_gate() {
        let t = day(2024, 7, 11);
        let src = Fake::new(
            ndvi(),
            &[
                ("2024-07-11", Some((5322, 3))),
                ("2024-06-25", Some((-3000, 0))),
                ("2024-07-27", Some((-2500, 0))),
                ("2024-06-09", Some((6100, 1))),
            ],
        );
        let w = Window::new(ndvi(), DatedPick::Nearest(t), now());
        let p = pick(ndvi(), &src, w).await.unwrap();
        let Pick::Found {
            start, value, px, ..
        } = p
        else {
            panic!("expected a pick, got {p:?}")
        };
        assert_eq!((start, px.qa), (day(2024, 6, 9), 1));
        assert_eq!(value, 6100.0 * 1e-4);
        assert_eq!(
            src.reads(),
            vec!["2024-07-11", "2024-06-25", "2024-07-27", "2024-06-09"]
        );
    }

    /// An as-of slot takes the newest composite at or before its end and
    /// never reads one after it, though the catalogue holds it.
    #[tokio::test]
    async fn an_as_of_slot_skips_later_composites() {
        let start = day(2024, 7, 1);
        let pick_ = DatedPick::Slot {
            start,
            end: start + 8 * DAY - 1,
            at_or_before: true,
        };
        let src = Fake::new(
            ndvi(),
            &[
                ("2024-06-09", Some((4000, 0))),
                ("2024-06-25", Some((5000, 0))),
                ("2024-07-11", Some((6000, 0))),
            ],
        );
        let w = Window::new(ndvi(), pick_, now());
        let Pick::Found { start: got, .. } = pick(ndvi(), &src, w).await.unwrap() else {
            panic!("expected a pick")
        };
        assert_eq!(got, day(2024, 6, 25));
        assert_eq!(src.reads(), vec!["2024-06-25"]);
    }

    async fn ornl_fake(asked: &AtomicBool) -> Result<FactCid, String> {
        asked.store(true, Ordering::SeqCst);
        Err("ornl subset: status 400".to_string())
    }

    /// The fallback path: a Planetary Computer that cannot answer hands the
    /// read to ORNL, and the error names both sources.
    #[tokio::test]
    async fn a_failing_planetary_computer_falls_back_to_ornl() {
        let s = crate::tests::test_app_state();
        let cell = emem_codec::cell64_from_latlng(-12.5, 132.5);
        let mut src = Fake::new(burn(), &[]);
        src.items = Err("stac status 503".into());
        let asked = AtomicBool::new(false);
        let e = pc_then(
            burn(),
            &cell,
            &s,
            &src,
            -12.5,
            132.5,
            aug_2023(),
            PC_BUDGET,
            || ornl_fake(&asked),
        )
        .await
        .unwrap_err();
        assert!(asked.load(Ordering::SeqCst), "ORNL was asked");
        assert!(e.contains("ornl subset: status 400"), "{e}");
        assert!(e.contains("Planetary Computer: stac status 503"), "{e}");

        // A Planetary Computer that does not answer in its budget too.
        let mut src = Fake::new(burn(), &[("2023-08-01", Some((230, 3)))]);
        src.stall = Some(Duration::from_secs(5));
        let asked = AtomicBool::new(false);
        let e = pc_then(
            burn(),
            &cell,
            &s,
            &src,
            -12.5,
            132.5,
            aug_2023(),
            Duration::from_millis(50),
            || ornl_fake(&asked),
        )
        .await
        .unwrap_err();
        assert!(asked.load(Ordering::SeqCst));
        assert!(e.contains("no answer in 50 ms"), "{e}");
    }

    /// A band the Planetary Computer does not hold goes straight to ORNL.
    #[tokio::test]
    async fn a_band_not_on_the_planetary_computer_reads_ornl() {
        let s = crate::tests::test_app_state();
        let cell = emem_codec::cell64_from_latlng(-12.5, 132.5);
        let asked = AtomicBool::new(false);
        let e = pc_then_ornl(
            band("modis.et_8day"),
            &cell,
            &s,
            -12.5,
            132.5,
            DatedPick::Newest,
            || ornl_fake(&asked),
        )
        .await
        .unwrap_err();
        assert!(asked.load(Ordering::SeqCst));
        assert_eq!(e, "ornl subset: status 400");
    }

    /// A read the Planetary Computer answers is signed from it, names it,
    /// and never asks ORNL.
    #[tokio::test]
    async fn a_planetary_computer_read_names_its_source() {
        let s = crate::tests::test_app_state();
        let cell = emem_codec::cell64_from_latlng(-12.5, 132.5);
        let src = Fake::new(burn(), &[("2023-08-01", Some((230, 3)))]);
        let asked = AtomicBool::new(false);
        let cid = pc_then(
            burn(),
            &cell,
            &s,
            &src,
            -12.5,
            132.5,
            aug_2023(),
            PC_BUDGET,
            || ornl_fake(&asked),
        )
        .await
        .expect("signed");
        assert!(!asked.load(Ordering::SeqCst), "ORNL not asked");
        let got = s
            .storage
            .get_facts_many_uncited(std::slice::from_ref(&cid))
            .await
            .unwrap();
        let Some(Fact::Primary(f)) = got.into_iter().next().flatten() else {
            panic!("a primary fact")
        };
        assert_eq!(f.band, "modis.burned_area_monthly");
        assert_eq!(f.value, ciborium::Value::Float(230.0));
        assert_eq!(f.unit.as_deref(), Some("doy"));
        assert_eq!(f.tslot, Tslot::from_unix(day(2023, 8, 1), Tempo::Medium).0);
        assert_eq!(f.confidence, crate::ornl_qc_confidence("MCD64A1", 3));
        assert_eq!(f.sources.len(), 1);
        assert_eq!(f.sources[0].scheme, SCHEME);
        assert_eq!(f.sources[0].captured_at.as_deref(), Some("2023-08-01"));
        assert!(
            f.sources[0].id.contains("_Burn_Date.tif ; "),
            "{}",
            f.sources[0].id
        );
        assert!(f.sources[0].id.ends_with("_QA.tif"));
        assert!(!f.sources[0].id.contains("ornl"));
        assert_eq!(f.derivation.fn_key, "modis_pc_mcd64a1_burndate@1");
        let Some(ciborium::Value::Array(args)) = &f.derivation.args else {
            panic!("args")
        };
        assert_eq!(args[2], ciborium::Value::Text("MCD64A1".into()));
        assert_eq!(args[4], ciborium::Value::Text("2023-08-01".into()));
        assert_eq!(args[6], ciborium::Value::Integer(3.into()));
        assert!(args.contains(&ciborium::Value::Text("h30v10".into())));
        assert!(
            args.contains(&ciborium::Value::Text(crate::COG_READER.into())),
            "the COG reader stamp"
        );
    }

    /// An 8-day fact keeps the ORNL unit, scale, tempo and QC confidence,
    /// and names its own reader.
    #[tokio::test]
    async fn an_lst_fact_keeps_the_ornl_meaning() {
        let s = crate::tests::test_app_state();
        let cell = emem_codec::cell64_from_latlng(42.0, -93.6);
        let t = day(2024, 7, 12);
        let src = Fake::new(lst(), &[("2024-07-11", Some((15147, 0b0100_0001)))]);
        let w = Window::new(lst(), DatedPick::Nearest(t), now());
        let asked = AtomicBool::new(false);
        let cid = pc_then(lst(), &cell, &s, &src, 42.0, -93.6, w, PC_BUDGET, || {
            ornl_fake(&asked)
        })
        .await
        .expect("signed");
        assert!(!asked.load(Ordering::SeqCst));
        let got = s
            .storage
            .get_facts_many_uncited(std::slice::from_ref(&cid))
            .await
            .unwrap();
        let Some(Fact::Primary(f)) = got.into_iter().next().flatten() else {
            panic!("a primary fact")
        };
        assert_eq!(f.band, "modis.lst_day_8day");
        assert_eq!(f.value, ciborium::Value::Float(15147.0 * 0.02));
        assert_eq!(f.unit.as_deref(), Some("K"));
        assert_eq!(
            f.tslot,
            Tslot::from_unix(day(2024, 7, 11), Tempo::Composite8Day).0
        );
        assert_eq!(
            f.confidence,
            crate::ornl_qc_confidence("MOD11A2", 0b0100_0001)
        );
        assert_eq!(f.derivation.fn_key, "modis_pc_mod11a2_lstday@1");
        assert!(f.sources[0].id.contains("_LST_Day_1km.tif ; "));
        assert!(f.sources[0].id.ends_with("_QC_Day.tif"));
    }

    /// Nothing valid in the window: an Absence from the Planetary Computer,
    /// at the tslot the ORNL reader anchors one to, without asking ORNL.
    #[tokio::test]
    async fn nothing_valid_is_a_planetary_computer_absence() {
        let s = crate::tests::test_app_state();
        let cell = emem_codec::cell64_from_latlng(-12.5, 132.5);
        let src = Fake::new(
            burn(),
            &[("2023-08-01", Some((-2, 2))), ("2023-09-01", Some((-2, 2)))],
        );
        let asked = AtomicBool::new(false);
        let cid = pc_then(
            burn(),
            &cell,
            &s,
            &src,
            -12.5,
            132.5,
            aug_2023(),
            PC_BUDGET,
            || ornl_fake(&asked),
        )
        .await
        .expect("signed");
        assert!(!asked.load(Ordering::SeqCst));
        let got = s
            .storage
            .get_facts_many_uncited(std::slice::from_ref(&cid))
            .await
            .unwrap();
        let Some(Fact::Absence(f)) = got.into_iter().next().flatten() else {
            panic!("an absence")
        };
        assert_eq!(f.band, "modis.burned_area_monthly");
        assert_eq!(f.tslot, Tslot::from_unix(day(2023, 8, 14), Tempo::Medium).0);
        assert_eq!(f.sources[0].scheme, SCHEME);
        assert!(f.sources[0].id.contains("modis-64A1-061"));
        assert!(f.sources[0].id.contains("h30v10"));
    }

    /// Nothing valid for NDVI is the MOD13Q1 reader's error, not an
    /// Absence, and ORNL is not asked for the same answer.
    #[tokio::test]
    async fn nothing_valid_for_ndvi_is_the_ornl_error() {
        let s = crate::tests::test_app_state();
        let cell = emem_codec::cell64_from_latlng(-3.0, -60.0);
        let t = day(2024, 7, 11);
        let src = Fake::new(
            ndvi(),
            &[
                ("2024-06-09", Some((7000, 3))),
                ("2024-06-25", Some((7000, 3))),
                ("2024-07-11", Some((7000, 2))),
                ("2024-07-27", Some((-3000, 0))),
                ("2024-08-12", Some((7000, 3))),
            ],
        );
        let w = Window::new(ndvi(), DatedPick::Nearest(t), now());
        let asked = AtomicBool::new(false);
        let e = pc_then(ndvi(), &cell, &s, &src, -3.0, -60.0, w, PC_BUDGET, || {
            ornl_fake(&asked)
        })
        .await
        .unwrap_err();
        assert!(!asked.load(Ordering::SeqCst));
        assert!(e.starts_with("no valid NDVI observation from A2024"), "{e}");
        assert!(
            e.contains("(4 composite(s) rejected by pixel_reliability"),
            "{e}"
        );
        assert!(e.contains("on the Planetary Computer, tile h30v10"), "{e}");
    }
}
