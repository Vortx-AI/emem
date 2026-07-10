//! RFC 6962-style append-only Merkle transparency tree over the
//! attestation log.
//!
//! The attestation batch-root machinery in this crate ([`merkle_root_v1`])
//! is built for a *sorted, deduplicated, fixed* set of leaves and folds an
//! odd layer's last node against itself (`root([A,B,C]) == root([A,B,C,C])`,
//! the CVE-2012-2459 shape it explicitly rejects duplicates to avoid). That
//! construction cannot answer *consistency* queries over an append-ordered
//! log, so this module implements the genuine RFC 6962 tree instead:
//!
//! - leaves stay in **append order** (never sorted, never deduplicated —
//!   two identical attestations are two distinct log entries);
//! - a lone node is **promoted unchanged** to its parent (no self-pairing),
//!   so the Merkle Tree Hash of the first `m` leaves is a stable prefix of
//!   the hash of the first `n >= m` leaves, which is what makes append-only
//!   consistency provable.
//!
//! Domain separation matches the rest of emem: a leaf is
//! `blake3(0x00 || leaf_bytes)` and an internal node is
//! `blake3(0x01 || left || right)`. Hash is BLAKE3-256 throughout.
//!
//! References: RFC 6962 §2.1 (Merkle Tree Hash), §2.1.1 (audit/inclusion
//! path), §2.1.2 (consistency proof).

use blake3::Hasher;

/// Domain-separation prefix for a leaf hash (RFC 6962 uses 0x00).
const LEAF_PREFIX: u8 = 0x00;
/// Domain-separation prefix for an internal node hash (RFC 6962 uses 0x01).
const NODE_PREFIX: u8 = 0x01;

/// Hash of the empty tree: `blake3("")`. RFC 6962 defines MTH({}) as the
/// hash of the empty string; we keep that convention with BLAKE3.
pub fn empty_root() -> [u8; 32] {
    *blake3::hash(b"").as_bytes()
}

/// Leaf hash of one log entry: `blake3(0x00 || entry)`. `entry` is the
/// 32-byte per-record hash the attestation log already persists (the
/// trailing `blake3(attestation_cbor)` of each record).
pub fn leaf_hash(entry: &[u8; 32]) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(&[LEAF_PREFIX]);
    h.update(entry);
    *h.finalize().as_bytes()
}

/// Internal node hash: `blake3(0x01 || left || right)`.
fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(&[NODE_PREFIX]);
    h.update(left);
    h.update(right);
    *h.finalize().as_bytes()
}

/// Largest power of two strictly less than `n` (RFC 6962's `k`). Requires
/// `n >= 2`.
fn largest_pow2_below(n: usize) -> usize {
    debug_assert!(n >= 2);
    // Highest set bit of n-1, i.e. 2^floor(log2(n-1)).
    let mut k = 1usize;
    while k << 1 < n {
        k <<= 1;
    }
    k
}

/// Merkle Tree Hash of `leaves` in **append order** (RFC 6962 §2.1). Leaves
/// are NOT sorted or deduplicated. `O(n)` per call; the caller caches STHs.
pub fn merkle_tree_hash(leaves: &[[u8; 32]]) -> [u8; 32] {
    match leaves.len() {
        0 => empty_root(),
        1 => leaf_hash(&leaves[0]),
        n => {
            let k = largest_pow2_below(n);
            let left = merkle_tree_hash(&leaves[..k]);
            let right = merkle_tree_hash(&leaves[k..]);
            node_hash(&left, &right)
        }
    }
}

/// Inclusion (audit) path for leaf index `m` in a tree of `leaves`
/// (RFC 6962 §2.1.1). Returns the bottom-up sibling hashes a verifier
/// needs to reconstruct the root from `leaf_hash(leaves[m])`. Returns
/// `None` if `m` is out of range.
pub fn inclusion_path(m: usize, leaves: &[[u8; 32]]) -> Option<Vec<[u8; 32]>> {
    if m >= leaves.len() {
        return None;
    }
    Some(inclusion_path_inner(m, leaves))
}

fn inclusion_path_inner(m: usize, leaves: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let n = leaves.len();
    if n == 1 {
        // PATH(0, {d0}) = {}
        return Vec::new();
    }
    let k = largest_pow2_below(n);
    if m < k {
        // m in the left subtree: its path, then the right subtree hash.
        let mut p = inclusion_path_inner(m, &leaves[..k]);
        p.push(merkle_tree_hash(&leaves[k..]));
        p
    } else {
        // m in the right subtree: its path, then the left subtree hash.
        let mut p = inclusion_path_inner(m - k, &leaves[k..]);
        p.push(merkle_tree_hash(&leaves[..k]));
        p
    }
}

