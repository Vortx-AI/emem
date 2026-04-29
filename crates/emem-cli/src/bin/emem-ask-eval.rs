//! emem-ask-eval — trigger-rate harness for /v1/ask.
//!
//! Walks a corpus of "should-be-emem" free-text questions (the kind an
//! LLM with emem connected ought to forward to the protocol instead of
//! refusing with "I can't access live data") and measures end-to-end
//! routing success against a running responder. Reports the per-question
//! result and a single trigger-rate percentage.
//!
//! Usage:
//!   EMEM_BASE_URL=http://127.0.0.1:5051 cargo run -p emem-cli --bin emem-ask-eval
//!   cargo run -p emem-cli --bin emem-ask-eval -- --base https://emem.dev
//!
//! Exit code: 0 if every prompt routes to its expected topic AND returns
//! a signed receipt; 1 otherwise. Suitable for CI as a regression check
//! whenever TOPIC_KEYWORDS or the locate description changes.

use serde_json::Value;

const DEFAULT_BASE: &str = "http://127.0.0.1:5051";

/// (free-text question, place, expected_topic). Places drawn from the
/// embedded gazetteer (no Nominatim round-trip needed) so the eval is
/// hermetic against external geocoder rate limits.
const CORPUS: &[(&str, &str, &str)] = &[
    // Lifestyle / decision-making — the original Ashok-Nagar shape that
    // historically triggered "I can't access live data" refusals.
    (
        "is this neighbourhood flood-prone for a flat purchase",
        "Mumbai",
        "flood_risk_composite",
    ),
    (
        "should I buy a house here, is the area safe to live",
        "Bengaluru",
        "flood_risk_composite",
    ),
    (
        "does this area have monsoon waterlogging issues",
        "Chennai",
        "flood_risk_composite",
    ),
    // Insurance / property risk.
    (
        "estimate the insurance premium for this neighbourhood",
        "Delhi",
        "real_estate",
    ),
    (
        "what is the climate risk score for this place",
        "Kolkata",
        "real_estate",
    ),
    // Livability.
    ("how walkable is this area", "Tokyo", "urban_livability"),
    (
        "urban heat island intensity here",
        "Singapore",
        "urban_livability",
    ),
    // Direct flood / water.
    (
        "flood history of this place",
        "Jakarta",
        "flood_history_long_term",
    ),
    (
        "has this area ever flooded",
        "Bangkok",
        "flood_history_long_term",
    ),
    // Vegetation.
    ("what's the NDVI here", "Paris", "vegetation_condition"),
    // Built-up.
    (
        "is this area densely built up",
        "Seoul",
        "built_up_human_geography",
    ),
    // Topography.
    (
        "elevation of this place above sea level",
        "London",
        "elevation_land_only",
    ),
    ("how rugged is the terrain here", "Beijing", "topography"),
    // Out-of-scope canary — should NOT route to a topic.
    ("what is the meaning of life", "New York", "OUT_OF_SCOPE"),
];

#[derive(Debug)]
#[allow(dead_code)] // Fields surfaced via Debug formatting in the human-readable report.
struct ResultRow {
    q: String,
    place: String,
    expected: String,
    matched_topics: Vec<String>,
    fact_count: usize,
    out_of_scope: bool,
    receipt_cids: usize,
    pass: bool,
    note: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut base = std::env::var("EMEM_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE.into());
    while let Some(a) = args.next() {
        if a == "--base" {
            if let Some(v) = args.next() {
                base = v;
            }
        }
    }

    println!("emem-ask-eval — base={base}");
    println!("─────────────────────────────────────────────────────────────────────");

    // First-call materialization on a fresh cell can pull from STAC (S2),
    // JRC GSW, Cop-DEM, Overture, and met.no in one shot — easily 60 s
    // worst-case on a cloud-prone tropical cell. 90 s gives headroom
    // without making CI hang forever.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()?;

    let mut rows: Vec<ResultRow> = Vec::with_capacity(CORPUS.len());
    for (q, place, expected) in CORPUS {
        let body = serde_json::json!({ "q": q, "place": place });
        let resp = client
            .post(format!("{base}/v1/ask"))
            .json(&body)
            .send()
            .await;
        let row = match resp {
            Ok(r) => {
                let status = r.status();
                let v: Value = r.json().await.unwrap_or(Value::Null);
                evaluate(q, place, expected, status.as_u16(), &v)
            }
            Err(e) => ResultRow {
                q: q.to_string(),
                place: place.to_string(),
                expected: expected.to_string(),
                matched_topics: vec![],
                fact_count: 0,
                out_of_scope: true,
                receipt_cids: 0,
                pass: false,
                note: format!("transport error: {e}"),
            },
        };
        let mark = if row.pass { "✓" } else { "✗" };
        println!(
            "{mark} {:<58}  topic={:<32}  facts={:>2}  cids={:>2}  {}",
            truncate(&row.q, 56),
            row.matched_topics
                .first()
                .cloned()
                .unwrap_or_else(|| "<none>".into()),
            row.fact_count,
            row.receipt_cids,
            row.note,
        );
        rows.push(row);
    }

    println!("─────────────────────────────────────────────────────────────────────");
    let passed = rows.iter().filter(|r| r.pass).count();
    let total = rows.len();
    let pct = if total == 0 {
        0.0
    } else {
        100.0 * passed as f64 / total as f64
    };
    println!("trigger-rate: {passed}/{total}  ({pct:.1}%)");

    if passed < total {
        std::process::exit(1);
    }
    Ok(())
}

fn evaluate(q: &str, place: &str, expected: &str, status: u16, v: &Value) -> ResultRow {
    let matched_topics: Vec<String> = v
        .pointer("/topic_routing/matched_topics")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let out_of_scope = v
        .pointer("/topic_routing/out_of_scope")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let fact_count = v
        .pointer("/facts/facts")
        .and_then(|x| x.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let receipt_cids = v
        .pointer("/facts/receipt/fact_cids")
        .and_then(|x| x.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let mut pass = true;
    let mut notes: Vec<String> = Vec::new();
    if status != 200 {
        pass = false;
        notes.push(format!("HTTP {status}"));
    }
    if expected == "OUT_OF_SCOPE" {
        if !out_of_scope {
            pass = false;
            notes.push(format!("expected out-of-scope, matched {matched_topics:?}"));
        }
    } else {
        let first = matched_topics.first().map(String::as_str);
        if first != Some(expected) {
            pass = false;
            notes.push(format!("expected={expected} got={first:?}"));
        }
        if receipt_cids == 0 {
            pass = false;
            notes.push("no receipt fact_cids".into());
        }
    }

    ResultRow {
        q: q.to_string(),
        place: place.to_string(),
        expected: expected.to_string(),
        matched_topics,
        fact_count,
        out_of_scope,
        receipt_cids,
        pass,
        note: notes.join("; "),
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(n.saturating_sub(1)).collect::<String>()
        )
    }
}
