//! Phase-0 latency baseline for the read hot path.
//!
//! There was no committed perf harness in this repo — every latency
//! figure in the docs (`~180ms cold`, `~10ms warm`, brute-force vs ANN)
//! was prose. This bench makes the two numbers that gate the embedding
//! upgrade measurable and repeatable:
//!
//!   1. **vector-op inner loop** — `cosine` / `cosine_finite` / the CBOR
//!      `as_vec_f32` decode, at the four dims the cube actually carries
//!      (128 Tessera, 768 Galileo, 1024 Clay/Prithvi, 1152 Tessera
//!      multi-year). These are the functions Phase 2 SIMD-ifies; the
//!      `decode_then_score` group isolates the per-candidate CBOR
//!      coercion cost that dominates the brute-force scan.
//!   2. **find_similar scaling** — the brute-force k-NN over an N-cell
//!      corpus (no Lance index installed → the O(N) `iter_index` path).
//!      This is the curve the ANN fast-path and SIMD work must beat.
//!
//! Run: `cargo bench -p emem-primitives --bench hot_paths`
//!
//! The in-memory `MemStorage` mirrors the one in
//! `tests/lance_find_similar.rs` — the same six required `Storage`
//! methods `find_similar`'s brute-force path actually touches.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ciborium::Value as CborValue;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use emem_cache::CanonicalKey;
use emem_core::AttesterKey;
use emem_fact::{Derivation, Fact, FactCid, PrimaryFact, RegistryCid, SchemaCid, Source};
use emem_primitives::cbor_ops::{as_vec_f32, cosine, cosine_finite};
use emem_primitives::find_similar::{
    find_similar, set_lance_index, FindSimilarMode, FindSimilarReq,
};
use emem_primitives::LanceIndex;
use emem_storage::server::{ManifestCids, ResponderIdentity};
use emem_storage::{Server, Storage, StorageError};
use tempfile::TempDir;

// ── in-memory storage harness ──────────────────────────────────────

struct MemStorage {
    entries: Mutex<Vec<(CanonicalKey, FactCid)>>,
    facts: Mutex<HashMap<String, Fact>>,
}

impl MemStorage {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            facts: Mutex::new(HashMap::new()),
        }
    }

    fn insert_vector(&self, cell: &str, band: &str, tslot: u64, vec: Vec<f32>) -> FactCid {
        let cid_str = format!("bench-cid-{cell}-{band}-{tslot}");
        let cid = FactCid::new(&cid_str);
        let fact = Fact::Primary(PrimaryFact {
            cell: cell.into(),
            band: band.into(),
            tslot,
            value: CborValue::Array(
                vec.into_iter()
                    .map(|v| CborValue::Float(v as f64))
                    .collect(),
            ),
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source {
                scheme: "bench".into(),
                id: cid_str.clone(),
                cid: None,
                hash: None,
                captured_at: None,
                url: None,
            }],
            derivation: Derivation {
                fn_key: "bench@1".into(),
                args: None,
            },
            privacy_class: "public".into(),
            schema_cid: SchemaCid::new("bench-schema"),
            signer: AttesterKey([0u8; 32]),
            signed_at: "2026-06-23T00:00:00Z".into(),
            served_via: None,
        });
        self.entries.lock().unwrap().push((
            CanonicalKey {
                cell: cell.into(),
                band: band.into(),
                tslot,
            },
            cid.clone(),
        ));
        self.facts.lock().unwrap().insert(cid_str, fact);
        cid
    }
}

#[async_trait]
impl Storage for MemStorage {
    async fn lookup_canonical_many(
        &self,
        _keys: &[CanonicalKey],
    ) -> Result<Vec<Option<FactCid>>, StorageError> {
        unimplemented!("not used by the brute-force find_similar path")
    }

    async fn get_facts_many(&self, cids: &[FactCid]) -> Result<Vec<Option<Fact>>, StorageError> {
        let map = self.facts.lock().unwrap();
        Ok(cids.iter().map(|c| map.get(c.as_str()).cloned()).collect())
    }

    async fn put_attestation(
        &self,
        _att: &emem_fact::Attestation,
    ) -> Result<Vec<FactCid>, StorageError> {
        unimplemented!("not used")
    }

