//! WRI / Google DeepMind Global Drivers of Forest Loss connector.
//!
//! Source: **Sims, M. J., R. Stanimirova, A. Raichuk, M. Neumann,
//! J. Richter, F. Follett, J. MacCarthy, K. Lister, C. Randle, L. Sloat,
//! E. Esipova, J. Jupiter, C. Stanton, D. Morris, C. M. Slay, D. Purves,
//! N. Harris (2025). *Global drivers of forest loss at 1 km resolution*.
//! Environmental Research Letters 20, 074027.
//! doi:10.1088/1748-9326/add606**. Open-license (CC-BY-4.0) raster
//! mirrored on Zenodo (concept DOI 10.5281/zenodo.14162799) and
//! distributed on Google Earth Engine as
//! `projects/landandcarbon/assets/wri_gdm/drivers_forest_loss_1km_v1_2_2001_2024`.
//!
//! The product is a **single global 1 km GeoTIFF** at 36 000 × 14 000 px
//! (EPSG:4326, ~0.01° pixel size). It is co-published with Hansen GFC:
//! the discrete driver class is meaningful **only** where Hansen reports
//! a tree-cover-loss event, so callers MUST treat
//! `lossyear=0 → fetch_driver_class = Ok(None)` as the expected,
//! non-error "no loss → no driver to attribute" path.
//!
//! Driver classes (uint8, 1..=7):
//! 1. **Permanent agriculture** — long-term cropland or perennial tree
//!    crops (oil palm, cocoa, rubber, coffee, orchards, pasture, seasonal
//!    crops). Includes plantation-scale clearance for established
//!    agricultural use.
//! 2. **Hard commodities** — mining (small-scale to industrial),
//!    oil/gas, hydroelectric flooding, wind/solar farms, transmission
//!    corridors, and other energy-infrastructure clearance.
//! 3. **Shifting cultivation** — small- to medium-scale clearing for
//!    temporary cultivation followed by fallow regrowth. Distinct from
//!    permanent agriculture by its cyclical-abandonment pattern.
//! 4. **Logging** — timber harvest, selective logging, plantation
//!    harvest in wood-fiber estates, salvage / sanitation logging, and
//!    logging-road clearance. Regrowth or replanting is expected.
//! 5. **Wildfire** — burn scars with no subsequent human conversion.
//!    Natural-ignition or anthropogenic-ignition fires are both class 5
//!    so long as the post-fire trajectory is regrowth, not conversion.
//! 6. **Settlements and infrastructure** — urban / built-up expansion,
//!    new roads (not logging roads), and other non-commodity built
//!    infrastructure.
//! 7. **Other natural disturbances** — non-fire natural causes such as
//!    windthrow, drought die-off, landslides, lava flows, river
//!    meandering, insect outbreaks, etc.
//!
//! The "commodity-driven" vs "natural" split used by the WRI
//! Deforestation Monitoring framework groups classes 1, 2, 3 as
//! commodity-driven loss (human-driven, anthropogenic-conversion) and
//! classes 5, 7 as natural disturbances. Classes 4 (Logging) and 6
//! (Settlements) sit in neither bucket — logging is anthropogenic but
//! typically followed by regrowth, settlements are anthropogenic but
//! not commodity-export-linked. Callers that need a binary
//! commodity-vs-natural label MUST pick a policy for 4 and 6
//! explicitly; this module surfaces the WRI convention via
//! [`is_commodity_driven`] (1/2/3) and [`is_natural`] (5/7) and leaves
//! 4 and 6 as `false` for both.
//!
//! Honest defaults (firm protocol contract):
//! - `Ok(None)` from [`fetch_driver_class`] means "no Hansen-loss event
//!   at this cell", which is the **expected** absence-of-attribution
//!   path — materializers sign this as a Primary fact with `class=0`,
//!   not an `Absence`.
//! - `Err(CoverageGap)` is reserved for cells outside the dataset's
//!   ±60°S–~80°N envelope (it inherits Hansen's coverage bounds).
//! - Network / decode errors propagate as `Transport` / `Decode`.

use reqwest::Client;

use crate::cog::CogError;

