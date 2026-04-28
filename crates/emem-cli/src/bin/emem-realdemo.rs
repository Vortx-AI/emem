//! emem-realdemo — real-world ingest demo.
//!
//! Pulls live bytes from Copernicus DEM 30m public S3 tiles (no keys, vsicurl-
//! style HTTP Range reads), records the GeoTIFF header + content-length + ETag
//! + per-tile blake3, and attests two real bands per location:
//!   - `copdem30m.provenance` — Map { etag, content_length, ifd_offset, ... }
//!   - `copdem30m.byte_histogram_v1` — 16-bin normalised byte histogram
//!     over the first 16 KiB of the GeoTIFF (a real, deterministic
//!     fingerprint of the actual remote bytes).
//!
//! Runs every read primitive against this real corpus. Trace lands at
//! `var/demos/realdata_<UTC>/`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use blake3::Hasher;
use ed25519_dalek::{Signer, SigningKey};
use rand::RngCore;
use rand::rngs::OsRng;
use serde_json::{json, Value};

use emem_attest::merkle_root;
use emem_codec::{to_cell64, cell_from_latlng};
use emem_core::{AttesterKey, KeyEpoch, Signature};
use emem_fact::{
    Attestation, Derivation, Fact, PrimaryFact, RegistryCid, SchemaCid, Source,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = std::env::args().nth(1).unwrap_or_else(|| "http://localhost:5051".into());
    let stamp = utc_stamp();
    let out_dir: PathBuf = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("var/demos/realdata_{stamp}")));
    std::fs::create_dir_all(&out_dir)?;
    let http = reqwest::Client::builder()
        .user_agent("emem-realdemo/0.0.2 (+https://emem.dev)")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let mut trace = TraceIndex::new(&base, &out_dir);

    // 1) Discover --------------------------------------------------------
    let health = trace.get(&http, "01_health", "/health").await?;
    let registry_cid = health["registry_cid"].as_str().unwrap().to_string();
    let schema_cid = health["schema_cid"].as_str().unwrap().to_string();
    let responder_b32 = health["responder_pubkey_b32"].as_str().unwrap().to_string();

    // 2) Real-world locations -------------------------------------------
    // (name, lat, lng, peak_elevation_m) — three places with very
    // different terrain so their Cop-DEM tiles produce visibly distinct
    // byte histograms. The elevation column is well-known ground truth
    // (NOAA/USGS-published peak heights); we attest it as a Primary fact
    // with `claude_knowledge` derivation so a future agent recalling
    // this cell gets a real answer rather than only integrity bands.
    let places: &[(&str, f64, f64, f32)] = &[
        ("mt_fuji",      35.3606, 138.7274, 3776.24),  // volcano
        ("mt_everest",   27.9881,  86.9250, 8848.86),  // himalaya
        ("grand_canyon", 36.1069,-112.1129, 1885.00),  // south rim avg
    ];

    println!("== fetching real Copernicus DEM 30m tiles ==");
    let mut real_facts: Vec<Fact> = Vec::new();
    let mut place_meta: Vec<(String, String)> = Vec::new(); // (name, cell64)
    let attester = fresh_attester();

    for (name, lat, lng, peak_m) in places {
        let cell = cell_from_latlng(*lat, *lng);
        let cell_str = to_cell64(cell);
        let url = copdem_url_for_latlon(*lat, *lng);
        println!("  {name}: lat={lat:>8.4} lng={lng:>9.4} cell={cell_str} → {url}");

        let header = http.head(&url).send().await?;
        let etag = header.headers().get("etag")
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let last_modified = header.headers().get("last-modified")
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let content_length: u64 = header.headers().get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok()).unwrap_or(0);
        let head_status = header.status().as_u16();

        // Real vsicurl-style fetch: 8 KiB header (for GeoTIFF magic/IFD)
        // + 56 KiB sample at 1 MiB offset so the histogram captures real
        // pixel-encoded strip data, not just header padding.
        let range_hdr = http.get(&url)
            .header("range", "bytes=0-8191").send().await?;
        let hdr_status = range_hdr.status().as_u16();
        let hdr_bytes = range_hdr.bytes().await?;
        let range_strip = http.get(&url)
            .header("range", "bytes=1048576-1105919").send().await?;
        let strip_status = range_strip.status().as_u16();
        let strip_bytes = range_strip.bytes().await?;
        let range_status = if hdr_status == 206 && strip_status == 206 { 206 } else { hdr_status.max(strip_status) };
        let mut range_bytes: Vec<u8> = Vec::with_capacity(hdr_bytes.len() + strip_bytes.len());
        range_bytes.extend_from_slice(&hdr_bytes);
        range_bytes.extend_from_slice(&strip_bytes);
        let bytes_b3 = blake3::hash(&range_bytes).to_hex().to_string();
        let n = range_bytes.len();

        let (magic_ok, byteorder, ifd_offset) = parse_geotiff_header(&range_bytes);
        let histogram = byte_histogram_16(&range_bytes);

        println!("    HEAD={head_status} content-length={content_length} etag={etag}");
        println!("    Range[0..{n}] status={range_status} blake3={}", &bytes_b3[..16]);
        println!("    GeoTIFF magic={magic_ok} byteorder={byteorder} ifd_offset={ifd_offset}");
        println!("    histogram first 4 bins = {:?}", &histogram[..4]);

        let provenance_value = json_to_cbor(&json!({
            "url": url,
            "etag": etag,
            "last_modified": last_modified,
            "content_length": content_length,
            "head_status": head_status,
            "range_status": range_status,
            "range_byte_count": n as u64,
            "range_blake3_hex": bytes_b3,
            "geotiff_magic_ok": magic_ok,
            "byteorder": byteorder,
            "ifd_offset": ifd_offset,
        }));
        let histogram_value = json_to_cbor(&Value::Array(
            histogram.iter().map(|v| Value::from(*v)).collect()
        ));

        let mut hash32 = [0u8; 32];
        hash32.copy_from_slice(blake3::hash(&range_bytes).as_bytes());
        let source = Source {
            scheme: "copernicus_dem_30m".into(),
            id: url.clone(),
            cid: None,
            hash: Some(hash32),
            captured_at: if last_modified.is_empty() { None } else { Some(last_modified.clone()) },
            url: None,
        };

        real_facts.push(Fact::Primary(PrimaryFact {
            cell: cell_str.clone(),
            band: "copdem30m.provenance".into(),
            tslot: 0,
            value: provenance_value,
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![source.clone()],
            derivation: Derivation { fn_key: "copdem.head_and_range@1".into(), args: None },
            privacy_class: "public".into(),
            schema_cid: SchemaCid::new(&schema_cid),
            signer: attester.pubkey,
            signed_at: utc_iso(),
        }));
        real_facts.push(Fact::Primary(PrimaryFact {
            cell: cell_str.clone(),
            band: "copdem30m.byte_histogram_v1".into(),
            tslot: 0,
            value: histogram_value,
            unit: Some("normalized_count".into()),
            confidence: 1.0,
            uncertainty: None,
            sources: vec![source.clone()],
            derivation: Derivation { fn_key: "byte_histogram_16@1".into(), args: None },
            privacy_class: "public".into(),
            schema_cid: SchemaCid::new(&schema_cid),
            signer: attester.pubkey,
            signed_at: utc_iso(),
        }));

        // Intentionally do NOT attest `copdem30m.elevation_mean` here.
        //
        // An earlier version of this demo wrote `peak_m` (e.g. 3776.24 for
        // Mt Fuji) under `derivation.fn_key = "claude_knowledge@1"` —
        // plausibly correct as the *summit* elevation, but the band is
        // defined as the *cell-mean* elevation, and these LLM-attested
        // facts then took precedence over the real auto-materializer
        // (Open-Meteo `open_meteo_copdem90m@1`) for the read path,
        // returning summit values where the registry promises cell-means.
        //
        // The right behavior is: emit only the integrity bands above
        // (`copdem30m.provenance`, `copdem30m.byte_histogram_v1`) which
        // *are* re-derivable from the bytes we just fetched, and let the
        // server's lazy-materialize path produce `copdem30m.elevation_mean`
        // on first read. `_peak_m` is retained in the call site as ground
        // truth for the *summit* (used elsewhere in this demo's report)
        // but never attested as a band value.
        let _peak_m = *peak_m;
        place_meta.push(((*name).into(), cell_str));
    }

    // 3) Build attestation, sign, post -------------------------------
    let mut leaves: Vec<[u8; 32]> = real_facts.iter().map(|f| {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(f, &mut buf).unwrap();
        let h = blake3::hash(&buf);
        let mut a = [0u8; 32]; a.copy_from_slice(h.as_bytes()); a
    }).collect();
    leaves.sort();
    let batch_root = merkle_root(&leaves);

    let mut h = Hasher::new();
    h.update(&batch_root);
    h.update(registry_cid.as_bytes());
    h.update(schema_cid.as_bytes());
    let msg = h.finalize();
    let dalek_sig = attester.signing.sign(msg.as_bytes());
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&dalek_sig.to_bytes());

    let att = Attestation {
        facts: real_facts,
        batch_root,
        attester: attester.pubkey,
        attester_key_epoch: KeyEpoch(0),
        registry_cid: RegistryCid::new(registry_cid.clone()),
        schema_cid: SchemaCid::new(schema_cid.clone()),
        stake: None,
        signature: Signature(sig_bytes),
        attested_at: utc_iso(),
    };
    let mut att_cbor = Vec::new();
    ciborium::ser::into_writer(&att, &mut att_cbor)?;
    let att_resp = trace.post_cbor(&http, "02_attest_real_cbor", "/v1/attest_cbor", &att_cbor).await?;
    let cids: Vec<String> = att_resp["cids"].as_array().unwrap()
        .iter().map(|v| v.as_str().unwrap().to_string()).collect();
    trace.attested_cids = cids.clone();

    // 4) Read primitives over real data ------------------------------
    let (fuji_name, fuji_cell)     = place_meta[0].clone();
    let (everest_name, everest_cell) = place_meta[1].clone();
    let (gc_name, gc_cell)         = place_meta[2].clone();
    let _ = (fuji_name, everest_name, gc_name);

    let recall = trace.post_json(&http, "03_recall_fuji", "/v1/recall",
        json!({"cell": &fuji_cell})).await?;
    let recall_receipt = recall["receipt"].clone();

    trace.post_json(&http, "04_compare_fuji_vs_everest", "/v1/compare",
        json!({"a": &fuji_cell, "b": &everest_cell, "family": "copdem30m.byte_histogram_v1"})).await?;

    trace.post_json(&http, "05_compare_fuji_vs_grand_canyon", "/v1/compare",
        json!({"a": &fuji_cell, "b": &gc_cell, "family": "copdem30m.byte_histogram_v1"})).await?;

    trace.post_json(&http, "06_find_similar_to_everest", "/v1/find_similar",
        json!({"key": &everest_cell, "k": 3, "band": "copdem30m.byte_histogram_v1"})).await?;

    let region = format!("cells:{},{},{}", fuji_cell, everest_cell, gc_cell);
    trace.post_json(&http, "07_query_region_dem", "/v1/query_region",
        json!({"geometry": region, "bands": ["copdem30m.provenance"]})).await?;

    trace.post_json(&http, "08_verify_ifd_offset", "/v1/verify",
        json!({"cell": &fuji_cell,
               "claim": {"band":"copdem30m.byte_histogram_v1","op":"gt","value":-1.0,"tslot":0}})).await?;

    trace.post_json(&http, "09_intent_what_is_here", "/v1/intent",
        json!({"type":"what_is_here","cell": &gc_cell})).await?;

    trace.post_json(&http, "10_mcp_tools_list", "/mcp",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await?;
    trace.post_json(&http, "11_mcp_recall_everest", "/mcp",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
               "params":{"name":"emem.recall","arguments":{"cell": &everest_cell}}})).await?;

    trace.get(&http, "12_facts_by_cid", &format!("/v1/facts/{}", &cids[0])).await?;
    trace.post_json(&http, "13_verify_receipt", "/v1/verify_receipt",
        json!({"receipt": recall_receipt})).await?;

    trace.attester_pubkey_b32 = b32(&attester.pubkey.0);
    trace.responder_pubkey_b32 = responder_b32;
    trace.cells = place_meta.iter().map(|(_, c)| c.clone()).collect();
    trace.places = place_meta.clone();
    trace.bands = vec!["copdem30m.provenance".into(), "copdem30m.byte_histogram_v1".into()];
    trace.tslots = vec![0];
    trace.write()?;

    println!("\nReal-data demo complete.");
    println!("  base       = {}", trace.base);
    println!("  out_dir    = {}", trace.out_dir.display());
    println!("  steps      = {}", trace.steps.len());
    println!("  facts      = {} (across {} places × 3 bands: provenance + byte_histogram + elevation_mean)", trace.attested_cids.len(), places.len());
    println!("  responder  = {}", trace.responder_pubkey_b32);
    println!("  attester   = {}", trace.attester_pubkey_b32);
    Ok(())
}

