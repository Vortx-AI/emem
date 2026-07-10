//! emem-attest — attestation envelope construction, Merkle batching, signing.

#![forbid(unsafe_code)]

/// RFC 6962-style append-only transparency tree (inclusion + consistency
/// proofs) over the attestation log. Distinct from the batch-root Merkle
/// functions in this module, which are for sorted/deduplicated fixed
/// batches and cannot answer append-only consistency queries.
pub mod translog;

use blake3::Hasher;

/// Build a blake3 Merkle root over a list of fact CIDs.
///
/// CIDs MUST be sorted (canonical order) before passing to this function.
/// Returns the 32-byte root.
pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    let layer0 = self_hashed_layer(leaves);
    fold_to_root(layer0)
}

/// Compute the merkle root **and** the sibling path for every leaf in
/// one pass. Each returned `Vec<[u8; 32]>` is the bottom-up sibling
/// sequence a verifier needs to re-derive the root from `leaves[i]`
/// (after applying the same self-hash promotion this module uses).
///
/// Pre-condition: `leaves` is in canonical sort order — same as
/// `merkle_root`.
pub fn merkle_root_and_paths(leaves: &[[u8; 32]]) -> ([u8; 32], Vec<Vec<[u8; 32]>>) {
    if leaves.is_empty() {
        return ([0u8; 32], Vec::new());
    }
    let mut paths: Vec<Vec<[u8; 32]>> = vec![Vec::new(); leaves.len()];
    let mut layer = self_hashed_layer(leaves);
    // Track each input leaf's index in the current layer; this remains
    // its index because we never reorder, and parents land at floor(i/2).
    let mut indices: Vec<usize> = (0..leaves.len()).collect();
    while layer.len() > 1 {
        let mut next: Vec<[u8; 32]> = Vec::with_capacity(layer.len().div_ceil(2));
        for pair in layer.chunks(2) {
            let mut h = Hasher::new();
            h.update(&pair[0]);
            h.update(if pair.len() == 2 { &pair[1] } else { &pair[0] });
            let mut out = [0u8; 32];
            out.copy_from_slice(h.finalize().as_bytes());
            next.push(out);
        }
        // Record each leaf's sibling at this layer.
        for (leaf_pos, idx) in indices.iter_mut().enumerate() {
            let sibling_idx = if (*idx).is_multiple_of(2) {
                *idx + 1
            } else {
                *idx - 1
            };
            // Odd-cardinality layer: last element is paired with itself.
            let resolved = sibling_idx.min(layer.len() - 1);
            // If the leaf's pair is itself (last unpaired element), the
            // sibling is its own value — surface it explicitly so a
            // verifier can reproduce the duplicate-pair branch.
            paths[leaf_pos].push(layer[resolved]);
            *idx /= 2;
        }
        layer = next;
    }
    (layer[0], paths)
}

fn self_hashed_layer(leaves: &[[u8; 32]]) -> Vec<[u8; 32]> {
    leaves
        .iter()
        .map(|leaf| {
            let mut h = Hasher::new();
            h.update(leaf);
            h.update(leaf);
            let mut out = [0u8; 32];
            out.copy_from_slice(h.finalize().as_bytes());
            out
        })
        .collect()
}

fn fold_to_root(mut layer: Vec<[u8; 32]>) -> [u8; 32] {
    while layer.len() > 1 {
        let mut next: Vec<[u8; 32]> = Vec::with_capacity(layer.len().div_ceil(2));
        for pair in layer.chunks(2) {
            let mut h = Hasher::new();
            h.update(&pair[0]);
            h.update(if pair.len() == 2 { &pair[1] } else { &pair[0] });
            let mut out = [0u8; 32];
            out.copy_from_slice(h.finalize().as_bytes());
            next.push(out);
        }
        layer = next;
    }
    layer[0]
}