/// Direct URL to the v1.2 (2001-2024) single global COG on Zenodo
/// record 15366671 (a child of concept-record 14162799). Verified live
/// 2026-05 (HTTP 200, content-type image/tiff, ~295 MB, tiled COG with
/// LZW compression and 8 bands: 1 discrete-class band + 7 per-class
/// probability bands).
///
/// **Range-request caveat:** Zenodo's nginx returns the full file body
/// for any `Range:` header rather than a 206. That means callers
/// CANNOT rely on the shared [`cog`](crate::cog) sampler to
/// range-read a single 512×512 tile — every request pulls the whole
/// 295 MB COG. Until a range-readable mirror is wired (Earth Engine
/// asset export to a public GCS bucket, or a Cloudflare-fronted
/// proxy), [`fetch_driver_class`] returns
/// [`WriGdmError::NotImplemented`] rather than silently pulling 295 MB
/// per cell. The pure-logic helpers ([`class_label`],
/// [`is_commodity_driven`], [`is_natural`]) ship today.
const WRI_GDM_BASE_URL: &str =
    "https://zenodo.org/records/15366671/files/drivers_forest_loss_1km_2001_2024_v1_2.tif";

/// Public version tag for the v1.2 release (2001-2024 cumulative).
/// Bumped to "v1.3_2001-2025" when Zenodo record 19485190 (already
/// published 2026-04-29) is promoted to the wired version — that bump
/// is a one-line edit to [`WRI_GDM_BASE_URL`] plus this constant.
pub const WRI_GDM_VERSION_TAG: &str = "v1.2_2001-2024";

/// Earth Engine asset path for the v1.2 product. Surfaced as a public
/// constant so callers that prefer the EE Python / JS APIs can pick it
/// up without re-deriving the path. The Rust connector does not depend
/// on EE — the path is here for documentation / cross-tool parity.
pub const WRI_GDM_EE_ASSET: &str =
    "projects/landandcarbon/assets/wri_gdm/drivers_forest_loss_1km_v1_2_2001_2024";

/// Minimum valid driver-class value in the raster (inclusive).
pub const WRI_GDM_CLASS_MIN: u8 = 1;

/// Maximum valid driver-class value in the raster (inclusive).
pub const WRI_GDM_CLASS_MAX: u8 = 7;

/// Maximum latitude (deg, absolute) within which the WRI GDM raster is
/// defined. The product inherits Hansen GFC's tile bounds — there is
/// no driver attribution below ~60°S (Antarctica) or above ~80°N
/// (Arctic interior). The Zenodo COG's geo-transform clips to these
/// bounds; cells outside the envelope surface as
/// [`WriGdmError::CoverageGap`] so the materializer can sign an
/// Absence rather than fabricate a class value.
const WRI_GDM_LAT_BOUND: f64 = 80.0;

/// Errors specific to the WRI GDM Drivers connector.
///
/// Bubbled up through [`crate::FetchError::Transport`] at the
/// dispatcher boundary so callers do not have to thread two error
/// types. Each variant carries enough context for a materializer to
/// sign the correct fact shape (Primary, Absence, or hard error).
#[derive(Debug, thiserror::Error)]
pub enum WriGdmError {
    /// HTTP / network failure. Caller should treat as a transport
    /// error and let the dispatcher retry.
    #[error("transport: {0}")]
    Transport(String),
    /// COG parse / decode failure (TIFF layout, codec stream
    /// corruption, pixel out of dataset range). Indicates upstream
    /// corruption — the no-fallback rule applies.
    #[error("decode: {0}")]
    Decode(String),
    /// Cell sits outside the dataset's documented latitude envelope
    /// (Antarctic interior, high Arctic). Materializers MUST sign this
    /// as an `Absence` — the cell is genuinely outside the dataset's
    /// coverage, distinct from the "no Hansen loss" no-attribution
    /// path (which is `Ok(None)`).
    #[error(
        "coverage_gap: lat={lat:.6} lng={lng:.6} is outside WRI GDM ±{bound:.0}° latitude envelope"
    )]
    CoverageGap {
        /// Cell latitude that triggered the gap.
        lat: f64,
        /// Cell longitude (carried for diagnostics).
        lng: f64,
        /// The latitude bound that was exceeded (for the error
        /// message — currently 80.0°).
        bound: f64,
    },
    /// The fetch path is not yet wired for this release. The pure
    /// class-label helpers ship today; the per-cell fetch will land
    /// once a range-readable mirror is available (Zenodo's nginx does
    /// not honour HTTP Range, so the shared COG sampler would pull
    /// the full 295 MB file on every cell — unacceptable for a
    /// per-cell materializer). See module docs for the full rationale.
    #[error("not_implemented: {reason}")]
    NotImplemented {
        /// Human-readable explanation of what's missing.
        reason: String,
    },
}