/// Verify an inclusion proof (RFC 6962 §2.1.1). Recomputes the root from
/// the `leaf` (the promoted [`leaf_hash`], NOT the raw entry) at index `m`
/// in a tree of `tree_size`, following `path`, and checks it equals
/// `root`. This is a pure function an offline auditor can run.
pub fn verify_inclusion(
    leaf: &[u8; 32],
    m: usize,
    tree_size: usize,
    path: &[[u8; 32]],
    root: &[u8; 32],
) -> bool {
    if m >= tree_size {
        return false;
    }
    // RFC 6962 §2.1.1 verification: walk the shrinking [sn_start, sn_end)
    // node range, consuming one sibling per level until the range is the
    // whole tree.
    let mut fn_ = m; // index of the node we are recomputing, within its level
    let mut sn = tree_size - 1; // index of the last node in its level
    let mut acc = *leaf;
    let mut it = path.iter();
    while sn > 0 {
        let sibling = match it.next() {
            Some(s) => s,
            None => return false, // path too short
        };
        if !fn_.is_multiple_of(2) || fn_ == sn {
            // acc is a right child (or the promoted lone node paired now):
            // combine sibling on the left.
            acc = node_hash(sibling, &acc);
            // Skip the promotions: shift down until fn_ is even or zero.
            while fn_.is_multiple_of(2) && fn_ != 0 {
                fn_ >>= 1;
                sn >>= 1;
            }
        } else {
            // acc is a left child: sibling on the right.
            acc = node_hash(&acc, sibling);
        }
        fn_ >>= 1;
        sn >>= 1;
    }
    it.next().is_none() && &acc == root
}

/// Consistency proof that the tree of the first `m` leaves is a prefix of
/// the tree of all `leaves` (RFC 6962 §2.1.2). `m` must satisfy
/// `0 < m <= leaves.len()`. Returns `None` otherwise.
pub fn consistency_proof(m: usize, leaves: &[[u8; 32]]) -> Option<Vec<[u8; 32]>> {
    let n = leaves.len();
    if m == 0 || m > n {
        return None;
    }
    if m == n {
        // A tree is trivially consistent with itself; empty proof.
        return Some(Vec::new());
    }
    Some(subproof(m, leaves, true))
}

fn subproof(m: usize, leaves: &[[u8; 32]], b: bool) -> Vec<[u8; 32]> {
    let n = leaves.len();
    if m == n {
        // SUBPROOF(m, D[m], true)  = {}
        // SUBPROOF(m, D[m], false) = { MTH(D[m]) }
        if b {
            return Vec::new();
        }
        return vec![merkle_tree_hash(leaves)];
    }
    let k = largest_pow2_below(n);
    if m <= k {
        // Left subtree still contains the split; append the right root.
        let mut p = subproof(m, &leaves[..k], b);
        p.push(merkle_tree_hash(&leaves[k..]));
        p
    } else {
        // Split is in the right subtree; the left root is now fixed, so the
        // recursion is no longer at the tree's left edge (b = false).
        let mut p = subproof(m - k, &leaves[k..], false);
        p.push(merkle_tree_hash(&leaves[..k]));
        p
    }
}