/// Verify that `leaf` (already self-hashed-promoted form) reaches `root`
/// via `path` starting from position `leaf_index`. Returns true if the
/// proof is consistent with the root.
pub fn verify_merkle_path(
    leaf: &[u8; 32],
    leaf_index: usize,
    path: &[[u8; 32]],
    root: &[u8; 32],
) -> bool {
    let mut acc = *leaf;
    let mut idx = leaf_index;
    for sibling in path {
        let mut h = Hasher::new();
        if idx.is_multiple_of(2) {
            h.update(&acc);
            h.update(sibling);
        } else {
            h.update(sibling);
            h.update(&acc);
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(h.finalize().as_bytes());
        acc = out;
        idx /= 2;
    }
    &acc == root
}

// ---------------------------------------------------------------------------
// v1 — domain-separated hashing (preimage_version = 1)
//
// The v0 surfaces above have two structural weaknesses that v1 closes:
//
//   1. Signature preimages concatenated variable-length fields with no
//      length prefixes ("abc"+"def" and "abcd"+"ef" hash identically) and
//      optional segments were untagged 64-char hex inserted at the same
//      position (a scope digest and an as_of digest were indistinguishable
//      to the signature).
//   2. Merkle leaves and internal nodes were hashed with the same function
//      (leaf promotion blake3(L||L) is byte-identical in shape to an
//      internal node blake3(A||A)), and odd layers duplicate their last
//      element, so root([A,B,C]) == root([A,B,C,C]) — the CVE-2012-2459
//      pattern. v1 prefixes leaves with 0x00 and nodes with 0x01
//      (RFC 6962 style); verifiers of v1 attestations must additionally
//      reject duplicate leaves (canonical order is sorted, so duplicates
//      are adjacent — see [`has_adjacent_duplicate`]).
//
// v0 functions are kept verbatim so every receipt/attestation signed
// before the cutover verifies byte-for-byte under its original rule.
// ---------------------------------------------------------------------------

/// Wire value of `preimage_version` for the v1 rules in this module.
pub const PREIMAGE_V1: u8 = 1;

const MERKLE_LEAF_PREFIX_V1: u8 = 0x00;
const MERKLE_NODE_PREFIX_V1: u8 = 0x01;

/// v1 signature-preimage builder: every segment is `tag || u32-LE length
/// || bytes`, the whole stream is domain-separated by a context string,
/// and the digest is `blake3(stream)`. Two preimages with different
/// domains, different segment tags, or different segment boundaries can
/// never collide without a blake3 collision.
pub struct PreimageV1 {
    h: Hasher,
}

impl PreimageV1 {
    /// Start a preimage for `domain` (e.g. `"receipt"`, `"attestation"`).
    pub fn new(domain: &str) -> Self {
        let mut h = Hasher::new();
        h.update(b"emem.preimage.v1\x00");
        let d = domain.as_bytes();
        h.update(&(d.len() as u32).to_le_bytes());
        h.update(d);
        Self { h }
    }

    /// Append one tagged, length-prefixed segment. Optional fields are
    /// simply omitted — the tag makes presence/absence unambiguous.
    pub fn seg(&mut self, tag: u8, bytes: &[u8]) -> &mut Self {
        self.h.update(&[tag]);
        self.h.update(&(bytes.len() as u32).to_le_bytes());
        self.h.update(bytes);
        self
    }

    /// Append a tagged list segment: `tag || u32-LE count || (u32-LE len
    /// || bytes)*`. An empty list is still written (count = 0), which is
    /// distinct from the segment being absent.
    pub fn seg_list<'a, I>(&mut self, tag: u8, items: I) -> &mut Self
    where
        I: IntoIterator<Item = &'a str>,
    {
        self.h.update(&[tag]);
        let mut count: u32 = 0;
        let mut body = Vec::new();
        for it in items {
            let b = it.as_bytes();
            body.extend_from_slice(&(b.len() as u32).to_le_bytes());
            body.extend_from_slice(b);
            count += 1;
        }
        self.h.update(&count.to_le_bytes());
        self.h.update(&body);
        self
    }

    /// blake3 digest of the accumulated stream — the 32 bytes the
    /// responder's ed25519 key signs.
    pub fn finalize(&self) -> [u8; 32] {
        *self.h.finalize().as_bytes()
    }
}

/// Segment tags for the v1 receipt preimage. Stable wire constants —
/// never renumber.
pub mod receipt_tag {
    pub const REQUEST_ID: u8 = 0x01;
    pub const SERVED_AT: u8 = 0x02;
    pub const SCOPE: u8 = 0x03;
    pub const AS_OF: u8 = 0x04;
    pub const EDGES: u8 = 0x05;
    pub const MANIFEST: u8 = 0x06;
    pub const PRIMITIVE: u8 = 0x07;
    pub const CELLS: u8 = 0x08;
    pub const FACT_CIDS: u8 = 0x09;
}

/// Canonical v1 receipt preimage digest. The single source of truth for
/// both signer (emem-storage) and verifiers (REST `/v1/verify_receipt`,
/// the in-browser `/verify` page mirrors these bytes in JS). Optional
/// digests (`scope_hex` etc.) enter as tagged segments only when present.
#[allow(clippy::too_many_arguments)]
pub fn receipt_preimage_v1<'a, C, F>(
    request_id: &str,
    served_at: &str,
    scope_hex: Option<&str>,
    as_of_hex: Option<&str>,
    edges_hex: Option<&str>,
    manifest_hex: Option<&str>,
    primitive: &str,
    cells: C,
    fact_cids: F,
) -> [u8; 32]
where
    C: IntoIterator<Item = &'a str>,
    F: IntoIterator<Item = &'a str>,
{
    let mut p = PreimageV1::new("receipt");
    p.seg(receipt_tag::REQUEST_ID, request_id.as_bytes());
    p.seg(receipt_tag::SERVED_AT, served_at.as_bytes());
    if let Some(s) = scope_hex {
        p.seg(receipt_tag::SCOPE, s.as_bytes());
    }
    if let Some(a) = as_of_hex {
        p.seg(receipt_tag::AS_OF, a.as_bytes());
    }
    if let Some(e) = edges_hex {
        p.seg(receipt_tag::EDGES, e.as_bytes());
    }
    if let Some(m) = manifest_hex {
        p.seg(receipt_tag::MANIFEST, m.as_bytes());
    }
    p.seg(receipt_tag::PRIMITIVE, primitive.as_bytes());
    p.seg_list(receipt_tag::CELLS, cells);
    p.seg_list(receipt_tag::FACT_CIDS, fact_cids);
    p.finalize()
}