impl WriGdmError {
    /// Map a [`CogError`] into the appropriate connector-specific
    /// variant. Transport errors surface as `Transport`; everything
    /// else (decode, codec, layout) becomes `Decode`. Reserved for the
    /// fetch path once a range-readable mirror is wired.
    #[allow(dead_code)] // used once the fetch path lands
    fn from_cog(e: CogError) -> Self {
        match e {
            CogError::Transport(s) => WriGdmError::Transport(s),
            other => WriGdmError::Decode(other.to_string()),
        }
    }
}

/// Return the single global COG URL. Stable function rather than a
/// constant so callers can reach it through the module's public API
/// without depending on the private constant. Pure — no I/O.
pub fn cog_url() -> &'static str {
    WRI_GDM_BASE_URL
}

/// Read one pixel from the WRI GDM Drivers raster and return the
/// dominant driver class (1..=7) for the cell, or `Ok(None)` if the
/// cell had no Hansen tree-cover-loss event in the 2001-2024 window
/// (driver attribution only applies where there is a loss event).
///
/// Returns:
/// - `Ok(Some(class))` for `class ∈ 1..=7` (a real driver attribution).
/// - `Ok(None)` for a confirmed on-land pixel with no Hansen loss
///   event — a meaningful Primary fact ("no loss → no driver").
/// - `Err(CoverageGap)` for cells outside the ±80° latitude envelope.
/// - `Err(NotImplemented)` while the wired fetch path is pending a
///   range-readable mirror (see module docs).
/// - `Err(Transport)` / `Err(Decode)` for the wired-fetch failure
///   modes once it lands.
pub async fn fetch_driver_class(
    client: &Client,
    lat: f64,
    lng: f64,
) -> Result<Option<u8>, WriGdmError> {
    // Bounds check fires before any I/O — same contract as
    // jrc_gfc2020::fetch_forest_2020. Surfaces CoverageGap, not
    // NotImplemented, so the protocol's Absence path is preserved
    // even before the fetch wires up.
    if !lat.is_finite() || lat.abs() > WRI_GDM_LAT_BOUND {
        return Err(WriGdmError::CoverageGap {
            lat,
            lng,
            bound: WRI_GDM_LAT_BOUND,
        });
    }
    if !lng.is_finite() || !(-180.0..=180.0).contains(&lng) {
        return Err(WriGdmError::CoverageGap {
            lat,
            lng,
            bound: WRI_GDM_LAT_BOUND,
        });
    }

    // Touch the client argument so the signature is the canonical
    // shape the dispatcher expects once the fetch wires up. No HTTP
    // call is issued — the NotImplemented path returns immediately.
    let _ = client;

    Err(WriGdmError::NotImplemented {
        reason: format!(
            "fetch path pending range-readable mirror; Zenodo at {url} returns HTTP 200 with full body for Range: requests (verified 2026-05), and the shared cog sampler does not yet handle the multi-band LZW layout the Zenodo COG ships (compression=5, samples_per_pixel=8). Pure-logic helpers (class_label, is_commodity_driven, is_natural) are live.",
            url = WRI_GDM_BASE_URL
        ),
    })
}

/// Map a discrete driver-class byte (1..=7) to the human-readable
/// label from Sims et al. 2025 (Table 1). Returns `None` for `class=0`
/// (no Hansen loss, no attribution) and for any value outside 1..=7
/// (upstream corruption).
///
/// Labels match the published dataset's class definitions verbatim so
/// downstream consumers can quote them directly in receipts and UI
/// without paraphrasing.
pub fn class_label(class: u8) -> Option<&'static str> {
    match class {
        1 => Some("Permanent agriculture"),
        2 => Some("Hard commodities"),
        3 => Some("Shifting cultivation"),
        4 => Some("Logging"),
        5 => Some("Wildfire"),
        6 => Some("Settlements and infrastructure"),
        7 => Some("Other natural disturbances"),
        _ => None,
    }
}

