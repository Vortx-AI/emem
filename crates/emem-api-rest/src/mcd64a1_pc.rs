//! `modis.burned_area_monthly` from the Planetary Computer's MCD64A1 COGs,
//! with ORNL DAAC's subset service as the fallback.
//!
//! ORNL's service took 16 to 28 s for one month at one point on
//! 2026-11-14, and it was the whole latency of `field_burn_scar`. The
//! Planetary Computer serves the same LP DAAC granules (the item id carries
//! the same production time ORNL reports as `proc_date`) as COGs, and one
//! month is a tile search plus two pixel reads.
//!
//! A fact read here means what the ORNL fact meant. The window is the same,
//! and so is the pick: the valid month nearest the target, or the newest
//! with no target. The fill values are the same (a negative Burn_Date is
//! unmapped or water), and so are the QA gate (bit 0, land) and the
//! confidence it gives. Unit and tempo do not change. The source and the
//! reader stamp change, because the fact names where its bytes came from.
//!
//! The catalogue has holes: months LP DAAC produced and ORNL serves that
//! the Planetary Computer does not hold (on 2026-11-14 every tile checked
//! lacked 2023-09, 2024-07, 2025-05 and 2025-08 to 2025-10). A missing month
//! that ORNL could hold sends the read to ORNL, so a hole never moves the
//! pick to another month. A missing month too recent to have been produced
//! is skipped instead, because no source has it yet.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use emem_core::tslot::{Tempo, Tslot};
use emem_fact::{Derivation, Fact, FactCid, PrimaryFact, SchemaCid, Source};
use emem_fetch::modis_pc::{self, ModisPcItem};

use crate::AppState;

const DAY: i64 = 86_400;
const BAND: &str = "modis.burned_area_monthly";
const PRODUCT: &str = "MCD64A1";
const COLLECTION: &str = "modis-64A1-061";
const VALUE_ASSET: &str = "Burn_Date";
const QA_ASSET: &str = "QA";
/// Reader stamp of a fact read from the Planetary Computer's COGs. The ORNL
/// path keeps `modis_ornl_mcd64a1_burndate@1`.
const FN_KEY: &str = "modis_pc_mcd64a1_burndate@1";
/// Source scheme of a fact read from the Planetary Computer's COGs.
const SCHEME: &str = "mpc_modis_cog";
/// A month that ended less than this long ago may not be produced yet. LP
/// DAAC produced the July, August and September 2026 granules 41 to 43 days
/// after each month ended, and ORNL served August but not September on
/// 2026-11-14.
const NOT_YET_PRODUCED_DAYS: i64 = 45;
/// How long the Planetary Computer read may take before ORNL is asked.
const PC_BUDGET: Duration = Duration::from_secs(12);

/// The window and the target the ORNL path would use.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Window {
    lo: i64,
    hi: i64,
    target: Option<i64>,
    now: i64,
}

impl Window {
    fn new(target: Option<i64>, now: i64) -> Self {
        let (lo, hi) = crate::ornl_band_window(
            crate::MCD64A1_HALF_WINDOW_DAYS,
            crate::MCD64A1_TILE_DAYS,
            target,
            now,
        );
        Window {
            lo,
            hi,
            target,
            now,
        }
    }
}

/// Raw Burn_Date and QA at the point in one item, and the pixel they are in.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MonthPixel {
    burn: i64,
    qa: i64,
    col: i64,
    row: i64,
}

/// Where the month walk gets its items and pixels: the Planetary Computer,
/// or a fake in tests.
trait MonthSource {
    /// `h30v10`, the tile the point is in.
    fn tile(&self) -> String;
    /// The tile's items whose month intersects `[from, to]` (unix seconds).
    async fn items(&self, from: i64, to: i64) -> Result<Vec<ModisPcItem>, String>;
    /// The pixel holding the point in `item`.
    async fn read(&self, item: &ModisPcItem) -> Result<MonthPixel, String>;
}

struct PcSource {
    client: reqwest::Client,
    x: f64,
    y: f64,
    h: u32,
    v: u32,
}

impl MonthSource for PcSource {
    fn tile(&self) -> String {
        format!("h{:02}v{:02}", self.h, self.v)
    }

