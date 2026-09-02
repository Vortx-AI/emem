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

### 7a. Open questions

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

**And it was idle.** On 2026-09-02 the log stood at 1,539,209 entries with
`head_is_witnessed: false`, three distinct witnesses, and the freshest
co-signature 58,145 entries behind. The signatures that existed were made by
hand and stopped. The mechanism that makes a multi-node network safe **without
consensus** was built, deployed, documented, and not running.

It runs now. `scripts/witness_peers.py` fetches every peer's `/v1/log/sth`,
verifies the responder's signature over the PreimageV1 bytes, proves growth
from the head this node last co-signed with `/v1/log/consistency` (an exact
port of `emem_attest::translog::verify_consistency`, checked against four
served proofs and a tampered one before its first signature left the box),
and co-signs. `deploy/systemd/emem-witness.timer` runs it hourly. Later that
day `head_is_witnessed` was `true` at 1,541,075, zero entries behind. Each new
node installs the same unit pointed at the other three; the pin it keeps in
`~/.config/emem/witness_state.json` is the evidence a peer's consistency proof
must satisfy.

Two things the controls caught before they shipped, kept here because every
node operator will write this job once. A verifier written from memory of RFC
6962 had the spine-advance parity inverted: it rejected every real proof while
still rejecting tampered ones, which looks like rigour and is not. And
"same tree_size" is not "unchanged": a log that rewrites an entry in place
keeps its count and changes its root, and the first draft co-signed that
without comparing roots. The job now refuses a same-size, different-root head
as a rewrite, and a dry run never advances the pin, because the pin records
what was co-signed, not what was looked at.

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

## 9. What the DePIN and open-protocol peers built that we did not

Read §8b first: no token, no chain, no consensus. This section is about what
the physical-infrastructure networks and the federated protocols learned that
needs none of those, and where emem is behind them. Every "emem today" claim
below was checked against the code on 2026-09-02, with paths. Every peer claim
links to the document it came from, in the references at the end.

### 9a. DePIN with the token removed

A DePIN network meets three problems a hosted service never does. The token is
how most of them pay for the answers. The answers are separable from the
token.

**Proof that physical work happened.** Helium's Proof of Coverage: a
challenger picks a target from verifiable entropy (block hash plus an
ephemeral key), the target broadcasts, nearby hotspots witness the packet and
report time of arrival, signal strength and quality, and the challenger
submits those receipts. Verification is deterministic replay. Gaming was
handled first by physics (RF has range) and later by a witness-similarity
denylist. emem's equivalent is the fact plane: a band-typed measurement,
signed by the responder that made it, with the upstream named. What emem lacks
is the **witness receipt**: nobody but the responder signs that a fact was
served. A dispute needs two signatures on the same bytes, and today there is
one.

**Proof that custody continues.** Filecoin's WindowPoSt: every sealed sector
is challenged inside a 24-hour proving period cut into 48 deadlines, and a
missed proof is a fault that forfeits collateral. Take the collateral away and
the mechanism is still there: random challenges, sampled from a commitment,
answered from the bytes. emem's log is exactly such a commitment, 1.54M leaves
under a signed root, and **we sample nothing from it**. The witness job
co-signs a head. It never asks the peer for a leaf.

**Roles with different exposure.** The Graph separates Indexers (stake and
serve), Fishermen (dispute with a Proof of Indexing), and Arbitrators
(decide), with slashing at 2.5% of self-stake. Without stake the roles are
still real. emem has a responder (writes facts), attesters (write notes),
witnesses (co-sign heads), and a reputation score with a leaderboard at
`/v1/contributors` that no code path reads. It has no **auditor** (checks
served bytes against the log) and no working **disputer**: a signed
`disagrees_with` edge exists, and `memory_view` never surfaces an inbound one.

The three jobs a token does, and what does them here:

| Token job | Web3 mechanism | emem today | Gap |
|---|---|---|---|
| Sybil cost on writes | stake (The Graph: 100,000 GRT minimum) | ladder T0 to T5 in `enlistment.rs`; 240 writes/min per attester; DNS TXT and org vouching | the ladder gate runs in **shadow mode by default** (`enlistment_enforcing()`); nothing scales cost with reach |
| Reward for serving | issuance (The Graph: 3% a year) | none; operators run a node because they need the data, the Nostr relay and ATProto PDS model | none needed at four nodes |
| Settlement | on-chain transfer | none | a payment rail without a token exists (9b) |

### 9b. Tokenisation: what it settles, and the rail that replaced it

The thing a token settled in 2021 is settled in 2026 by a header. x402 is an
HTTP 402 flow: the server answers `402` with a `PAYMENT-REQUIRED` header
carrying the requirements, the client retries with `PAYMENT-SIGNATURE`, a
facilitator verifies and settles (stablecoins on several chains, extensible to
card rails), and the server answers with `PAYMENT-RESPONSE`. It is stewarded
by a Linux Foundation body, has no native token, and is in production. The
official MCP registry already lists x402-metered MCP servers next to ours.
Google's Agent Payments Protocol (v0.2, April 2026) supplies the human-mandate
side and carries x402 as an A2A extension.

emem already answers `402`: `ComputeQuotaExceeded` maps to `PAYMENT_REQUIRED`
in `lib.rs`. It is a quota error wearing a payment status. The header is what
tells the two apart, and ours has none, so no agent can mistake the one for the
other today. That stays true until someone decides otherwise.

The decision this document records: **reads are free at every tier, on every
node, and a self-hosted node never sees a price.** The two surfaces where sybil
pressure is real are `SharedEntitySpace` (T3) and `FactPlane` (T4). The public
node may, behind a flag that ships off, answer those in the x402 shape with a
facilitator URL read from the environment. The smaller move comes first: turn
the enlistment gate's enforcement on for `SharedEntitySpace`. The ladder is
built, it is measured, and it is a cost before money is.

Two surfaces promise what no code does, and both are the kind of drift this
project has learned to treat as lying. `Cost.credits` in `receipt.rs` is a
`u64` that is always 0. The error text in the `compute_quota_exceeded` error text tells a throttled
caller that high-score attesters get larger quotas, and nothing reads
`AttesterStats::score()` to widen anything. Given "tiers are records of what
was checked, never scores", the sentence goes, not the code.

### 9c. The missing layers, each with the standard it should speak

1. **Checkpoint interop.** The C2SP checkpoint is three lines (origin, tree
   size in decimal, base64 root) plus signature lines of the form
   `— <name> base64(4-byte key id || signature)`, key id being the first four
   bytes of SHA-256 over `name || 0x0A || 0x01 || pubkey`. A C2SP witness takes
   `POST /add-checkpoint` with an `old <size>` line, the consistency proof,
   a blank line and the checkpoint, and answers 200 with cosignature lines,
   409 with the size it holds, 422 when the proof fails. Our witness surface
   is a JSON body over a private preimage. The wrinkle: our tree hashes with
   blake3, and a C2SP witness checks consistency per RFC 6962 §2.1.2, which is
   SHA-256, so an off-the-shelf witness could co-sign our signature and could
   not check our proof. The fix is a second, SHA-256 tree over the same entry
   bytes, one extra 32-byte hash per append. The payoff is every existing
   witness in the Sigstore and Go ecosystems running against emem with no
   code, and our job witnessing any of their logs.

2. **SCITT receipts.** RFC 9943 (June 2026) is the model emem grew into
   without the envelope: a Transparency Service, Signed Statements
   (`COSE_Sign1`), Receipts (COSE, carrying a proof over a verifiable data
   structure), a Registration Policy, an append-only log. A Relying Party
   "MAY decide to verify only a single Receipt that is acceptable to them".
   Adapter: `Accept: application/cose` on `/v1/log/inclusion` answers with a
   COSE receipt per draft-ietf-cose-merkle-tree-proofs. Same tree caveat: the
   registered algorithm is RFC 9162 over SHA-256, so this waits on item 1.
   Payoff: a supply-chain verifier that has never heard of emem checks an emem
   fact with the tooling it already runs.