    async fn materialize_many(&self, _keys: &[CanonicalKey]) -> Result<Vec<FactCid>, StorageError> {
        unimplemented!("not used")
    }

    async fn scan_cell(
        &self,
        cell: &str,
        _tslot: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        let entries = self.entries.lock().unwrap();
        Ok(entries
            .iter()
            .filter(|(k, _)| k.cell == cell)
            .cloned()
            .collect())
    }

    async fn iter_index(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        let entries = self.entries.lock().unwrap();
        let mut out: Vec<_> = entries.clone();
        if let Some(n) = limit {
            out.truncate(n);
        }
        Ok(out)
    }
}

fn bench_server(storage: Arc<MemStorage>) -> Server {
    Server {
        storage,
        identity: ResponderIdentity::fresh(),
        manifests: ManifestCids {
            registry_cid: RegistryCid::new("bench-registry"),
            schema_cid: SchemaCid::new("bench-schema"),
            bands_cid: "bench-bands".into(),
            sources_cid: "bench-sources".into(),
        },
        started_at_unix_s: 0,
    }
}

fn unit_vec(rng: &mut StdRng, dim: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..dim).map(|_| rng.gen::<f32>() * 2.0 - 1.0).collect();
    let mag: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag > 0.0 {
        for x in v.iter_mut() {
            *x /= mag;
        }
    }
    v
}

// ── 1. vector-op inner loop ────────────────────────────────────────

const DIMS: &[(&str, usize)] = &[
    ("128_tessera", 128),
    ("768_galileo", 768),
    ("1024_clay_prithvi", 1024),
    ("1152_tessera_multiyear", 1152),
];

fn bench_vector_ops(c: &mut Criterion) {
    let mut rng = StdRng::seed_from_u64(20_260_623);

    let mut g = c.benchmark_group("cosine");
    for &(label, dim) in DIMS {
        let a = unit_vec(&mut rng, dim);
        let b = unit_vec(&mut rng, dim);
        g.throughput(Throughput::Elements(dim as u64));
        g.bench_with_input(BenchmarkId::from_parameter(label), &(a, b), |bn, (a, b)| {
            bn.iter(|| cosine(black_box(a), black_box(b)))
        });
    }
    g.finish();

    // cosine_finite over a multi-year-shaped vector where one 128-d
    // vintage block is NaN (the masked-404-year case it exists to handle).
    let mut g = c.benchmark_group("cosine_finite");
    for &(label, dim) in DIMS {
        let mut a = unit_vec(&mut rng, dim);
        let b = unit_vec(&mut rng, dim);
        for x in a.iter_mut().take(dim.min(128)) {
            *x = f32::NAN;
        }
        g.throughput(Throughput::Elements(dim as u64));
        g.bench_with_input(BenchmarkId::from_parameter(label), &(a, b), |bn, (a, b)| {
            bn.iter(|| cosine_finite(black_box(a), black_box(b)))
        });
    }
    g.finish();

    // The cost the brute-force scan pays per candidate: decode a CBOR
    // Array → Vec<f32>, then score. `as_vec_f32` allocates + coerces
    // every element; this is what fp16-resident vectors / SIMD remove.
    let mut g = c.benchmark_group("decode_then_score");
    for &(label, dim) in DIMS {
        let q = unit_vec(&mut rng, dim);
        let cand = unit_vec(&mut rng, dim);
        let cbor = CborValue::Array(cand.iter().map(|v| CborValue::Float(*v as f64)).collect());
        g.throughput(Throughput::Elements(dim as u64));
        g.bench_with_input(
            BenchmarkId::from_parameter(label),
            &(cbor, q),
            |bn, (cbor, q)| {
                bn.iter(|| {
                    let v = as_vec_f32(black_box(cbor)).unwrap();
                    cosine(&v, black_box(q))
                })
            },
        );
    }
    g.finish();
}

// ── 2. find_similar brute-force scaling ────────────────────────────

