//! Errata about the transparency log itself: signed by this responder once,
//! when an operator's log repair completes (`ops_log_repair.rs`), kept under
//! `$EMEM_DATA/log-errata/`, and served beside the head (`GET /v1/log/sth`)
//! and beside each leaf they name (`/v1/log/entries`, `/v1/log/inclusion`).
//!
//! An erratum names leaves the tree committed that are the hash of no entry,
//! the tears that caused them, and a relocation list (served at
//! `/v1/artifacts/<list_blake3_b32>`) that maps each entry the tree missed to
//! the leaf that now holds it. The signature is ed25519 over
//! `PreimageV1("emem.translog.erratum.v1"){1: blake3(JCS(body)),
//! 2: signed_at, 3: responder_pubkey}`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use ed25519_dalek::Signer;
use serde_json::{json, Value as JsonValue};

use crate::AppState;

pub(crate) const DOMAIN: &str = "emem.translog.erratum.v1";

/// Segment tags of the erratum preimage. Stable wire constants.
pub(crate) mod tag {
    pub const BODY_BLAKE3: u8 = 1;
    pub const SIGNED_AT: u8 = 2;
    pub const RESPONDER_PUBKEY: u8 = 3;
}

const RULE: &str = "PreimageV1(\"emem.translog.erratum.v1\"){1: blake3(JCS(body)), 2: signed_at, 3: responder_pubkey}";

/// Published errata and what they say about single leaves.
#[derive(Default)]
struct Loaded {
    docs: Vec<JsonValue>,
    /// Leaf index to the erratum note served beside that leaf.
    leaves: HashMap<u64, JsonValue>,
    /// Relocation list bytes by their blake3, base32.
    lists: HashMap<String, Arc<Vec<u8>>>,
}

static LOADED: RwLock<Option<Arc<Loaded>>> = RwLock::new(None);

fn errata_dir() -> PathBuf {
    emem_core::data_dir::data_dir().join("log-errata")
}

fn load(dir: &Path) -> Loaded {
    let mut out = Loaded::default();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut names: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && !p.to_string_lossy().ends_with(".relocations.json")
        })
        .collect();
    names.sort();
    for p in names {
        let Some(doc) = std::fs::read(&p)
            .ok()
            .and_then(|b| serde_json::from_slice::<JsonValue>(&b).ok())
        else {
            continue;
        };
        let id = doc
            .pointer("/body/id")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string();
        for m in doc
            .pointer("/body/misframed_leaves")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(i) = m.get("leaf_index").and_then(JsonValue::as_u64) {
                out.leaves.insert(
                    i,
                    json!({
                        "id": id,
                        "status": "misframed_leaf",
                        "tree_leaf_b32": m["tree_leaf_b32"],
                        "what": "The tree committed the 32 bytes in tree_leaf_b32 as this leaf. They are the hash of no entry: a torn length made the walk read across the next record, so the bytes served here do not hash to the leaf. The leaf stays, because every later head commits to it. Accept it as committed and skip the re-hash check here; the entries after the tear are relocated (see the erratum at /v1/log/sth).",
                    }),
                );
            }
        }
        if let Some(cid) = doc
            .pointer("/body/relocated/list_blake3_b32")
            .and_then(JsonValue::as_str)
        {
            let list_path = dir.join(format!("{id}.relocations.json"));
            if let Ok(bytes) = std::fs::read(&list_path) {
                if crate::b32_lower(blake3::hash(&bytes).as_bytes()) == cid {
                    out.lists.insert(cid.to_string(), Arc::new(bytes));
                }
            }
        }
        out.docs.push(doc);
    }
    out
}

fn loaded() -> Arc<Loaded> {
    if let Some(l) = LOADED.read().ok().and_then(|g| g.clone()) {
        return l;
    }
    let l = Arc::new(load(&errata_dir()));
    if let Ok(mut g) = LOADED.write() {
        *g = Some(Arc::clone(&l));
    }
    l
}

/// Every published erratum, signed, oldest id first.
pub(crate) fn all() -> Vec<JsonValue> {
    loaded().docs.clone()
}

/// The note served beside leaf `i`, when an erratum names it.
pub(crate) fn for_leaf(i: u64) -> Option<JsonValue> {
    loaded().leaves.get(&i).cloned()
}

/// A relocation list by its blake3, for `/v1/artifacts/<cid>`: the
/// artifact store may evict it, the erratum never does.
pub(crate) fn list_bytes(cid: &str) -> Option<Arc<Vec<u8>>> {
    loaded().lists.get(cid).cloned()
}

/// The 32 bytes the responder signs for an erratum body.
pub(crate) fn preimage(body: &JsonValue, signed_at: &str, pubkey: &[u8; 32]) -> [u8; 32] {
    let body_hash = blake3::hash(crate::jcs(body).as_bytes());
    let mut pb = emem_attest::PreimageV1::new(DOMAIN);
    pb.seg(tag::BODY_BLAKE3, body_hash.as_bytes());
    pb.seg(tag::SIGNED_AT, signed_at.as_bytes());
    pb.seg(tag::RESPONDER_PUBKEY, pubkey);
    pb.finalize()
}

