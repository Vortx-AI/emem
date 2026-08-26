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

/// An RFC 6962 tree that grows by extension instead of rebuilding.
///
/// [`merkle_tree_hash`] is `O(n)` and correct, and for a log that is read
/// far more often than it is written that is the wrong shape: this
/// responder's `/v1/log/sth` cache is keyed on the record count, so ONE
/// arriving leaf re-folded 1.48M of them. Measured from outside at a
/// constant 35 s probe interval, five calls that spanned no append answered
/// in 9-32 ms and the one call that spanned a single append took 2.886 s.
/// The elapsed time was held fixed across all six, so the cost tracked
/// growth, not staleness.
///
/// The saving is a property of append-only trees. Level `l+1` entry `i` is
/// `node(level l [2i], [2i+1])`, and once that pair is complete neither
/// input can ever move, so the entry is final. The only entry of a level
/// that an append can invalidate is its LAST one, because a lone rightmost
/// node is promoted unchanged and may since have gained a sibling. So an
/// extension drops one entry per level and refolds from there: `O(k + log n)`
/// for `k` new leaves, against `O(n)`.
///
/// `levels[0]` holds the promoted [`leaf_hash`] of each entry, so the raw
/// per-record hashes are hashed exactly once, on arrival.
#[derive(Debug, Clone, Default)]
pub struct IncrementalTree {
    levels: Vec<Vec<[u8; 32]>>,
}

impl IncrementalTree {
    /// An empty tree. `root()` is [`empty_root`] until leaves are added.
    pub fn new() -> Self {
        Self::default()
    }

    /// Leaves committed so far.
    pub fn len(&self) -> usize {
        self.levels.first().map_or(0, Vec::len)
    }

    /// Whether the tree holds no leaves.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Append `entries` (raw per-record hashes, in log order) and refold the
    /// right spine.
    pub fn extend(&mut self, entries: &[[u8; 32]]) {
        if entries.is_empty() {
            return;
        }
        if self.levels.is_empty() {
            self.levels.push(Vec::new());
        }
        self.levels[0].extend(entries.iter().map(leaf_hash));
        let mut l = 0usize;
        loop {
            if self.levels[l].len() <= 1 {
                // This level is the root; anything above it is stale.
                self.levels.truncate(l + 1);
                return;
            }
            if self.levels.len() == l + 1 {
                self.levels.push(Vec::new());
            }
            // Drop the one entry an append can invalidate: the last, which
            // may have been a lone promotion that now has a sibling. Every
            // earlier entry is a complete pair of settled inputs.
            let keep = self.levels[l + 1].len().saturating_sub(1);
            self.levels[l + 1].truncate(keep);
            let want = self.levels[l].len().div_ceil(2);
            for i in keep..want {
                let src = &self.levels[l];
                let node = if 2 * i + 1 < src.len() {
                    node_hash(&src[2 * i], &src[2 * i + 1])
                } else {
                    // Lone rightmost node: promoted unchanged, never
                    // self-paired (see the module header on CVE-2012-2459).
                    src[2 * i]
                };
                self.levels[l + 1].push(node);
            }
            l += 1;
        }
    }