fn seed_corpus(n: usize, dim: usize) -> (Arc<MemStorage>, String) {
    let storage = Arc::new(MemStorage::new());
    let mut rng = StdRng::seed_from_u64(0xE_E_E_E);
    let query_cell = "cell-query";
    storage.insert_vector(query_cell, "geotessera", 0, unit_vec(&mut rng, dim));
    for i in 0..n {
        storage.insert_vector(&format!("c{i}"), "geotessera", 0, unit_vec(&mut rng, dim));
    }
    (storage, query_cell.to_string())
}

fn bench_find_similar_scaling(c: &mut Criterion) {
    // No Lance index installed in this process → find_similar takes the
    // brute-force iter_index path. (Belt-and-braces: also force it off.)
    std::env::set_var("EMEM_DISABLE_LANCE", "1");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let dim = 128; // Tessera, the primary embedding
    let mut g = c.benchmark_group("find_similar_brute_force_cosine_128d");
    g.sample_size(20); // the 100k point is slow; keep wall-clock sane
    for &n in &[1_000usize, 10_000, 100_000] {
        let (storage, query_cell) = seed_corpus(n, dim);
        let srv = bench_server(storage);
        let req = FindSimilarReq {
            key: query_cell,
            k: Some(10),
            band: Some("geotessera".into()),
            mode: FindSimilarMode::Cosine,
            ..Default::default()
        };
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &req, |bn, req| {
            bn.iter(|| {
                let resp = rt.block_on(find_similar(black_box(req), &srv)).unwrap();
                black_box(resp.neighbors.len())
            })
        });
    }
    g.finish();
}

// ── 3. find_similar ANN candidate pool + exact rerank (filtered) ───
//
// The Phase-2 win: a filtered/bounded query that used to take the O(N)
// brute scan in group 2 now retrieves a bounded ANN candidate pool from
// the Lance index and exact-reranks the survivors. Benched at the same
// 100k corpus so the number reads directly against
// `find_similar_brute_force_cosine_128d/100000`.

fn bench_find_similar_ann_rerank(c: &mut Criterion) {
    // This path uses the installed Lance index, so clear the brute-force
    // group's kill-switch (set earlier in the run and never unset).
    std::env::remove_var("EMEM_DISABLE_LANCE");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    let dim = 128;
    let n = 100_000;
    let (storage, query_cell) = seed_corpus(n, dim);

    // Hydrate + install a Lance index over the corpus. One-time setup,
    // OUTSIDE the timed loop. `set_lance_index` is write-once per process;
    // this bench is the only installer (the brute-force group forces the
    // index off via EMEM_DISABLE_LANCE).
    let tmp = TempDir::new().unwrap();
    let idx = Arc::new(LanceIndex::open(tmp.path()).unwrap());
    let storage_dyn: Arc<dyn Storage + Send + Sync> = storage.clone();
    rt.block_on(idx.hydrate_from_storage(storage_dyn.as_ref()))
        .unwrap();
    set_lance_index(idx);

    let srv = bench_server(storage);
    // The as_of bound routes through the filtered ANN + exact-rerank path;
    // every seeded fact is at tslot 0, so the bound (tslot <= 1) keeps all.
    let req = FindSimilarReq {
        key: query_cell,
        k: Some(10),
        band: Some("geotessera".into()),
        as_of_tslot: Some(1),
        mode: FindSimilarMode::Cosine,
        ..Default::default()
    };
    // Guard: confirm we measure the ANN-rerank path, not a silent
    // brute-force fallback (which would make this number meaningless).
    let probe = rt.block_on(find_similar(&req, &srv)).unwrap();
    assert!(
        probe.ranking_method.is_some(),
        "bench must exercise the ANN-rerank path (got brute-force fallback)"
    );

    let mut g = c.benchmark_group("find_similar_ann_rerank_cosine_128d");
    g.sample_size(20);
    g.throughput(Throughput::Elements(n as u64));
    g.bench_with_input(BenchmarkId::from_parameter(n), &req, |bn, req| {
        bn.iter(|| {
            let resp = rt.block_on(find_similar(black_box(req), &srv)).unwrap();
            black_box(resp.neighbors.len())
        })
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_vector_ops,
    bench_find_similar_scaling,
    bench_find_similar_ann_rerank
);
criterion_main!(benches);
