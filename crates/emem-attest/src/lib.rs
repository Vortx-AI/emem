//! emem-attest — attestation envelope construction, Merkle batching, signing.

#![forbid(unsafe_code)]

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
            let sibling_idx = if *idx % 2 == 0 { *idx + 1 } else { *idx - 1 };
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
        if idx % 2 == 0 {
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
}