    /// Merkle Tree Hash of everything appended so far. Equal, for every
    /// size, to `merkle_tree_hash` over the same entries in the same order —
    /// which is asserted rather than assumed, at every size across several
    /// append plans, in this module's tests.
    pub fn root(&self) -> [u8; 32] {
        match self.levels.last() {
            None => empty_root(),
            Some(top) => match top.first() {
                None => empty_root(),
                Some(r) => *r,
            },
        }
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;

    fn entry(i: usize) -> [u8; 32] {
        let mut e = [0u8; 32];
        e[..8].copy_from_slice(&(i as u64).to_le_bytes());
        e[8] = 0xa5;
        e
    }

    /// The claim the whole endpoint rests on: growing a tree by extension
    /// and folding it from scratch give the same root, at EVERY size, no
    /// matter how the leaves were grouped on the way in. Sizes and grouping
    /// are separated deliberately — a bug in the right-spine refold shows up
    /// only when a lone promoted node later gains a sibling, which depends
    /// on where the chunk boundaries fall, not on the final size.
    #[test]
    fn incremental_root_matches_the_naive_fold_at_every_size() {
        const N: usize = 400;
        let all: Vec<[u8; 32]> = (0..N).map(entry).collect();
        // Chunk plans: one at a time, powers of two either side of a level
        // boundary, primes (so boundaries land off every alignment), and a
        // deliberately irregular one.
        let plans: Vec<Vec<usize>> = vec![
            vec![1],
            vec![2],
            vec![3],
            vec![4],
            vec![5],
            vec![7],
            vec![8],
            vec![16],
            vec![N],
            vec![1, 2, 3, 5, 8, 13, 21],
            vec![9, 1, 1, 1, 17, 2],
            vec![63, 1, 64, 1],
        ];
        for plan in &plans {
            let mut tree = IncrementalTree::new();
            let mut fed = 0usize;
            assert_eq!(
                tree.root(),
                merkle_tree_hash(&all[..0]),
                "empty tree must be the empty root, plan {plan:?}"
            );
            let mut step = 0usize;
            while fed < N {
                let take = plan[step % plan.len()].min(N - fed);
                tree.extend(&all[fed..fed + take]);
                fed += take;
                step += 1;
                assert_eq!(tree.len(), fed, "leaf count drifted, plan {plan:?}");
                assert_eq!(
                    tree.root(),
                    merkle_tree_hash(&all[..fed]),
                    "root diverged at size {fed} with plan {plan:?}"
                );
            }
        }
    }

    /// The control. A checker that cannot fail proves nothing, and the
    /// specific way this optimisation goes wrong is subtle: keep a lone
    /// promoted node instead of refolding it once it gains a sibling and
    /// every root is still a plausible 32 bytes. So break it exactly that
    /// way and require the comparison above to catch it.
    ///
    /// It must be caught at n=4, the first size where a promoted node
    /// acquires a sibling: leaves [0,1,2] promote node(0,1) and leaf 2, and
    /// the arrival of leaf 3 turns leaf 2 into the left half of a pair.
    #[test]
    fn a_broken_increment_is_caught_at_four_leaves() {
        /// `extend` with the one line that matters removed: the last entry
        /// of each level is KEPT rather than refolded.
        fn extend_without_refolding(t: &mut IncrementalTree, entries: &[[u8; 32]]) {
            if t.levels.is_empty() {
                t.levels.push(Vec::new());
            }
            t.levels[0].extend(entries.iter().map(leaf_hash));
            let mut l = 0usize;
            loop {
                if t.levels[l].len() <= 1 {
                    t.levels.truncate(l + 1);
                    return;
                }
                if t.levels.len() == l + 1 {
                    t.levels.push(Vec::new());
                }
                let keep = t.levels[l + 1].len(); // <- the defect
                let want = t.levels[l].len().div_ceil(2);
                for i in keep..want {
                    let src = &t.levels[l];
                    let node = if 2 * i + 1 < src.len() {
                        node_hash(&src[2 * i], &src[2 * i + 1])
                    } else {
                        src[2 * i]
                    };
                    t.levels[l + 1].push(node);
                }
                l += 1;
            }
        }

        let all: Vec<[u8; 32]> = (0..4).map(entry).collect();
        let mut broken = IncrementalTree::new();
        for e in &all {
            extend_without_refolding(&mut broken, std::slice::from_ref(e));
        }
        assert_ne!(
            broken.root(),
            merkle_tree_hash(&all),
            "the broken increment produced the CORRECT root, so this test \
             cannot detect the bug it exists to detect"
        );

        // And the real one, fed identically, does not.
        let mut good = IncrementalTree::new();
        for e in &all {
            good.extend(std::slice::from_ref(e));
        }
        assert_eq!(good.root(), merkle_tree_hash(&all));
    }

    /// An incrementally grown root has to be usable, not merely equal: every
    /// leaf's inclusion path must still verify against it. This is the
    /// property an auditor actually exercises after pinning an STH.
    #[test]
    fn inclusion_paths_verify_against_an_incrementally_grown_root() {
        const N: usize = 70;
        let all: Vec<[u8; 32]> = (0..N).map(entry).collect();
        let mut tree = IncrementalTree::new();
        for (i, e) in all.iter().enumerate() {
            tree.extend(std::slice::from_ref(e));
            let size = i + 1;
            let root = tree.root();
            for m in 0..size {
                let path = inclusion_path(m, &all[..size]).expect("path in range");
                assert!(
                    verify_inclusion(&leaf_hash(&all[m]), m, size, &path, &root),
                    "leaf {m} of {size} failed to verify against the incremental root"
                );
            }
        }
    }

    /// Extending by nothing is not a way to change the tree.
    #[test]
    fn extending_with_no_entries_leaves_the_root_alone() {
        let all: Vec<[u8; 32]> = (0..5).map(entry).collect();
        let mut tree = IncrementalTree::new();
        tree.extend(&all);
        let before = tree.root();
        tree.extend(&[]);
        assert_eq!(tree.root(), before);
        assert_eq!(tree.len(), 5);
    }
}
