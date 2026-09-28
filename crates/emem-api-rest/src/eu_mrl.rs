//! The EU maximum residue levels in force, from the Commission's own
//! pesticides database (DG SANTE, Regulation (EC) 396/2005), for reading a
//! lab report against the limit that applies rather than the one it prints.
//!
//! Anonymous JSON API, paged by `nextLink`. A product's current MRLs are
//! ~700 rows in 7 pages; each product and the product list are kept a day.

use serde_json::{json, Value as JsonValue};

const BASE: &str = "https://api.datalake.sante.service.ec.europa.eu/sante/pesticides";
const DAY: std::time::Duration = std::time::Duration::from_secs(86_400);

#[derive(Debug, Clone)]
pub struct Product {
    pub id: u64,
    pub code: String,
    pub name: String,
    synonyms: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Mrl {
    pub residue: String,
    pub display: String,
    /// `None` for "No MRL required" and similar non-numeric entries.
    pub mg_kg: Option<f64>,
    /// `*`: set at the limit of determination, i.e. no use authorised.
    pub at_lod: bool,
}

async fn get_all(url: String) -> Result<Vec<JsonValue>, String> {
    let cli = crate::reqwest_client();
    let mut out = Vec::new();
    let mut next = Some(url);
    let mut pages = 0;
    while let Some(u) = next.take() {
        pages += 1;
        if pages > 40 {
            return Err("EU pesticides API: more than 40 pages".into());
        }
        let r = tokio::time::timeout(std::time::Duration::from_secs(30), cli.get(&u).send())
            .await
            .map_err(|_| format!("EU pesticides API timed out at {u}"))?
            .map_err(|e| format!("EU pesticides API: {e}"))?;
        if !r.status().is_success() {
            return Err(format!("EU pesticides API status {} at {u}", r.status()));
        }
        let d: JsonValue = r
            .json()
            .await
            .map_err(|e| format!("EU pesticides API json: {e}"))?;
        out.extend(d["value"].as_array().cloned().unwrap_or_default());
        next = d["nextLink"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
    }
    Ok(out)
}

type Cached<T> = std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, T)>>;

fn cached<T: Clone>(slot: &'static std::sync::OnceLock<Cached<T>>, key: &str) -> Option<T> {
    let m = slot
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    m.get(key)
        .filter(|(at, _)| at.elapsed() < DAY)
        .map(|(_, v)| v.clone())
}

fn store<T>(slot: &'static std::sync::OnceLock<Cached<T>>, key: &str, v: T) {
    let mut m = slot
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if m.len() > 512 {
        m.clear();
    }
    m.insert(key.to_string(), (std::time::Instant::now(), v));
}

async fn products() -> Result<Vec<Product>, String> {
    static P: std::sync::OnceLock<Cached<Vec<Product>>> = std::sync::OnceLock::new();
    if let Some(v) = cached(&P, "EN") {
        return Ok(v);
    }
    let rows = get_all(format!(
        "{BASE}/pesticide-residues-products?format=json&language=EN&api-version=v3.0"
    ))
    .await?;
    let v: Vec<Product> = rows
        .iter()
        .filter_map(|r| {
            Some(Product {
                id: r["product_id"].as_u64()?,
                code: r["product_code"].as_str()?.to_string(),
                name: r["product_name"].as_str()?.to_string(),
                synonyms: r["product_synonym_names"]
                    .as_str()
                    .map(|s| s.split(", ").map(str::to_string).collect())
                    .unwrap_or_default(),
            })
        })
        .collect();
    store(&P, "EN", v.clone());
    Ok(v)
}

/// Whole words of a name, lowercased: "Sweet peppers/bell peppers".
fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.trim_end_matches('s').to_string())
        .collect()
}

/// A product by EU code, or by name or synonym matched on whole words (a
/// substring match put blueberries under "chilli" by way of "Chilean
/// guavas"). Returns the candidates when more than one matches equally.
pub async fn resolve_product(q: &str) -> Result<Result<Product, Vec<Product>>, String> {
    let all = products().await?;
    let q = q.trim();
    if let Some(p) = all.iter().find(|p| p.code == q) {
        return Ok(Ok(p.clone()));
    }
    let want = words(q);
    if want.is_empty() {
        return Ok(Err(Vec::new()));
    }
    let score = |p: &Product| -> usize {
        std::iter::once(&p.name)
            .chain(p.synonyms.iter())
            .map(|n| {
                let have = words(n);
                if want.iter().all(|w| have.contains(w)) {
                    // A match on the product's own name beats one on a synonym.
                    if n == &p.name {
                        2
                    } else {
                        1
                    }
                } else {
                    0
                }
            })
            .max()
            .unwrap_or(0)
    };
    let best = all.iter().map(score).max().unwrap_or(0);
    let hits: Vec<Product> = all
        .iter()
        .filter(|p| best > 0 && score(p) == best)
        .cloned()
        .collect();
    Ok(match hits.len() {
        1 => Ok(hits[0].clone()),
        _ => Err(hits),
    })
}

pub async fn current_mrls(product_id: u64) -> Result<Vec<Mrl>, String> {
    static M: std::sync::OnceLock<Cached<Vec<Mrl>>> = std::sync::OnceLock::new();
    let key = product_id.to_string();
    if let Some(v) = cached(&M, &key) {
        return Ok(v);
    }
    let rows = get_all(format!(
        "{BASE}/product-current-mrl-all-residues?format=json&PRODUCT_ID={product_id}&api-version=v3.0"
    ))
    .await?;
    let v: Vec<Mrl> = rows
        .iter()
        .filter_map(|r| {
            Some(Mrl {
                residue: r["PESTICIDE_RESIDUE_NAME"].as_str()?.to_string(),
                // Empty for an MRL set above the limit of determination.
                display: r["MRL_DISPLAY"]
                    .as_str()
                    .filter(|d| !d.is_empty())
                    .or(r["MRL_VALUE"].as_str())
                    .unwrap_or("")
                    .to_string(),
                mg_kg: r["MRL_VALUE"].as_str().and_then(|v| v.trim().parse().ok()),
                at_lod: r["MRL_LOD"].as_str() == Some("*"),
            })
        })
        .collect();
    store(&M, &key, v.clone());
    Ok(v)
}

