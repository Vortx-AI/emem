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
    /// Field responses (docs/plans/field-tokens.md): the digest of the
    /// (aoi_cid, derivation_cid) binding. Appended after FACT_CIDS so a
    /// receipt without a field binding hashes byte-identically to every
    /// receipt signed before this tag existed. Append-only, like all of
    /// these.
    pub const FIELD: u8 = 0x0a;
    /// Inclusion-proof binding (v2 only): the digest of the receipt's
    /// `merkle_proof`, or the explicit absence marker when it carries
    /// none. See [`merkle_binding_v2`] and [`receipt_preimage_v2`].
    ///
    /// This tag is what makes proof STRIPPING detectable. Under v1 the
    /// signature covered the receipt's fields but not its proof, so an
    /// intermediary could delete `merkle_proof` wholesale and the
    /// signature still verified — the receipt reported `valid: true`
    /// with `merkle_proof_valid: null`, i.e. downgrade by removal.
    pub const MERKLE: u8 = 0x0b;
}

/// Segment tags for the field-binding sub-preimage, hashed into the
/// receipt's FIELD segment: `blake3(domain("field") || tagged(aoi_cid)
/// || tagged(derivation_cid))`. Own module for the same no-drift reason
/// as [`receipt_tag`].
pub mod field_tag {
    pub const AOI_CID: u8 = 0x01;
    pub const DERIVATION_CID: u8 = 0x02;
}

/// The field-binding digest a field receipt carries in its FIELD
/// segment. One rule, used by the signer and every verifier.
pub fn field_binding_v1(aoi_cid: &str, derivation_cid: &str) -> [u8; 32] {
    let mut p = PreimageV1::new("field");
    p.seg(field_tag::AOI_CID, aoi_cid.as_bytes());
    p.seg(field_tag::DERIVATION_CID, derivation_cid.as_bytes());
    p.finalize()
}

/// Wire value of `preimage_version` for the v2 rules: v1 plus a binding
/// over the receipt's inclusion proof.
pub const PREIMAGE_V2: u8 = 2;

/// Segment tags for the merkle-binding sub-preimage hashed into a v2
/// receipt's [`receipt_tag::MERKLE`] segment.
pub mod merkle_tag {
    pub const ROOT: u8 = 0x01;
    pub const LEAF_INDEX: u8 = 0x02;
    pub const PATH: u8 = 0x03;
    pub const RULE_VERSION: u8 = 0x04;
    /// Written instead of the three above when the receipt legitimately
    /// carries no proof, so "no proof" is a signed statement rather than
    /// the absence of one.
    pub const ABSENT: u8 = 0x05;
}

/// The parts of an inclusion proof a v2 receipt binds: `(root, leaf_index,
/// path, rule_version)`.
///
/// Named rather than left as a bare tuple because every caller has to get
/// the order right and a four-element tuple of two 32-byte arrays, a u32 and
/// a u8 gives the compiler nothing to catch a transposition with. The alias
/// puts the field order in one place that the signer, the REST verifier, the
/// storage test helper and the /verify JS mirror can all be read against.
pub type MerkleBinding<'a> = (&'a [u8; 32], u32, &'a [[u8; 32]], u8);

/// The inclusion-proof digest a v2 receipt carries in its MERKLE segment.
///
/// `None` is NOT the same as skipping the segment: it hashes an explicit
/// absence marker. That asymmetry is the whole point. If absence were
/// encoded by omitting the segment, a stripped proof would produce the
/// same digest as a receipt that never had one, and stripping would stay
/// invisible — which is exactly the v1 behaviour being fixed. Under v2 a
/// receipt says, under signature, either "here is my proof" or "I have
/// none", and an intermediary cannot rewrite one into the other.
pub fn merkle_binding_v2(proof: Option<MerkleBinding<'_>>) -> [u8; 32] {
    let mut p = PreimageV1::new("merkle");
    match proof {
        None => {
            p.seg(merkle_tag::ABSENT, &[]);
        }
        Some((root, leaf_index, path, rule_version)) => {
            p.seg(merkle_tag::ROOT, root);
            p.seg(merkle_tag::LEAF_INDEX, &leaf_index.to_le_bytes());
            // The path is length-prefixed as a unit and each element is
            // fixed width, so a truncated or extended path cannot collide
            // with the original.
            let mut body = Vec::with_capacity(path.len() * 32);
            for h in path {
                body.extend_from_slice(h);
            }
            p.seg(merkle_tag::PATH, &body);
            p.seg(merkle_tag::RULE_VERSION, &[rule_version]);
        }
    }
    p.finalize()
}