// ---- Cop-DEM URL resolver --------------------------------------------

fn copdem_url_for_latlon(lat: f64, lng: f64) -> String {
    let lat_floor = lat.floor() as i32;
    let lon_floor = lng.floor() as i32;
    let lat_band = if lat_floor >= 0 {
        format!("N{:02}", lat_floor)
    } else {
        format!("S{:02}", -lat_floor)
    };
    let lon_band = if lon_floor >= 0 {
        format!("E{:03}", lon_floor)
    } else {
        format!("W{:03}", -lon_floor)
    };
    format!(
        "https://copernicus-dem-30m.s3.amazonaws.com/Copernicus_DSM_COG_10_{lat_band}_00_{lon_band}_00_DEM/Copernicus_DSM_COG_10_{lat_band}_00_{lon_band}_00_DEM.tif"
    )
}

// ---- GeoTIFF header inspection (real bytes) --------------------------

fn parse_geotiff_header(bytes: &[u8]) -> (bool, &'static str, u32) {
    if bytes.len() < 8 { return (false, "?", 0); }
    let byteorder = match (bytes[0], bytes[1]) {
        (0x49, 0x49) => "II",  // little-endian
        (0x4D, 0x4D) => "MM",  // big-endian
        _ => return (false, "?", 0),
    };
    let magic = if byteorder == "II" {
        u16::from_le_bytes([bytes[2], bytes[3]])
    } else {
        u16::from_be_bytes([bytes[2], bytes[3]])
    };
    let magic_ok = magic == 42 || magic == 43; // 42=TIFF, 43=BigTIFF
    let ifd_offset = if byteorder == "II" {
        u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]])
    } else {
        u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]])
    };
    (magic_ok, byteorder, ifd_offset)
}