/// Return `true` if the driver class is commodity-driven under the
/// WRI Deforestation Monitoring grouping: permanent agriculture (1),
/// hard commodities (2), or shifting cultivation (3). All three are
/// anthropogenic-conversion drivers tied to commodity production or
/// subsistence agriculture; the post-loss trajectory is land-use
/// change rather than forest regrowth.
///
/// Logging (4) is anthropogenic but typically followed by regrowth,
/// so it is **not** commodity-driven under this convention.
/// Settlements (6) is anthropogenic but not commodity-export-linked.
/// Both return `false`. Values outside 1..=7 also return `false`.
pub fn is_commodity_driven(class: u8) -> bool {
    matches!(class, 1..=3)
}

/// Return `true` if the driver class is a natural disturbance:
/// wildfire (5) or other natural disturbance (7). Both are
/// non-anthropogenic-conversion events where the post-loss trajectory
/// is regrowth rather than land-use change.
///
/// Wildfire is class 5 even when the ignition source is human (a
/// camp-fire that escapes, an arson) so long as the post-fire
/// trajectory is regrowth — the class is keyed on the loss
/// **mechanism** (combustion, no subsequent conversion), not the
/// ignition agent. Values outside 1..=7 return `false`.
pub fn is_natural(class: u8) -> bool {
    matches!(class, 5 | 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cog_url()` must point at the verified live Zenodo v1.2 COG.
    /// Pinned literally to catch accidental path edits — the
    /// `15366671/files/drivers_forest_loss_1km_2001_2024_v1_2.tif`
    /// segment is the load-bearing identifier (the record ID 15366671
    /// is the v1.2 child of concept-DOI 10.5281/zenodo.14162799).
    #[test]
    fn cog_url_is_zenodo_v1_2_path() {
        assert_eq!(
            cog_url(),
            "https://zenodo.org/records/15366671/files/drivers_forest_loss_1km_2001_2024_v1_2.tif",
            "WRI GDM v1.2 lives on Zenodo record 15366671 (a child of concept-DOI 10.5281/zenodo.14162799)"
        );
        // Sanity: the constant the module uses internally is the same
        // string (no shadowing via a stale literal somewhere else).
        assert_eq!(cog_url(), WRI_GDM_BASE_URL);
    }

    /// `WRI_GDM_VERSION_TAG` pins the v1.2 (2001-2024) release the URL
    /// points at. If we promote v1.3 (2001-2025, Zenodo record
    /// 19485190, already published 2026-04-29) the constant bump
    /// should be a one-line, reviewable change visible in diff.
    #[test]
    fn version_tag_is_v1_2() {
        assert_eq!(WRI_GDM_VERSION_TAG, "v1.2_2001-2024");
    }

    /// The Earth Engine asset path is published alongside the Zenodo
    /// mirror — surfaced as a constant so EE-using callers don't have
    /// to re-derive it. Pinned literally so a typo (wrong project,
    /// wrong sub-folder) shows up in code review.
    #[test]
    fn ee_asset_path_matches_publication() {
        assert_eq!(
            WRI_GDM_EE_ASSET,
            "projects/landandcarbon/assets/wri_gdm/drivers_forest_loss_1km_v1_2_2001_2024"
        );
    }

    /// `class_label` covers every documented class 1..=7 with the
    /// verbatim Sims et al. 2025 (Table 1) labels and returns `None`
    /// for the boundary values (0 = no Hansen loss; 8/255 = upstream
    /// corruption). Labels are quoted in receipts and UI, so any
    /// rephrasing must be a deliberate, reviewable change.
    #[test]
    fn class_label_covers_all_seven_classes() {
        assert_eq!(class_label(1), Some("Permanent agriculture"));
        assert_eq!(class_label(2), Some("Hard commodities"));
        assert_eq!(class_label(3), Some("Shifting cultivation"));
        assert_eq!(class_label(4), Some("Logging"));
        assert_eq!(class_label(5), Some("Wildfire"));
        assert_eq!(class_label(6), Some("Settlements and infrastructure"));
        assert_eq!(class_label(7), Some("Other natural disturbances"));
    }

    /// Boundary cases for `class_label`: `0` is reserved for "no
    /// Hansen loss → no attribution" (the [`fetch_driver_class`]
    /// `Ok(None)` path) and is NOT a valid driver label; values >7
    /// indicate upstream corruption and also yield `None`. Pins the
    /// "no silent fallback" rule — we do not invent a default label.
    #[test]
    fn class_label_returns_none_for_out_of_range() {
        assert_eq!(class_label(0), None, "class=0 is 'no loss', not a label");
        assert_eq!(class_label(8), None, "class=8 is out of the documented 1..=7 range");
        assert_eq!(class_label(255), None, "uint8 sentinel must not map to a label");
        // The exposed MIN/MAX constants must agree with the label
        // function — drift between them would produce silent gaps.
        assert!(class_label(WRI_GDM_CLASS_MIN).is_some());
        assert!(class_label(WRI_GDM_CLASS_MAX).is_some());
        assert!(class_label(WRI_GDM_CLASS_MAX + 1).is_none());
    }

    /// `is_commodity_driven` returns `true` exactly for classes 1, 2,
    /// 3 — the WRI Deforestation Monitoring "commodity-driven" group.
    /// Pinned per-class so any future regrouping (e.g. shifting
    /// cultivation reassigned to its own bucket) is a visible,
    /// reviewable diff.
    #[test]
    fn is_commodity_driven_splits_one_two_three() {
        assert!(is_commodity_driven(1), "Permanent agriculture is commodity-driven");
        assert!(is_commodity_driven(2), "Hard commodities is commodity-driven");
        assert!(is_commodity_driven(3), "Shifting cultivation is commodity-driven");
        // Logging (4) is anthropogenic but typically followed by
        // regrowth — NOT commodity-driven under the WRI convention.
        assert!(!is_commodity_driven(4), "Logging is NOT commodity-driven under WRI grouping");
        // Wildfire (5) and Other natural (7) are natural disturbances.
        assert!(!is_commodity_driven(5));
        assert!(!is_commodity_driven(7));
        // Settlements (6) is anthropogenic but not commodity-linked.
        assert!(!is_commodity_driven(6), "Settlements is NOT commodity-driven under WRI grouping");
        // Boundary values: 0 = no loss, >7 = corruption — neither
        // should map to commodity-driven.
        assert!(!is_commodity_driven(0));
        assert!(!is_commodity_driven(8));
        assert!(!is_commodity_driven(255));
    }

    /// `is_natural` returns `true` exactly for classes 5, 7 — the
    /// non-anthropogenic-conversion drivers. Classes 4 (Logging) and
    /// 6 (Settlements) sit in neither commodity nor natural buckets;
    /// callers that need a binary label have to pick a policy.
    #[test]
    fn is_natural_splits_five_seven() {
        assert!(is_natural(5), "Wildfire is a natural disturbance");
        assert!(is_natural(7), "Other natural disturbance is in the natural bucket");
        // Commodity-driven classes are NOT natural.
        assert!(!is_natural(1));
        assert!(!is_natural(2));
        assert!(!is_natural(3));
        // Logging and Settlements are anthropogenic but the WRI
        // grouping leaves them in neither bucket. Both must return
        // false here — `is_commodity_driven(4)` is also false, so
        // 4 ∈ {commodity, natural} = ∅. Same for 6.
        assert!(!is_natural(4), "Logging is anthropogenic — NOT natural");
        assert!(!is_natural(6), "Settlements is anthropogenic — NOT natural");
        // Boundary values.
        assert!(!is_natural(0));
        assert!(!is_natural(8));
        assert!(!is_natural(255));
    }

    /// The commodity / natural / unclassified split must partition
    /// classes 1..=7 with no overlap. Pinned as a structural
    /// invariant — if a future regrouping moves Logging into one of
    /// the buckets the partition test fires before any silent
    /// downstream miscount.
    #[test]
    fn commodity_and_natural_buckets_do_not_overlap() {
        for class in WRI_GDM_CLASS_MIN..=WRI_GDM_CLASS_MAX {
            let c = is_commodity_driven(class);
            let n = is_natural(class);
            assert!(
                !(c && n),
                "class {class} is in BOTH commodity and natural buckets — partition broken"
            );
        }
        // The neither-bucket set is exactly {4, 6} under the WRI
        // convention. Pin both so any drift surfaces in diff.
        for class in [4u8, 6u8] {
            assert!(!is_commodity_driven(class));
            assert!(!is_natural(class));
        }
    }

    /// `fetch_driver_class` MUST surface `CoverageGap` (not Transport,
    /// not NotImplemented, not a fabricated class) for cells outside
    /// the documented ±80° latitude envelope. This pins the
    /// protocol's "Antarctic / high-Arctic → Absence, not Err" rule
    /// for downstream materializers — and importantly, the bounds
    /// check fires BEFORE the NotImplemented short-circuit, so once
    /// the fetch path wires up the same test keeps passing.
    #[tokio::test]
    async fn fetch_outside_latitude_envelope_is_coverage_gap() {
        let client = reqwest::Client::new();
        // -85° latitude: deep Antarctic interior, outside the ±80°
        // bound. Longitude is in-range so the gap is unambiguously
        // attributable to latitude.
        let err = fetch_driver_class(&client, -85.0, 0.0).await.unwrap_err();
        match err {
            WriGdmError::CoverageGap { lat, lng, bound } => {
                assert!((lat - (-85.0)).abs() < 1e-9, "lat must round-trip");
                assert!((lng - 0.0).abs() < 1e-9, "lng must round-trip");
                assert!((bound - WRI_GDM_LAT_BOUND).abs() < 1e-9);
            }
            other => panic!("expected CoverageGap, got {other:?}"),
        }
        // High Arctic above +80°N — same envelope, same result.
        let err = fetch_driver_class(&client, 84.5, 10.0).await.unwrap_err();
        assert!(
            matches!(err, WriGdmError::CoverageGap { .. }),
            "lat above +80°N must surface CoverageGap, got {err:?}"
        );
        // NaN lat is also a coverage gap (not a Transport / Decode
        // error). The protocol forbids silently treating NaN as 0.
        let err = fetch_driver_class(&client, f64::NAN, 0.0).await.unwrap_err();
        assert!(
            matches!(err, WriGdmError::CoverageGap { .. }),
            "NaN lat must surface CoverageGap, got {err:?}"
        );
    }

    /// `fetch_driver_class` for an in-envelope (lat, lng) MUST surface
    /// `NotImplemented` rather than fabricating a class or silently
    /// pulling the full 295 MB Zenodo COG. The reason string MUST
    /// mention the missing-range-support root cause and the Zenodo
    /// URL so operators can grep for it in logs.
    ///
    /// This is the only place in the module where the
    /// `NotImplemented` shape is exercised. Once a range-readable
    /// mirror wires up, the variant goes away and this test flips to
    /// asserting `Ok(Some(class))` or `Ok(None)` (the no-loss path).
    #[tokio::test]
    async fn fetch_in_envelope_is_not_implemented_with_clear_reason() {
        let client = reqwest::Client::new();
        // Central Amazon — well inside the ±80° envelope, a region
        // where Hansen GFC ships dense loss attribution. If/when the
        // fetch path lands, this exact (lat, lng) is a good live
        // smoke-test cell.
        let err = fetch_driver_class(&client, -3.0, -60.5).await.unwrap_err();
        match err {
            WriGdmError::NotImplemented { reason } => {
                assert!(
                    reason.contains("range-readable") || reason.contains("Range"),
                    "reason must mention the missing-range root cause: got {reason:?}"
                );
                assert!(
                    reason.contains("zenodo.org"),
                    "reason must mention the Zenodo URL so operators can grep for it: got {reason:?}"
                );
            }
            other => panic!("expected NotImplemented, got {other:?}"),
        }
    }

    /// `WriGdmError::from_cog` maps a `CogError::Transport` to the
    /// connector's `Transport` variant and everything else to
    /// `Decode`. Pinned here so the helper stays correct even though
    /// it's currently dead code (the fetch path is not implemented).
    /// Without this test the helper's behaviour drifts silently when
    /// the fetch wires up later.
    #[test]
    fn from_cog_transport_maps_to_transport_decode_otherwise() {
        let err = WriGdmError::from_cog(CogError::Transport("status 503".into()));
        assert!(
            matches!(err, WriGdmError::Transport(_)),
            "Transport CogError must surface as WriGdmError::Transport, got {err:?}"
        );
        let err = WriGdmError::from_cog(CogError::BadMagic(0xdeadbeef));
        assert!(
            matches!(err, WriGdmError::Decode(_)),
            "Non-transport CogError must surface as WriGdmError::Decode, got {err:?}"
        );
        let err = WriGdmError::from_cog(CogError::MissingTag(322));
        assert!(matches!(err, WriGdmError::Decode(_)));
    }

    /// Pinned class-range constants — the raster is uint8 with classes
    /// 1..=7. Drift in either bound breaks the partition tests above
    /// (and the materializer's range-validation), so surface it as a
    /// dedicated assertion rather than relying on the partition test
    /// to catch it indirectly.
    #[test]
    fn class_range_constants_are_one_through_seven() {
        assert_eq!(WRI_GDM_CLASS_MIN, 1);
        assert_eq!(WRI_GDM_CLASS_MAX, 7);
    }
}