/// Verify a consistency proof (RFC 6962 §2.1.2): that a log which had root
/// `first_root` at size `first_size` grew append-only into `second_root`
/// at `second_size`. Pure; an offline auditor can run it against two
/// signed tree heads. Returns `false` on any malformed input.
pub fn verify_consistency(
    first_size: usize,
    first_root: &[u8; 32],
    second_size: usize,
    second_root: &[u8; 32],
    proof: &[[u8; 32]],
) -> bool {
    if first_size == 0 || first_size > second_size {
        return false;
    }
    if first_size == second_size {
        // Equal sizes are consistent iff the roots match and the proof is
        // empty (RFC 6962 lets the proof be empty here).
        return proof.is_empty() && first_root == second_root;
    }

    // RFC 6962 §2.1.2 verification algorithm.
    let mut proof = proof.to_vec();
    // If first_size is an exact power of two, the first node is the old
    // root itself and is not transmitted; prepend it.
    let mut fn_ = first_size - 1;
    let mut sn = second_size - 1;
    if first_size.is_power_of_two() {
        proof.insert(0, *first_root);
    }
    if proof.is_empty() {
        return false;
    }
    // Shift fn/sn right until fn is even (advance past the old tree's
    // right spine).
    while !fn_.is_multiple_of(2) {
        fn_ >>= 1;
        sn >>= 1;
    }
    let mut it = proof.iter();
    let first_node = match it.next() {
        Some(c) => *c,
        None => return false,
    };
    let mut fr = first_node;
    let mut sr = first_node;
    for c in it {
        if sn == 0 {
            return false; // proof too long
        }
        if !fn_.is_multiple_of(2) || fn_ == sn {
            // Node is a right child in both trees (or promoted): combine on
            // the left.
            fr = node_hash(c, &fr);
            sr = node_hash(c, &sr);
            while fn_.is_multiple_of(2) && fn_ != 0 {
                fn_ >>= 1;
                sn >>= 1;
            }
        } else {
            // Node is a left child only in the larger tree.
            sr = node_hash(&sr, c);
        }
        fn_ >>= 1;
        sn >>= 1;
    }
    // The reconstructed old/new roots must match, and sn must be exhausted.
    sn == 0 && &fr == first_root && &sr == second_root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(n: usize) -> Vec<[u8; 32]> {
        (0..n)
            .map(|i| {
                let mut e = [0u8; 32];
                e[..8].copy_from_slice(&(i as u64).to_be_bytes());
                e
            })
            .collect()
    }

    #[test]
    fn mth_matches_manual_small_trees() {
        let e = entries(3);
        // n=1
        assert_eq!(merkle_tree_hash(&e[..1]), leaf_hash(&e[0]));
        // n=2: node(leaf0, leaf1)
        assert_eq!(
            merkle_tree_hash(&e[..2]),
            node_hash(&leaf_hash(&e[0]), &leaf_hash(&e[1]))
        );
        // n=3: k=2 -> node( node(l0,l1), l2 ) — l2 promoted, NOT duplicated
        let left = node_hash(&leaf_hash(&e[0]), &leaf_hash(&e[1]));
        let right = leaf_hash(&e[2]);
        assert_eq!(merkle_tree_hash(&e[..3]), node_hash(&left, &right));
    }

    #[test]
    fn promotion_is_not_duplication() {
        // The whole point: root([A,B,C]) != root([A,B,C,C]). The malleable
        // batch-root construction fails this; the RFC 6962 tree must pass.
        let mut abc = entries(3);
        let abcc = {
            let mut v = abc.clone();
            v.push(abc[2]);
            v
        };
        assert_ne!(merkle_tree_hash(&abc), merkle_tree_hash(&abcc));
        // And appending a genuinely new leaf changes the root.
        let before = merkle_tree_hash(&abc);
        abc.push([9u8; 32]);
        assert_ne!(before, merkle_tree_hash(&abc));
    }

    #[test]
    fn inclusion_proof_round_trips_for_every_leaf_and_size() {
        for n in 1..=33 {
            let e = entries(n);
            let root = merkle_tree_hash(&e);
            for m in 0..n {
                let path = inclusion_path(m, &e).expect("in range");
                assert!(
                    verify_inclusion(&leaf_hash(&e[m]), m, n, &path, &root),
                    "inclusion must verify (n={n}, m={m})"
                );
                // A tampered leaf must fail.
                assert!(
                    !verify_inclusion(&leaf_hash(&[0xff; 32]), m, n, &path, &root),
                    "tampered leaf must not verify (n={n}, m={m})"
                );
            }
        }
    }

    #[test]
    fn consistency_proof_round_trips_for_all_prefixes() {
        for n in 1..=33 {
            let e = entries(n);
            let second_root = merkle_tree_hash(&e);
            for m in 1..=n {
                let first_root = merkle_tree_hash(&e[..m]);
                let proof = consistency_proof(m, &e).expect("valid m");
                assert!(
                    verify_consistency(m, &first_root, n, &second_root, &proof),
                    "consistency must verify (m={m}, n={n})"
                );
            }
        }
    }

    #[test]
    fn consistency_rejects_a_forked_history() {
        // Two logs that agree on the first m leaves but diverge afterward
        // must NOT produce a passing consistency proof against each other's
        // later root.
        let e = entries(16);
        let m = 5;
        let first_root = merkle_tree_hash(&e[..m]);
        let honest_root = merkle_tree_hash(&e);
        let proof = consistency_proof(m, &e).unwrap();
        // Tamper with the final root: consistency must fail.
        let mut forked = honest_root;
        forked[0] ^= 0x01;
        assert!(!verify_consistency(m, &first_root, 16, &forked, &proof));
        // Tamper with the claimed old root: must fail.
        let mut bad_first = first_root;
        bad_first[0] ^= 0x01;
        assert!(!verify_consistency(m, &bad_first, 16, &honest_root, &proof));
    }

    #[test]
    fn consistency_equal_size_is_identity() {
        let e = entries(7);
        let r = merkle_tree_hash(&e);
        assert_eq!(consistency_proof(7, &e).unwrap(), Vec::<[u8; 32]>::new());
        assert!(verify_consistency(7, &r, 7, &r, &[]));
        let mut r2 = r;
        r2[0] ^= 1;
        assert!(!verify_consistency(7, &r, 7, &r2, &[]));
    }

    #[test]
    fn out_of_range_queries_are_none() {
        let e = entries(4);
        assert!(inclusion_path(4, &e).is_none());
        assert!(consistency_proof(0, &e).is_none());
        assert!(consistency_proof(5, &e).is_none());
    }
}