/// Canonical v2 receipt preimage digest: every v1 segment, plus a
/// trailing MERKLE segment binding the inclusion proof (or its declared
/// absence).
///
/// Kept as a separate function rather than a flag on v1 because every
/// receipt signed before the cutover has to keep verifying byte-for-byte
/// under its original rule. A verifier picks the function from the
/// receipt's own `preimage_version`, and must refuse to verify a v2
/// receipt under v1 rules — otherwise an attacker downgrades the version
/// field and strips the proof exactly as before.
#[allow(clippy::too_many_arguments)]
pub fn receipt_preimage_v2<'a, C, F>(
    request_id: &str,
    served_at: &str,
    scope_hex: Option<&str>,
    as_of_hex: Option<&str>,
    edges_hex: Option<&str>,
    manifest_hex: Option<&str>,
    field_hex: Option<&str>,
    primitive: &str,
    cells: C,
    fact_cids: F,
    merkle_hex: &str,
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
    if let Some(f) = field_hex {
        p.seg(receipt_tag::FIELD, f.as_bytes());
    }
    // Always written, never conditional: an absent proof is signed as
    // absent. See `merkle_binding_v2`.
    p.seg(receipt_tag::MERKLE, merkle_hex.as_bytes());
    p.finalize()
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
    field_hex: Option<&str>,
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
    // FIELD comes last: a receipt without one must hash byte-identically
    // to every receipt signed before the tag existed.
    if let Some(f) = field_hex {
        p.seg(receipt_tag::FIELD, f.as_bytes());
    }
    p.finalize()
}

/// Segment tags for the v1 attestation preimage. Stable wire constants —
/// never renumber. Named for the same reason [`receipt_tag`] is: the
/// generated verifier spec (`GET /v1/verifier_spec`) serializes these
/// symbols rather than re-typed integers, so the published spec cannot
/// drift from the signer.
pub mod attestation_tag {
    pub const BATCH_ROOT: u8 = 0x01;
    pub const REGISTRY_CID: u8 = 0x02;
    pub const SCHEMA_CID: u8 = 0x03;
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
    p.seg(attestation_tag::BATCH_ROOT, batch_root);
    p.seg(attestation_tag::REGISTRY_CID, registry_cid.as_bytes());
    p.seg(attestation_tag::SCHEMA_CID, schema_cid.as_bytes());
    p.finalize()
}

/// Segment tags for the v1 join-request preimage — the digest a node's own
/// key signs to prove it holds that key and wants to be enrolled. Stable wire
/// constants: never renumber, append only.
pub mod join_request_tag {
    /// Join-request schema identifier (`"emem.join_request.v1"`).
    pub const SCHEMA: u8 = 0x01;
    /// The node key asking to be enrolled, base32-nopad lowercase.
    pub const NODE_KEY: u8 = 0x02;
    /// Substrate profile the node intends to write under.
    pub const PROFILE: u8 = 0x03;
    /// Device platform id the node claims to be.
    pub const PLATFORM: u8 = 0x04;
    /// EAT `hwmodel` claim the node reports for itself.
    pub const HWMODEL: u8 = 0x05;
    /// When the request was made, RFC 3339 UTC.
    pub const CREATED_AT: u8 = 0x06;
}

/// Canonical v1 join-request preimage — the 32 bytes a node signs to say
/// "this is my key, and I would like to be enrolled".
///
/// This proves ONE thing: whoever produced the request holds the private half
/// of `node_key`. Everything else in it, the platform, the hardware model, is
/// the node's own claim about itself and is not evidence of anything. That is
/// exactly why a join request is not an enrolment: the endorser reads these
/// claims, decides whether they are true by means outside this protocol
/// (usually by having physically installed the machine), and only then signs a
/// platform attestation. Giving the request its own preimage domain keeps a
/// self-signed request from ever being mistaken for an endorser's signature.
pub fn join_request_preimage_v1(
    schema: &str,
    node_key: &str,
    profile_id: &str,
    platform_id: &str,
    hwmodel: &str,
    created_at: &str,
) -> [u8; 32] {
    let mut p = PreimageV1::new("join_request");
    p.seg(join_request_tag::SCHEMA, schema.as_bytes());
    p.seg(join_request_tag::NODE_KEY, node_key.as_bytes());
    p.seg(join_request_tag::PROFILE, profile_id.as_bytes());
    p.seg(join_request_tag::PLATFORM, platform_id.as_bytes());
    p.seg(join_request_tag::HWMODEL, hwmodel.as_bytes());
    p.seg(join_request_tag::CREATED_AT, created_at.as_bytes());
    p.finalize()
}

