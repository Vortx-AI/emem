//! emem-demo — end-to-end protocol round-trip against a live server.
//!
//! Generates an ephemeral ed25519 attester, builds 3 Primary facts at
//! adjacent cell64 codepoints, signs an Attestation, posts it to the
//! server's `/v1/attest`, then exercises every read primitive.

use std::collections::BTreeMap;

use blake3::Hasher;
use ed25519_dalek::{Signer, SigningKey};
use rand::RngCore;
use rand::rngs::OsRng;
use serde_json::{json, Value};

use emem_attest::merkle_root;
use emem_codec::to_cell64;
use emem_core::{AttesterKey, Cell, KeyEpoch, Signature};
use emem_fact::{
    Attestation, Derivation, Fact, FactCid, PrimaryFact, RegistryCid, SchemaCid, Source,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = std::env::args().nth(1).unwrap_or_else(|| "http://localhost:5051".into());
    let client = reqwest::Client::new();

    println!("== /health ==");
    let health: Value = client.get(format!("{base}/health")).send().await?.json().await?;
    let registry_cid = health["registry_cid"].as_str().unwrap().to_string();
    let schema_cid = health["schema_cid"].as_str().unwrap().to_string();
    println!("  ok={} version={}", health["ok"], health["version"]);

    let mut secret = [0u8; 32];
    OsRng.fill_bytes(&mut secret);
    let signing = SigningKey::from_bytes(&secret);
    let mut pubkey = [0u8; 32];
    pubkey.copy_from_slice(signing.verifying_key().as_bytes());
    let attester = AttesterKey(pubkey);
    println!("== attester ==");
    println!("  pubkey_b32={}", b32(&pubkey));

    let cells: Vec<String> = (0u64..3).map(|i| {
        let raw = (1u64 << 59) | (13u64 << 52) | ((i + 17) << 45) | (0x123456789abcu64 & 0xfffffffffff);
        to_cell64(Cell::from_raw(raw))
    }).collect();
    println!("== cells ==");
    for c in &cells { println!("  {c}"); }

    let facts: Vec<Fact> = cells.iter().enumerate().map(|(i, cell)| {
        let vec_val: Vec<Value> = (0..16).map(|d| {
            let v = (((i as i32) - 1) as f64) * 0.1 + (d as f64) * 0.01;
            Value::from(v)
        }).collect();
        let vec_cbor = json_to_cbor(&Value::Array(vec_val));
        Fact::Primary(PrimaryFact {
            cell: cell.clone(),
            band: "geotessera".into(),
            tslot: 0,
            value: vec_cbor,
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source { scheme: "demo".into(), id: format!("d-{i}"), cid: None, hash: None, captured_at: None, url: None }],
            derivation: Derivation { fn_key: "demo@1".into(), args: None },
            privacy_class: "public".into(),
            schema_cid: SchemaCid::new(&schema_cid),
            signer: attester,
            signed_at: "2026-04-26T14:00:00Z".into(),
        })
    }).collect();

    let mut leaves: Vec<[u8; 32]> = facts.iter().map(|f| {
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
        stake: None,
        signature: Signature(sig_bytes),
        attested_at: "2026-04-26T14:00:00Z".into(),
    };

    println!("== POST /v1/attest_cbor ==");
    let mut att_cbor = Vec::new();
    ciborium::ser::into_writer(&att, &mut att_cbor)?;
    let r = client.post(format!("{base}/v1/attest_cbor"))
        .header("content-type", "application/cbor")
        .body(att_cbor).send().await?;
    let body: Value = r.json().await?;
    println!("  → {}", serde_json::to_string(&body)?);
    let cids: Vec<String> = body["cids"].as_array().unwrap()
        .iter().map(|v| v.as_str().unwrap().to_string()).collect();

    println!("== GET /v1/cells/{} ==", &cells[1]);
    let r: Value = client.get(format!("{base}/v1/cells/{}", &cells[1]))
        .send().await?.json().await?;
    println!("  facts={} fact_cids_in_receipt={}",
        r["facts"].as_array().unwrap().len(),
        r["receipt"]["fact_cids"].as_array().unwrap().len());

    println!("== POST /v1/compare ==");
    let r: Value = client.post(format!("{base}/v1/compare"))
        .json(&json!({"a": &cells[0], "b": &cells[2]}))
        .send().await?.json().await?;
    println!("  cosine={} per_band={}", r["cosine"], serde_json::to_string(&r["per_band"])?);

    println!("== POST /v1/find_similar (cell key) ==");
    let r: Value = client.post(format!("{base}/v1/find_similar"))
        .json(&json!({"key": &cells[1], "k": 5}))
        .send().await?.json().await?;
    println!("  neighbors={}", serde_json::to_string(&r["neighbors"])?);

    println!("== POST /v1/find_similar (inline vec) ==");
    let r: Value = client.post(format!("{base}/v1/find_similar"))
        .json(&json!({"key": "inline:[0.0,0.01,0.02,0.03,0.04,0.05,0.06,0.07,0.08,0.09,0.10,0.11,0.12,0.13,0.14,0.15]", "k": 3}))
        .send().await?.json().await?;
    println!("  neighbors={}", serde_json::to_string(&r["neighbors"])?);

    println!("== POST /v1/intent (where_is) ==");
    let r: Value = client.post(format!("{base}/v1/intent"))
        .json(&json!({"type": "what_is_here", "cell": &cells[1]}))
        .send().await?.json().await?;
    println!("  plan={}", serde_json::to_string(&r)?);

    println!("== POST /mcp tools/call emem.recall ==");
    let r: Value = client.post(format!("{base}/mcp"))
        .json(&json!({
            "jsonrpc":"2.0", "id": 7, "method": "tools/call",
            "params": {
                "name": "emem.recall",
                "arguments": {"cell": &cells[0]},
            }
        }))
        .send().await?.json().await?;
    println!("  facts_in_result={}", r["result"]["facts"].as_array().unwrap().len());

    println!("== GET /v1/facts/{} ==", &cids[0]);
    let r: Value = client.get(format!("{base}/v1/facts/{}", &cids[0])).send().await?.json().await?;
    println!("  fact_kind={}", r["kind"]);

    println!("\nAll round-trips green.");
    Ok(())
}

fn b32(b: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(b).to_lowercase()
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
