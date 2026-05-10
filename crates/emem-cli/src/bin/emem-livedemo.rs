//! emem-livedemo — comprehensive end-to-end exercise of every primitive
//! against a live emem server, with full traceability written to disk.
//!
//! Each step's request and response is saved as JSON to a chosen output
//! directory along with a `trace.json` index tying request_id → fact_cids
//! → file paths so a third party can replay every byte we exchanged.
//!
//! Usage:
//!     emem-livedemo [BASE_URL] [OUT_DIR]
//! Defaults: http://localhost:5051   ./var/demos/<UTC stamp>

use std::collections::BTreeMap;
use std::path::PathBuf;

use blake3::Hasher;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rand::RngCore;
use serde_json::{json, Value};

use emem_attest::merkle_root;
use emem_codec::to_cell64;
use emem_core::{AttesterKey, Cell, KeyEpoch, Signature};
use emem_fact::{Attestation, Derivation, Fact, PrimaryFact, RegistryCid, SchemaCid, Source};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = resolve_base_url();
    let stamp = utc_stamp();
    let out_dir: PathBuf = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("var/demos/{stamp}")));
    std::fs::create_dir_all(&out_dir)?;
    let client = reqwest::Client::new();

    let mut trace = TraceIndex::new(&base, &out_dir);

    // ----- Step 1: discover (one GET each) ----------------------------
    let health = trace.get(&client, "01_health", "/health").await?;
    let registry_cid = health["registry_cid"].as_str().unwrap().to_string();
    let schema_cid = health["schema_cid"].as_str().unwrap().to_string();
    let pubkey_b32 = health["responder_pubkey_b32"].as_str().unwrap().to_string();

    trace
        .get(&client, "02_well_known", "/.well-known/emem.json")
        .await?;
    trace
        .get(&client, "03_agent_card", "/v1/agent_card")
        .await?;
    trace
        .get(&client, "04_quickstart", "/v1/quickstart")
        .await?;
    trace.get(&client, "05_bands", "/v1/bands").await?;
    trace.get(&client, "06_manifests", "/v1/manifests").await?;

    // ----- Step 2: build attester + facts -----------------------------
    let mut secret = [0u8; 32];
    OsRng.fill_bytes(&mut secret);
    let signing = SigningKey::from_bytes(&secret);
    let mut pubkey = [0u8; 32];
    pubkey.copy_from_slice(signing.verifying_key().as_bytes());
    let attester = AttesterKey(pubkey);
    let attester_b32 = b32(&pubkey);

    // 4 cells × 2 bands × 3 tslots = 24 facts. Adjacent on a Hilbert path.
    let cells: Vec<String> = (0u64..4)
        .map(|i| {
            let raw = (1u64 << 59) | (13u64 << 52) | ((i + 17) << 45) | 0xabcde12345u64;
            to_cell64(Cell::from_raw(raw))
        })
        .collect();
    let bands: &[(&str, usize)] = &[("geotessera", 16), ("sentinel2_raw", 10)];
    let tslots: &[u64] = &[0, 1, 2];

    let mut facts: Vec<Fact> = Vec::new();
    for (ci, cell) in cells.iter().enumerate() {
        for &(band, dims) in bands {
            for &t in tslots {
                let vec_val: Vec<Value> = (0..dims)
                    .map(|d| {
                        // value drifts with cell index, tslot, and dim — gives meaningful
                        // diff / trajectory / cosine signal.
                        let v = (ci as f64 - 1.5) * 0.10 + (t as f64) * 0.05 + (d as f64) * 0.01;
                        Value::from(v)
                    })
                    .collect();
                let vec_cbor = json_to_cbor(&Value::Array(vec_val));
                facts.push(Fact::Primary(PrimaryFact {
                    cell: cell.clone(),
                    band: band.into(),
                    tslot: t,
                    value: vec_cbor,
                    unit: None,
                    confidence: 0.95,
                    uncertainty: None,
                    sources: vec![Source {
                        scheme: "demo".into(),
                        id: format!("c{ci}-{band}-t{t}"),
                        cid: None,
                        hash: None,
                        captured_at: None,
                        url: None,
                    }],
                    derivation: Derivation {
                        fn_key: "livedemo@1".into(),
                        args: None,
                    },
                    privacy_class: "public".into(),
                    schema_cid: SchemaCid::new(&schema_cid),
                    signer: attester,
                    signed_at: "2026-04-26T15:00:00Z".into(),
                    served_via: None,
                }));
            }
        }
    }

    let mut leaves: Vec<[u8; 32]> = facts
        .iter()
        .map(|f| {
            let mut buf = Vec::new();
            ciborium::ser::into_writer(f, &mut buf).unwrap();
            let h = blake3::hash(&buf);
            let mut a = [0u8; 32];
            a.copy_from_slice(h.as_bytes());
            a
        })
        .collect();
    leaves.sort();
    let batch_root = merkle_root(&leaves);

    let mut h = Hasher::new();
    h.update(&batch_root);
    h.update(registry_cid.as_bytes());
    h.update(schema_cid.as_bytes());
    let msg = h.finalize();
    let dalek_sig = signing.sign(msg.as_bytes());
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&dalek_sig.to_bytes());

    let att = Attestation {
        facts,
        batch_root,
        attester,
        attester_key_epoch: KeyEpoch(0),
        registry_cid: RegistryCid::new(registry_cid.clone()),
        schema_cid: SchemaCid::new(schema_cid.clone()),
        signature: Signature(sig_bytes),
        attested_at: "2026-04-26T15:00:00Z".into(),
    };

    // POST attestation as canonical CBOR for byte-exact merkle agreement.
    let mut att_cbor = Vec::new();
    ciborium::ser::into_writer(&att, &mut att_cbor)?;
    let att_resp = trace
        .post_cbor(&client, "07_attest_cbor", "/v1/attest_cbor", &att_cbor)
        .await?;
    let cids: Vec<String> = att_resp["cids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    trace.attested_cids = cids.clone();

    // ----- Step 3: read primitives ------------------------------------
    let recall = trace
        .post_json(
            &client,
            "08_recall",
            "/v1/recall",
            json!({"cell": &cells[1]}),
        )
        .await?;
    let recall_receipt = recall["receipt"].clone();

    trace
        .post_json(
            &client,
            "09_compare",
            "/v1/compare",
            json!({"a": &cells[0], "b": &cells[3]}),
        )
        .await?;

    trace
        .post_json(
            &client,
            "10_find_similar_cell",
            "/v1/find_similar",
            json!({"key": &cells[1], "k": 5}),
        )
        .await?;

    trace.post_json(&client, "11_find_similar_inline", "/v1/find_similar",
        json!({"key": "inline:[-0.05,-0.04,-0.03,-0.02,-0.01,0.00,0.01,0.02,0.03,0.04,0.05,0.06,0.07,0.08,0.09,0.10]", "k": 3})).await?;

    trace
        .post_json(
            &client,
            "12_diff",
            "/v1/diff",
            json!({"cell": &cells[1], "band": "geotessera", "tslot_a": 0u64, "tslot_b": 2u64}),
        )
        .await?;

    trace
        .post_json(
            &client,
            "13_trajectory",
            "/v1/trajectory",
            json!({"cell": &cells[1], "band": "geotessera", "window": [0u64, 3u64]}),
        )
        .await?;

    trace.post_json(&client, "14_verify_claim", "/v1/verify",
        json!({"cell": &cells[2], "claim": {"band": "sentinel2_raw", "op": "gt", "value": -1.0, "tslot": 1u64}})).await?;

    let region_geom = format!("cells:{},{},{},{}", cells[0], cells[1], cells[2], cells[3]);
    trace
        .post_json(
            &client,
            "15_query_region",
            "/v1/query_region",
            json!({"geometry": region_geom, "bands": ["geotessera"], "agg": "mean"}),
        )
        .await?;

    trace
        .post_json(
            &client,
            "16_intent_what_is_here",
            "/v1/intent",
            json!({"type": "what_is_here", "cell": &cells[0]}),
        )
        .await?;

    // ----- Step 4: MCP round-trip -------------------------------------
    trace
        .post_json(
            &client,
            "17_mcp_tools_list",
            "/mcp",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await?;

    trace
        .post_json(
            &client,
            "18_mcp_call_recall",
            "/mcp",
            json!({
                "jsonrpc":"2.0","id":2,"method":"tools/call",
                "params": {"name":"emem.recall","arguments":{"cell": &cells[2]}}
            }),
        )
        .await?;

    // ----- Step 5: content-addressed retrieval ------------------------
    trace
        .get(
            &client,
            "19_facts_by_cid",
            &format!("/v1/facts/{}", &cids[0]),
        )
        .await?;

    // ----- Step 6: offline receipt verification -----------------------
    trace
        .post_json(
            &client,
            "20_verify_receipt",
            "/v1/verify_receipt",
            json!({"receipt": recall_receipt}),
        )
        .await?;

    // ----- Final write-out --------------------------------------------
    trace.attester_pubkey_b32 = attester_b32;
    trace.responder_pubkey_b32 = pubkey_b32;
    trace.cells = cells;
    trace.bands = bands.iter().map(|(b, _)| (*b).to_string()).collect();
    trace.tslots = tslots.to_vec();
    trace.write()?;

    println!("\nLive demo complete.");
    println!("  base       = {}", trace.base);
    println!("  out_dir    = {}", trace.out_dir.display());
    println!("  steps      = {}", trace.steps.len());
    println!("  attested   = {} cids", trace.attested_cids.len());
    println!("  responder  = {}", trace.responder_pubkey_b32);
    println!("  attester   = {}", trace.attester_pubkey_b32);
    println!("  trace      = {}/trace.json", trace.out_dir.display());
    Ok(())
}

// ---- trace harness ----------------------------------------------------

struct TraceIndex {
    base: String,
    out_dir: PathBuf,
    started_at: String,
    steps: Vec<StepRecord>,
    attested_cids: Vec<String>,
    cells: Vec<String>,
    bands: Vec<String>,
    tslots: Vec<u64>,
    attester_pubkey_b32: String,
    responder_pubkey_b32: String,
}

#[derive(serde::Serialize)]
struct StepRecord {
    name: String,
    method: String,
    path: String,
    status: u16,
    request_file: Option<String>,
    response_file: String,
    request_cid_b3: Option<String>,
    response_cid_b3: String,
    request_id: Option<String>,
    served_at: Option<String>,
    primitive: Option<String>,
    fact_cids: Vec<String>,
    curl_repro: String,
}

impl TraceIndex {
    fn new(base: &str, out_dir: &std::path::Path) -> Self {
        Self {
            base: base.to_string(),
            out_dir: out_dir.to_path_buf(),
            started_at: utc_iso(),
            steps: Vec::new(),
            attested_cids: Vec::new(),
            cells: Vec::new(),
            bands: Vec::new(),
            tslots: Vec::new(),
            attester_pubkey_b32: String::new(),
            responder_pubkey_b32: String::new(),
        }
    }

    async fn get(&mut self, c: &reqwest::Client, name: &str, path: &str) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base, path);
        let resp = c.get(&url).send().await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        let value: Value =
            serde_json::from_str(&body).unwrap_or_else(|_| Value::String(body.clone()));
        let resp_file = self.write_response(name, &body)?;
        let curl = format!("curl -s '{url}'");
        self.record(StepRecord {
            name: name.into(),
            method: "GET".into(),
            path: path.into(),
            status,
            request_file: None,
            response_file: resp_file,
            request_cid_b3: None,
            response_cid_b3: blake3_hex(body.as_bytes()),
            request_id: extract_str(&value, &["receipt", "request_id"]),
            served_at: extract_str(&value, &["receipt", "served_at"]),
            primitive: extract_str(&value, &["receipt", "primitive"]),
            fact_cids: extract_cids(&value),
            curl_repro: curl,
        });
        Ok(value)
    }

    async fn post_json(
        &mut self,
        c: &reqwest::Client,
        name: &str,
        path: &str,
        body: Value,
    ) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base, path);
        let body_str = serde_json::to_string(&body)?;
        let resp = c
            .post(&url)
            .header("content-type", "application/json")
            .body(body_str.clone())
            .send()
            .await?;
        let status = resp.status().as_u16();
        let resp_text = resp.text().await?;
        let value: Value =
            serde_json::from_str(&resp_text).unwrap_or_else(|_| Value::String(resp_text.clone()));
        let req_file = self.write_request(name, &body_str)?;
        let resp_file = self.write_response(name, &resp_text)?;
        let curl = format!(
            "curl -s -X POST '{url}' -H 'content-type: application/json' -d @{}",
            std::path::Path::new(&req_file)
                .file_name()
                .unwrap()
                .to_string_lossy()
        );
        self.record(StepRecord {
            name: name.into(),
            method: "POST".into(),
            path: path.into(),
            status,
            request_file: Some(req_file),
            response_file: resp_file,
            request_cid_b3: Some(blake3_hex(body_str.as_bytes())),
            response_cid_b3: blake3_hex(resp_text.as_bytes()),
            request_id: extract_str(&value, &["receipt", "request_id"])
                .or_else(|| extract_str(&value, &["request_id"])),
            served_at: extract_str(&value, &["receipt", "served_at"])
                .or_else(|| extract_str(&value, &["served_at"])),
            primitive: extract_str(&value, &["receipt", "primitive"]),
            fact_cids: extract_cids(&value),
            curl_repro: curl,
        });
        Ok(value)
    }

    async fn post_cbor(
        &mut self,
        c: &reqwest::Client,
        name: &str,
        path: &str,
        body: &[u8],
    ) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base, path);
        let resp = c
            .post(&url)
            .header("content-type", "application/cbor")
            .body(body.to_vec())
            .send()
            .await?;
        let status = resp.status().as_u16();
        let resp_text = resp.text().await?;
        let value: Value =
            serde_json::from_str(&resp_text).unwrap_or_else(|_| Value::String(resp_text.clone()));
        let req_file = self.write_request_bytes(name, body)?;
        let resp_file = self.write_response(name, &resp_text)?;
        let curl = format!(
            "curl -s -X POST '{url}' -H 'content-type: application/cbor' --data-binary @{}",
            std::path::Path::new(&req_file)
                .file_name()
                .unwrap()
                .to_string_lossy()
        );
        self.record(StepRecord {
            name: name.into(),
            method: "POST".into(),
            path: path.into(),
            status,
            request_file: Some(req_file),
            response_file: resp_file,
            request_cid_b3: Some(blake3_hex(body)),
            response_cid_b3: blake3_hex(resp_text.as_bytes()),
            request_id: extract_str(&value, &["request_id"]),
            served_at: extract_str(&value, &["attested_at"]),
            primitive: Some("emem.attest".into()),
            fact_cids: extract_cids(&value),
            curl_repro: curl,
        });
        Ok(value)
    }

    fn record(&mut self, s: StepRecord) {
        println!(
            "[{:>3}] {:>5} {:<6} {:<22} → {} ({})",
            self.steps.len() + 1,
            s.status,
            s.method,
            s.name,
            s.request_id.as_deref().unwrap_or("-"),
            s.fact_cids.len()
        );
        self.steps.push(s);
    }

    fn write_request(&self, name: &str, body: &str) -> anyhow::Result<String> {
        let p = self.out_dir.join(format!("{name}.req.json"));
        std::fs::write(&p, pretty(body))?;
        Ok(p.file_name().unwrap().to_string_lossy().into_owned())
    }
    fn write_request_bytes(&self, name: &str, body: &[u8]) -> anyhow::Result<String> {
        let p = self.out_dir.join(format!("{name}.req.cbor"));
        std::fs::write(&p, body)?;
        Ok(p.file_name().unwrap().to_string_lossy().into_owned())
    }
    fn write_response(&self, name: &str, body: &str) -> anyhow::Result<String> {
        let p = self.out_dir.join(format!("{name}.resp.json"));
        std::fs::write(&p, pretty(body))?;
        Ok(p.file_name().unwrap().to_string_lossy().into_owned())
    }

    fn write(&self) -> anyhow::Result<()> {
        let trace = json!({
            "schema": "emem.livedemo.trace.v1",
            "base_url": self.base,
            "started_at": self.started_at,
            "finished_at": utc_iso(),
            "responder_pubkey_b32": self.responder_pubkey_b32,
            "attester_pubkey_b32": self.attester_pubkey_b32,
            "cells": self.cells,
            "bands": self.bands,
            "tslots": self.tslots,
            "attested_fact_cids": self.attested_cids,
            "step_count": self.steps.len(),
            "steps": serde_json::to_value(&self.steps)?,
        });
        std::fs::write(
            self.out_dir.join("trace.json"),
            serde_json::to_string_pretty(&trace)?,
        )?;

        let md = render_markdown(self);
        std::fs::write(self.out_dir.join("README.md"), md)?;
        Ok(())
    }
}

