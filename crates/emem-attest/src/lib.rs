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
    // Promote each leaf through one hashing pass so single-leaf and
    // multi-leaf trees both produce a *hash* root (closes the
    // leaf-vs-root domain confusion that would otherwise let a raw leaf
    // be claimed as a one-fact attestation root).
    let mut layer: Vec<[u8; 32]> = leaves
        .iter()
        .map(|leaf| {
            let mut h = Hasher::new();
            h.update(leaf);
            h.update(leaf);
            let mut out = [0u8; 32];
            out.copy_from_slice(h.finalize().as_bytes());
            out
        })
        .collect();
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
}