    async fn items(&self, from: i64, to: i64) -> Result<Vec<ModisPcItem>, String> {
        let datetime = format!(
            "{}/{}",
            crate::iso8601_utc(from.max(0) as u64),
            crate::iso8601_utc(to.max(0) as u64)
        );
        modis_pc::search_tile(&self.client, COLLECTION, self.h, self.v, &datetime, 40).await
    }

    async fn read(&self, item: &ModisPcItem) -> Result<MonthPixel, String> {
        let asset = |name: &str| {
            item.assets
                .get(name)
                .ok_or_else(|| format!("item {} has no {name} asset", item.id))
        };
        let (burn_href, qa_href) = (asset(VALUE_ASSET)?, asset(QA_ASSET)?);
        let (burn, qa) = tokio::join!(
            modis_pc::read_point(&self.client, burn_href, self.x, self.y),
            modis_pc::read_point(&self.client, qa_href, self.x, self.y),
        );
        let (burn, qa) = (burn?, qa?);
        if (burn.col, burn.row) != (qa.col, qa.row) {
            return Err(format!(
                "{} and {} put the point in different pixels",
                VALUE_ASSET, QA_ASSET
            ));
        }
        if !burn.raw.is_finite() || !qa.raw.is_finite() {
            return Err(format!("non-finite pixel in {}", item.id));
        }
        Ok(MonthPixel {
            burn: burn.raw as i64,
            qa: qa.raw as i64,
            col: burn.col,
            row: burn.row,
        })
    }
}