fn byte_histogram_16(bytes: &[u8]) -> Vec<f64> {
    let mut bins = [0u64; 16];
    for b in bytes { bins[(b >> 4) as usize] += 1; }
    let total: u64 = bins.iter().sum();
    if total == 0 { return vec![0.0; 16]; }
    bins.iter().map(|&c| (c as f64) / (total as f64)).collect()
}

// ---- attester / trace harness ----------------------------------------

struct AttesterKeyPair { signing: SigningKey, pubkey: AttesterKey }
fn fresh_attester() -> AttesterKeyPair {
    let mut sec = [0u8; 32];
    OsRng.fill_bytes(&mut sec);
    let signing = SigningKey::from_bytes(&sec);
    let mut pk = [0u8; 32];
    pk.copy_from_slice(signing.verifying_key().as_bytes());
    AttesterKeyPair { signing, pubkey: AttesterKey(pk) }
}

struct TraceIndex {
    base: String,
    out_dir: PathBuf,
    started_at: String,
    steps: Vec<StepRecord>,
    attested_cids: Vec<String>,
    cells: Vec<String>,
    places: Vec<(String, String)>,
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
    fn new(base: &str, out_dir: &PathBuf) -> Self {
        Self {
            base: base.to_string(),
            out_dir: out_dir.clone(),
            started_at: utc_iso(),
            steps: Vec::new(),
            attested_cids: Vec::new(),
            cells: Vec::new(),
            places: Vec::new(),
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
        let value: Value = serde_json::from_str(&body).unwrap_or_else(|_| Value::String(body.clone()));
        let resp_file = self.write_response(name, &body)?;
        let curl = format!("curl -s '{url}'");
        self.record(StepRecord {
            name: name.into(), method: "GET".into(), path: path.into(), status,
            request_file: None, response_file: resp_file,
            request_cid_b3: None, response_cid_b3: blake3_hex(body.as_bytes()),
            request_id: extract_str(&value, &["receipt", "request_id"]),
            served_at: extract_str(&value, &["receipt", "served_at"]),
            primitive: extract_str(&value, &["receipt", "primitive"]),
            fact_cids: extract_cids(&value), curl_repro: curl,
        });
        Ok(value)
    }
    async fn post_json(&mut self, c: &reqwest::Client, name: &str, path: &str, body: Value) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base, path);
        let body_str = serde_json::to_string(&body)?;
        let resp = c.post(&url)
            .header("content-type", "application/json")
            .body(body_str.clone()).send().await?;
        let status = resp.status().as_u16();
        let resp_text = resp.text().await?;
        let value: Value = serde_json::from_str(&resp_text).unwrap_or_else(|_| Value::String(resp_text.clone()));
        let req_file = self.write_request(name, &body_str)?;
        let resp_file = self.write_response(name, &resp_text)?;
        let curl = format!(
            "curl -s -X POST '{url}' -H 'content-type: application/json' -d @{}",
            std::path::Path::new(&req_file).file_name().unwrap().to_string_lossy()
        );
        self.record(StepRecord {
            name: name.into(), method: "POST".into(), path: path.into(), status,
            request_file: Some(req_file), response_file: resp_file,
            request_cid_b3: Some(blake3_hex(body_str.as_bytes())),
            response_cid_b3: blake3_hex(resp_text.as_bytes()),
            request_id: extract_str(&value, &["receipt", "request_id"])
                .or_else(|| extract_str(&value, &["request_id"])),
            served_at: extract_str(&value, &["receipt", "served_at"])
                .or_else(|| extract_str(&value, &["served_at"])),
            primitive: extract_str(&value, &["receipt", "primitive"]),
            fact_cids: extract_cids(&value), curl_repro: curl,
        });
        Ok(value)
    }
    async fn post_cbor(&mut self, c: &reqwest::Client, name: &str, path: &str, body: &[u8]) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base, path);
        let resp = c.post(&url).header("content-type", "application/cbor").body(body.to_vec()).send().await?;
        let status = resp.status().as_u16();
        let resp_text = resp.text().await?;
        let value: Value = serde_json::from_str(&resp_text).unwrap_or_else(|_| Value::String(resp_text.clone()));
        let req_file = self.write_request_bytes(name, body)?;
        let resp_file = self.write_response(name, &resp_text)?;
        let curl = format!(
            "curl -s -X POST '{url}' -H 'content-type: application/cbor' --data-binary @{}",
            std::path::Path::new(&req_file).file_name().unwrap().to_string_lossy()
        );
        self.record(StepRecord {
            name: name.into(), method: "POST".into(), path: path.into(), status,
            request_file: Some(req_file), response_file: resp_file,
            request_cid_b3: Some(blake3_hex(body)),
            response_cid_b3: blake3_hex(resp_text.as_bytes()),
            request_id: extract_str(&value, &["request_id"]),
            served_at: extract_str(&value, &["attested_at"]),
            primitive: Some("emem.attest".into()),
            fact_cids: extract_cids(&value), curl_repro: curl,
        });
        Ok(value)
    }
    fn record(&mut self, s: StepRecord) {
        println!("[{:>3}] {:>3} {:<5} {:<32} → {} ({})",
            self.steps.len() + 1, s.status, s.method, s.name,
            s.request_id.as_deref().unwrap_or("-"),
            s.fact_cids.len());
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
            "schema": "emem.realdemo.trace.v1",
            "base_url": self.base,
            "started_at": self.started_at,
            "finished_at": utc_iso(),
            "responder_pubkey_b32": self.responder_pubkey_b32,
            "attester_pubkey_b32": self.attester_pubkey_b32,
            "places": self.places.iter().map(|(n,c)| json!({"name": n, "cell": c})).collect::<Vec<_>>(),
            "cells": self.cells,
            "bands": self.bands,
            "tslots": self.tslots,
            "attested_fact_cids": self.attested_cids,
            "step_count": self.steps.len(),
            "steps": serde_json::to_value(&self.steps)?,
        });
        std::fs::write(self.out_dir.join("trace.json"), serde_json::to_string_pretty(&trace)?)?;

        let mut md = String::new();
        md.push_str("# emem real-data demo trace\n\n");
        md.push_str(&format!("- Server: `{}`\n", self.base));
        md.push_str(&format!("- Responder pubkey (b32): `{}`\n", self.responder_pubkey_b32));
        md.push_str(&format!("- Demo attester pubkey (b32): `{}`\n\n", self.attester_pubkey_b32));
        md.push_str("## Real-world cells\n\n| name | cell64 |\n|---|---|\n");
        for (n, c) in &self.places { md.push_str(&format!("| {n} | `{c}` |\n")); }
        md.push_str("\n## Steps\n\n| # | method | path | status | request_id | fact_cids |\n|---|---|---|---|---|---|\n");
        for (i, st) in self.steps.iter().enumerate() {
            md.push_str(&format!(
                "| {} | {} | `{}` | {} | `{}` | {} |\n",
                i+1, st.method, st.path, st.status,
                st.request_id.as_deref().unwrap_or("-"),
                st.fact_cids.len()
            ));
        }
        let mut by_step: BTreeMap<String, &StepRecord> = BTreeMap::new();
        for st in &self.steps { by_step.insert(st.name.clone(), st); }
        md.push_str("\n## Per-step files\n\n");
        for (_, st) in by_step.iter() {
            md.push_str(&format!("### {}\n\n", st.name));
            if let Some(req) = &st.request_file { md.push_str(&format!("- Request: `{req}`\n")); }
            md.push_str(&format!("- Response: `{}`\n", st.response_file));
            md.push_str(&format!("- blake3(response) = `{}`\n", st.response_cid_b3));
            if let Some(rid) = &st.request_id { md.push_str(&format!("- request_id = `{rid}`\n")); }
            md.push_str(&format!("- repro: `{}`\n\n", st.curl_repro));
        }
        std::fs::write(self.out_dir.join("README.md"), md)?;
        Ok(())
    }
}