/// Segment tags for the v1 custody preimage — the digest a node's key signs
/// to say it held some bytes. Stable wire constants: never renumber, append
/// only.
pub mod custody_tag {
    /// Custody schema identifier (`"emem.custody.v1"`).
    pub const SCHEMA: u8 = 0x01;
    /// blake3 of the payload the node received, base32-nopad lowercase.
    pub const PAYLOAD: u8 = 0x02;
    /// Substrate profile the node is operating under.
    pub const PROFILE: u8 = 0x03;
    /// Device platform id from the device-platform registry.
    pub const PLATFORM: u8 = 0x04;
    /// Wall clock at observation, RFC 3339 UTC.
    pub const OBSERVED_AT: u8 = 0x05;
    /// Byte length of the payload, u64 little-endian.
    pub const SIZE: u8 = 0x06;
    /// The name the payload arrived under, as given. Bound because a file
    /// name is part of what the node received, and a record that omitted it
    /// would let two different arrivals of the same bytes look identical.
    pub const NAME: u8 = 0x07;
    /// Operator's label for the processing stage this payload sits at.
    /// Appended only when present.
    pub const STAGE: u8 = 0x08;
    /// Content id of the `emem.os_trace.v1` whose outputs include this
    /// payload. Appended only when present, which is what lets a record made
    /// before any encoder existed keep hashing byte-identically.
    pub const TRACE: u8 = 0x09;
}

/// Canonical v1 custody preimage — the 32 bytes a node's ed25519 key signs to
/// record that it received a payload.
///
/// What this deliberately does NOT bind: any trace root, any segment, any
/// execution evidence. A custody record is a weaker claim than an OS trace on
/// purpose, and giving it a distinct preimage domain is what stops one being
/// mistaken for the other. A verifier that checks this signature learns that
/// the holder of the node key says these bytes arrived under this name, at
/// this size, at this time. It learns nothing about how they were produced.
#[allow(clippy::too_many_arguments)]
pub fn custody_preimage_v1(
    schema: &str,
    payload_digest: &str,
    profile_id: &str,
    platform_id: &str,
    observed_at: &str,
    size_bytes: u64,
    name: &str,
    stage: Option<&str>,
    trace_cid: Option<&str>,
) -> [u8; 32] {
    let mut p = PreimageV1::new("custody");
    p.seg(custody_tag::SCHEMA, schema.as_bytes());
    p.seg(custody_tag::PAYLOAD, payload_digest.as_bytes());
    p.seg(custody_tag::PROFILE, profile_id.as_bytes());
    p.seg(custody_tag::PLATFORM, platform_id.as_bytes());
    p.seg(custody_tag::OBSERVED_AT, observed_at.as_bytes());
    p.seg(custody_tag::SIZE, &size_bytes.to_le_bytes());
    p.seg(custody_tag::NAME, name.as_bytes());
    // Appended only when present, following the same rule os_trace uses for
    // prev_trace_cid: a record that carries neither hashes exactly as it did
    // before these two segments existed, so nothing already signed is
    // invalidated by adding them.
    if let Some(v) = stage {
        p.seg(custody_tag::STAGE, v.as_bytes());
    }
    if let Some(v) = trace_cid {
        p.seg(custody_tag::TRACE, v.as_bytes());
    }
    p.finalize()
}