/// Canonical v1 attestation preimage digest:
/// `blake3(domain("attestation") || tagged(batch_root) ||
/// tagged(registry_cid) || tagged(schema_cid))`. Replaces the v0 rule
/// `blake3(batch_root || registry_cid || schema_cid)`, whose unseparated
/// string concatenation let `("abc","def")` and `("abcd","ef")` sign
/// identically.
pub fn attestation_preimage_v1(
    batch_root: &[u8; 32],
    registry_cid: &str,
    schema_cid: &str,
) -> [u8; 32] {
    let mut p = PreimageV1::new("attestation");
    p.seg(0x01, batch_root);
    p.seg(0x02, registry_cid.as_bytes());
    p.seg(0x03, schema_cid.as_bytes());
    p.finalize()
}

/// v1 leaf promotion: `blake3(0x00 || leaf)`. Verifiers re-derive a
/// proof's starting node from a fact's CID digest with this.
pub fn promote_leaf_v1(leaf: &[u8; 32]) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(&[MERKLE_LEAF_PREFIX_V1]);
    h.update(leaf);
    *h.finalize().as_bytes()
}

fn node_v1(l: &[u8; 32], r: &[u8; 32]) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(&[MERKLE_NODE_PREFIX_V1]);
    h.update(l);
    h.update(r);
    *h.finalize().as_bytes()
}

/// v1 merkle root: leaves promoted with the 0x00 prefix, internal nodes
/// hashed with the 0x01 prefix. Topology matches v0 (odd layers pair the
/// last element with itself) so path verification stays index-driven.
/// Callers MUST pass sorted, deduplicated leaves; verifiers MUST reject
/// inputs where [`has_adjacent_duplicate`] is true (the duplicate-last
/// fold makes `root([A,B,C]) == root([A,B,C,C])` in any such topology).
pub fn merkle_root_v1(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    fold_to_root_v1(leaves.iter().map(promote_leaf_v1).collect())
}