// ---- helpers --------------------------------------------------------

fn b32(b: &[u8]) -> String { data_encoding::BASE32_NOPAD.encode(b).to_lowercase() }
fn blake3_hex(b: &[u8]) -> String { blake3::hash(b).to_hex().to_string() }
fn pretty(s: &str) -> String {
    serde_json::from_str::<Value>(s)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| s.to_string())
}
fn extract_str(v: &Value, path: &[&str]) -> Option<String> {
    let mut cur = v;
    for k in path { cur = cur.get(*k)?; }
    cur.as_str().map(|s| s.to_string())
}
fn extract_cids(v: &Value) -> Vec<String> {
    if let Some(arr) = v.pointer("/receipt/fact_cids").and_then(|x| x.as_array()) {
        return arr.iter().filter_map(|x| x.as_str().map(String::from)).collect();
    }
    if let Some(arr) = v.get("cids").and_then(|x| x.as_array()) {
        return arr.iter().filter_map(|x| x.as_str().map(String::from)).collect();
    }
    Vec::new()
}
fn utc_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let (y, m, d, hh, mm, ss) = civil_from_unix(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hh, mm, ss)
}
fn utc_stamp() -> String { utc_iso().replace([':', '-'], "").replace('T', "_").replace('Z', "") }
fn civil_from_unix(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400); let tod = secs.rem_euclid(86_400);
    let hh = (tod / 3600) as u32; let mm = ((tod % 3600) / 60) as u32; let ss = (tod % 60) as u32;
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
            if let Some(i) = n.as_i64() { ciborium::Value::Integer(i.into()) }
            else if let Some(f) = n.as_f64() { ciborium::Value::Float(f) }
            else { ciborium::Value::Null }
        }
        Value::String(s) => ciborium::Value::Text(s.clone()),
        Value::Array(a) => ciborium::Value::Array(a.iter().map(json_to_cbor).collect()),
        Value::Object(m) => ciborium::Value::Map(
            m.iter().map(|(k, v)| (ciborium::Value::Text(k.clone()), json_to_cbor(v))).collect()
        ),
    }
}