/// Segment tags for the v1 OS-trace preimage — the digest a device's key
/// signs over its execution trace (`emem-trace` crate). Stable wire
/// constants — never renumber, append only.
pub mod os_trace_tag {
    /// Trace schema identifier (e.g. `"emem.os_trace.v1"`).
    pub const SCHEMA: u8 = 0x01;
    /// blake3 of the canonical CBOR of the device identity record.
    pub const DEVICE: u8 = 0x02;
    /// Substrate profile ID the device claims to write under.
    pub const PROFILE: u8 = 0x03;
    /// Capture window: `u64-LE start_ns || u64-LE end_ns`.
    pub const WINDOW: u8 = 0x04;
    /// v1 merkle root over the chained segment digests, in chain order.
    pub const TRACE_ROOT: u8 = 0x05;
    /// Digests of the sensor payloads emitted inside the window.
    pub const OUTPUTS: u8 = 0x06;
    /// Content ID of the previous trace in this device's stream. Appended
    /// only when present, so a trace with no predecessor (the stream head)
    /// hashes byte-identically to a pre-chain trace.
    pub const PREV_TRACE: u8 = 0x07;
}

/// Canonical v1 OS-trace preimage digest — the 32 bytes a device's
/// ed25519 key signs. Binds the trace schema, the device identity, the
/// substrate profile, the capture window, the merkle root of the chained
/// trace segments, and every emitted output digest into one signature,
/// so no part of the execution evidence can be swapped after signing.
#[allow(clippy::too_many_arguments)]
pub fn os_trace_preimage_v1<'a, O>(
    schema: &str,
    device_digest: &[u8; 32],
    profile_id: &str,
    window_start_ns: u64,
    window_end_ns: u64,
    trace_root: &[u8; 32],
    output_digests: O,
    prev_trace_cid: Option<&str>,
) -> [u8; 32]
where
    O: IntoIterator<Item = &'a str>,
{
    let mut window = [0u8; 16];
    window[..8].copy_from_slice(&window_start_ns.to_le_bytes());
    window[8..].copy_from_slice(&window_end_ns.to_le_bytes());
    let mut p = PreimageV1::new("os_trace");
    p.seg(os_trace_tag::SCHEMA, schema.as_bytes());
    p.seg(os_trace_tag::DEVICE, device_digest);
    p.seg(os_trace_tag::PROFILE, profile_id.as_bytes());
    p.seg(os_trace_tag::WINDOW, &window);
    p.seg(os_trace_tag::TRACE_ROOT, trace_root);
    p.seg_list(os_trace_tag::OUTPUTS, output_digests);
    // Appended only when present: the stream head (no predecessor) hashes
    // byte-identically to a pre-chain trace, so committed vectors and every
    // signature minted before chaining stay valid.
    if let Some(prev) = prev_trace_cid {
        p.seg(os_trace_tag::PREV_TRACE, prev.as_bytes());
    }
    p.finalize()
}

/// Segment tags for the v1 platform-attestation preimage — the 32 bytes
/// a platform's Endorser key signs to vouch that a device key runs on a
/// genuine, measured instance of a whitelisted platform. Stable wire
/// constants — never renumber.
pub mod platform_attestation_tag {
    /// EAT profile identifier (e.g. `"emem.platform_attestation.v0"`).
    pub const EAT_PROFILE: u8 = 0x01;
    /// Device-platform ID the evidence attests to (e.g. `"nvidia.jetson-orin"`).
    pub const PLATFORM: u8 = 0x02;
    /// The endorsed device public key (the EAT `ueid`), 32 bytes.
    pub const DEVICE_KEY: u8 = 0x03;
    /// EAT `hwmodel` claim.
    pub const HWMODEL: u8 = 0x04;
    /// EAT `oemid` claim.
    pub const OEMID: u8 = 0x05;
    /// Freshness nonce echoed from the enrolment challenge.
    pub const NONCE: u8 = 0x06;
    /// The measured-boot digests (EAT measurements), matched against a
    /// platform's reference values when those ship.
    pub const MEASUREMENTS: u8 = 0x07;
}