/// v1 sibling-path variant of [`merkle_root_v1`] — same contract as
/// [`merkle_root_and_paths`], with v1 hashing.
pub fn merkle_root_and_paths_v1(leaves: &[[u8; 32]]) -> ([u8; 32], Vec<Vec<[u8; 32]>>) {
    if leaves.is_empty() {
        return ([0u8; 32], Vec::new());
    }
    let mut paths: Vec<Vec<[u8; 32]>> = vec![Vec::new(); leaves.len()];
    let mut layer: Vec<[u8; 32]> = leaves.iter().map(promote_leaf_v1).collect();
    let mut indices: Vec<usize> = (0..leaves.len()).collect();
    while layer.len() > 1 {
        let mut next: Vec<[u8; 32]> = Vec::with_capacity(layer.len().div_ceil(2));
        for pair in layer.chunks(2) {
            next.push(node_v1(
                &pair[0],
                if pair.len() == 2 { &pair[1] } else { &pair[0] },
            ));
        }
        for (leaf_pos, idx) in indices.iter_mut().enumerate() {
            let sibling_idx = if (*idx).is_multiple_of(2) {
                *idx + 1
            } else {
                *idx - 1
            };
            let resolved = sibling_idx.min(layer.len() - 1);
            paths[leaf_pos].push(layer[resolved]);
            *idx /= 2;
        }
        layer = next;
    }
    (layer[0], paths)
}

fn fold_to_root_v1(mut layer: Vec<[u8; 32]>) -> [u8; 32] {
    while layer.len() > 1 {
        let mut next: Vec<[u8; 32]> = Vec::with_capacity(layer.len().div_ceil(2));
        for pair in layer.chunks(2) {
            next.push(node_v1(
                &pair[0],
                if pair.len() == 2 { &pair[1] } else { &pair[0] },
            ));
        }
        layer = next;
    }
    layer[0]
}

/// v1 path verification. `leaf` is the **promoted** form
/// ([`promote_leaf_v1`] of the fact's CID digest).
pub fn verify_merkle_path_v1(
    leaf: &[u8; 32],
    leaf_index: usize,
    path: &[[u8; 32]],
    root: &[u8; 32],
) -> bool {
    let mut acc = *leaf;
    let mut idx = leaf_index;
    for sibling in path {
        acc = if idx.is_multiple_of(2) {
            node_v1(&acc, sibling)
        } else {
            node_v1(sibling, &acc)
        };
        idx /= 2;
    }
    &acc == root
}