/// The EU residue whose definition names this analyte first. Residue names
/// are definitions ("Carbendazim and benomyl (sum of ...)(R)"), so the
/// analyte must be the leading name, not merely appear inside one.
pub fn match_residue<'a>(analyte: &str, mrls: &'a [Mrl]) -> Option<&'a Mrl> {
    let a = analyte.trim().to_lowercase();
    if a.len() < 3 {
        return None;
    }
    let lead = |r: &str| {
        let r = r.to_lowercase();
        let cut = r
            .find(" (")
            .into_iter()
            .chain(r.find(" and "))
            .chain(r.find('('))
            .min()
            .unwrap_or(r.len());
        r[..cut].trim().to_string()
    };
    mrls.iter()
        .filter(|m| lead(&m.residue) == a)
        .min_by_key(|m| m.residue.len())
}

/// The lab report's rows set against the MRLs in force for `product`.
pub async fn annotate(result: &mut JsonValue, product: &str) -> JsonValue {
    let p = match resolve_product(product).await {
        Ok(Ok(p)) => p,
        Ok(Err(c)) if c.is_empty() => {
            return json!({"status": "product_not_found", "product": product})
        }
        Ok(Err(c)) => {
            return json!({"status": "product_ambiguous", "product": product,
                "candidates": c.iter().take(10).map(|p| json!({"code": p.code, "name": p.name})).collect::<Vec<_>>()})
        }
        Err(e) => return json!({"status": "unavailable", "error": e}),
    };
    let mrls = match current_mrls(p.id).await {
        Ok(m) => m,
        Err(e) => return json!({"status": "unavailable", "error": e, "product": p.name}),
    };
    let (mut matched, mut exceed) = (0usize, 0usize);
    let mut unmatched = Vec::new();
    if let Some(rows) = result["rows"].as_array_mut() {
        for row in rows.iter_mut() {
            let analyte = row["analyte"].as_str().unwrap_or("").to_string();
            let Some(m) = match_residue(&analyte, &mrls) else {
                unmatched.push(analyte);
                continue;
            };
            matched += 1;
            let r = row["result_mg_kg"].as_f64();
            // Under a limit is within the MRL only when that limit is.
            let factor = match row["unit"].as_str() {
                Some("µg/kg") => 0.001,
                Some("g/kg") => 1000.0,
                _ => 1.0,
            };
            let under = match row["result"]["kind"].as_str() {
                Some("below_limit") => row["result"]["limit"]
                    .as_f64()
                    .map(|l| l * factor)
                    .or(row["loq_mg_kg"].as_f64()),
                Some("not_detected") => row["loq_mg_kg"].as_f64(),
                _ => None,
            };
            let exceeds = match (r, m.mg_kg) {
                (Some(r), Some(l)) => Some(r > l),
                (None, Some(l)) => under.filter(|u| *u <= l).map(|_| false),
                _ => None,
            };
            if exceeds == Some(true) {
                exceed += 1;
            }
            row["eu_mrl"] = json!({"residue": m.residue, "display": m.display, "mg_kg": m.mg_kg,
                "at_limit_of_determination": m.at_lod, "exceeds": exceeds});
        }
    }
    json!({
        "status": "applied",
        "product": {"code": p.code, "name": p.name, "id": p.id},
        "source": format!("{BASE}/product-current-mrl-all-residues?format=json&PRODUCT_ID={}&api-version=v3.0", p.id),
        "regulation": "Regulation (EC) No 396/2005, MRLs currently applicable per the EU Pesticides Database",
        "rows_matched": matched,
        "rows_unmatched": unmatched,
        "exceedances": exceed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, v: Option<f64>) -> Mrl {
        Mrl {
            residue: name.into(),
            display: String::new(),
            mg_kg: v,
            at_lod: false,
        }
    }

    #[test]
    fn an_analyte_matches_the_residue_it_leads_not_one_it_appears_in() {
        let list = vec![
            m("Carbendazim and benomyl (sum of benomyl and carbendazim expressed as carbendazim)(R)", Some(0.1)),
            m("Chlorpyrifos (F)", Some(0.01)),
            m("Chlorpyrifos-methyl (F)", Some(0.01)),
            m("Thiophanate-methyl (R)", Some(0.1)),
        ];
        assert_eq!(
            match_residue("Carbendazim", &list).unwrap().mg_kg,
            Some(0.1)
        );
        assert_eq!(
            match_residue("Chlorpyrifos", &list).unwrap().residue,
            "Chlorpyrifos (F)"
        );
        assert_eq!(
            match_residue("chlorpyrifos-methyl", &list).unwrap().residue,
            "Chlorpyrifos-methyl (F)"
        );
        assert!(
            match_residue("Benomyl", &list).is_none(),
            "named inside a definition, not leading it"
        );
    }

    #[test]
    fn product_words_are_whole_words() {
        assert!(words("Ugniberries/Chilean guavas")
            .iter()
            .all(|w| w != "chilli"));
        assert!(words("Sweet peppers/bell peppers").contains(&"pepper".to_string()));
    }
}
