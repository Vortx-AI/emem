//! emem-connect-demo — runs the v0.0.9 "connect & evolve" loop end-to-end
//! against a live responder, using ONLY real signed bytes.
//!
//! Mirror of `examples/connect-and-evolve.md`. Nothing here is faked: two
//! ephemeral ed25519 attesters sign *disagreeing* scalar facts at the same
//! `(cell, band, tslot)`, a signed `EdgeFact` is folded into a signed
//! Attestation and POSTed to `/v1/edges`, bi-temporal supersession is
//! demonstrated against `/v1/edges/recall`, the scored disagreement is read
//! from `/v1/memory_contradictions`, the optional refinement loop's
//! auto-emitted `disagrees_with` edge is polled, the cell is recalled with
//! `include:["edges"]`, and a returned receipt is verified offline.
//!
//! Signs through `Attestation::build_and_sign_v1`, the one canonical
//! signing path, which folds the edge digests into the merkle leaf set,
//! so the responder accepts every envelope.
//!
//! Steps 1-7 each print a clear header. Any HTTP error prints status + body
//! and exits non-zero, so this is a real test rather than a silent pass.

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rand::RngCore;
use serde_json::{json, Value};

use emem_codec::{cell_from_latlng, to_cell64};
use emem_core::{AttesterKey, KeyEpoch};
use emem_fact::{
    Attestation, Derivation, EdgeFact, Fact, FactCid, PrimaryFact, RegistryCid, SchemaCid, Source,
};

