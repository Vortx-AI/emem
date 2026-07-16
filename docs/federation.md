# Federation & scale-out (design)

> Status: **design**. None of the multi-host routing below ships yet in the 1.x line.
> What ships today is the substrate that makes it safe: content
> addressing, signed receipts, multi-attester contradiction scoring, and a
> deterministic refinement loop. This document is the plan for turning one
> hosted responder + self-hosted nodes into a federation, and for breaking
> the single-host write ceiling along the way.

## 1. Why scale out: the single-host ceiling

emem today is one process: an `emem-server` binary over a single embedded
`sled` store plus an append-only merkle log. Two limits are already
visible in production:

- **Write contention is a single-writer bottleneck.** Every cold write
  funnels through one `sled` instance and one append-only merkle log. The
  log append was moved off the async worker (group-fsync via
  `spawn_blocking`) and the per-write index flushes were coalesced, but the
  *ordering* is still a single writer. On 2026-06-02 a fan-out of ~96
  in-flight materialise tasks (16 cells × 6 bands on one EUDR plot)
  saturated sled + the HTTP client and wedged `/v1/recall` for *every*
  cell. The fix that day was to cap concurrency; the structural fix is to
  stop funnelling all writes through one store.
- **The runtime has silently stalled** (2026-05-31, -06-12, -06-15): the
  process stays alive but the tokio runtime wedges (CLOSE-WAIT pileup,
  accept backlog saturates). The `emem-watchdog` turns that into a
  ~2-minute self-heal, but a single process is a single failure domain.

