//! Facts read from Element84's first baseline 04.00 Sentinel-2 items before
//! the fix went live carry ESA's `BOA_ADD_OFFSET` in their DNs, read with
//! offset 0 ([`emem_fetch::stac::element84_dn_offset`] has the finding). A
//! signed fact cannot change, so the responder says so beside one it serves.
//!
//! The rule reads only the fact: when it was signed and which Element84 items
//! it names. It needs no list, so it covers facts in storage that predate the
//! transparency log. A fact served by cid also gets `replaced_by` once the
//! responder has re-read its address (`annotate_fact_erratum` in lib.rs). A scan of the log on 12 November 2026 found 1,890 such
//! facts (2,891 attestations) on 601 cells, from 83 items, signed from 7 July
//! to 5 November 2026: 1,464 NDVI, 389 band reflectances, 2 NDMI and 35
//! derived records.

use std::collections::BTreeSet;

use serde_json::{json, Map, Value as JsonValue};

pub(crate) const ID: &str = "s2-element84-boa-offset-2022";

/// When the fix (c11aef7) went live on emem.dev. A read after it applied the
/// offset to these items or refused them.
const FIXED_AT: &str = "2026-11-06T21:39:18Z";

/// The baseline of every Element84 item of 2022-01-25 to 2022-02-28 that the
/// log names (83 of 83, checked against Earth Search on 12 November 2026). A
/// fact does not carry its item's baseline, so the rule assumes this one.
const ITEM_BASELINE: &str = "04.00";

/// The acquisition day of an Element84 Sentinel-2 item id
/// (`S2B_43RGP_20220208_0_L2A`), as `YYYY-MM-DD`. Planetary Computer ids have
/// another shape and never match.
fn element84_item_day(token: &str) -> Option<String> {
    let p: Vec<&str> = token.split('_').collect();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let tile = |s: &str| {
        let b = s.as_bytes();
        b.len() == 5
            && b[..2].iter().all(u8::is_ascii_digit)
            && b[2..].iter().all(u8::is_ascii_uppercase)
    };
    let ok = p.len() == 5
        && matches!(p[0], "S2A" | "S2B" | "S2C")
        && tile(p[1])
        && p[2].len() == 8
        && digits(p[2])
        && digits(p[3])
        && p[4] == "L2A";
    ok.then(|| format!("{}-{}-{}", &p[2][..4], &p[2][4..6], &p[2][6..]))
}

/// Every Element84 item id inside the strings of `v`, as (id, day).
fn element84_items(v: &JsonValue, out: &mut BTreeSet<(String, String)>) {
    match v {
        JsonValue::String(s) => {
            for t in s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                if let Some(day) = element84_item_day(t) {
                    out.insert((t.to_string(), day));
                }
            }
        }
        JsonValue::Array(a) => a.iter().for_each(|x| element84_items(x, out)),
        JsonValue::Object(o) => o.values().for_each(|x| element84_items(x, out)),
        _ => {}
    }
}

