//! Fact proofs derived from the transparency log instead of stored per fact.
//!
//! Every attestation is already in the log, whole, and a fact's inclusion
//! proof is a pure function of its batch: the sorted fact leaves, the
//! hashing rule, and the log entry. Storing a ~1 KB proof per fact under a
//! random key was the second largest table in `facts.redb` and half of the
//! random page writes of a cold plot (see docs/plans/storage-v2.md).
//!
//! With `EMEM_DERIVED_PROOFS` set, each appended attestation writes two
//! small things instead:
//!
//! - one locator per fact, keyed by its 32-byte digest: the log segment,
//!   the record's offset in it, the fact's byte span inside the record's
//!   CBOR, and its leaf index in the sorted batch;
//! - one batch header and the sorted leaves in chunks of [`CHUNK_LEAVES`],
//!   keyed by log position, which appends at the end of the table.
//!
//! A proof is then rebuilt from one chunk and the header: O(chunk) hashes
//! whatever the batch size. The spans are checked at write time
//! (`blake3(span) == cid`), so the next phase can serve fact bodies from
//! the log by the same locator.
//!
//! Modes: `off` (default) writes and reads nothing new; `shadow` writes the
//! locators beside the stored proofs and compares the two on every proof
//! read; `on` stops writing per-fact proof rows.

use emem_cache::{KvTable, RedbFacts};
use emem_fact::{FactCid, MerkleProof};

/// `EMEM_DERIVED_PROOFS`: `off` (default), `shadow` or `on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DerivedProofs {
    Off,
    Shadow,
    On,
}

impl DerivedProofs {
    pub(crate) fn from_env() -> Self {
        let raw = std::env::var("EMEM_DERIVED_PROOFS").ok();
        let mode = Self::parse(raw.as_deref());
        if mode == DerivedProofs::Off {
            if let Some(v) = raw.as_deref().map(str::trim).filter(|v| {
                !v.is_empty() && !matches!(v.to_ascii_lowercase().as_str(), "off" | "0" | "false")
            }) {
                tracing::warn!(
                    target: "emem::storage",
                    value = v,
                    "EMEM_DERIVED_PROOFS is not off, shadow or on; treated as off"
                );
            }
        }
        mode
    }

    fn parse(v: Option<&str>) -> Self {
        match v.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("shadow") => DerivedProofs::Shadow,
            Some("on") | Some("1") | Some("true") => DerivedProofs::On,
            _ => DerivedProofs::Off,
        }
    }

    /// Locators and batch leaves are written for new attestations.
    pub(crate) fn writes_locators(self) -> bool {
        self != DerivedProofs::Off
    }
}

/// Leaves per stored chunk. A proof reads one chunk (32 KiB) and the
/// header, and hashes about twice this many nodes, at any batch size.
pub(crate) const CHUNK_LEAVES: u32 = 1024;

const LOCATOR_FORMAT: u8 = 1;
const BATCH_FORMAT: u8 = 1;
const LOCATOR_LEN: usize = 1 + 8 + 8 + 4 + 4 + 4;
const HEAD_FIXED: usize = 1 + 1 + 32 + 32 + 4 + 4;
const TAG_HEAD: u8 = 0;
const TAG_CHUNK: u8 = 1;

/// One fact's bytes inside its attestation's CBOR, and its digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FactSpan {
    pub start: u32,
    pub len: u32,
    pub digest: [u8; 32],
}

/// Where one appended attestation landed, and its facts' spans in order.
#[derive(Debug, Clone)]
pub(crate) struct LocatorInput {
    pub segment: u64,
    pub offset: u64,
    pub entry_hash: [u8; 32],
    pub spans: Vec<FactSpan>,
}

/// Where a fact's bytes are: `offset + 4 + fact_start` in
/// `merkle.log.<segment>` (the 4 is the record's length prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Locator {
    pub segment: u64,
    pub offset: u64,
    // The span is for the next phase, which reads fact bodies from the log.
    // It is written now so that phase needs no backfill of new rows.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fact_start: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub fact_len: u32,
    pub leaf_index: u32,
}

struct BatchHead {
    version: u8,
    root: [u8; 32],
    entry_hash: [u8; 32],
    n: u32,
    chunk: u32,
    top: Vec<[u8; 32]>,
}

pub(crate) fn cid_digest(cid: &FactCid) -> Option<[u8; 32]> {
    let raw = data_encoding::BASE32_NOPAD
        .decode(cid.as_str().to_ascii_uppercase().as_bytes())
        .ok()?;
    raw.try_into().ok()
}

// ── CBOR walk ────────────────────────────────────────────────────────────