3. **Node identity.** A node today is a responder key. `_emem-node` is one
   line of prose at §8d. The agent ladder already follows NIP-05's rule, which
   is "identify, not verify": a DNS name maps to a key, redirects are refused,
   private targets are refused. Do the same for nodes, and publish
   `/.well-known/did.json` (`did:web:emem.dev`) listing the responder key and
   the witness key. ATProto's DID documents separate a signing key from
   rotation keys; emem's revocation is a doc comment in `key.rs`. One JSON
   document, no new cryptography, and every DID and Verifiable Credential
   verifier can resolve our keys.

4. **Portability.** ATProto's promise is that an account migrates to a new
   PDS "without the server's involvement", because a repository is
   self-certifying: a signed commit names an MST root, the tree names records
   by CID, and the whole thing exports as one CAR file. emem has
   `memory_bundle` (the caller picks the triples), `/v1/log/entries`, per-fact
   `GET`, and the air-gapped container. It has no "everything I wrote, with
   its inclusion proofs and the head, as one file", and no import at all.
   This is the feature that makes "voluntary, full privacy" a fact instead of
   a sentence: an agent can leave a node with its memory and its proofs.
   Build `GET /v1/agents/:pubkey/export` (CBOR: the STH, this attester's
   entries, one inclusion proof each) and `emem import`, which re-verifies
   every proof before it writes a byte.

5. **Content addressing interop.** blake3 is multihash `0x1e` in the
   multiformats table (draft). A CIDv1 for an emem fact is
   `0x01 0x55 0x1e 0x20` followed by the 32-byte digest, base32 lower with
   the `b` prefix, no rehashing. Expose it as `cid_v1` next to `fact_cid`.
   IPFS, Filecoin and ATProto tooling then addresses our facts natively. The
   `IpldConnector` stub stays unbound until an operator registers a
   blockstore, as it says.

6. **Replication with proof of custody.** `SegmentBackup` in `merkle_log.rs`
   is a trait with zero implementors. The first one pulls a peer's
   `/v1/log/entries` into a local segment. Then the witness job gains an
   audit step: each tick, derive k indices from the co-signed root, fetch those
   entries and their inclusion proofs from the peer, and check the leaf hash
   and the proof against the root it just co-signed. That is WindowPoSt with
   no ZK and no collateral. A peer that dropped bytes fails inside one tick,
   and the failure is a signed head plus a failed fetch, which is evidence.

7. **Peer discovery without a DHT.** Nostr puts `relays` in the NIP-05
   record; ATProto puts the PDS endpoint in the DID document. Put `peers` and
   `witnesses` (origins and node keys) into `/.well-known/emem.json`. Four
   entries in a signed file. The DHT is refused until there are more nodes
   than a file can hold.

8. **C2PA on rendered pixels.** `cell_scene_rgb` and `scene_png` produce
   images. A C2PA 2.2 manifest (a `c2pa.hash.data` hard binding, a
   `c2pa.actions` `created` entry with `digitalSourceType`, `COSE_Sign1` over
   an X.509 key on the C2PA trust list) lets that image carry provenance into
   browsers and newsrooms that will never speak emem, with the `fact_cid` as
   an assertion. C2PA has no hardware capture attestation; our OS-trace
   substrate is the device half it lacks. Later.

### 9d. Acceptors: where emem is accepted, where it is missing, where it drifted

**Accepted.** The official MCP registry (`io.github.Vortx-AI/emem`, 2.3.0
latest, remote `https://emem.dev/mcp`). The Docker MCP catalog. ghcr for
`emem`, `emem-airgap` and `emem-encode`. PyPI `ememdev` 2.3.0. npm
`@vortxai/emem`. The Dify marketplace.

**Missing.**