/// The erratum for one served fact, or `None` when it does not apply. Takes
/// the fact as JSON, flat (`kind`) or wrapped (`{"Primary": {...}}`).
pub(crate) fn for_fact(obj: &Map<String, JsonValue>) -> Option<JsonValue> {
    let (kind, f) = match ["Primary", "Derivative", "Absence"]
        .iter()
        .find_map(|k| obj.get(*k).and_then(|v| v.as_object()).map(|f| (*k, f)))
    {
        Some((k, f)) => (k.to_ascii_lowercase(), f),
        None => (obj.get("kind")?.as_str()?.to_ascii_lowercase(), obj),
    };
    if kind == "absence" || f.get("signed_at")?.as_str()? >= FIXED_AT {
        return None;
    }
    let mut items = BTreeSet::new();
    for k in ["derivation", "sources", "value"] {
        if let Some(v) = f.get(k) {
            element84_items(v, &mut items);
        }
    }
    let (mut wrong, mut unknown) = (Vec::new(), Vec::new());
    for (id, day) in items {
        let at = format!("{day}T12:00:00Z");
        match emem_fetch::stac::element84_dn_offset(Some(ITEM_BASELINE), &at) {
            Some(o) if o != 0.0 => wrong.push(id),
            None => unknown.push(id),
            Some(_) => {}
        }
    }
    if wrong.is_empty() && unknown.is_empty() {
        return None;
    }
    let derivation = f.get("derivation");
    let fn_key = derivation
        .and_then(|d| d.get("fn_key"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut e = json!({
        "id": ID,
        "fixed": format!("in c11aef7, live from {FIXED_AT}: a read after it applies the offset to these items, or refuses the ones acquired on 27 and 28 February 2022"),
    });
    if kind == "primary" && fn_key.starts_with(crate::S2_FN_KEY_PREFIX) {
        let band = f.get("band").and_then(|v| v.as_str()).unwrap_or("");
        if crate::s2_band_plan(band).map(|(_, k, _)| k) == Some("scl_categorical") {
            // A class index, not reflectance: the offset never touched it.
            return None;
        }
        let args = derivation
            .and_then(|d| d.get("args"))
            .and_then(|v| v.as_array());
        let item = args
            .and_then(|a| a.get(crate::S2_ARGS_ITEM_INDEX))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if wrong.iter().any(|w| w == item) {
            e["status"] = json!("wrong");
            e["what"] = json!("Element84 served this acquisition's DNs with ESA's BOA_ADD_OFFSET (-1000 DN) still in them, and this fact read them with offset 0: each band's reflectance is 0.1 too high, and a ratio index is pulled toward zero.");
        } else if unknown.iter().any(|u| u == item) {
            e["status"] = json!("unknown");
            e["what"] = json!("acquired on 27 or 28 February 2022, while Element84 began removing ESA's BOA_ADD_OFFSET: whether these DNs still carry it is not known, so this value may be 0.1 too bright per band, and emem no longer reads these items.");
        } else {
            return None;
        }
        e["items"] = json!([item]);
        // No value is corrected from the fact's own DNs: every such fact the
        // log holds was signed before the pixel fix too, so its DNs may be a
        // neighbouring pixel's. A fresh read fixes both; see `replaced_by`.
        if f.get("signed_at").and_then(|v| v.as_str()) < Some(crate::PIXEL_FIX_AT) {
            e["pixel"] = json!(format!("also signed before {}, when the point reader took the south-east neighbour for about 3 points in 4, so these DNs may be another pixel's", crate::PIXEL_FIX_AT));
        }
        e["remedy"] = json!("emem began re-reading these passes on 12 November 2026. Served by cid, `replaced_by` names the fresh fact now at this address when there is one. Otherwise POST /v1/backfill with refresh over that day signs one");
    } else {
        e["status"] = json!(if wrong.is_empty() { "unknown" } else { "wrong" });
        e["what"] = json!("this record took values from the Element84 scenes named in `items`, whose DNs still carry ESA's BOA_ADD_OFFSET (-1000 DN) or, for 27 and 28 February 2022, may: their reflectance reads 0.1 too high and ratio indices are pulled toward zero. Values from other scenes are not affected. Run the request again for a corrected record.");
        e["items"] = json!(wrong.iter().chain(&unknown).collect::<Vec<_>>());
    }
    Some(e)
}

/// Adds `erratum` to a served fact object when [`for_fact`] applies.
pub(crate) fn annotate(fact: &mut JsonValue) {
    if let Some(e) = fact.as_object().and_then(for_fact) {
        if let Some(o) = fact.as_object_mut() {
            o.insert("erratum".into(), e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fact the log scan found first: an NDVI read from DNs 2706 and 2218
    /// with offset 0 (0.099). With the offset the same DNs give 0.167.
    fn ndvi_fact(item: &str, signed_at: &str) -> JsonValue {
        json!({
            "kind": "primary", "cell": "defi.zb554.zbaed.ze5b1", "band": "indices.ndvi",
            "tslot": 19031, "value": 0.09910641754671001, "signed_at": signed_at,
            "derivation": {"fn_key": "sentinel2_l2a_indices_ndvi@1",
                "args": [29.9, 78.1, item, 32643, "NDVI", [2706.0, 2218.0], 1.83, 40.0, 30, 5, 1]},
            "sources": [{"scheme": "sentinel_s2_l2a", "id": format!("https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/43/R/GP/2022/2/{item}/B08.tif")}],
        })
    }

    #[test]
    fn a_point_fact_is_wrong_and_says_its_pixel_may_be_too() {
        let mut f = ndvi_fact("S2B_43RGP_20220208_0_L2A", "2026-07-14T17:55:50Z");
        annotate(&mut f);
        let e = &f["erratum"];
        assert_eq!(e["status"], "wrong");
        assert_eq!(e["items"], json!(["S2B_43RGP_20220208_0_L2A"]));
        // Its DNs may be the neighbour's, so no value is corrected from them.
        assert!(e.get("corrected_value").is_none());
        assert!(e["pixel"].as_str().is_some_and(|p| p.contains("neighbour")));
        // Signed between the pixel fix and the offset fix: no pixel note.
        let mut f = ndvi_fact("S2B_43RGP_20220208_0_L2A", "2026-10-01T00:00:00Z");
        annotate(&mut f);
        assert_eq!(f["erratum"]["status"], "wrong");
        assert!(f["erratum"].get("pixel").is_none());
    }

    #[test]
    fn leaves_alone_what_the_rule_does_not_cover() {
        // Signed after the fix went live.
        let mut f = ndvi_fact("S2B_43RGP_20220208_0_L2A", "2026-11-07T00:00:00Z");
        annotate(&mut f);
        assert!(f.get("erratum").is_none());
        // An item from after Element84 removed the offset.
        let mut f = ndvi_fact("S2B_43RGP_20220310_0_L2A", "2026-07-14T17:55:50Z");
        annotate(&mut f);
        assert!(f.get("erratum").is_none());
        // A Planetary Computer item of an affected day: never matched.
        let mut f = ndvi_fact(
            "S2B_MSIL2A_20220208T054039_R005_T43RGP_20220208T080000",
            "2026-07-14T17:55:50Z",
        );
        f["sources"] = json!([]);
        annotate(&mut f);
        assert!(f.get("erratum").is_none());
    }

    #[test]
    fn the_day_of_the_change_is_unknown_not_corrected() {
        let mut f = ndvi_fact("S2A_44RNQ_20220227_1_L2A", "2026-07-14T17:55:50Z");
        annotate(&mut f);
        assert_eq!(f["erratum"]["status"], "unknown");
        assert!(f["erratum"].get("corrected_value").is_none());
    }

    #[test]
    fn a_derived_record_names_the_scenes_it_took() {
        let mut f = json!({
            "kind": "derivative", "band": "field.series", "signed_at": "2026-10-01T00:00:00Z",
            "derivation": {"fn_key": "s2_index_series@1"},
            "value": {"scenes": [{"id": "S2A_43SFS_20220127_1_L2A"}, {"id": "S2A_43SFS_20230127_0_L2A"}]},
        });
        annotate(&mut f);
        assert_eq!(f["erratum"]["status"], "wrong");
        assert_eq!(f["erratum"]["items"], json!(["S2A_43SFS_20220127_1_L2A"]));
    }

    #[test]
    fn a_wrapped_fact_is_read_too() {
        let inner = ndvi_fact("S2B_43RGP_20220208_0_L2A", "2026-07-14T17:55:50Z");
        let mut f = json!({"Primary": inner});
        annotate(&mut f);
        assert_eq!(f["erratum"]["status"], "wrong");
    }
}