/// Sign and publish the erratum of a completed log repair in `run_dir`.
/// Publishing the same body again keeps the first signature.
pub(crate) fn publish(s: &AppState, run_dir: &Path, id: &str) -> Result<JsonValue, String> {
    let doc = publish_into(s, &errata_dir(), run_dir, id)?;
    if let Ok(mut g) = LOADED.write() {
        *g = None;
    }
    Ok(doc)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

fn publish_into(
    s: &AppState,
    errata: &Path,
    run_dir: &Path,
    id: &str,
) -> Result<JsonValue, String> {
    let pk = s.identity.pubkey.0;
    let (body, list) = emem_storage::log_repair::erratum_body(run_dir, id, &crate::b32_lower(&pk))
        .map_err(|e| e.to_string())?;
    let body_b32 = crate::b32_lower(blake3::hash(crate::jcs(&body).as_bytes()).as_bytes());
    std::fs::create_dir_all(errata).map_err(|e| e.to_string())?;
    let doc_path = errata.join(format!("{id}.json"));
    if let Some(old) = std::fs::read(&doc_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<JsonValue>(&b).ok())
    {
        if old.get("body_blake3_b32").and_then(JsonValue::as_str) == Some(body_b32.as_str()) {
            return Ok(old);
        }
    }
    let list_cid = crate::b32_lower(blake3::hash(&list).as_bytes());
    write_atomic(&errata.join(format!("{id}.relocations.json")), &list)
        .map_err(|e| e.to_string())?;
    let signed_at = crate::chrono_iso8601_utc();
    let sig = s
        .identity
        .signing
        .sign(&preimage(&body, &signed_at, &pk))
        .to_bytes();
    let doc = json!({
        "body": body,
        "body_blake3_b32": body_b32,
        "signed_at": signed_at,
        "responder_pubkey_b32": crate::b32_lower(&pk),
        "signature_b32": crate::b32_lower(&sig),
        "preimage": RULE,
        "body_canonical": "RFC 8785 JSON canonicalisation of body",
        "relocations_url": format!("/v1/artifacts/{list_cid}"),
    });
    let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
    write_atomic(&doc_path, &bytes).map_err(|e| e.to_string())?;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run directory as a complete relog leaves it, with one misframed
    /// leaf, one tear and two relocated entries.
    fn run_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let list = br#"[{"segment":3,"offset":4096,"entry_hash":"aa","kind":"attestation","facts":2,"leaf_index":40,"relogged":true},{"segment":3,"offset":5000,"entry_hash":"bb","kind":"emem.memory_write.v1","facts":0,"leaf_index":41,"relogged":true}]
"#;
        std::fs::write(d.path().join("relocations.json"), list).unwrap();
        let cid = crate::b32_lower(blake3::hash(list).as_bytes());
        std::fs::write(
            d.path().join("report.json"),
            serde_json::to_vec(&json!({
                "tears": [{"segment": 3, "offset": 1000, "kind": "bad_hash", "declared_len": 5000,
                           "resume_offset": 4096, "leaf_index": 7, "tree": "misframed_leaf"}],
                "misframed_leaves": [{"leaf_index": 7, "segment": 3, "offset": 1000,
                                      "declared_len": 5000, "tree_leaf_b32": "cc"}],
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            d.path().join("relog.report.json"),
            serde_json::to_vec(&json!({"complete": true, "relocations": {"blake3_b32": cid}}))
                .unwrap(),
        )
        .unwrap();
        d
    }

    /// The published erratum verifies under the responder's key, names the
    /// misframed leaf, keeps its list, and a second publish keeps the first
    /// signature.
    #[test]
    fn a_published_erratum_verifies_and_names_its_leaves() {
        use ed25519_dalek::Verifier;
        let s = crate::tests::test_app_state();
        let run = run_dir();
        let errata = tempfile::tempdir().unwrap();
        let doc = publish_into(&s, errata.path(), run.path(), "log-repair-t").unwrap();
        let body = &doc["body"];
        assert_eq!(body["misframed_leaves"][0]["leaf_index"], 7);
        assert_eq!(body["relocated"]["entries"], 2);
        assert_eq!(body["relocated"]["memory_entries"], 1);
        let signed_at = doc["signed_at"].as_str().unwrap();
        let digest = preimage(body, signed_at, &s.identity.pubkey.0);
        let sig_bytes: [u8; 64] = data_encoding::BASE32_NOPAD
            .decode(
                doc["signature_b32"]
                    .as_str()
                    .unwrap()
                    .to_uppercase()
                    .as_bytes(),
            )
            .unwrap()
            .try_into()
            .unwrap();
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&s.identity.pubkey.0).unwrap();
        assert!(vk
            .verify(&digest, &ed25519_dalek::Signature::from_bytes(&sig_bytes))
            .is_ok());

        let l = load(errata.path());
        assert_eq!(l.docs.len(), 1);
        assert_eq!(l.leaves[&7]["status"], "misframed_leaf");
        let cid = body["relocated"]["list_blake3_b32"].as_str().unwrap();
        assert!(l.lists.contains_key(cid));
        assert!(doc["relocations_url"].as_str().unwrap().ends_with(cid));

        let again = publish_into(&s, errata.path(), run.path(), "log-repair-t").unwrap();
        assert_eq!(again["signature_b32"], doc["signature_b32"]);
    }

    /// No erratum is published for a relog that did not finish.
    #[test]
    fn an_incomplete_relog_publishes_nothing() {
        let s = crate::tests::test_app_state();
        let run = run_dir();
        std::fs::write(
            run.path().join("relog.report.json"),
            br#"{"complete": false}"#,
        )
        .unwrap();
        let errata = tempfile::tempdir().unwrap();
        assert!(publish_into(&s, errata.path(), run.path(), "x").is_err());
        assert!(load(errata.path()).docs.is_empty());
    }
}