/// True when a sorted leaf slice contains a duplicate. Canonical leaf
/// order is bytewise-sorted, so any duplicate is adjacent. v1 verifiers
/// reject such attestations — a duplicated fact CID adds nothing honest
/// and enables root-equivocation via the duplicate-last fold.
pub fn has_adjacent_duplicate(sorted_leaves: &[[u8; 32]]) -> bool {
    sorted_leaves.windows(2).any(|w| w[0] == w[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_root_is_zero() {
        assert_eq!(merkle_root(&[]), [0u8; 32]);
    }

    #[test]
    fn single_leaf_is_self_hashed() {
        let leaf = [7u8; 32];
        let r = merkle_root(&[leaf]);
        // single-leaf path: hash(leaf || leaf)
        let mut h = Hasher::new();
        h.update(&leaf);
        h.update(&leaf);
        let mut expected = [0u8; 32];
        expected.copy_from_slice(h.finalize().as_bytes());
        assert_eq!(r, expected);
    }

    #[test]
    fn root_and_paths_match_root_only() {
        // The path-aware variant must produce the same root as the bare
        // helper for any leaf set — paths are pure metadata, never alter
        // the root.
        for n in [1usize, 2, 3, 4, 7, 8, 9, 17] {
            let leaves: Vec<[u8; 32]> = (0..n as u8)
                .map(|i| {
                    let mut a = [0u8; 32];
                    a[0] = i;
                    a
                })
                .collect();
            let (r1, _) = merkle_root_and_paths(&leaves);
            let r0 = merkle_root(&leaves);
            assert_eq!(r0, r1, "root differs at n={n}");
        }
    }

    #[test]
    fn single_leaf_path_is_empty_and_root_is_self_hash() {
        // Single-fact attestation: the proof's path is empty (no
        // siblings to combine) and `verify_merkle_path` must accept the
        // promoted leaf as the root directly.
        let leaf = [0xa5u8; 32];
        let (root, paths) = merkle_root_and_paths(&[leaf]);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].is_empty(), "single-leaf path must be empty");
        let promoted = self_hashed_layer(&[leaf]);
        assert!(verify_merkle_path(&promoted[0], 0, &paths[0], &root));
    }

    #[test]
    fn odd_cardinality_last_leaf_self_pairs() {
        // Odd-cardinality layer: the last leaf is paired with itself in
        // the fold. The recorded sibling for that leaf at that level is
        // the leaf itself; `verify_merkle_path` must still produce the
        // same root.
        let leaves: Vec<[u8; 32]> = (0..3u8)
            .map(|i| {
                let mut a = [0u8; 32];
                a[0] = i;
                a
            })
            .collect();
        let (root, paths) = merkle_root_and_paths(&leaves);
        let promoted = self_hashed_layer(&leaves);
        for (i, leaf) in promoted.iter().enumerate() {
            assert!(
                verify_merkle_path(leaf, i, &paths[i], &root),
                "leaf {i} did not verify under odd cardinality"
            );
        }
    }

    #[test]
    fn paths_round_trip_via_verify() {
        // For every leaf in a non-trivial tree the recorded path must
        // reproduce the root via verify_merkle_path. This is the
        // contract receipts will expose to clients.
        let leaves: Vec<[u8; 32]> = (0..7u8)
            .map(|i| {
                let mut a = [0u8; 32];
                a[0] = i;
                a
            })
            .collect();
        let (root, paths) = merkle_root_and_paths(&leaves);
        // The leaf form a verifier should hand to verify_merkle_path is
        // the self-hashed promotion this module uses internally.
        let promoted = self_hashed_layer(&leaves);
        for (i, leaf) in promoted.iter().enumerate() {
            assert!(
                verify_merkle_path(leaf, i, &paths[i], &root),
                "leaf {i} did not verify"
            );
        }
    }

    // ----------------------------- v1 ---------------------------------

    fn mk_leaves(n: u8) -> Vec<[u8; 32]> {
        (0..n)
            .map(|i| {
                let mut a = [0u8; 32];
                a[0] = i;
                a
            })
            .collect()
    }

    #[test]
    fn v1_attestation_preimage_rejects_boundary_shift() {
        // The exact v0 collision: ("abc","def") vs ("abcd","ef") signed
        // identically under blake3(root || reg || schema). v1 must split
        // them.
        let root = [9u8; 32];
        let a = attestation_preimage_v1(&root, "abc", "def");
        let b = attestation_preimage_v1(&root, "abcd", "ef");
        assert_ne!(a, b, "v1 attestation preimage must length-prefix fields");
    }

    #[test]
    fn v1_receipt_preimage_distinguishes_optional_segments() {
        // Under v0, a scope digest and an as_of digest occupied the same
        // untagged position. Under v1 the same 64-hex string in either
        // slot must produce different signed bytes.
        let hex64 = "ab".repeat(32);
        let with_scope = receipt_preimage_v1(
            "rid",
            "t",
            Some(&hex64),
            None,
            None,
            None,
            "recall",
            ["c1"],
            ["f1"],
        );
        let with_as_of = receipt_preimage_v1(
            "rid",
            "t",
            None,
            Some(&hex64),
            None,
            None,
            "recall",
            ["c1"],
            ["f1"],
        );
        assert_ne!(with_scope, with_as_of);
    }

    #[test]
    fn v1_receipt_preimage_rejects_list_boundary_shift() {
        // cells=["a,b"] fact_cids=["c"] vs cells=["a"] fact_cids=["b,c"]
        // could be massaged into colliding byte streams under v0's
        // comma-join. v1 length-prefixes every list item.
        let a = receipt_preimage_v1("rid", "t", None, None, None, None, "recall", ["a,b"], ["c"]);
        let b = receipt_preimage_v1("rid", "t", None, None, None, None, "recall", ["a"], ["b,c"]);
        assert_ne!(a, b);
        // Item-boundary shift within one list must also split.
        let c = receipt_preimage_v1(
            "rid",
            "t",
            None,
            None,
            None,
            None,
            "recall",
            ["ab", "c"],
            ["f"],
        );
        let d = receipt_preimage_v1(
            "rid",
            "t",
            None,
            None,
            None,
            None,
            "recall",
            ["a", "bc"],
            ["f"],
        );
        assert_ne!(c, d);
    }

    #[test]
    fn v1_root_differs_from_v0_root() {
        let leaves = mk_leaves(4);
        assert_ne!(merkle_root(&leaves), merkle_root_v1(&leaves));
    }

    #[test]
    fn v1_leaf_and_node_domains_are_separated() {
        // A v1 promoted leaf must never equal a v1 internal node over the
        // same bytes — the 0x00/0x01 prefixes guarantee it.
        let x = [3u8; 32];
        assert_ne!(promote_leaf_v1(&x), node_v1(&x, &x));
    }

    #[test]
    fn v1_duplicate_last_leaf_detected() {
        // root([A,B,C]) == root([A,B,C,C]) by topology — v1 closes the
        // hole by requiring verifiers to reject adjacent duplicates.
        let three = mk_leaves(3);
        let mut four = three.clone();
        four.push(three[2]);
        assert_eq!(merkle_root_v1(&three), merkle_root_v1(&four));
        assert!(!has_adjacent_duplicate(&three));
        assert!(has_adjacent_duplicate(&four));
    }

    #[test]
    fn v1_paths_round_trip_via_verify() {
        for n in [1u8, 2, 3, 4, 7, 8, 9, 17] {
            let leaves = mk_leaves(n);
            let (root, paths) = merkle_root_and_paths_v1(&leaves);
            assert_eq!(root, merkle_root_v1(&leaves), "root mismatch n={n}");
            for (i, leaf) in leaves.iter().enumerate() {
                let promoted = promote_leaf_v1(leaf);
                assert!(
                    verify_merkle_path_v1(&promoted, i, &paths[i], &root),
                    "v1 leaf {i}/{n} did not verify"
                );
            }
        }
    }
}