// ---- helpers ---------------------------------------------------------

fn b32(b: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(b).to_lowercase()
}
fn blake3_hex(b: &[u8]) -> String {
    blake3::hash(b).to_hex().to_string()
}
fn pretty(s: &str) -> String {
    serde_json::from_str::<Value>(s)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| s.to_string())
}
fn extract_str(v: &Value, path: &[&str]) -> Option<String> {
    let mut cur = v;
    for k in path {
        cur = cur.get(*k)?;
    }
    cur.as_str().map(|s| s.to_string())
}
fn extract_cids(v: &Value) -> Vec<String> {
    if let Some(arr) = v.pointer("/receipt/fact_cids").and_then(|x| x.as_array()) {
        return arr
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
    }
    if let Some(arr) = v.get("cids").and_then(|x| x.as_array()) {
        return arr
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
    }
    Vec::new()
}
fn utc_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    civil_iso(secs)
}
fn utc_stamp() -> String {
    utc_iso()
        .replace([':', '-'], "")
        .replace('T', "_")
        .replace('Z', "")
}
fn civil_iso(secs: i64) -> String {
    let (y, m, d, hh, mm, ss) = civil_from_unix(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hh, mm, ss)
}
fn civil_from_unix(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let hh = (tod / 3600) as u32;
    let mm = ((tod % 3600) / 60) as u32;
    let ss = (tod % 60) as u32;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = (if m <= 2 { y + 1 } else { y }) as i32;
    (y, m, d, hh, mm, ss)
}
fn json_to_cbor(v: &Value) -> ciborium::Value {
    match v {
        Value::Null => ciborium::Value::Null,
        Value::Bool(b) => ciborium::Value::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                ciborium::Value::Integer(i.into())
            } else if let Some(f) = n.as_f64() {
                ciborium::Value::Float(f)
            } else {
                ciborium::Value::Null
            }
        }
        Value::String(s) => ciborium::Value::Text(s.clone()),
        Value::Array(a) => ciborium::Value::Array(a.iter().map(json_to_cbor).collect()),
        Value::Object(m) => ciborium::Value::Map(
            m.iter()
                .map(|(k, v)| (ciborium::Value::Text(k.clone()), json_to_cbor(v)))
                .collect(),
        ),
    }
}