/// Canonical v1 platform-attestation preimage digest — the 32 bytes an
/// Endorser (a whitelisted trust anchor) signs to attest that
/// `device_key` runs on a genuine, measured instance of `platform_id`.
/// Binds the EAT profile, the platform, the endorsed device key, the
/// hardware model and OEM, the freshness nonce, and every boot
/// measurement into one signature, so no claim can be swapped after
/// signing. The Endorser key itself is committed to by the platform's
/// trust anchor in the device-platforms manifest.
#[allow(clippy::too_many_arguments)]
pub fn platform_attestation_preimage_v1<'a, M>(
    eat_profile: &str,
    platform_id: &str,
    device_key: &[u8; 32],
    hwmodel: &str,
    oemid: &str,
    nonce: &str,
    measurements: M,
) -> [u8; 32]
where
    M: IntoIterator<Item = &'a str>,
{
    let mut p = PreimageV1::new("platform_attestation");
    p.seg(
        platform_attestation_tag::EAT_PROFILE,
        eat_profile.as_bytes(),
    );
    p.seg(platform_attestation_tag::PLATFORM, platform_id.as_bytes());
    p.seg(platform_attestation_tag::DEVICE_KEY, device_key);
    p.seg(platform_attestation_tag::HWMODEL, hwmodel.as_bytes());
    p.seg(platform_attestation_tag::OEMID, oemid.as_bytes());
    p.seg(platform_attestation_tag::NONCE, nonce.as_bytes());
    p.seg_list(platform_attestation_tag::MEASUREMENTS, measurements);
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
    fn os_trace_preimage_binds_every_segment() {
        let base = os_trace_preimage_v1(
            "emem.os_trace.v1",
            &[7u8; 32],
            "robot.fleet.v1",
            100,
            200,
            &[9u8; 32],
            ["abc", "def"],
            None,
        );
        // Any moved field changes the digest.
        let other_window = os_trace_preimage_v1(
            "emem.os_trace.v1",
            &[7u8; 32],
            "robot.fleet.v1",
            101,
            200,
            &[9u8; 32],
            ["abc", "def"],
            None,
        );
        assert_ne!(base, other_window);
        // Output list boundaries are unambiguous: ["abc","def"] is not
        // ["abcd","ef"], the collision the v0 concatenation rule allowed.
        let shifted = os_trace_preimage_v1(
            "emem.os_trace.v1",
            &[7u8; 32],
            "robot.fleet.v1",
            100,
            200,
            &[9u8; 32],
            ["abcd", "ef"],
            None,
        );
        assert_ne!(base, shifted);
        // The stream link is bound: naming a predecessor changes the
        // digest, and absence (the head) is distinct from any present link.
        let chained = os_trace_preimage_v1(
            "emem.os_trace.v1",
            &[7u8; 32],
            "robot.fleet.v1",
            100,
            200,
            &[9u8; 32],
            ["abc", "def"],
            Some("prevcid"),
        );
        assert_ne!(base, chained);
        // And the domain is separated from the attestation preimage.
        assert_ne!(
            base,
            attestation_preimage_v1(&[9u8; 32], "robot.fleet.v1", "emem.os_trace.v1")
        );
    }

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
        let a = receipt_preimage_v1(
            "rid",
            "t",
            None,
            None,
            None,
            None,
            None,
            "recall",
            ["a,b"],
            ["c"],
        );
        let b = receipt_preimage_v1(
            "rid",
            "t",
            None,
            None,
            None,
            None,
            None,
            "recall",
            ["a"],
            ["b,c"],
        );
        assert_ne!(a, b);
        // Item-boundary shift within one list must also split.
        let c = receipt_preimage_v1(
            "rid",
            "t",
            None,
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
    fn field_segment_changes_digest_and_absent_field_is_pre_field_identical() {
        // The invariant the whole FIELD design leans on: a receipt with no
        // field binding hashes exactly as it did before the tag existed,
        // which is what a hand-built stream without the segment computes.
        let mut p = PreimageV1::new("receipt");
        p.seg(receipt_tag::REQUEST_ID, b"rid");
        p.seg(receipt_tag::SERVED_AT, b"t");
        p.seg(receipt_tag::PRIMITIVE, b"recall");
        p.seg_list(receipt_tag::CELLS, ["c1"]);
        p.seg_list(receipt_tag::FACT_CIDS, ["f1"]);
        let hand_built = p.finalize();
        let without = receipt_preimage_v1(
            "rid",
            "t",
            None,
            None,
            None,
            None,
            None,
            "recall",
            ["c1"],
            ["f1"],
        );
        assert_eq!(
            hand_built, without,
            "absent FIELD must not change the digest"
        );

        let fh: String = field_binding_v1("aoi", "deriv")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let with = receipt_preimage_v1(
            "rid",
            "t",
            None,
            None,
            None,
            None,
            Some(&fh),
            "recall",
            ["c1"],
            ["f1"],
        );
        assert_ne!(without, with, "a FIELD segment must change the digest");

        // And the binding digest itself must length-prefix its parts: the
        // v0 boundary-shift collision must not reappear here.
        assert_ne!(
            field_binding_v1("abc", "def"),
            field_binding_v1("abcd", "ef")
        );
    }

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
            None,
            "emem.recall",
            ["cellA", "cellB"],
            ["fc1"],
        );
        assert_eq!(
            digest, expect,
            "raw stream must hash to receipt_preimage_v1"
        );
    }

    /// Frozen vectors for the v2 merkle binding, so the in-browser JS
    /// mirror can be checked against the signer rather than assumed to
    /// match. A drift here is the "verifier drift" state on /verify.
    fn hexlo(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn v2_merkle_binding_vectors() {
        let absent = merkle_binding_v2(None);
        let root = [0xAAu8; 32];
        let path = [[0x11u8; 32], [0x22u8; 32]];
        let present = merkle_binding_v2(Some((&root, 5u32, &path[..], 1u8)));
        assert_ne!(absent, present, "absence must not hash like a proof");
        let full = receipt_preimage_v2(
            "RID",
            "2026-06-12T00:00:00Z",
            None,
            None,
            None,
            None,
            None,
            "emem.recall",
            ["cellA", "cellB"],
            ["fc1"],
            &hexlo(&present),
        );
        // Frozen. The /verify page reimplements these bytes in JS, and a
        // silent divergence there is precisely the "verifier drift" state
        // that page now reports: the browser recomputes a different digest,
        // the server says the signature is fine, and the page can no longer
        // verify anything independently. Pinning the vectors turns that
        // into a failing build instead. Verified byte-for-byte against an
        // independent reimplementation of the JS before freezing.
        assert_eq!(
            hexlo(&absent),
            "734fa6cf403c2b204f3d86a43cc51873b4d6477326e9c712c5eb51bb303aeab2"
        );
        assert_eq!(
            hexlo(&present),
            "361c2db6e05ce0f66fcee3d2c16d4ae86b348073ef9c0b45ce88300f5593558a"
        );
        assert_eq!(
            hexlo(&full),
            "ce4ab6af99f35f02f5228db976c5ed31b66895474f9f18a7ddad241b6a0606be"
        );
    }

    /// Restoring a receipt's real proof rescues a receipt whose proof was
    /// dropped, and ONLY that receipt.
    ///
    /// `/v1/verify_receipt` uses this to tell a serialiser that reshaped a
    /// receipt apart from an attacker that tampered with one, because both
    /// arrive as a failed signature and calling both "forged" teaches an
    /// agent to distrust the thing that was provable. The distinction is
    /// only safe if restoring the proof cannot rescue anything else, so
    /// that is what this pins: same body + real proof reproduces the signed
    /// digest; same body + no proof does not; a body altered anywhere else
    /// does not, with the real proof back in place.
    #[test]
    fn restoring_a_proof_rescues_only_an_untouched_body() {
        let root = [0xAAu8; 32];
        let path = [[0x11u8; 32], [0x22u8; 32]];
        let real = hexlo(&merkle_binding_v2(Some((&root, 5u32, &path[..], 1u8))));
        let absent = hexlo(&merkle_binding_v2(None));
        let signed = |primitive: &str, cid: &str, merkle: &str| {
            receipt_preimage_v2(
                "RID",
                "2026-06-12T00:00:00Z",
                None,
                None,
                None,
                None,
                None,
                primitive,
                ["cellA"],
                [cid],
                merkle,
            )
        };
        let original = signed("emem.recall", "fc1", &real);
        assert_eq!(
            signed("emem.recall", "fc1", &real),
            original,
            "restoring the recorded proof must reproduce the signed digest"
        );
        assert_ne!(
            signed("emem.recall", "fc1", &absent),
            original,
            "a dropped proof must not hash like the proof it replaced"
        );
        // The two that keep this from becoming a downgrade: a body tampered
        // anywhere else stays broken even with the true proof restored.
        assert_ne!(
            signed("emem.evolve", "fc1", &real),
            original,
            "restoring the proof must not rescue an altered primitive"
        );
        assert_ne!(
            signed("emem.recall", "fc2", &real),
            original,
            "restoring the proof must not rescue an altered fact cid"
        );
    }
}
