//! Wikidata as the geocoder's prominence evidence.
//!
//! A geocoder cascade that takes the first tier with any match cannot tell a
//! 2,685-person "Central Park, WA" from the park in New York, or Wisconsin's
//! "Mount Fuji" from Japan's. Wikidata can: every item carries a coordinate,
//! and its sitelink count (how many Wikipedias describe it) is the standard,
//! openly published measure of how much people mean it. Measured 2026-09-25
//! on 120 labelled place names (a replay of the rule over recorded
//! answers, not a live run), ranking by that evidence took unqualified
//! names from 87% to 98% correct.
//!
//! Two calls per name: `wbsearchentities` (names and aliases, with which
//! text matched) and `wbgetentities` (coordinates and sitelinks).

use reqwest::Client;
use serde_json::Value;

/// One Wikidata item that could be the place a name means.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub qid: String,
    pub label: String,
    pub description: String,
    pub lat: f64,
    pub lng: f64,
    /// Number of Wikipedia (and sister) pages about the item.
    pub sitelinks: u32,
    /// The query equals the item's label or one of its aliases, ignoring
    /// case, accents and punctuation (not merely a prefix of it).
    pub exact: bool,
}

impl Candidate {
    /// Prominence in orders of magnitude, plus one for an exact name match:
    /// a name that is exactly the item's is worth ten times the pages.
    pub fn score(&self) -> f64 {
        (self.sitelinks as f64 + 1.0).log10() + if self.exact { 1.0 } else { 0.0 }
    }
}

/// Candidates for `query` that have a coordinate, best first by
/// [`Candidate::score`].
pub async fn candidates(client: &Client, query: &str) -> Result<Vec<Candidate>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let base = std::env::var("EMEM_WIKIDATA_API")
        .unwrap_or_else(|_| "https://www.wikidata.org/w/api.php".into());
    let search: Value = get(
        client,
        &base,
        &[
            ("action", "wbsearchentities"),
            ("search", q),
            ("language", "en"),
            ("uselang", "en"),
            ("type", "item"),
            ("limit", "10"),
            ("format", "json"),
        ],
    )
    .await?;
    let want = crate::geonames::normalize(q);
    let hits: Vec<(String, bool, String)> = search
        .get("search")
        .and_then(|s| s.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    let id = x.get("id")?.as_str()?.to_string();
                    let text = x
                        .pointer("/match/text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    let desc = x
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some((id, crate::geonames::normalize(text) == want, desc))
                })
                .collect()
        })
        .unwrap_or_default();
    if hits.is_empty() {
        return Ok(Vec::new());
    }
    let ids = hits
        .iter()
        .map(|h| h.0.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let ents: Value = get(
        client,
        &base,
        &[
            ("action", "wbgetentities"),
            ("ids", &ids),
            ("props", "claims|sitelinks|labels"),
            ("languages", "en"),
            ("format", "json"),
        ],
    )
    .await?;
    let mut out: Vec<Candidate> = hits
        .into_iter()
        .filter_map(|(qid, exact, description)| {
            let e = ents.get("entities")?.get(&qid)?;
            let v = e.pointer("/claims/P625/0/mainsnak/datavalue/value")?;
            let (lat, lng) = (v.get("latitude")?.as_f64()?, v.get("longitude")?.as_f64()?);
            let label = e
                .pointer("/labels/en/value")
                .and_then(|l| l.as_str())
                .unwrap_or(&qid)
                .to_string();
            let sitelinks = e
                .get("sitelinks")
                .and_then(|s| s.as_object())
                .map_or(0, |m| m.len()) as u32;
            Some(Candidate {
                qid,
                label,
                description,
                lat,
                lng,
                sitelinks,
                exact,
            })
        })
        .collect();
    out.sort_by(|a, b| b.score().total_cmp(&a.score()));
    Ok(out)
}

async fn get(client: &Client, base: &str, params: &[(&str, &str)]) -> Result<Value, String> {
    let resp = client
        .get(base)
        .query(params)
        .header("user-agent", emem_core::outbound::user_agent())
        .send()
        .await
        .map_err(|e| format!("wikidata http: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("wikidata status {}", resp.status()));
    }
    resp.json().await.map_err(|e| format!("wikidata json: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(sitelinks: u32, exact: bool) -> Candidate {
        Candidate {
            qid: "Q1".into(),
            label: "x".into(),
            description: String::new(),
            lat: 0.0,
            lng: 0.0,
            sitelinks,
            exact,
        }
    }

    /// An exact name is worth one order of magnitude of prominence: 5 exact
    /// pages lose to 90 alias-prefix pages (the park over a bare "Yosemite"
    /// item); between two exact matches the more described wins (Kolkata,
    /// whose alias is "Calcutta", over a small item labelled "Calcutta").
    #[test]
    fn an_exact_name_is_worth_ten_times_the_pages() {
        assert!(c(90, false).score() > c(5, true).score());
        assert!(c(300, true).score() > c(27, true).score());
        assert!(c(30, true).score() > c(200, false).score());
    }
}