fn render_markdown(t: &TraceIndex) -> String {
    let mut s = String::new();
    s.push_str("# emem live-demo trace\n\n");
    s.push_str(&format!("- Server: `{}`\n", t.base));
    s.push_str(&format!("- Started: `{}`\n", t.started_at));
    s.push_str(&format!(
        "- Responder pubkey (b32): `{}`\n",
        t.responder_pubkey_b32
    ));
    s.push_str(&format!(
        "- Demo attester pubkey (b32): `{}`\n",
        t.attester_pubkey_b32
    ));
    s.push_str(&format!("- Cells: `{}`\n", t.cells.join(", ")));
    s.push_str(&format!("- Bands: `{}`\n", t.bands.join(", ")));
    s.push_str(&format!("- Tslots: `{:?}`\n", t.tslots));
    s.push_str(&format!(
        "- Attested CIDs: {} (first = `{}`)\n\n",
        t.attested_cids.len(),
        t.attested_cids.first().cloned().unwrap_or_default()
    ));

    s.push_str("## Steps\n\n");
    s.push_str("| # | method | path | status | request_id | fact_cids |\n");
    s.push_str("|---|--------|------|--------|------------|-----------|\n");
    for (i, st) in t.steps.iter().enumerate() {
        s.push_str(&format!(
            "| {} | {} | `{}` | {} | `{}` | {} |\n",
            i + 1,
            st.method,
            st.path,
            st.status,
            st.request_id.as_deref().unwrap_or("-"),
            st.fact_cids.len()
        ));
    }

    s.push_str("\n## Per-step files\n\n");
    let mut by_step: BTreeMap<String, &StepRecord> = BTreeMap::new();
    for st in &t.steps {
        by_step.insert(st.name.clone(), st);
    }
    for (_, st) in by_step.iter() {
        s.push_str(&format!("### {}\n\n", st.name));
        if let Some(req) = &st.request_file {
            s.push_str(&format!("- Request: `{}`\n", req));
        }
        s.push_str(&format!("- Response: `{}`\n", st.response_file));
        s.push_str(&format!("- blake3(response) = `{}`\n", st.response_cid_b3));
        if let Some(rid) = &st.request_id {
            s.push_str(&format!("- request_id = `{}`\n", rid));
        }
        s.push_str(&format!("- repro: `{}`\n\n", st.curl_repro));
    }
    s
}

/// Resolve the server base URL with this precedence: positional CLI arg →
/// `EMEM_BASE_URL` → `http://localhost:5051` (the emem-server default bind
/// per `crates/emem-cli/src/bin/emem-server.rs`). When the default fires,
/// emit a one-line stderr note so the behavior is visible in CI logs.
fn resolve_base_url() -> String {
    const DEFAULT_BASE: &str = "http://localhost:5051";
    if let Some(arg) = std::env::args().nth(1) {
        return arg;
    }
    if let Ok(env_base) = std::env::var("EMEM_BASE_URL") {
        if !env_base.is_empty() {
            return env_base;
        }
    }
    eprintln!("note: using default base url {DEFAULT_BASE}; set EMEM_BASE_URL to override");
    DEFAULT_BASE.to_string()
}