/// A data item's header: (major type, argument, header bytes). ciborium
/// writes definite lengths only, so an indefinite one is refused.
fn head(buf: &[u8], i: usize) -> Option<(u8, u64, usize)> {
    let ib = *buf.get(i)?;
    let (major, info) = (ib >> 5, ib & 0x1f);
    let (arg, extra) = match info {
        0..=23 => (u64::from(info), 0usize),
        24 => (u64::from(*buf.get(i + 1)?), 1),
        25 => (
            u64::from(u16::from_be_bytes(buf.get(i + 1..i + 3)?.try_into().ok()?)),
            2,
        ),
        26 => (
            u64::from(u32::from_be_bytes(buf.get(i + 1..i + 5)?.try_into().ok()?)),
            4,
        ),
        27 => (
            u64::from_be_bytes(buf.get(i + 1..i + 9)?.try_into().ok()?),
            8,
        ),
        _ => return None,
    };
    Some((major, arg, 1 + extra))
}

/// The end of the data item that starts at `i`.
fn skip(buf: &[u8], i: usize, depth: u32) -> Option<usize> {
    if depth > 128 {
        return None;
    }
    let (major, arg, h) = head(buf, i)?;
    let mut j = i.checked_add(h)?;
    match major {
        0 | 1 | 7 => Some(j),
        2 | 3 => {
            let end = j.checked_add(usize::try_from(arg).ok()?)?;
            (end <= buf.len()).then_some(end)
        }
        4 => {
            for _ in 0..arg {
                j = skip(buf, j, depth + 1)?;
            }
            Some(j)
        }
        5 => {
            for _ in 0..arg.checked_mul(2)? {
                j = skip(buf, j, depth + 1)?;
            }
            Some(j)
        }
        6 => skip(buf, j, depth + 1),
        _ => None,
    }
}

/// The byte span of every element of the top-level map's `facts` array.
fn fact_spans(buf: &[u8]) -> Option<Vec<(usize, usize)>> {
    match entry_fact_spans(buf) {
        EntryFacts::Spans(s) => Some(s),
        EntryFacts::NoFacts | EntryFacts::Malformed => None,
    }
}

/// What one log record holds, for a reader that must not mistake a record it
/// could not read for a record with no facts in it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EntryFacts {
    /// An attestation: the span of each element of its `facts` array.
    Spans(Vec<(usize, usize)>),
    /// Not a map, or a map with no `facts` key (a memory write). Holds no fact.
    NoFacts,
    /// A map this walk could not read to its `facts` array, or a `facts`
    /// value it could not read.
    Malformed,
}

pub(crate) fn entry_fact_spans(buf: &[u8]) -> EntryFacts {
    let Some((major, entries, h)) = head(buf, 0) else {
        return EntryFacts::Malformed;
    };
    if major != 5 {
        return EntryFacts::NoFacts;
    }
    let walk = || -> Option<EntryFacts> {
        let mut i = h;
        for _ in 0..entries {
            let (kmajor, _, kh) = head(buf, i)?;
            let key_end = skip(buf, i, 0)?;
            if kmajor == 3 && buf.get(i + kh..key_end)? == b"facts" {
                let (amajor, count, ah) = head(buf, key_end)?;
                if amajor != 4 {
                    return None;
                }
                let mut j = key_end + ah;
                let mut out = Vec::with_capacity(usize::try_from(count).ok()?.min(buf.len()));
                for _ in 0..count {
                    let end = skip(buf, j, 0)?;
                    out.push((j, end));
                    j = end;
                }
                return Some(EntryFacts::Spans(out));
            }
            i = skip(buf, key_end, 0)?;
        }
        Some(EntryFacts::NoFacts)
    };
    walk().unwrap_or(EntryFacts::Malformed)
}

/// Each fact's span in `buf` (an attestation's CBOR, exactly as the log
/// stores it), checked against the cids `put_many` returned for the same
/// facts in the same order. `None` when any span does not hash to its cid:
/// then no locator is written for the attestation and its proofs are
/// stored the old way.
pub(crate) fn verified_fact_spans(buf: &[u8], cids: &[FactCid]) -> Option<Vec<FactSpan>> {
    let raw = fact_spans(buf)?;
    if raw.len() != cids.len() {
        return None;
    }
    raw.iter()
        .zip(cids)
        .map(|(&(a, b), cid)| {
            let digest = cid_digest(cid)?;
            if blake3::hash(buf.get(a..b)?).as_bytes() != &digest {
                return None;
            }
            Some(FactSpan {
                start: u32::try_from(a).ok()?,
                len: u32::try_from(b - a).ok()?,
                digest,
            })
        })
        .collect()
}