- **LlamaIndex.** `sdks/llama-index-tools-emem` is at 2.3.0 and its tests run
  in CI. It is not on PyPI and not in `run-llama/llama_index`, which is where
  LlamaHub lists from. Publish it, then open the upstream PR.
- **n8n.** Not started, per `docs/registries/integration-targets.md`. n8n has
  a community-node registry; Zapier needs their platform. n8n first.
- **The C2SP and SCITT ecosystems.** Acceptors only once we speak the format
  (9c.1, 9c.2). Every witness already running is a free auditor after that.
- **A one-command node.** No compose file exists; there are twenty systemd
  units. "Anyone can host" is priced by this. Write it for the small CPU
  replica first, because that is what eudr.dev is.

**Drifted.**

- **Dify** is live at 2.2.0 while every other surface is 2.3.0. The plugin
  source is not in this repository, so `version_surfaces.py` cannot see it and
  it will drift on every release until it is vendored under `integrations/`
  and added to the bump surfaces.
- **The MCP registry** carries thirteen older versions, 0.0.2 through 2.1.0,
  all `active`. A client that lists the server sees fourteen entries. Mark the
  old ones deprecated from the publisher.
- **The quota promise** in the `compute_quota_exceeded` error text (9b).
- **The Claude connector directory** needs a re-submission; the compliance
  surface is built (`docs/registries/anthropic-claude-connectors-submission.md`).

### 9e. Order of work, with the core kept first

Ranked by what each buys the four-node network per unit of change. Nothing
below touches the fact plane, the token scheme, or the preimages.

1. **Audit sampling in the witness job.** Python, small. "Co-signed" becomes
   "co-signed and spot-checked".
2. **Enforce the ladder on `SharedEntitySpace`.** A flag flip on a measured
   gate. Before any 402.
3. **`peers`, `witnesses` and `_emem-node` for the four nodes, plus
   `did:web`.** Rust, small, one deploy.
4. **Remove the quota promise; drop or wire `Cost.credits`.** Same deploy.
5. **`cid_v1`.** Same deploy.
6. **A compose file for a CPU replica and the first `SegmentBackup`
   implementor.** Medium. This is what brings eudr.dev up.
7. **Namespace export and import.** Medium to large. The portability keystone.
8. **C2SP checkpoint and `add-checkpoint`, with the SHA-256 shadow tree.**
   Medium. Then the SCITT receipt on top of it.
9. **Publish the LlamaIndex tool and open the upstream PR; vendor Dify;
   deprecate the old registry versions.** Owner work, hours.
10. **x402 on T3 and T4 writes, flag off.** A decision before a line of code.
11. **C2PA on rendered scenes.** Later.

Still refused: a token, a chain, consensus, a DHT at this size, and the word
"trustless".

References, all read on 2026-09-02:
x402 <https://www.x402.org/> and <https://github.com/coinbase/x402>;
AP2 <https://ap2-protocol.org/>;
RFC 9943 <https://datatracker.ietf.org/doc/rfc9943/>;
COSE receipts <https://datatracker.ietf.org/doc/draft-ietf-cose-merkle-tree-proofs/>;
C2SP <https://c2sp.org/tlog-checkpoint>, <https://c2sp.org/tlog-witness>, <https://c2sp.org/signed-note>;
AT Protocol <https://atproto.com/guides/overview>, <https://atproto.com/specs/repository>;
NIP-05 <https://github.com/nostr-protocol/nips/blob/master/05.md>;
C2PA 2.2 <https://spec.c2pa.org/specifications/specifications/2.2/specs/C2PA_Specification.html>;
Helium PoC <https://github.com/novalabsxyz/devdocs/blob/master/blockchain/proof-of-coverage.md>;
Filecoin PoSt <https://spec.filecoin.io/algorithms/pos/post/>;
The Graph glossary <https://thegraph.com/docs/en/resources/glossary/>;
multicodec table <https://github.com/multiformats/multicodec/blob/master/table.csv>;
MCP registry <https://registry.modelcontextprotocol.io/v0/servers?search=emem>.