#[cfg(test)]
mod v1_wire_anchor {
    use super::*;
    /// Pins the exact v1 receipt-preimage byte stream. This is the
    /// cross-language anchor for the in-browser verifier (web/verify.html
    /// `CBOR_VECTORS.preimageV1` + `buildPreimageV1WithHex`): both must
    /// produce THIS stream and hash. If this test changes, the JS vectors
    /// must be regenerated or browser verification of v1 receipts breaks.
    #[test]
    fn raw_stream_matches_receipt_preimage_v1() {
        let mut raw: Vec<u8> = Vec::new();
        raw.extend_from_slice(b"emem.preimage.v1\x00");
        let d = b"receipt";
        raw.extend_from_slice(&(d.len() as u32).to_le_bytes());
        raw.extend_from_slice(d);
        let seg = |raw: &mut Vec<u8>, tag: u8, bytes: &[u8]| {
            raw.push(tag);
            raw.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            raw.extend_from_slice(bytes);
        };
        seg(&mut raw, 1, b"RID");
        seg(&mut raw, 2, b"2026-06-12T00:00:00Z");
        seg(&mut raw, 3, b"aa");
        seg(&mut raw, 5, b"bb");
        seg(&mut raw, 6, b"cc");
        seg(&mut raw, 7, b"emem.recall");
        raw.push(8);
        raw.extend_from_slice(&2u32.to_le_bytes());
        for c in ["cellA", "cellB"] {
            raw.extend_from_slice(&(c.len() as u32).to_le_bytes());
            raw.extend_from_slice(c.as_bytes());
        }
        raw.push(9);
        raw.extend_from_slice(&1u32.to_le_bytes());
        let fc = "fc1";
        raw.extend_from_slice(&(fc.len() as u32).to_le_bytes());
        raw.extend_from_slice(fc.as_bytes());
        // The JS port (web/verify.html) builds this exact hex.
        let hx: String = raw.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hx,
            "656d656d2e707265696d6167652e763100070000007265636569707401030000005249440214000000323032362d30362d31325430303a30303a30305a030200000061610502000000626206020000006363070b000000656d656d2e726563616c6c08020000000500000063656c6c410500000063656c6c42090100000003000000666331",
            "v1 receipt wire stream drifted — regenerate web/verify.html CBOR_VECTORS"
        );
        let digest = *blake3::hash(&raw).as_bytes();
        let expect = receipt_preimage_v1(
            "RID",
            "2026-06-12T00:00:00Z",
            Some("aa"),
            None,
            Some("bb"),
            Some("cc"),
            "emem.recall",
            ["cellA", "cellB"],
            ["fc1"],
        );
        assert_eq!(
            digest, expect,
            "raw stream must hash to receipt_preimage_v1"
        );
    }
}