// ── merkle, the same rule `persist_fact_proofs` stores ───────────────────

fn proof_version(preimage_version: u8) -> u8 {
    if preimage_version >= emem_attest::PREIMAGE_V1 {
        emem_attest::PREIMAGE_V1
    } else {
        0
    }
}

fn promote(version: u8, leaf: &[u8; 32]) -> [u8; 32] {
    if version >= emem_attest::PREIMAGE_V1 {
        emem_attest::promote_leaf_v1(leaf)
    } else {
        let mut h = blake3::Hasher::new();
        h.update(leaf);
        h.update(leaf);
        *h.finalize().as_bytes()
    }
}

fn node(version: u8, l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    if version >= emem_attest::PREIMAGE_V1 {
        h.update(&[0x01]);
    }
    h.update(l);
    h.update(r);
    *h.finalize().as_bytes()
}

/// Fold `layer` upward and collect `idx`'s siblings: `levels` times, or
/// until one node is left. An odd layer pairs its last node with itself,
/// which is the topology of `emem_attest::merkle_root_and_paths{,_v1}`.
/// Folding a fixed number of levels is what makes a chunk of 2^levels
/// leaves (or the shorter last chunk) land on the node the whole tree has
/// at that level and position: every chunk starts at an even index on
/// every level below.
fn fold_path(
    mut layer: Vec<[u8; 32]>,
    mut idx: usize,
    levels: Option<u32>,
    version: u8,
) -> Option<([u8; 32], Vec<[u8; 32]>)> {
    let mut path = Vec::new();
    let mut done = 0u32;
    loop {
        let finished = match levels {
            Some(l) => done >= l,
            None => layer.len() <= 1,
        };
        if finished {
            break;
        }
        let last = layer.len().checked_sub(1)?;
        let sibling = if idx.is_multiple_of(2) {
            idx + 1
        } else {
            idx - 1
        };
        path.push(*layer.get(sibling.min(last))?);
        layer = layer
            .chunks(2)
            .map(|p| node(version, &p[0], p.get(1).unwrap_or(&p[0])))
            .collect();
        idx /= 2;
        done += 1;
    }
    Some((*layer.first()?, path))
}

// ── rows ─────────────────────────────────────────────────────────────────

fn batch_key(segment: u64, offset: u64, tag: u8, idx: u32) -> Vec<u8> {
    let mut k = Vec::with_capacity(21);
    k.extend_from_slice(&segment.to_be_bytes());
    k.extend_from_slice(&offset.to_be_bytes());
    k.push(tag);
    k.extend_from_slice(&idx.to_be_bytes());
    k
}

/// Rows for one attestation: (batch rows, locator rows).
pub(crate) type LocatorRows = (Vec<(Vec<u8>, Vec<u8>)>, Vec<(Vec<u8>, Vec<u8>)>);