/// What the Planetary Computer can say for the window.
#[derive(Debug, Clone, PartialEq)]
enum Pick {
    /// The month the ORNL path would pick, read.
    Found {
        item: ModisPcItem,
        month: i64,
        px: MonthPixel,
    },
    /// Every month in the window was read, or is not produced yet, and
    /// none is a valid observation: the ORNL path would sign an Absence.
    Nothing {
        nodata: usize,
        rejected_for_qa: usize,
        not_yet: Vec<String>,
        searched: String,
    },
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

/// First days (unix) of the months whose first day falls in `[lo, hi]` by
/// calendar day: the composites ORNL's subset returns for
/// `startDate = day(lo)`, `endDate = day(hi)`.
fn months_in(lo: i64, hi: i64) -> Vec<i64> {
    let (first, last) = (lo.div_euclid(DAY), hi.div_euclid(DAY));
    let mut out = Vec::new();
    let (y, m, d) = crate::civil_from_days(first);
    let mut month = crate::days_from_civil(y, m, 1) * DAY;
    if d != 1 {
        month = next_month(month);
    }
    while month.div_euclid(DAY) <= last {
        out.push(month);
        month = next_month(month);
    }
    out
}

/// The order the ORNL path ranks months in: nearest the target first, the
/// earlier of two at the same distance; with no target, newest first.
fn rank(months: &mut [i64], target: Option<i64>) {
    match target {
        Some(t) => months.sort_by_key(|m| ((m - t).abs(), *m)),
        None => months.sort_by(|a, b| b.cmp(a)),
    }
}

/// Walk the window's months in the ORNL path's order and stop at the first
/// valid one, so the pick is the one ORNL would make. `Err` means the
/// Planetary Computer cannot answer for this window and ORNL should be
/// asked: the search failed, a read failed, or a month that ranks before
/// the pick is missing and old enough that ORNL may hold it.
async fn pick<S: MonthSource>(src: &S, w: Window) -> Result<Pick, String> {
    let mut months = months_in(w.lo, w.hi);
    rank(&mut months, w.target);
    // Search past the window, up to now, to see whether the catalogue holds
    // anything newer than a month it lacks.
    let to = (w.hi + 92 * DAY).min(w.now).max(w.hi);
    let mut by_month: BTreeMap<i64, ModisPcItem> = BTreeMap::new();
    for item in src.items(w.lo, to).await? {
        let Some(m) = date_unix(&item.start_date) else {
            continue;
        };
        // Two granules for one month: the later production, whose id ends
        // in the later production time.
        if by_month.get(&m).is_none_or(|prev| prev.id < item.id) {
            by_month.insert(m, item);
        }
    }
    let newest = by_month.keys().next_back().copied();
    let (mut nodata, mut rejected_for_qa, mut not_yet) = (0, 0, Vec::new());
    for month in months {
        let Some(item) = by_month.get(&month) else {
            let recent = next_month(month) > w.now - NOT_YET_PRODUCED_DAYS * DAY;
            if recent && newest.is_none_or(|n| n < month) {
                not_yet.push(date_of(month)[..7].to_string());
                continue;
            }
            return Err(format!(
                "no {COLLECTION} item for {} at {}",
                &date_of(month)[..7],
                src.tile()
            ));
        };
        let px = src.read(item).await?;
        if crate::ornl_is_nodata(PRODUCT, px.burn) {
            nodata += 1;
            continue;
        }
        if !crate::ornl_qc_gate(PRODUCT, px.qa) {
            rejected_for_qa += 1;
            continue;
        }
        return Ok(Pick::Found {
            item: item.clone(),
            month,
            px,
        });
    }
    Ok(Pick::Nothing {
        nodata,
        rejected_for_qa,
        not_yet,
        searched: format!(
            "{}?collections={COLLECTION}&modis:tile={}&datetime={}/{}",
            emem_fetch::stac::STAC_MPC_V1,
            src.tile(),
            date_of(w.lo),
            date_of(to)
        ),
    })
}

/// Sign what the Planetary Computer said: the month's Burn_Date, or an
/// Absence at the tslot the ORNL path would anchor one to.
async fn sign(
    cell64: &str,
    s: &AppState,
    tile: &str,
    lat: f64,
    lng: f64,
    w: Window,
    p: Pick,
) -> Result<FactCid, String> {
    let signed_at = crate::chrono_iso8601_utc();
    match p {
        Pick::Found { item, month, px } => {
            let href = |name: &str| item.assets.get(name).cloned().unwrap_or_default();
            let cal_date = date_of(month);
            let fact = Fact::Primary(PrimaryFact {
                cell: cell64.to_string(),
                band: BAND.to_string(),
                tslot: Tslot::from_unix(month, Tempo::Medium).0,
                value: ciborium::Value::Float(px.burn as f64),
                unit: Some("doy".to_string()),
                confidence: crate::ornl_qc_confidence(PRODUCT, px.qa),
                uncertainty: None,
                sources: vec![Source {
                    scheme: SCHEME.into(),
                    // Unsigned asset URLs: a SAS token expires within the
                    // hour, and anyone can mint a new one.
                    id: format!("{} ; {}", href(VALUE_ASSET), href(QA_ASSET)),
                    cid: None,
                    hash: None,
                    captured_at: Some(cal_date.clone()),
                    url: None,
                }],
                derivation: Derivation {
                    fn_key: FN_KEY.into(),
                    // The ORNL path's first seven arguments, in its order
                    // (lat, lng, product, variable, composite date, scale,
                    // QA byte), then the item and the pixel read.
                    args: Some(ciborium::Value::Array(vec![
                        ciborium::Value::Float(lat),
                        ciborium::Value::Float(lng),
                        ciborium::Value::Text(PRODUCT.into()),
                        ciborium::Value::Text(VALUE_ASSET.into()),
                        ciborium::Value::Text(cal_date),
                        ciborium::Value::Float(1.0),
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
            let mut reason = match w.target {
                Some(t) => format!(
                    "no valid {PRODUCT} {VALUE_ASSET} observation in ±{}d around unix={t}",
                    crate::MCD64A1_HALF_WINDOW_DAYS
                ),
                None => format!(
                    "no valid {PRODUCT} {VALUE_ASSET} observation from {} to {}",
                    date_of(w.lo),
                    date_of(w.hi)
                ),
            };
            reason.push_str(&format!(" on the Planetary Computer, tile {tile}"));
            if nodata > 0 {
                reason.push_str(&format!(" ({nodata} month(s) unmapped or water)"));
            }
            if rejected_for_qa > 0 {
                reason.push_str(&format!(
                    " ({rejected_for_qa} composite(s) rejected by {PRODUCT} QC gate)"
                ));
            }
            if !not_yet.is_empty() {
                reason.push_str(&format!(" (not produced yet: {})", not_yet.join(", ")));
            }
            let tslot = Tslot::from_unix(w.target.unwrap_or(w.now), Tempo::Medium).0;
            crate::sign_band_absence(
                cell64, s, BAND, tslot, SCHEME, &searched, &signed_at, &reason,
            )
            .await
        }
    }
}

/// Read the month from the Planetary Computer and sign it, or ask `ornl`
/// when the Planetary Computer cannot answer within `budget`. A signing
/// error is returned as is: ORNL would hit the same store.
#[allow(clippy::too_many_arguments)]
async fn pc_then<S, F, Fut>(
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
    S: MonthSource,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<FactCid, String>>,
{
    let picked = match tokio::time::timeout(budget, pick(src, w)).await {
        Ok(r) => r,
        Err(_) => Err(format!("no answer in {} ms", budget.as_millis())),
    };
    match picked {
        Ok(p) => sign(cell64, s, &src.tile(), lat, lng, w, p).await,
        Err(why) => {
            tracing::info!(
                target: "emem::materialize",
                materialize_cell = %cell64,
                materialize_band = BAND,
                pc_miss = %why,
                "planetary computer cannot answer; asking ORNL"
            );
            ornl()
                .await
                .map_err(|e| format!("{e} (after the Planetary Computer: {why})"))
        }
    }
}

/// `modis.burned_area_monthly` at a point: the Planetary Computer's COGs
/// first, `ornl` when they cannot answer.
pub(crate) async fn burned_area_pc_then_ornl<F, Fut>(
    cell64: &str,
    s: &AppState,
    lat: f64,
    lng: f64,
    target_unix: Option<i64>,
    ornl: F,
) -> Result<FactCid, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<FactCid, String>>,
{
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (x, y) = modis_pc::sinusoidal_xy(lat, lng);
    let Some((h, v)) = modis_pc::tile_of(x, y) else {
        return ornl().await;
    };
    let src = PcSource {
        client: crate::s2_http_client(),
        x,
        y,
        h,
        v,
    };
    let w = Window::new(target_unix, now);
    pc_then(cell64, s, &src, lat, lng, w, PC_BUDGET, ornl).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    fn day(y: i32, m: u32, d: u32) -> i64 {
        crate::days_from_civil(y, m, d) * DAY
    }

    fn item(month: &str, prod: &str) -> ModisPcItem {
        let base = format!(
            "https://modiseuwest.blob.core.windows.net/modis-061-cogs/MCD64A1/30/10/{month}/x"
        );
        ModisPcItem {
            id: format!("MCD64A1.A{month}.h30v10.061.{prod}"),
            start_date: month.to_string(),
            h: 30,
            v: 10,
            assets: BTreeMap::from([
                (VALUE_ASSET.to_string(), format!("{base}_Burn_Date.tif")),
                (QA_ASSET.to_string(), format!("{base}_QA.tif")),
            ]),
        }
    }

    /// Items and pixels by month; a month with no pixel entry fails to read.
    struct Fake {
        items: Result<Vec<ModisPcItem>, String>,
        pixels: BTreeMap<String, (i64, i64)>,
        read_log: Mutex<Vec<String>>,
        stall: Option<Duration>,
    }

    impl Fake {
        fn new(items: &[(&str, Option<(i64, i64)>)]) -> Self {
            Fake {
                items: Ok(items
                    .iter()
                    .map(|(m, _)| item(m, "2024001000000"))
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

    impl MonthSource for Fake {
        fn tile(&self) -> String {
            "h30v10".into()
        }
        async fn items(&self, _from: i64, _to: i64) -> Result<Vec<ModisPcItem>, String> {
            if let Some(d) = self.stall {
                tokio::time::sleep(d).await;
            }
            self.items.clone()
        }
        async fn read(&self, item: &ModisPcItem) -> Result<MonthPixel, String> {
            self.read_log.lock().unwrap().push(item.start_date.clone());
            let (burn, qa) = self
                .pixels
                .get(&item.start_date)
                .copied()
                .ok_or_else(|| format!("read {}: status 503", item.id))?;
            Ok(MonthPixel {
                burn,
                qa,
                col: 2246,
                row: 600,
            })
        }
    }

    /// A target in mid-August 2023, a long time before `now`.
    fn aug_2023() -> Window {
        Window::new(Some(day(2023, 8, 14)), day(2026, 11, 14))
    }

    #[test]
    fn months_in_take_whole_days() {
        assert_eq!(
            months_in(day(2023, 7, 13), day(2023, 9, 15)),
            vec![day(2023, 8, 1), day(2023, 9, 1)]
        );
        // A month whose first day is the window's first day is in it, at
        // any hour of that day, as ORNL's startDate takes it.
        assert_eq!(
            months_in(day(2023, 8, 1) + 3600 * 13, day(2023, 8, 2)),
            vec![day(2023, 8, 1)]
        );
        assert_eq!(
            months_in(day(2023, 12, 2), day(2024, 2, 1)),
            vec![day(2024, 1, 1), day(2024, 2, 1)]
        );
        assert!(months_in(day(2023, 8, 2), day(2023, 8, 30)).is_empty());
    }

    #[test]
    fn rank_matches_ornl_priority() {
        let mut m = vec![day(2023, 8, 1), day(2023, 9, 1)];
        rank(&mut m, Some(day(2023, 8, 25)));
        assert_eq!(m, vec![day(2023, 9, 1), day(2023, 8, 1)]);
        // Equal distance: the earlier month, as ORNL's chronological scan
        // with a strict `<` keeps the first.
        let mut m = vec![day(2023, 9, 1), day(2023, 8, 1)];
        let mid = (day(2023, 8, 1) + day(2023, 9, 1)) / 2;
        rank(&mut m, Some(mid));
        assert_eq!(m, vec![day(2023, 8, 1), day(2023, 9, 1)]);
        let mut m = vec![day(2023, 8, 1), day(2023, 10, 1), day(2023, 9, 1)];
        rank(&mut m, None);
        assert_eq!(m, vec![day(2023, 10, 1), day(2023, 9, 1), day(2023, 8, 1)]);
    }

    #[test]
    fn window_is_the_ornl_window() {
        let w = aug_2023();
        assert_eq!(w.lo, day(2023, 8, 14) - 32 * DAY);
        assert_eq!(w.hi, day(2023, 8, 14) + 32 * DAY);
        let now = day(2026, 11, 14);
        let w = Window::new(None, now);
        assert_eq!((w.lo, w.hi), (now - 9 * 31 * DAY, now));
    }

    #[tokio::test]
    async fn picks_the_month_nearest_the_target() {
        let src = Fake::new(&[
            ("2023-07-01", Some((190, 3))),
            ("2023-08-01", Some((230, 3))),
            ("2023-09-01", Some((250, 3))),
        ]);
        let p = pick(&src, aug_2023()).await.unwrap();
        let Pick::Found { month, px, item } = p else {
            panic!("expected a pick, got {p:?}")
        };
        assert_eq!(month, day(2023, 8, 1));
        assert_eq!(px.burn, 230);
        assert!(item.id.contains("2023-08-01"));
        assert_eq!(src.reads(), vec!["2023-08-01"], "one read, no more");
    }

    #[tokio::test]
    async fn unburned_land_is_a_valid_zero() {
        let src = Fake::new(&[("2023-08-01", Some((0, 3))), ("2023-09-01", Some((250, 3)))]);
        let Pick::Found { px, .. } = pick(&src, aug_2023()).await.unwrap() else {
            panic!("expected a pick")
        };
        assert_eq!(px.burn, 0);
    }

    /// -1 (unmapped) and -2 (water) are not days of year: the next month
    /// in the ORNL order is read instead, as the ORNL path skips them.
    #[tokio::test]
    async fn fill_values_pass_to_the_next_month() {
        let src = Fake::new(&[("2023-08-01", Some((-1, 3))), ("2023-09-01", Some((-2, 2)))]);
        let p = pick(&src, aug_2023()).await.unwrap();
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
        let src = Fake::new(&[
            ("2023-08-01", Some((-1, 3))),
            ("2023-09-01", Some((250, 3))),
        ]);
        let Pick::Found { month, .. } = pick(&src, aug_2023()).await.unwrap() else {
            panic!("expected a pick")
        };
        assert_eq!(month, day(2023, 9, 1));
    }

    /// QA bit 0 clear is a non-land cell: rejected even with a burn date,
    /// the gate the ORNL path applies.
    #[tokio::test]
    async fn qa_land_bit_gates_the_value() {
        let src = Fake::new(&[("2023-08-01", Some((230, 2))), ("2023-09-01", Some((0, 3)))]);
        let Pick::Found { month, px, .. } = pick(&src, aug_2023()).await.unwrap() else {
            panic!("expected a pick")
        };
        assert_eq!((month, px.burn), (day(2023, 9, 1), 0));
        let src = Fake::new(&[
            ("2023-08-01", Some((230, 0))),
            ("2023-09-01", Some((231, 2))),
        ]);
        let p = pick(&src, aug_2023()).await.unwrap();
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
        let mut src = Fake::new(&[]);
        src.items = Err("stac status 503 Service Unavailable".into());
        let e = pick(&src, aug_2023()).await.unwrap_err();
        assert!(e.contains("503"), "{e}");
    }

    #[tokio::test]
    async fn read_failure_is_a_miss() {
        let src = Fake::new(&[("2023-08-01", None), ("2023-09-01", Some((250, 3)))]);
        let e = pick(&src, aug_2023()).await.unwrap_err();
        assert!(e.contains("503"), "{e}");
    }

    #[tokio::test]
    async fn no_item_for_an_old_month_is_a_miss() {
        let src = Fake::new(&[]);
        let e = pick(&src, aug_2023()).await.unwrap_err();
        assert!(e.contains("no modis-64A1-061 item for 2023-08"), "{e}");
    }

    /// A hole that ranks before a month the catalogue holds: ORNL may hold
    /// the hole, so the read goes there rather than moving the pick.
    #[tokio::test]
    async fn a_hole_never_moves_the_pick() {
        let src = Fake::new(&[
            ("2023-07-01", Some((190, 3))),
            ("2023-09-01", Some((250, 3))),
        ]);
        let w = Window::new(Some(day(2023, 8, 10)), day(2026, 11, 14));
        let e = pick(&src, w).await.unwrap_err();
        assert!(e.contains("2023-08"), "{e}");
        assert!(src.reads().is_empty(), "nothing read past the hole");
    }

    /// The newest months are not produced yet, by anyone: skipped, and the
    /// newest month the catalogue holds is the answer.
    #[tokio::test]
    async fn recent_months_not_yet_produced_are_skipped() {
        let now = day(2026, 11, 14);
        let src = Fake::new(&[("2026-08-01", Some((0, 3))), ("2026-09-01", Some((262, 3)))]);
        let Pick::Found { month, .. } = pick(&src, Window::new(None, now)).await.unwrap() else {
            panic!("expected a pick")
        };
        assert_eq!(month, day(2026, 9, 1));
        assert_eq!(src.reads(), vec!["2026-09-01"]);
        // A target in a month not produced yet: an Absence from here, as
        // ORNL has no composite to give either.
        let src = Fake::new(&[("2026-09-01", Some((262, 3)))]);
        let w = Window::new(Some(day(2026, 11, 1)), now);
        let p = pick(&src, w).await.unwrap();
        let Pick::Nothing { not_yet, .. } = p else {
            panic!("expected nothing, got {p:?}")
        };
        assert_eq!(not_yet, vec!["2026-11", "2026-10"]);
    }

    /// A recent month missing while a newer one is held is a hole, not a
    /// month still being produced.
    #[tokio::test]
    async fn a_recent_hole_behind_a_newer_month_is_a_miss() {
        let now = day(2026, 11, 14);
        let src = Fake::new(&[("2026-08-01", Some((0, 3))), ("2026-10-01", Some((0, 3)))]);
        let w = Window::new(Some(day(2026, 9, 3)), now);
        let e = pick(&src, w).await.unwrap_err();
        assert!(e.contains("2026-09"), "{e}");
    }

    /// An old month the catalogue lacks is never taken as "not produced
    /// yet", even when it is past the newest item: the catalogue may have
    /// stopped. September 2026 ended 45 days before `now`, so it still
    /// counts as recent; August is the first month ORNL may hold.
    #[tokio::test]
    async fn a_stalled_catalogue_is_a_miss() {
        let now = day(2026, 11, 14);
        let src = Fake::new(&[("2026-05-01", Some((0, 3)))]);
        let e = pick(&src, Window::new(None, now)).await.unwrap_err();
        assert!(e.contains("no modis-64A1-061 item for 2026-08"), "{e}");
        assert!(src.reads().is_empty());
    }

    #[tokio::test]
    async fn the_later_granule_of_a_month_wins() {
        let mut src = Fake::new(&[("2023-08-01", Some((230, 3)))]);
        src.items = Ok(vec![
            item("2023-08-01", "2023284120835"),
            item("2023-08-01", "2021307215246"),
        ]);
        let Pick::Found { item, .. } = pick(&src, aug_2023()).await.unwrap() else {
            panic!("expected a pick")
        };
        assert!(item.id.ends_with("2023284120835"));
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
        let mut src = Fake::new(&[]);
        src.items = Err("stac status 503".into());
        let asked = AtomicBool::new(false);
        let e = pc_then(&cell, &s, &src, -12.5, 132.5, aug_2023(), PC_BUDGET, || {
            ornl_fake(&asked)
        })
        .await
        .unwrap_err();
        assert!(asked.load(Ordering::SeqCst), "ORNL was asked");
        assert!(e.contains("ornl subset: status 400"), "{e}");
        assert!(e.contains("Planetary Computer: stac status 503"), "{e}");

        // A Planetary Computer that does not answer in its budget too.
        let mut src = Fake::new(&[("2023-08-01", Some((230, 3)))]);
        src.stall = Some(Duration::from_secs(5));
        let asked = AtomicBool::new(false);
        let e = pc_then(
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

    /// A read the Planetary Computer answers is signed from it, names it,
    /// and never asks ORNL.
    #[tokio::test]
    async fn a_planetary_computer_read_names_its_source() {
        let s = crate::tests::test_app_state();
        let cell = emem_codec::cell64_from_latlng(-12.5, 132.5);
        let src = Fake::new(&[("2023-08-01", Some((230, 3)))]);
        let asked = AtomicBool::new(false);
        let cid = pc_then(&cell, &s, &src, -12.5, 132.5, aug_2023(), PC_BUDGET, || {
            ornl_fake(&asked)
        })
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
        assert_eq!(f.band, BAND);
        assert_eq!(f.value, ciborium::Value::Float(230.0));
        assert_eq!(f.unit.as_deref(), Some("doy"));
        assert_eq!(f.tslot, Tslot::from_unix(day(2023, 8, 1), Tempo::Medium).0);
        assert_eq!(f.confidence, crate::ornl_qc_confidence(PRODUCT, 3));
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
        assert_eq!(f.derivation.fn_key, FN_KEY);
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

    /// Nothing valid in the window: an Absence from the Planetary Computer,
    /// at the tslot the ORNL path anchors one to, without asking ORNL.
    #[tokio::test]
    async fn nothing_valid_is_a_planetary_computer_absence() {
        let s = crate::tests::test_app_state();
        let cell = emem_codec::cell64_from_latlng(-12.5, 132.5);
        let src = Fake::new(&[("2023-08-01", Some((-2, 2))), ("2023-09-01", Some((-2, 2)))]);
        let asked = AtomicBool::new(false);
        let cid = pc_then(&cell, &s, &src, -12.5, 132.5, aug_2023(), PC_BUDGET, || {
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
        let Some(Fact::Absence(f)) = got.into_iter().next().flatten() else {
            panic!("an absence")
        };
        assert_eq!(f.band, BAND);
        assert_eq!(f.tslot, Tslot::from_unix(day(2023, 8, 14), Tempo::Medium).0);
        assert_eq!(f.sources[0].scheme, SCHEME);
        assert!(f.sources[0].id.contains("modis-64A1-061"));
        assert!(f.sources[0].id.contains("h30v10"));
    }
}
