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

Shard the write path by `cell64` prefix. **The premise this plan used to
rest on is false and the plan needs re-deriving before anyone builds it.**

It said the grid is Hilbert-ordered, so a contiguous prefix range is a
contiguous patch of Earth. The active codec is 21 bits of latitude by 22
of longitude, and a Hilbert curve requires equal-bit axes, which
`crates/emem-codec/src/geo.rs` states plainly: "Hilbert locality at the
cell-key level is dropped here". Only the bigram ALPHABET is
Hilbert-ordered.

Measured against the live responder rather than argued: from one origin,
a neighbour 10 m north and a cell 1 km north share the SAME two leading
bigrams of four. A neighbour 10 m east shares three. So prefix depth
separates the longitude axis from the latitude axis and carries no
information about distance along latitude, which is exactly the property
a spatial shard would need.

What survives: prefix ranges still partition the space disjointly, so
sharding by prefix still gives N parallel single-writers, which was the
load-bearing half. What does not survive is "spatially coherent region":
a node owning a prefix range owns a set of cells that is not a patch, so
any argument about locality of upstream fetches, tile reuse or cache
warmth has to be re-derived or dropped.

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

### 5a. Enlistment: who may write, and what they proved

Federation makes the write question sharper, not softer. One responder's bad
entity binding becomes every peer's bad entity binding, so the property that
has to survive scale-out is **who was entitled to write, checkable by a third
party that trusts neither node**.

The objection that forced this is worth quoting, because it is correct and it
is not "add OAuth". `NousResearch/hermes-agent#79583`, closed 2026-08-25:

> a no-auth shared memory that arbitrary agents read AND write is a textbook
> cross-agent prompt-injection and data-poisoning surface: anyone can plant
> content that other agents will recall as fact.
>
> Signed provenance mitigates attribution, not injected-instruction risk.

The second sentence is the one to internalise. A signature says *who* wrote a
thing. It does not stop a reader obeying it, and it does not say the writer was
entitled to write *there*. Anyone can mint an ed25519 key, so "signed" is a
floor, not a bar.

**Reads are never gated, at any tier, on any surface.** A reader cannot poison
anything, and gating reads would trade the one property that makes this
substrate worth using for no security gain at all. There is no account, no
bearer token that grants anything, and no payment anywhere in the ladder.

**Writes are gated on blast radius, never on identity.** The question is not
"how much do we trust this key" but "how far does what it writes reach, and
what did it prove commensurate with that":

| Surface | Min tier | Why |
|---|---|---|
| read anything | `T0` | never gated |
| own-namespace prose | `T1` | the floor, and free: a stranger's agent writes on first contact with nothing but a signature |
| shared entity address space | `T3` | `entity` + `entity_link` change what *every* agent resolves a name to |
| fact plane | `T4` | no caller can write a fact today by any route; this states the rule rather than relying on the absence of a door |

A **tier is a record of which check passed, never a score**. `trust:
caller_decides` is the best property on the roster and this must not erode it.
No tier says a party is trustworthy; each says what was verified and what a
peer may conclude. The full ladder is served, machine-readable, at
`GET /v1/enlist`.

**Why DNS and `.well-known` rather than OAuth.** Browser OAuth 2.1 + Dynamic
Client Registration authenticates a *session*: did a human, in a browser, just
now authorise this client. An autonomous agent has neither, so DCR degrades to
a bearer token that proves possession and says nothing about accountability,
and it is structurally uncompletable headless. What an agent needs
authenticated is the **principal**: who is accountable for what this key says.
That is name control, solved three times already by DKIM, ACME and Certificate
Transparency.

    _emem-agent.vortx.ai   TXT   "v=emem1; k=<52-char key>; nick=cosmos-eye"

The decisive property is **re-verification by a third party**. A bearer token
proves nothing to a third agent; a DNS record proves the same thing to
everyone, for ever, without trusting the responder that recorded it, and it
survives that responder's compromise, because the evidence does not live on
its disk. That is the same argument that makes emem's receipts worth having,
turned on identity. For federation it is the load-bearing one: a peer node can
re-check an affiliation itself instead of inheriting our verdict.

Every attestation carries `checked_at`, is rendered with its age, and expires,
because a check from last month is a claim about the present made from the
past. Verification targets must be public names: IP literals, local names, and
any name resolving into private, loopback, link-local or CGNAT space are
refused, and redirects are not followed. The residual gap between resolving a
name and connecting to it is not closed, and is documented as a bound rather
than described as safe.

**Shipping state.** The ladder, the checks and the gate are live; enforcement
is off by default (`EMEM_ENLISTMENT_ENFORCE=1`). `entity` and `entity_link`
have accepted anonymous writes since they shipped, so a gate turned on in one
deploy would break every peer mid-flight to close a hole that has been open for
months. Shadow mode records exactly who would be refused and why, and returns
that verdict to the caller in the response rather than only to a log, so a
writer learns before the day it matters.

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

## 8. The four-node network, and anyone after that

The nodes, decided 2026-09-02:

| Node | Hardware | Role |
|---|---|---|
| `emem.dev` | A100, moving to AWS | full node: materialisers, GPU paths, the write plane it has today |
| `eudr.dev` | small CPU | read replica, verifier, witness |
| `vrtx.ai` | good CPU | read replica, witness, writes under its own attester namespace |
| `geo.qa` | good CPU | read replica, witness, writes under its own attester namespace |
| anyone else | theirs | read federation and witnessing, writes under their own namespace |

### 8a. Phase 0 is not new code. It is turning on what is already built.

`/v1/log/witness` accepts an ed25519 co-signature over `(tree_size, root)` and
verifies both the signature and that the root matches the tree at that size.
`/v1/log/consistency` proves one head is an append-only prefix of another.
`/v1/log/witnesses` lists what has been co-signed. All of it ships today.

**And it is idle.** On 2026-09-02 the log stood at 1,539,209 entries with
`head_is_witnessed: false`, three distinct witnesses, and the freshest
co-signature 58,145 entries behind. Nothing in `scripts/` drives it; the
signatures that exist were made by hand and stopped. The mechanism that makes a
multi-node network safe **without consensus** is built, deployed, documented,
and not running.

So the first phase is a scheduled job on each node: fetch every peer's
`/v1/log/sth`, verify it, co-sign it, POST it back, and check
`/v1/log/consistency` against the head that node last saw. Four nodes, four
cron entries, no server change.

What that buys, precisely: **a node cannot show two different histories to two
different peers without one of them holding a signed head that fails a
consistency proof.** That is Certificate Transparency's gossip property, and it
is the whole reason this network does not need a chain.

### 8b. What we take from web3, and what we refuse

Taken, because they were the real advances and emem already has them:
content addressing, so an id is a fingerprint and any node may serve it;
key-based identity with no accounts; Merkle inclusion proofs; and gossiped
transparency logs, which are Certificate Transparency's idea rather than a
blockchain's.

Refused, each for a stated reason:

- **No token, and no incentive layer.** A token turns "who may write" into a
  market and creates the sybil pressure it then has to defend against. Nothing
  here is scarce: the expensive resource is upstream fetches, and those are
  already bounded per node.
- **No global consensus, no chain, no validator set.** This is a
  content-addressed CRDT: facts merge by id and **conflict is recorded, not
  voted on**. Consensus would be strictly worse than what exists, because it
  forces one answer where two attesters legitimately disagree, and that
  disagreement is the signal `memory_contradictions` exists to surface. There
  is nothing to capture because there is no leader.
- **No on-chain anchoring as a dependency.** A node may anchor a tree head
  anywhere it likes; nothing may require it, or the network inherits that
  chain's liveness and cost.
- **No "decentralised therefore trustless".** Every node is trusted for
  nothing and verified for two things: the bytes hash to the id, and the
  signature checks against a published key. That is the entire trust model and
  it does not improve with more nodes.

### 8c. The three attacks, and which one is actually open

**Serving wrong bytes: closed.** A peer returning bytes whose blake3 does not
equal the requested id is detected by the reader, always, without trusting
anyone. A hostile node can withhold, not forge.

**Split view: closed by 8a, open until then.** A node showing different
histories to different peers is exactly what witnessing detects, and today
nothing is witnessing.

**Sybil writes: open, and not closable by identity.** Anyone can mint unlimited
ed25519 keys, so counting signatures is worthless and always will be. The
answer is not to gate identity -- that would cost the property this substrate
exists for -- but that **weight is the reader's judgement, not the network's**:
`trust: caller_decides`. Sybils multiply signatures, not credibility. What the
network owes a reader is the evidence to judge with: who signed, what they
proved (the T0-T4 ladder), whether that proof is re-checkable by a third party
(DNS/`.well-known`, per §5a), and where signers disagree. A reader who weighs
an unaffiliated key the same as a DNS-verified one has made a choice; a network
that made it for them would be the thing we are avoiding.

The honest bound: this makes poisoning **attributable and visible**, not
impossible. A reader that ignores provenance is still poisonable, and no
protocol fixes that.

### 8d. Order of work

1. **Witness mesh across the four.** No server change. Closes split view.
2. **Peer resolve** (§4a, `EMEM_PEERS`). Not implemented today -- grep finds no
   `EMEM_PEERS` in any crate. Read-only, verify-before-cache.
3. **Node identity in DNS.** Extend §5a from agents to nodes:
   `_emem-node.eudr.dev TXT "v=emem1; k=<pubkey>"`, so a peer's key is
   re-checkable by a third party rather than asserted by a config file.
4. **Cross-node disagreement** (§4c): a peer's facts land as a distinct
   attester, so existing contradiction scoring works unchanged.
5. **Routing** (§4d) last, and only if read federation proves insufficient.

Write sharding (§4b) stays where §4b leaves it: its premise was found false and
it needs re-deriving before anyone builds it.

### 8e. What each new node must publish before it counts as joined

`/v1/log/sth`, a stable `responder_pubkey_b32`, the four registry CIDs, and a
DNS record binding its key to its name. A node that serves reads without these
is usable and unaccountable, which is fine for a mirror and not fine for a
writer.

---

*Companion to the "Where this is going" section of the README and the
`connect-and-evolve` doc. The federation diagram is at
`docs/diagrams/federation.png`.*