Vertical tuning (done in Phases 0 and 1) lowered the constant factors. The
ceiling itself (one writer, one failure domain, one machine's RAM/disk/GPU)
is structural. Scale-out removes it.

## 2. What already makes federation possible

Federation is unusually cheap here because the hard part (*trust without a
central authority*) is already solved by the data model:

- **Content addressing.** A fact id is `blake3(canonical_cbor(fact))`. The
  same `(cell, band, tslot, value, derivation)` hashes to the same id on
  every machine. An id *is* the bytes' fingerprint, so any node can serve
  any id and the client re-derives the hash to confirm it got the right
  bytes.
- **Offline-verifiable signatures.** Every receipt is ed25519-signed over a
  domain-separated, length-prefixed preimage and verifies against the
  responder's public key with no call back to that responder. A client
  trusts the *signature*, not the server.
- **Registry CIDs pin provenance.** Every receipt carries `bands_cid`,
  `algorithms_cid`, `sources_cid`, `schema_cid`. A peer running drifted
  registries returns a different `bands_cid` on `/health`; divergence is
  visible *before* any data flows.
- **Read-resolution already exists.** `/v1/fetch` resolves a fact by id;
  `memory_token` / `memory_bundle` mint `fact_url`s explicitly meant to be
  "handed to any other peer"; `responder_pubkey_b32` is surfaced so callers
  can detect a multi-responder setup.
- **Disagreement is already a first-class value.** The multi-attester index
  + `memory_contradictions` score where two attesters signed different
  values at the same `(cell, band, tslot)`, per band kind (scalar spread /
  vector cosine / categorical mode-share). Federation makes "two attesters"
  mean "two nodes" without changing the scoring.
- **Refinement is deterministic.** A fact is re-derived when a newer
  attestation or a `disagrees_with` edge lands; the same loop works
  whether the trigger is local or from a peer.

The model is effectively a **content-addressed CRDT**: facts merge by id,
and conflicts are *recorded* (multi-attester + `disagrees_with` edges), not
resolved by a vote. That is the property that lets independent nodes
converge without consensus.

## 3. End state

Many responders, one address space. A content id means the same bytes
everywhere; every responder signs under its own key; a client trusts the
signature, not the server. Where two responders disagree at a key, the
network records it, and the shared memory gets *more* trustworthy as more
agents read and write against it.

## 4. Design, in phases

Each phase is independently shippable and useful on its own. Read
federation first (lowest risk, immediate value); write sharding second (the
contention fix); cross-node disagreement third; automatic routing last.

### 4a: Read federation (peer resolve)  ·  *the thin prototype*

On a local id miss, fetch the id from configured peers and **verify before
trusting**: re-derive `blake3(canonical_cbor(fact)) == requested_id` and
check the responder signature against the peer's published pubkey. Cache
the verified fact locally (it is now equally ours; the id proves it).

- Config: `EMEM_PEERS=https://a.example,https://b.example` (static list).
- Surface: extend `/v1/fetch` (and `memory_token/resolve`) so a 404
  becomes a peer fan-out → first verified hit wins.
- Trust: a peer that returns bytes whose hash ≠ the requested id, or a bad
  signature, is ignored (and the mismatch logged). **No peer is trusted to
  be honest, only to be checkable.**
- Risk: low. Read-only, verify-before-cache, no write-path change.

This is the recommended first build; see §6.

### 4b: Write sharding (break the single-writer ceiling)

Shard the write path by `cell64` prefix. The grid is **Hilbert-ordered**,
so a contiguous `cell64` prefix range is a contiguous patch of Earth: a
node that owns a prefix range owns a spatially coherent region, and its
sled write load + spatial locality are both bounded.

- Each node owns one or more `cell64` prefix ranges; writes for a cell go
  to its owner (or are forwarded there). N owners ⇒ N parallel
  single-writers instead of one global one.
- The merkle log shards per owner-range: parallel append-only logs, each a
  single writer, each independently snapshot/replayable. The global
  inclusion-proof story becomes per-shard proofs + a shard directory (see
  §4d) rather than one global tree.
- Reads stay global via §4a: any node resolves any id from the owner (or a
  cached copy) and verifies offline.

### 4c: Cross-node disagreement (federate the multi-attester index)

When a node resolves an id from a peer and already holds a *different*
value at the same `(cell, band, tslot)`, append it to the multi-attester
index exactly as a second local attester would. `memory_contradictions`
then scores cross-node disagreement with zero new machinery, and a
`disagrees_with` edge triggers the existing refinement loop.

### 4d: Routing / directory (which node owns a cell)

Start static, evolve to gossip:

1. **Static ownership map** (config): `cell64`-prefix → owner URL, plus a
   peer list. Deterministic, debuggable, good enough for a handful of
   nodes.
2. **Rendezvous / consistent hashing** on prefix ranges so ownership
   rebalances predictably as nodes join/leave.
3. **Gossip directory**: nodes advertise which `cell64` ranges they hold
   (and their pubkey + registry CIDs), so resolution and write-routing stop
   needing static config. This is the only genuinely new distributed
   component; everything before it is config + the existing verify path.

## 5. Trust model

No node trusts another node's *compute*; it only verifies its *output*:

- **Bytes**: re-derive the blake3 id; reject on mismatch.
- **Signature**: ed25519 verify offline against the peer's published
  pubkey; reject on mismatch.
- **Registry alignment**: compare the peer's four manifest CIDs against
  ours; a drifted peer is *usable but flagged* (its facts land as a
  distinct attester, so disagreements surface in contradiction scoring
  rather than silently overwriting).

There is no global lock, no cross-node transaction, no leader. Convergence
is by content address; conflict is recorded, not voted on.

## 6. The first build: a peer-resolve prototype

The smallest slice that proves federation end-to-end and is safe on the
live serving path:

```
EMEM_PEERS=https://emem.dev,https://mirror.example   # static peer list

GET /v1/fetch/<fact_cid>:
  1. local hit?            → serve (unchanged)
  2. local miss + peers?   → for each peer, GET /v1/fetch/<cid>:
       a. re-derive blake3(canonical_cbor(body)) == <cid>   else skip peer
       b. verify ed25519 receipt signature vs peer pubkey   else skip peer
       c. first peer that passes both → cache locally + serve, citing the
          originating responder pubkey in the response
  3. no peer has it        → typed 404 (unchanged)
```

Properties: read-only, verify-before-cache, no write-path or schema
change, no new distributed state. It exercises content addressing +
offline verification across a process boundary (the whole trust model)
without committing to sharding or routing. Everything in §4b through §4d builds on
the verify primitive this establishes.

## 7. Non-goals (for now)

- **No consensus protocol.** Facts merge by id; disagreements are recorded.
  A BFT/raft layer would contradict the "trust the signature, not the
  server" model.
- **No global deletion guarantee.** Content-addressed + replicated data
  cannot be unilaterally un-published from peers; deletion/redaction across
  a federation is an open policy question (see §8), not a v1 feature.
- **No cross-node compute trust.** A node never runs another node's
  algorithm and trusts the result; it re-derives or re-verifies.

## 8. Open questions

- **Prefix-ownership rebalancing** when a node joins/leaves mid-write:
  hand-off protocol + the window where two nodes think they own a range.
- **Pubkey distribution / rotation**: bootstrapping which pubkeys to trust
  (the `key_epoch` field anticipates rotation; the distribution channel is
  undesigned).
- **Per-shard merkle proofs.** The global inclusion story under §4b: a
  client verifying a fact needs the right shard's proof + a trustable shard
  directory.
- **GDPR / redaction** across replicas (see §7 non-goal); likely a
  signed-tombstone + refusal-to-serve convention rather than true deletion.

---

*Companion to the "Where this is going" section of the README and the
`connect-and-evolve` doc. The federation diagram is at
`docs/diagrams/federation.png`.*