/// Band the two attesters disagree over.
const BAND: &str = "indices.ndvi";
/// Shared time slot (Unix seconds) for both disagreeing facts.
const TSLOT: u64 = 1_704_067_200;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let base = resolve_base_url();
    let client = reqwest::Client::new();

    // Pull the live registry/schema CIDs so the signature preimage matches
    // what the responder recomputes.
    let health: Value = get_json(&client, &format!("{base}/health")).await?;
    let registry_cid = health["registry_cid"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("/health missing registry_cid"))?
        .to_string();
    let schema_cid = health["schema_cid"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("/health missing schema_cid"))?
        .to_string();
    eprintln!(
        "responder ok={} version={} registry_cid={} schema_cid={}",
        health["ok"], health["version"], registry_cid, schema_cid
    );

    // A real cell64 from a real lat/lng (somewhere in the Amazon).
    let cell = to_cell64(cell_from_latlng(-3.4653, -62.2159));

    // ---------------------------------------------------------------
    // STEP 1 — mint two ephemeral attesters A and B.
    // ---------------------------------------------------------------
    println!("\n=== STEP 1: mint two ephemeral attesters ===");
    let a = Attester::new();
    let b = Attester::new();
    println!("  attester A pubkey_b32 = {}", b32(&a.pubkey));
    println!("  attester B pubkey_b32 = {}", b32(&b.pubkey));

    // ---------------------------------------------------------------
    // STEP 2 — attest two DISAGREEING scalar facts at the SAME
    //          (cell, band, tslot): A says 0.61, B says 0.19.
    // ---------------------------------------------------------------
    println!("\n=== STEP 2: attest two disagreeing facts at the same (cell, band, tslot) ===");
    println!("  cell={cell} band={BAND} tslot={TSLOT}");
    let a_fact = scalar_fact(&cell, &a, 0.61, &schema_cid);
    let b_fact = scalar_fact(&cell, &b, 0.19, &schema_cid);

    let a_att = sign_attestation(vec![a_fact], vec![], &a, &registry_cid, &schema_cid);
    let a_cids = post_attest(&client, &base, &a_att).await?;
    let a_fact_cid = a_cids
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("attester A returned no fact_cid"))?;
    println!("  A wrote value 0.61 → fact_cid = {a_fact_cid}");

    let b_att = sign_attestation(vec![b_fact], vec![], &b, &registry_cid, &schema_cid);
    let b_cids = post_attest(&client, &base, &b_att).await?;
    let b_fact_cid = b_cids
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("attester B returned no fact_cid"))?;
    println!("  B wrote value 0.19 → fact_cid = {b_fact_cid}");

    // ---------------------------------------------------------------
    // STEP 3 — author two signed edges (same subj,pred,obj, different
    //          valid_from) and demonstrate bi-temporal supersession.
    // ---------------------------------------------------------------
    println!("\n=== STEP 3: signed edges + bi-temporal supersession ===");
    // Edge #1: relates_to, valid_from = 10. Signed by A.
    let edge_vf10 = mk_edge(&a_fact_cid, "relates_to", &b_fact_cid, 10, &a);
    let edge1_att = sign_attestation(
        vec![],
        vec![edge_vf10.clone()],
        &a,
        &registry_cid,
        &schema_cid,
    );
    let r1 = post_edges(&client, &base, &edge1_att).await?;
    println!(
        "  POST /v1/edges (valid_from=10) → edge_cids={}",
        serde_json::to_string(&r1["edge_cids"])?
    );

    // Edge #2: same (subj,pred,obj), valid_from = 20. Signed by A.
    let edge_vf20 = mk_edge(&a_fact_cid, "relates_to", &b_fact_cid, 20, &a);
    let edge2_att = sign_attestation(
        vec![],
        vec![edge_vf20.clone()],
        &a,
        &registry_cid,
        &schema_cid,
    );
    let r2 = post_edges(&client, &base, &edge2_att).await?;
    println!(
        "  POST /v1/edges (valid_from=20) → edge_cids={}",
        serde_json::to_string(&r2["edge_cids"])?
    );

    // as_of_tslot = 15 → expect the valid_from=10 edge.
    let recall15 = post_json(
        &client,
        &format!("{base}/v1/edges/recall"),
        &json!({"subj": &a_fact_cid, "pred": "relates_to", "as_of_tslot": 15}),
    )
    .await?;
    let vf_at_15 = first_edge_valid_from(&recall15);
    println!(
        "  /v1/edges/recall as_of=15 → {} edge(s), newest valid_from={:?} (expect 10)",
        recall15["edges"].as_array().map(|a| a.len()).unwrap_or(0),
        vf_at_15
    );
    anyhow::ensure!(
        vf_at_15 == Some(10),
        "supersession FAILED: as_of=15 should yield valid_from=10, got {vf_at_15:?}"
    );

    // as_of_tslot = 25 → expect the valid_from=20 edge.
    let recall25 = post_json(
        &client,
        &format!("{base}/v1/edges/recall"),
        &json!({"subj": &a_fact_cid, "pred": "relates_to", "as_of_tslot": 25}),
    )
    .await?;
    let vf_at_25 = first_edge_valid_from(&recall25);
    println!(
        "  /v1/edges/recall as_of=25 → {} edge(s), newest valid_from={:?} (expect 20)",
        recall25["edges"].as_array().map(|a| a.len()).unwrap_or(0),
        vf_at_25
    );
    anyhow::ensure!(
        vf_at_25 == Some(20),
        "supersession FAILED: as_of=25 should yield valid_from=20, got {vf_at_25:?}"
    );
    println!("  bi-temporal supersession CONFIRMED (15→vf10, 25→vf20)");

    // ---------------------------------------------------------------
    // STEP 4 — read the scored disagreement.
    // ---------------------------------------------------------------
    println!("\n=== STEP 4: scored disagreement via /v1/memory_contradictions ===");
    let cell_prefix: String = cell.chars().take(4).collect();
    let contradictions = post_json(
        &client,
        &format!("{base}/v1/memory_contradictions"),
        &json!({"cell_prefix": cell_prefix, "band": BAND, "min_severity": 0.1}),
    )
    .await?;
    let c0 = contradictions["contradictions"]
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no contradiction surfaced (body={})",
                serde_json::to_string(&contradictions).unwrap_or_default()
            )
        })?;
    println!(
        "  contradiction: cell={} band={} tslot={} severity={} kind={}",
        c0["cell"], c0["band"], c0["tslot"], c0["severity"], c0["kind"]
    );
    if let Some(atts) = c0["attestations"].as_array() {
        for at in atts {
            println!(
                "    attester={} value={} fact_cid={}",
                at["attester_pubkey_b32"], at["value"], at["fact_cid"]
            );
        }
    }
    anyhow::ensure!(
        c0["severity"].as_f64().unwrap_or(0.0) > 0.0,
        "contradiction severity should be > 0"
    );
    println!("  scored disagreement CONFIRMED");

    // ---------------------------------------------------------------
    // STEP 5 — poll for the refinement loop's auto disagrees_with edge.
    // ---------------------------------------------------------------
    println!("\n=== STEP 5: refinement-loop auto disagrees_with edge (opt-in) ===");
    println!(
        "  note: this step only emits if the responder was started with \
         EMEM_REFINEMENT_ENABLED=1 EMEM_REFINEMENT_INTERVAL_SECS=3 \
         EMEM_REFINEMENT_MIN_SEVERITY=0.1"
    );
    let mut found = false;
    for attempt in 0..8 {
        let dw = post_json(
            &client,
            &format!("{base}/v1/edges/recall"),
            &json!({"subj": &a_fact_cid, "pred": "disagrees_with"}),
        )
        .await?;
        if let Some(edges) = dw["edges"].as_array() {
            if let Some(e0) = edges.first() {
                println!(
                    "  auto disagrees_with edge APPEARED (attempt {}): subj={} obj={} valid_from={} confidence={}",
                    attempt + 1,
                    e0["subj"],
                    e0["obj"],
                    e0["valid_from"],
                    e0["confidence"]
                );
                found = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
    }
    if !found {
        println!(
            "  no auto disagrees_with edge within ~16s — either the refinement \
             loop is disabled or it hasn't run yet. Re-run the responder with the \
             EMEM_REFINEMENT_* env flags above to see this step fire."
        );
    }

    // ---------------------------------------------------------------
    // STEP 6 — recall the cell with include:["edges"].
    // ---------------------------------------------------------------
    println!("\n=== STEP 6: recall cell with include:[\"edges\"] ===");
    let recall = post_json(
        &client,
        &format!("{base}/v1/recall"),
        &json!({"cell": &cell, "bands": [BAND], "tslot": TSLOT, "include": ["edges"]}),
    )
    .await?;
    let n_facts = recall["facts"].as_array().map(|a| a.len()).unwrap_or(0);
    let edges = recall["edges"].as_array().cloned().unwrap_or_default();
    println!(
        "  recalled {n_facts} fact(s); attached edges array present = {}",
        recall.get("edges").is_some()
    );
    for e in &edges {
        println!(
            "    edge: subj={} pred={} obj={} valid_from={}",
            e["subj"], e["pred"], e["obj"], e["valid_from"]
        );
    }
    let recall_receipt = recall["receipt"].clone();

    // ---------------------------------------------------------------
    // STEP 7 — verify a returned receipt offline.
    // ---------------------------------------------------------------
    println!("\n=== STEP 7: verify a returned receipt via /v1/verify_receipt ===");
    let receipt = if recall_receipt.is_object() {
        recall_receipt
    } else {
        // Fall back to the edges/recall receipt if recall didn't carry one.
        recall25["receipt"].clone()
    };
    anyhow::ensure!(receipt.is_object(), "no signed receipt available to verify");
    let verified = post_json(
        &client,
        &format!("{base}/v1/verify_receipt"),
        &json!({"receipt": receipt}),
    )
    .await?;
    println!(
        "  valid={} merkle_proof_valid={} reason={}",
        verified["valid"],
        verified["merkle_proof_valid"],
        verified.get("reason").unwrap_or(&Value::Null)
    );
    anyhow::ensure!(
        verified["valid"].as_bool() == Some(true),
        "verify_receipt returned valid != true: {}",
        serde_json::to_string(&verified)?
    );
    println!("  receipt signature CONFIRMED valid");

    println!("\nAll connect-and-evolve steps green.");
    Ok(())
}

/// An ephemeral ed25519 attester (signing key + 32-byte pubkey).
struct Attester {
    signing: SigningKey,
    pubkey: [u8; 32],
}

impl Attester {
    fn new() -> Self {
        let mut secret = [0u8; 32];
        OsRng.fill_bytes(&mut secret);
        let signing = SigningKey::from_bytes(&secret);
        let mut pubkey = [0u8; 32];
        pubkey.copy_from_slice(signing.verifying_key().as_bytes());
        Self { signing, pubkey }
    }

    fn key(&self) -> AttesterKey {
        AttesterKey(self.pubkey)
    }
}

/// Build a signed single-scalar `PrimaryFact` at `(cell, BAND, TSLOT)`.
fn scalar_fact(cell: &str, att: &Attester, value: f64, schema_cid: &str) -> Fact {
    Fact::Primary(PrimaryFact {
        cell: cell.to_string(),
        band: BAND.into(),
        tslot: TSLOT,
        value: ciborium::Value::Float(value),
        unit: None,
        confidence: 0.9,
        uncertainty: None,
        sources: vec![Source {
            scheme: "demo".into(),
            id: format!("connect-demo-{value}"),
            cid: None,
            hash: None,
            captured_at: None,
            url: None,
        }],
        derivation: Derivation {
            fn_key: "demo@1".into(),
            args: None,
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(schema_cid),
        signer: att.key(),
        signed_at: "2026-05-29T00:00:00Z".into(),
        served_via: None,
    })
}

/// Build an `EdgeFact` `subj --pred--> obj` valid from `valid_from`, signed
/// by `att` (the edge's `signer` field — the merkle/ed25519 binding happens
/// at the Attestation envelope).
fn mk_edge(subj: &str, pred: &str, obj: &str, valid_from: u64, att: &Attester) -> EdgeFact {
    EdgeFact {
        subj: FactCid::new(subj),
        pred: pred.into(),
        obj: FactCid::new(obj),
        valid_from,
        valid_to: None,
        confidence: 1.0,
        signer: att.key(),
        signed_at: "2026-05-29T00:00:00Z".into(),
        schema_cid: None,
        note: None,
    }
}

/// Construct a fully-signed Attestation over `facts` + `edges` through the
/// one canonical signing path, `Attestation::build_and_sign_v1`: fact leaves
/// are `blake3(canonical_cbor(fact))`, edge leaves are `edge.blake3_digest()`,
/// all pushed together then SORTED and merkled under the v1 rules, and the
/// root signed over the tagged `attestation_preimage_v1` stream.
fn sign_attestation(
    facts: Vec<Fact>,
    edges: Vec<EdgeFact>,
    att: &Attester,
    registry_cid: &str,
    schema_cid: &str,
) -> Attestation {
    Attestation::build_and_sign_v1(
        facts,
        edges,
        RegistryCid::new(registry_cid),
        SchemaCid::new(schema_cid),
        &att.signing,
        KeyEpoch(0),
        "2026-05-29T00:00:00Z".into(),
        None,
    )
    .expect("attestation build")
}

/// POST a signed Attestation to `/v1/attest_cbor`; return its `cids`.
async fn post_attest(
    client: &reqwest::Client,
    base: &str,
    att: &Attestation,
) -> anyhow::Result<Vec<String>> {
    let mut cbor = Vec::new();
    ciborium::ser::into_writer(att, &mut cbor)?;
    let resp = client
        .post(format!("{base}/v1/attest_cbor"))
        .header("content-type", "application/cbor")
        .body(cbor)
        .send()
        .await?;
    let body = check_status(resp).await?;
    let cids = body["cids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(cids)
}

/// POST a signed Attestation (carrying edges) to `/v1/edges`.
async fn post_edges(
    client: &reqwest::Client,
    base: &str,
    att: &Attestation,
) -> anyhow::Result<Value> {
    let resp = client
        .post(format!("{base}/v1/edges"))
        .json(att)
        .send()
        .await?;
    check_status(resp).await
}

/// POST arbitrary JSON; return the parsed body (non-2xx → error).
async fn post_json(client: &reqwest::Client, url: &str, body: &Value) -> anyhow::Result<Value> {
    let resp = client.post(url).json(body).send().await?;
    check_status(resp).await
}

/// GET JSON; non-2xx → error.
async fn get_json(client: &reqwest::Client, url: &str) -> anyhow::Result<Value> {
    let resp = client.get(url).send().await?;
    check_status(resp).await
}

/// Turn a non-2xx response into a hard error printing status + body, so a
/// failed step is a real (non-zero-exit) test failure rather than a silent
/// pass.
async fn check_status(resp: reqwest::Response) -> anyhow::Result<Value> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("HTTP {status}: {text}");
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Pull the `valid_from` of the first edge in an `/v1/edges/recall` body.
fn first_edge_valid_from(body: &Value) -> Option<u64> {
    body["edges"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|e| e["valid_from"].as_u64())
}

fn b32(b: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(b).to_lowercase()
}

/// Resolve the responder base URL: positional CLI arg → `EMEM_BASE_URL` →
/// `http://127.0.0.1:5051` (the emem-server default bind).
fn resolve_base_url() -> String {
    const DEFAULT_BASE: &str = "http://127.0.0.1:5051";
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