/// The rows one attestation writes. Leaves are sorted the way
/// `persist_fact_proofs` sorts them (stable, by digest), and locators are
/// emitted in that order, so a fact present twice keeps the later leaf
/// index exactly as its stored proof row did.
pub(crate) fn rows(inp: &LocatorInput, preimage_version: u8, chunk: u32) -> Option<LocatorRows> {
    if inp.spans.is_empty() || chunk < 2 || !chunk.is_power_of_two() {
        return None;
    }
    let version = proof_version(preimage_version);
    let mut sorted: Vec<&FactSpan> = inp.spans.iter().collect();
    sorted.sort_by_key(|s| s.digest);
    let leaves: Vec<[u8; 32]> = sorted.iter().map(|s| s.digest).collect();
    let n = u32::try_from(leaves.len()).ok()?;
    let chunk_len = usize::try_from(chunk).ok()?;
    let promoted = |c: &[[u8; 32]]| c.iter().map(|l| promote(version, l)).collect::<Vec<_>>();
    let (root, top) = if n <= chunk {
        (
            fold_path(promoted(&leaves), 0, None, version)?.0,
            Vec::new(),
        )
    } else {
        let levels = chunk.trailing_zeros();
        let top = leaves
            .chunks(chunk_len)
            .map(|c| fold_path(promoted(c), 0, Some(levels), version).map(|r| r.0))
            .collect::<Option<Vec<_>>>()?;
        (fold_path(top.clone(), 0, None, version)?.0, top)
    };
    let mut header = Vec::with_capacity(HEAD_FIXED + 32 * top.len());
    header.push(BATCH_FORMAT);
    header.push(version);
    header.extend_from_slice(&root);
    header.extend_from_slice(&inp.entry_hash);
    header.extend_from_slice(&n.to_le_bytes());
    header.extend_from_slice(&chunk.to_le_bytes());
    for t in &top {
        header.extend_from_slice(t);
    }
    let mut batches = Vec::with_capacity(1 + leaves.len().div_ceil(chunk_len));
    batches.push((batch_key(inp.segment, inp.offset, TAG_HEAD, 0), header));
    for (ci, c) in leaves.chunks(chunk_len).enumerate() {
        batches.push((
            batch_key(inp.segment, inp.offset, TAG_CHUNK, u32::try_from(ci).ok()?),
            c.concat(),
        ));
    }
    let locators = sorted
        .iter()
        .enumerate()
        .map(|(leaf_index, s)| {
            let mut v = Vec::with_capacity(LOCATOR_LEN);
            v.push(LOCATOR_FORMAT);
            v.extend_from_slice(&inp.segment.to_le_bytes());
            v.extend_from_slice(&inp.offset.to_le_bytes());
            v.extend_from_slice(&s.start.to_le_bytes());
            v.extend_from_slice(&s.len.to_le_bytes());
            v.extend_from_slice(&u32::try_from(leaf_index).ok()?.to_le_bytes());
            Some((s.digest.to_vec(), v))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((batches, locators))
}

/// Write one attestation's locator and batch rows in one transaction.
/// Not durable on its own, like the proof rows it replaces: the next
/// durable fact commit carries it, and the log it points into is fsynced
/// before this runs.
pub(crate) fn write(
    redb: &RedbFacts,
    inp: &LocatorInput,
    preimage_version: u8,
) -> Result<(), emem_cache::CacheError> {
    let (batches, locators) = rows(inp, preimage_version, CHUNK_LEAVES).ok_or_else(|| {
        emem_cache::CacheError::Backend("fact locator rows could not be built".into())
    })?;
    redb.kv_put_tables(
        &[(KvTable::Batches, &batches), (KvTable::Locators, &locators)],
        false,
    )
}

fn decode_locator(v: &[u8]) -> Option<Locator> {
    if v.len() != LOCATOR_LEN || v[0] != LOCATOR_FORMAT {
        return None;
    }
    let u64_at = |i: usize| v.get(i..i + 8)?.try_into().ok().map(u64::from_le_bytes);
    let u32_at = |i: usize| v.get(i..i + 4)?.try_into().ok().map(u32::from_le_bytes);
    Some(Locator {
        segment: u64_at(1)?,
        offset: u64_at(9)?,
        fact_start: u32_at(17)?,
        fact_len: u32_at(21)?,
        leaf_index: u32_at(25)?,
    })
}

fn nodes(b: &[u8]) -> Option<Vec<[u8; 32]>> {
    if !b.len().is_multiple_of(32) {
        return None;
    }
    b.chunks(32).map(|c| c.try_into().ok()).collect()
}

fn decode_head(v: &[u8]) -> Option<BatchHead> {
    if v.len() < HEAD_FIXED || v[0] != BATCH_FORMAT {
        return None;
    }
    let n = u32::from_le_bytes(v.get(66..70)?.try_into().ok()?);
    let chunk = u32::from_le_bytes(v.get(70..74)?.try_into().ok()?);
    if n == 0 || chunk < 2 || !chunk.is_power_of_two() {
        return None;
    }
    let top = nodes(v.get(HEAD_FIXED..)?)?;
    let want_top = if n > chunk { n.div_ceil(chunk) } else { 0 };
    if top.len() != usize::try_from(want_top).ok()? {
        return None;
    }
    Some(BatchHead {
        version: v[1],
        root: v.get(2..34)?.try_into().ok()?,
        entry_hash: v.get(34..66)?.try_into().ok()?,
        n,
        chunk,
        top,
    })
}

/// The proof `persist_fact_proofs` would have stored, rebuilt from a
/// batch header and the one chunk that holds the leaf. Every check that
/// fails (wrong leaf, chunk node not in the header, root mismatch) is a
/// `None`: a receipt then carries no proof, which is what it carried for
/// a fact with no stored proof.
fn derive(
    head: &BatchHead,
    leaf_index: u32,
    digest: &[u8; 32],
    chunk_bytes: impl FnOnce(u32) -> Option<Vec<u8>>,
) -> Option<MerkleProof> {
    if leaf_index >= head.n {
        return None;
    }
    let ci = leaf_index / head.chunk;
    let within = usize::try_from(leaf_index % head.chunk).ok()?;
    let leaves = nodes(&chunk_bytes(ci)?)?;
    if leaves.get(within)? != digest {
        return None;
    }
    let layer: Vec<[u8; 32]> = leaves.iter().map(|l| promote(head.version, l)).collect();
    let (root, path) = if head.n <= head.chunk {
        fold_path(layer, within, None, head.version)?
    } else {
        let levels = head.chunk.trailing_zeros();
        let (chunk_node, mut path) = fold_path(layer, within, Some(levels), head.version)?;
        if head.top.get(usize::try_from(ci).ok()?)? != &chunk_node {
            return None;
        }
        let (root, upper) = fold_path(
            head.top.clone(),
            usize::try_from(ci).ok()?,
            None,
            head.version,
        )?;
        path.extend(upper);
        (root, path)
    };
    if root != head.root {
        return None;
    }
    Some(MerkleProof {
        leaf_index,
        path,
        root,
        version: head.version,
        fact_cid: None,
        leaf_count: Some(head.n),
        log_entry_hash: Some(
            data_encoding::BASE32_NOPAD
                .encode(&head.entry_hash)
                .to_lowercase(),
        ),
    })
}

/// A fact's locator, when one was written for it.
#[cfg(test)]
pub(crate) fn locator(redb: &RedbFacts, cid: &FactCid) -> Option<Locator> {
    let digest = cid_digest(cid)?;
    decode_locator(&redb.kv_get(KvTable::Locators, &digest).ok()??)
}

/// The proof for `cid` rebuilt from its locator, or `None` when it has no
/// locator or any check fails.
pub(crate) fn derived_proof(redb: &RedbFacts, cid: &FactCid) -> Option<MerkleProof> {
    let digest = cid_digest(cid)?;
    let loc = decode_locator(&redb.kv_get(KvTable::Locators, &digest).ok()??)?;
    let head = decode_head(
        &redb
            .kv_get(
                KvTable::Batches,
                &batch_key(loc.segment, loc.offset, TAG_HEAD, 0),
            )
            .ok()??,
    )?;
    derive(&head, loc.leaf_index, &digest, |ci| {
        redb.kv_get(
            KvTable::Batches,
            &batch_key(loc.segment, loc.offset, TAG_CHUNK, ci),
        )
        .ok()
        .flatten()
    })
}

/// True when this store holds any locator, so proofs written while the
/// flag was on are still served after it is turned off.
pub(crate) fn any_locators(redb: &RedbFacts) -> bool {
    redb.kv_len(KvTable::Locators).unwrap_or(0) > 0
}

/// The CBOR a proof is compared by: the struct has no `PartialEq`, and the
/// bytes are what a receipt would carry.
pub(crate) fn proof_bytes(p: &MerkleProof) -> Vec<u8> {
    let mut b = Vec::new();
    // A MerkleProof always serialises; an empty buffer can only mismatch.
    let _ = ciborium::ser::into_writer(p, &mut b);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(i: u64) -> [u8; 32] {
        *blake3::hash(&i.to_le_bytes()).as_bytes()
    }

    fn sorted_leaves(n: u64) -> Vec<[u8; 32]> {
        let mut v: Vec<[u8; 32]> = (0..n).map(leaf).collect();
        v.sort();
        v
    }

    fn input(leaves: &[[u8; 32]]) -> LocatorInput {
        LocatorInput {
            segment: 7,
            offset: 4096,
            entry_hash: [9u8; 32],
            spans: leaves
                .iter()
                .enumerate()
                .map(|(i, d)| FactSpan {
                    start: 8 + 100 * i as u32,
                    len: 100,
                    digest: *d,
                })
                .collect(),
        }
    }

    /// Every proof rebuilt from the stored rows equals the one
    /// `merkle_root_and_paths{,_v1}` gives, for every leaf, for batch sizes
    /// on both sides of the chunk boundary and for both hashing rules. The
    /// oracle is the function whose output `persist_fact_proofs` stores.
    #[test]
    fn a_derived_path_is_the_path_the_stored_proof_had() {
        for pv in [0u8, 1u8] {
            for chunk in [2u32, 4, 8] {
                for n in 1..=41u64 {
                    check(n, chunk, pv, None);
                }
            }
            check(1024, CHUNK_LEAVES, pv, None);
            check(1025, CHUNK_LEAVES, pv, Some(&[0, 1, 511, 1023, 1024]));
            check(
                2049 + 300,
                CHUNK_LEAVES,
                pv,
                Some(&[0, 1023, 1024, 2047, 2048, 2348]),
            );
        }
    }

    fn check(n: u64, chunk: u32, pv: u8, only: Option<&[usize]>) {
        let leaves = sorted_leaves(n);
        let (root, paths) = if pv >= 1 {
            emem_attest::merkle_root_and_paths_v1(&leaves)
        } else {
            emem_attest::merkle_root_and_paths(&leaves)
        };
        let inp = input(&leaves);
        let (batches, locators) = rows(&inp, pv, chunk).expect("rows");
        let head = decode_head(&batches[0].1).expect("head");
        assert_eq!(head.root, root, "n={n} chunk={chunk} v={pv}: root");
        let idxs: Vec<usize> = match only {
            Some(o) => o.to_vec(),
            None => (0..leaves.len()).collect(),
        };
        for i in idxs {
            let loc = decode_locator(&locators[i].1).expect("locator");
            assert_eq!(loc.leaf_index as usize, i);
            let p = derive(&head, loc.leaf_index, &leaves[i], |ci| {
                let key = batch_key(7, 4096, TAG_CHUNK, ci);
                batches
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.clone())
            })
            .unwrap_or_else(|| panic!("n={n} chunk={chunk} v={pv} i={i}: no proof"));
            assert_eq!(p.path, paths[i], "n={n} chunk={chunk} v={pv} i={i}: path");
            assert_eq!(p.root, root);
            assert_eq!(p.version, if pv >= 1 { 1 } else { 0 });
            assert_eq!(p.leaf_count, Some(n as u32));
        }
    }

    /// The control: a wrong leaf, a chunk the header does not name, and a
    /// root that does not match each give no proof. A derivation that
    /// returned something for these would be checking nothing.
    #[test]
    fn a_proof_that_does_not_fold_to_its_root_is_refused() {
        let leaves = sorted_leaves(10);
        let (batches, _) = rows(&input(&leaves), 1, 4).expect("rows");
        let head = decode_head(&batches[0].1).expect("head");
        let chunk = |ci: u32| {
            let key = batch_key(7, 4096, TAG_CHUNK, ci);
            batches
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        };
        assert!(derive(&head, 5, &leaves[5], chunk).is_some());
        assert!(derive(&head, 5, &leaves[4], chunk).is_none(), "wrong leaf");
        assert!(
            derive(&head, 10, &leaves[9], chunk).is_none(),
            "past the end"
        );
        let mut bent = decode_head(&batches[0].1).expect("head");
        bent.root[0] ^= 1;
        assert!(derive(&bent, 5, &leaves[5], chunk).is_none(), "root");
        let mut bad_chunk = batches[2].1.clone();
        bad_chunk[0] ^= 1;
        assert!(
            derive(&head, 5, &leaves[5], |_| Some(bad_chunk.clone())).is_none(),
            "a chunk whose node is not in the header"
        );
    }

    #[test]
    fn modes_parse_and_default_to_off() {
        assert_eq!(DerivedProofs::parse(None), DerivedProofs::Off);
        assert_eq!(DerivedProofs::parse(Some("")), DerivedProofs::Off);
        assert_eq!(DerivedProofs::parse(Some("off")), DerivedProofs::Off);
        assert_eq!(DerivedProofs::parse(Some("yes please")), DerivedProofs::Off);
        assert_eq!(
            DerivedProofs::parse(Some(" Shadow ")),
            DerivedProofs::Shadow
        );
        assert_eq!(DerivedProofs::parse(Some("on")), DerivedProofs::On);
        assert_eq!(DerivedProofs::parse(Some("1")), DerivedProofs::On);
        assert!(!DerivedProofs::Off.writes_locators());
        assert!(DerivedProofs::Shadow.writes_locators());
    }
}

#[cfg(test)]
mod storage_tests {
    //! The flag through the real write and read paths of
    //! `MaterializingStorage`.
    use super::*;
    use crate::{FactPlanePolicy, MaterializingStorage, Storage};
    use ed25519_dalek::{Signer, SigningKey};
    use emem_core::{AttesterKey, KeyEpoch, Signature};
    use emem_fact::{Attestation, Derivation, Fact, PrimaryFact, RegistryCid, SchemaCid, Source};
    use std::sync::Arc;

    fn storage(mode: DerivedProofs) -> MaterializingStorage {
        let bands = Arc::new(emem_core::bands::DEFAULT.clone());
        let functions =
            Arc::new(emem_core::FunctionRegistry::parse_default().expect("default functions"));
        let sources =
            Arc::new(emem_core::SourceRegistry::parse_default().expect("default sources"));
        let mut s = MaterializingStorage::ephemeral(bands, functions, sources).expect("storage");
        s.set_fact_plane_policy(FactPlanePolicy {
            open: true,
            ..Default::default()
        });
        s.reopen_with_derived_proofs(mode);
        s
    }

    fn fact(tslot: u64, signer: [u8; 32]) -> Fact {
        Fact::Primary(PrimaryFact {
            cell: "damO.zb000.xUti.zde78".into(),
            band: "indices.ndvi".into(),
            tslot,
            value: ciborium::Value::Float(0.25 + tslot as f64 / 1e6),
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source {
                scheme: "test".into(),
                id: format!("scene-{tslot}"),
                cid: None,
                hash: None,
                captured_at: None,
                url: None,
            }],
            derivation: Derivation {
                fn_key: "test@1".into(),
                args: None,
            },
            privacy_class: "public".into(),
            schema_cid: SchemaCid::new("test-schema"),
            signer: AttesterKey(signer),
            signed_at: "2026-11-14T00:00:00Z".into(),
            served_via: None,
        })
    }

    /// A signed batch of `n` facts under hashing rule `pv`.
    fn batch(n: u64, pv: u8) -> Attestation {
        let signing = SigningKey::from_bytes(&[23u8; 32]);
        let pk = signing.verifying_key().to_bytes();
        let facts: Vec<Fact> = (0..n).map(|t| fact(t + 1, pk)).collect();
        if pv >= 1 {
            return Attestation::build_and_sign_v1(
                facts,
                vec![],
                RegistryCid::new("test-registry"),
                SchemaCid::new("test-schema"),
                &signing,
                KeyEpoch(0),
                "2026-11-14T00:00:00Z".into(),
                None,
            )
            .expect("v1 attestation");
        }
        let mut leaves: Vec<[u8; 32]> = facts
            .iter()
            .map(|f| {
                let mut b = Vec::new();
                ciborium::ser::into_writer(f, &mut b).unwrap();
                *blake3::hash(&b).as_bytes()
            })
            .collect();
        leaves.sort();
        let root = emem_attest::merkle_root(&leaves);
        let mut h = blake3::Hasher::new();
        h.update(&root);
        h.update(b"test-registry");
        h.update(b"test-schema");
        Attestation {
            facts,
            edges: vec![],
            batch_root: root,
            attester: AttesterKey(pk),
            attester_key_epoch: KeyEpoch(0),
            registry_cid: RegistryCid::new("test-registry"),
            schema_cid: SchemaCid::new("test-schema"),
            signature: Signature(signing.sign(h.finalize().as_bytes()).to_bytes()),
            attested_at: "2026-11-14T00:00:00Z".into(),
            scope: None,
            preimage_version: 0,
        }
    }

    fn rows_in(s: &MaterializingStorage, t: KvTable) -> u64 {
        s.redb().expect("redb").kv_len(t).unwrap_or(0)
    }

    fn stored(s: &MaterializingStorage, cid: &FactCid) -> bool {
        s.redb()
            .expect("redb")
            .kv_get(KvTable::Proofs, cid.as_str().as_bytes())
            .unwrap()
            .is_some()
    }

    /// Off is the old path: proof rows for every fact, and no locator or
    /// batch table at all.
    #[tokio::test]
    async fn off_stores_proofs_and_writes_no_locator() {
        let s = storage(DerivedProofs::Off);
        let cids = s.put_attestation(&batch(30, 1)).await.expect("write");
        assert!(cids.iter().all(|c| stored(&s, c)));
        assert!(cids.iter().all(|c| s.proof_for_cid(c).is_some()));
        assert_eq!(rows_in(&s, KvTable::Locators), 0);
        assert_eq!(rows_in(&s, KvTable::Batches), 0);
        assert!(
            s.redb()
                .unwrap()
                .kv_get(KvTable::Locators, &[0u8; 32])
                .is_err(),
            "the locator table must not even exist on a store that never set the flag"
        );
    }

    /// On writes no proof rows, and every proof it serves is byte for byte
    /// the proof Off stored for the same attestation: both hashing rules,
    /// and a batch that spans two chunks.
    #[tokio::test]
    async fn on_serves_the_proof_off_would_have_stored() {
        for (n, pv) in [(1u64, 1u8), (37, 1), (37, 0), (1100, 1)] {
            let att = batch(n, pv);
            let off = storage(DerivedProofs::Off);
            let on = storage(DerivedProofs::On);
            let cids = off.put_attestation(&att).await.expect("off write");
            let cids_on = on.put_attestation(&att).await.expect("on write");
            assert_eq!(cids, cids_on);
            assert!(
                cids.iter().all(|c| !stored(&on, c)),
                "n={n}: On stored a proof row"
            );
            assert_eq!(rows_in(&on, KvTable::Locators), n);
            for c in &cids {
                let want = off.proof_for_cid(c).expect("stored proof");
                let got = on.proof_for_cid(c).expect("derived proof");
                assert_eq!(proof_bytes(&got), proof_bytes(&want), "n={n} v={pv}");
            }
        }
    }

    /// Shadow writes both, serves the stored proof, and the two agree.
    #[tokio::test]
    async fn shadow_writes_both_and_they_agree() {
        let s = storage(DerivedProofs::Shadow);
        let cids = s.put_attestation(&batch(12, 1)).await.expect("write");
        let r = s.redb().unwrap();
        for c in &cids {
            assert!(stored(&s, c));
            let derived = derived_proof(&r, c).expect("locator");
            let served = s.proof_for_cid(c).expect("proof");
            assert_eq!(proof_bytes(&derived), proof_bytes(&served));
        }
    }

    /// The locator names the fact's own bytes in the log file: the premise
    /// of serving bodies from the log in the next phase.
    #[tokio::test]
    async fn a_locator_points_at_the_fact_bytes_in_the_log() {
        let s = storage(DerivedProofs::On);
        s.put_attestation(&batch(3, 1)).await.expect("first");
        let cids = s.put_attestation(&batch(20, 0)).await.expect("second");
        let r = s.redb().unwrap();
        for c in &cids {
            let loc = locator(&r, c).expect("locator");
            let seg = std::fs::read(s.log.root.join(format!("merkle.log.{}", loc.segment)))
                .expect("segment");
            let at = (loc.offset + 4 + u64::from(loc.fact_start)) as usize;
            let body = &seg[at..at + loc.fact_len as usize];
            assert_eq!(blake3::hash(body).as_bytes(), &cid_digest(c).unwrap());
            let fact = s.get_facts_many(std::slice::from_ref(c)).await.unwrap()[0]
                .clone()
                .expect("fact");
            let mut want = Vec::new();
            ciborium::ser::into_writer(&fact, &mut want).unwrap();
            assert_eq!(body, want.as_slice(), "the log bytes are the stored body");
        }
    }

    /// Turning the flag off after it was on: new writes store proofs again,
    /// and facts written while it was on keep their proofs. A store that
    /// never set the flag does not look for locators at all.
    #[tokio::test]
    async fn turning_it_off_keeps_the_proofs_written_while_on() {
        let mut s = storage(DerivedProofs::On);
        let early = s.put_attestation(&batch(8, 1)).await.expect("on write");
        s.reopen_with_derived_proofs(DerivedProofs::Off);
        let later = {
            let signing = SigningKey::from_bytes(&[23u8; 32]);
            let pk = signing.verifying_key().to_bytes();
            let att = Attestation::build_and_sign_v1(
                vec![fact(500, pk)],
                vec![],
                RegistryCid::new("test-registry"),
                SchemaCid::new("test-schema"),
                &signing,
                KeyEpoch(0),
                "2026-11-14T00:00:00Z".into(),
                None,
            )
            .unwrap();
            s.put_attestation(&att).await.expect("off write")
        };
        assert!(stored(&s, &later[0]), "off stores proofs again");
        assert_eq!(rows_in(&s, KvTable::Locators), 8, "off writes no locator");
        assert!(early.iter().all(|c| s.proof_for_cid(c).is_some()));

        // The control: the same rows, read by a store that believes it never
        // had locators, find nothing, which is the old behaviour for a fact
        // with no stored proof.
        s.locators_present = false;
        assert!(early.iter().all(|c| s.proof_for_cid(c).is_none()));
    }

    /// Spans that do not hash to their cids are refused, so a layout change
    /// in the attestation's CBOR falls back to stored proofs instead of
    /// writing locators that point at the wrong bytes.
    #[test]
    fn spans_that_do_not_match_their_cids_are_refused() {
        let att = batch(5, 1);
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&att, &mut buf).unwrap();
        let cids: Vec<FactCid> = att
            .facts
            .iter()
            .map(|f| emem_cache::fact_cid_of(f).unwrap())
            .collect();
        let spans = verified_fact_spans(&buf, &cids).expect("spans");
        assert_eq!(spans.len(), 5);
        let mut swapped = cids.clone();
        swapped.swap(0, 1);
        assert!(verified_fact_spans(&buf, &swapped).is_none());
        assert!(verified_fact_spans(&buf, &cids[..4]).is_none());
        let mut bent = buf.clone();
        let at = spans[2].start as usize + 3;
        bent[at] ^= 0x01;
        assert!(verified_fact_spans(&bent, &cids).is_none());
        assert!(verified_fact_spans(&buf[..buf.len() / 2], &cids).is_none());
    }
}
