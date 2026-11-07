# Federation and scale-out (design)

This page is a design. Most of it does not ship. The table below says what
runs today and what does not. The rest of the page explains the plan and the
reasons for it.

| Part | State in 2.4.4, checked on 6 November 2026 |
|---|---|
| Content addressing, signed receipts, offline verification | ships |
| Multi-attester index and `memory_contradictions` scoring | ships |
| Transparency log with witness co-signatures (`/v1/log/*`) | ships |
| Witness job `scripts/witness_peers.py` and its timer | ships, runs on emem.dev every 15 minutes |
| Node identity: `/.well-known/did.json`, `_emem-node` DNS check | ships |
| `federation` block in `/.well-known/emem.json` | ships |
| Enlistment ladder (`GET /v1/enlist`) | ships; enforcement is off by default |
| `cid_v1` beside `fact_cid` on recall | ships |
| `docker-compose.yml` for a witness node | ships |
| Read federation (peer resolve, §4a) | not built |
| Write sharding (§4b) | not built; its first premise was false |
| Cross-node disagreement (§4c) | not built |
| Routing and directory (§4d) | not built |
| C2SP checkpoints, SCITT receipts, export and import (§9c) | not built |

## 1. Why scale out: the single-host ceiling

Today emem is one process. The `emem-server` binary writes to one embedded
`sled` store and one append-only Merkle log. Production showed two limits.

The first limit is one writer. Every cold write goes through one `sled`
instance and one log. Two changes lowered the cost: the log append moved off
the async worker (group fsync in `spawn_blocking`), and per-write index
flushes were merged. The order of writes still comes from one writer. On
2026-06-02, about 96 materialise tasks ran at the same time (16 cells by 6
bands on one EUDR plot). They saturated `sled` and the HTTP client, and
`/v1/recall` stopped for every cell. The fix that day was a concurrency cap.
The structural fix is to stop sending all writes through one store.

The second limit is one failure domain. On 2026-05-31, 2026-06-12 and
2026-06-15 the process stayed alive but the tokio runtime stopped. Sockets
piled up in CLOSE-WAIT and the accept backlog filled. `emem-watchdog` restarts
the process in about 2 minutes. One process is still one failure domain.

Tuning in phases 0 and 1 made each write cheaper. It did not remove the
ceiling: one writer, one failure domain, and the RAM and disk of one machine.
Scale-out removes it.

## 2. What already makes federation possible

The hard part of federation is trust without a central authority. The data
model already gives that.

- Content addressing. A fact id is `blake3(canonical_cbor(fact))`. The same
  `(cell, band, tslot, value, derivation)` gives the same id on every machine.
  Any node can serve any id. The client hashes the bytes again to confirm it
  got the right ones.
- Offline signatures. Every receipt carries an Ed25519 signature over a
  domain-separated, length-prefixed preimage. It verifies against the
  responder's public key with no call back to that responder. A client trusts
  the signature, not the server.
- Registry CIDs. Every receipt carries `bands_cid`, `algorithms_cid`,
  `sources_cid` and `schema_cid`. A peer with different registries shows a
  different `bands_cid` on `/health`. You see the difference before any data
  moves.
- Fetch by id. `/v1/fetch` returns a fact by id. `memory_token` and
  `memory_bundle` make `fact_url` values that any peer can use.
  `responder_pubkey_b32` tells a caller which responder signed.
- Disagreement as data. The multi-attester index and `memory_contradictions`
  score places where two attesters signed different values at one
  `(cell, band, tslot)`. The score depends on the band kind: spread for
  scalars, cosine for vectors, mode share for categories. With federation,
  "two attesters" can mean "two nodes". The scoring does not change.
- Deterministic refinement. A newer attestation or a `disagrees_with` edge
  makes the responder derive the fact again. The same loop works for a
  trigger from a peer.

The model is a content-addressed CRDT. Facts merge by id. A conflict is
recorded (multi-attester rows and `disagrees_with` edges), not settled by a
vote. That lets independent nodes converge without consensus.

## 3. End state

Many responders share one address space. A content id means the same bytes on
every node. Each responder signs with its own key, and a client trusts the
signature, not the server. When two responders disagree at a key, the network
records the disagreement. More readers and writers make the shared memory
more trustworthy, because each disagreement becomes visible evidence.

## 4. Design, in phases

Each phase is useful alone and can ship alone. The order is: read federation
first (lowest risk), write sharding second (the contention fix), cross-node
disagreement third, and automatic routing last.

### 4a: Read federation (peer resolve), the thin prototype

When a node does not hold an id, it asks its configured peers. It verifies
the answer before it trusts it. It checks that
`blake3(canonical_cbor(fact)) == requested_id`. It checks the responder
signature against the peer's published key. Then it caches the fact. The id
proves the bytes, so the cached copy is as good as the original.

- Configuration: a static list, `EMEM_PEERS=https://a.example,https://b.example`.
- Surface: `/v1/fetch` and `memory_token/resolve`. A local 404 becomes a
  query to each peer. The first verified answer wins.
- Trust: the node ignores a peer that returns a wrong hash or a bad
  signature, and logs the mismatch. No peer needs to be honest. Each peer
  only needs to be checkable.
- Risk: low. It only reads, it verifies before it caches, and it does not
  change the write path.

This is the recommended first build. Section 6 gives the steps.

### 4b: Write sharding (break the single-writer ceiling)

The plan is to shard the write path by `cell64` prefix. The premise this plan
used to rest on is false. Re-derive the plan before anyone builds it.

The old premise said the grid is Hilbert-ordered, so a contiguous prefix range
is a contiguous patch of Earth. The active codec uses 21 bits of latitude and
22 bits of longitude. A Hilbert curve needs axes with equal bits.
`crates/emem-codec/src/geo.rs` says so: "Hilbert locality at the cell-key
level is dropped here". Only the bigram alphabet is Hilbert-ordered.

A test on the live responder confirmed it. From one origin, a cell 10 m north
and a cell 1 km north share the same 2 leading bigrams of 4. A cell 10 m east
shares 3. Prefix depth separates longitude from latitude. It carries no
information about distance along latitude, and a spatial shard needs exactly
that information.

What remains true: prefix ranges still split the space into disjoint sets. So
N prefix owners still give N parallel single writers, which was the main
benefit. What is no longer true: a prefix range is not a patch of land. Any
argument about local upstream fetches, tile reuse or warm caches must be
derived again or dropped.

- Each node owns one or more `cell64` prefix ranges. A write for a cell goes
  to its owner, or the receiving node forwards it there. N owners give N
  parallel writers.
- The Merkle log splits per owner range. Each range has its own append-only
  log with one writer, and each log can be snapshotted and replayed alone.
  Inclusion proofs become per-shard proofs plus a shard directory (§4d).
- Reads stay global through §4a. Any node resolves any id from the owner or a
  cached copy, and verifies it offline.

### 4c: Cross-node disagreement (federate the multi-attester index)

A node can resolve an id from a peer while it holds a different value at the
same `(cell, band, tslot)`. In that case it adds the peer's value to the
multi-attester index, as for a second local attester. `memory_contradictions`
then scores the cross-node disagreement with no new code. A `disagrees_with`
edge starts the existing refinement loop.

### 4d: Routing and directory (which node owns a cell)

Start with static configuration and move to gossip:

1. Static ownership map in configuration: `cell64` prefix to owner URL, plus
   a peer list. It is deterministic, easy to debug and enough for a few nodes.
2. Rendezvous or consistent hashing on prefix ranges. Ownership then moves in
   a predictable way when nodes join or leave.
3. Gossip directory. Each node advertises its `cell64` ranges, its public key
   and its registry CIDs. Resolution and write routing then need no static
   configuration. This is the only new distributed component. The steps before
   it use configuration and the existing verify path.

## 5. Trust model

No node trusts the compute of another node. It only verifies the output.

| Check | How | On failure |
|---|---|---|
| Bytes | compute the BLAKE3 id again | reject |
| Signature | verify Ed25519 offline against the peer's published key | reject |
| Registry alignment | compare the peer's four manifest CIDs with the local ones | accept, but flag |

A flagged peer's facts land as a separate attester. Their disagreements then
show in contradiction scoring and do not overwrite local facts.

There is no global lock, no cross-node transaction and no leader. Nodes
converge by content address. A conflict is recorded, not voted on.

### 5a. Enlistment: who may write, and what they proved

Federation makes the write question more important. One responder's bad
entity binding becomes the bad binding of every peer. So the property that
must survive scale-out is this: a third party that trusts neither node can
check who was entitled to write.

The objection that forced this design is correct, and its answer is not
"add OAuth". It is from `NousResearch/hermes-agent#79583`, closed 2026-08-25:

```text
a no-auth shared memory that arbitrary agents read AND write is a textbook
cross-agent prompt-injection and data-poisoning surface: anyone can plant
content that other agents will recall as fact.

Signed provenance mitigates attribution, not injected-instruction risk.
```

The second sentence matters most. A signature says who wrote a thing. It does
not stop a reader from obeying it. It does not say the writer was entitled to
write there. Anyone can make an Ed25519 key, so a signature is the minimum,
not the bar.

Reads are never gated, at any tier, on any surface. A reader cannot poison
anything. A read gate would cost the main property of the substrate and add
no security. There is no account, no bearer token that grants anything, and
no payment in the ladder.

Writes are gated on reach, never on identity. The question is how far a write
reaches, and what the writer proved in proportion to that reach. The live
ladder at `GET /v1/enlist` on 6 November 2026:

| Surface | Minimum tier | Why |
|---|---|---|
| read anything | `T0_anonymous` | never gated |
| own-namespace prose | `T1_keyed` | the floor, and free: a stranger's agent writes on first contact with only a signature |
| shared entity address space | `T3_declared` | `entity` and `entity_link` change what every agent resolves a name to |
| fact plane | `T4_affiliated` | only three writers may write an address: the responder's materialiser, a device enrolled through the OS-trace gate, and a key in `EMEM_FACT_PLANE_WRITERS`. Storage refuses all others with 403 `level_too_low` |

Derivations and edges take no address. They stay open at T1.

A tier records which check passed. It is never a score. `trust:
caller_decides` stays true. No tier says that a party is trustworthy. Each
tier says what was verified and what a peer may conclude from it.

#### Why DNS and `.well-known`, not OAuth

Browser OAuth 2.1 with Dynamic Client Registration (DCR) authenticates a
session: a human in a browser just authorised this client. An autonomous
agent has no human and no browser. DCR then becomes a bearer token. The token
proves possession and says nothing about accountability. A headless agent
also cannot complete the flow. What an agent needs is proof of who is
accountable for what a key says. That is name control, which DKIM, ACME and
Certificate Transparency already solve.

    _emem-agent.vortx.ai   TXT   "v=emem1; k=<52-char key>; nick=cosmos-eye"

The decisive property is that a third party can verify again. A bearer token
proves nothing to a third agent. A DNS record proves the same thing to
everyone, at any time, without trust in the responder that recorded it. It
also survives a compromise of that responder, because the evidence is not on
its disk. For federation this is the main argument: a peer node checks an
affiliation itself and does not inherit another node's verdict.

Each attestation carries `checked_at`, shows its age and expires after 30
days (`EVIDENCE_TTL_SECS` in `enlistment.rs`). A check from last month is a
claim about the present made from the past. A verification target must be a
public name. The responder refuses IP literals, local names, and names that
resolve into private, loopback, link-local or CGNAT space. It does not follow
redirects. The gap between resolving a name and connecting to it is still
open. The code documents it as a known limit, not as safe.

#### Shipping state

The ladder, the checks and the gate are live. Enforcement is off by default
in the code. `EMEM_ENLISTMENT_ENFORCE=1` turns it on, and emem.dev sets it
(`deploy/systemd/emem-server.service`). `entity` and `entity_link` accepted
anonymous writes from the day they shipped. Enforcement in one deploy would
break every peer at once. Shadow mode records who would be refused and why.
It returns that verdict to the caller in the response, not only to a log, so
a writer learns before enforcement starts.

## 6. The first build: a peer-resolve prototype

This is the smallest slice that proves federation end to end and is safe on
the live serving path. It is not built.

```
EMEM_PEERS=https://emem.dev,https://mirror.example   # static peer list

GET /v1/fetch/<fact_cid>:
  1. local hit?            -> serve (unchanged)
  2. local miss + peers?   -> for each peer, GET /v1/fetch/<cid>:
       a. re-derive blake3(canonical_cbor(body)) == <cid>   else skip peer
       b. verify ed25519 receipt signature vs peer pubkey   else skip peer
       c. first peer that passes both -> cache locally + serve, citing the
          originating responder pubkey in the response
  3. no peer has it        -> typed 404 (unchanged)
```

It only reads, it verifies before it caches, and it changes no write path and
no schema. It adds no distributed state. It tests content addressing and
offline verification across a process boundary, which is the whole trust
model. It does not commit to sharding or routing. Sections 4b to 4d build on
the verify step it adds.

## 7. Non-goals (for now)

- No consensus protocol. Facts merge by id and disagreements are recorded. A
  BFT or Raft layer would contradict "trust the signature, not the server".
- No global deletion guarantee. One node cannot remove content-addressed,
  replicated data from its peers. Deletion and redaction across a federation
  is an open policy question (§7a), not a feature.
- No cross-node compute trust. A node never runs the algorithm of another
  node and trusts the result. It derives or verifies again.

### 7a. Open questions

| Question | What is missing |
|---|---|
| Prefix-ownership rebalancing | a hand-off protocol for a node that joins or leaves during a write. Also a rule for the time when two nodes both think they own a range |
| Public key distribution and rotation | `key_epoch` allows rotation; how peers learn which keys to trust is not designed |
| Per-shard Merkle proofs | under §4b a client needs the right shard's proof and a shard directory it can trust |
| GDPR and redaction across replicas | probably a signed tombstone plus a refusal to serve, not true deletion |

## 8. The four-node network, and anyone after that

The owner decided these roles on 2026-09-02. The last column is what
`scripts/verify_node.py` reported for each origin on 6 November 2026.

| Node | Planned role | State on 6 November 2026 |
|---|---|---|
| `emem.dev` | full node: materialisers and the write plane | meets the witness bar (5 of 5) |
| `geo.qa/emem` | read replica and witness; writes under its own attester namespace | meets the witness bar (5 of 5) |
| `eudr.dev` | read replica, verifier and witness | not a node: 1 of 4 (`/.well-known/did.json` only) |
| `vrtx.ai` | read replica and witness; writes under its own attester namespace | no answer |
| anyone else | read federation and witnessing; writes under their own namespace | see §8e |

`eudr.dev` is not a node, but its domain vouches for one witness key. On 6
November 2026 `/v1/log/witnesses` listed it as the only independent operator
domain.

### 8a. Phase 0 is not new code. It is turning on what is already built.

These routes ship today:

| Route | What it does |
|---|---|
| `GET /v1/log/sth` | the signed tree head |
| `POST /v1/log/witness` | accepts an Ed25519 co-signature over `(tree_size, root)`; checks the signature and that the root matches the tree at that size |
| `GET /v1/log/consistency` | proves one head is an append-only prefix of another |
| `GET /v1/log/inclusion` | proves an entry is in the tree; `tree_size` selects a past head, and an unknown argument gets 400 |
| `GET /v1/log/witnesses` | lists the co-signatures and the witness lag |

On 2026-09-02 the routes were idle. The log held 1,539,209 entries,
`head_is_witnessed` was `false`, three witnesses existed, and the freshest
co-signature was 58,145 entries behind. People had made the signatures by
hand, and then stopped.

Now a job runs them. `scripts/witness_peers.py` does these steps for each
peer:

1. Fetch the peer's `/v1/log/sth` and verify the responder signature over
   the PreimageV1 bytes.
2. Check the signer against the peer's `did.json` and its `_emem-node` DNS
   record (§8d).
3. Prove growth from the last head it co-signed, with `/v1/log/consistency`.
   The proof check is a port of `emem_attest::translog::verify_consistency`.
4. Co-sign the head.
5. Spot-check custody (below). `EMEM_WITNESS_SAMPLES` sets the number of
   leaves, 4 by default.

`deploy/systemd/emem-witness.timer` runs the job every 15 minutes. The job
keeps its pin in `~/.config/emem/witness_state.json`. A peer's next
consistency proof must extend that pin. The job needs a witness key in
`~/.config/emem/agent_identity.json` and the `pynacl` package, so you cannot
run it without a key.

The custody check matters because a co-signature says nothing about custody.
The job samples 4 leaves per run from the co-signed root and the witness key,
so no peer knows them in advance. It re-hashes the bytes and proves inclusion
against the head the route returns. Then it proves that head extends the
co-signed head. A peer can ask that its entries stay private. For an origin
in `~/.config/emem/entries_private.json`, the job checks inclusion only and
does not read the entries. `geo.qa` asked for this on 2026-11-06. In September `/v1/log/inclusion` ignored `tree_size`, so the
first draft failed every proof against the older root. On 6 November 2026 the
route used `tree_size` and refused an unknown argument with 400.

Two bugs were caught before the job shipped. Every node operator will write
this job once, so they are recorded here.

- A verifier written from memory of RFC 6962 had the spine-advance parity
  inverted. It rejected every real proof and still rejected tampered ones.
  That looks like rigour and is not.
- The same `tree_size` does not mean "unchanged". A log that rewrites an
  entry in place keeps its size and changes its root. The first draft
  co-signed that without a root comparison. The job now refuses a same-size,
  different-root head as a rewrite. A dry run never moves the pin, because
  the pin records what was co-signed, not what was looked at.

What witnessing buys: a node cannot show two histories to two peers unless
one peer holds a signed head that fails a consistency proof. That is the
gossip property of Certificate Transparency. It is the reason this network
needs no chain.

#### Measured state

On 2026-09-05 both directions worked. Outbound, emem.dev co-signed
`https://geo.qa/emem` at tree size 72. The root was unchanged, the signer
resolved to `did:web:geo.qa:emem`, and 4 sampled leaves re-hashed and proved.
Inbound, an independent witness (`ttecxvf3drdt...`) co-signed emem.dev's log
88 entries behind. The emem.dev tree then held 1,618,506 entries.

On 6 November 2026 `GET /v1/log/witnesses` on emem.dev returned:

| Field | Value |
|---|---|
| `current_tree_size` | 3,243,071 |
| `count` (co-signatures) | 49,483 |
| `independent_witness_count` | 112 keys (111 `key_only`, 1 `org_vouched`) |
| `independent_operator_domains` | `eudr.dev` |
| `freshest_independent_witness_entries_behind` | 8 |
| `head_is_independently_witnessed` | `false` |

#### Read the independent fields, not `head_is_witnessed`

`head_is_witnessed` does not measure federation. `storage_liveness.py`, the
watchdog write canary, co-signs the current head every 2 minutes. It times a
write and catches a store that answers reads but refuses writes. It shares a
key and an endpoint with witnessing. So `head_is_witnessed` answers "did
anything sign this head", and it is almost always `true`. A node cannot catch
itself showing two histories.

The fields that bear on split view are `head_is_independently_witnessed` and
the other `independent_*` fields. They leave out `self_witness_pubkey_b32`.
`scripts/external_witness_lag.py` alarms on them. `emem-witness-lag.timer`
runs it every 30 minutes. It exits 2 on a stale peer and 3 when no peer
witnesses at all. The repairs differ, so the exit codes differ.

### 8b. What we take from web3, and what we refuse

We take the real advances, and emem already has them:

- content addressing, so an id is a fingerprint and any node may serve it;
- key-based identity with no accounts;
- Merkle inclusion proofs;
- gossiped transparency logs, which come from Certificate Transparency, not
  from a blockchain.

We refuse these, each for a stated reason:

- No token and no incentive layer. A token makes "who may write" a market,
  and that creates the sybil pressure it must then defend against. Nothing
  here is scarce. The expensive resource is upstream fetches, and each node
  already limits them.
- No global consensus, no chain, no validator set. This is a
  content-addressed CRDT: facts merge by id, and a conflict is recorded, not
  voted on. Consensus forces one answer where two attesters can both be
  right. That disagreement is the signal `memory_contradictions` surfaces.
  There is no leader, so there is nothing to capture.
- No on-chain anchoring as a dependency. A node may anchor a tree head on any
  chain. No rule may require it, or the network takes on that chain's
  liveness and cost.
- No "decentralised, therefore trustless". The network trusts no node for
  anything and verifies each node for two things. The bytes hash to the id,
  and the signature checks against a published key. That is the whole trust
  model, and more nodes do not improve it.

### 8c. The three attacks, and which one is open

| Attack | State | Why |
|---|---|---|
| Serving wrong bytes | closed | the reader checks the BLAKE3 hash against the id, with no trust in anyone; a hostile node can withhold bytes, not forge them |
| Split view | closed by §8a, open until independent witnesses keep up | a node that shows two histories fails a consistency proof at one of its witnesses. On 7 November 2026 emem.dev had one independent operator domain, and `head_is_independently_witnessed` was `false` (freshest independent co-signature 522 entries behind) |
| Sybil writes | open, and identity cannot close it | anyone can make unlimited Ed25519 keys, so a count of signatures is worthless |

The answer to sybil writes is not an identity gate. That gate would cost the
property this substrate exists for. Weight is the reader's judgement, not the
network's: `trust: caller_decides`. Sybils add signatures, not credibility.
The network owes the reader the evidence to judge:

- who signed;
- what they proved (the T0 to T5 ladder);
- whether a third party can check that proof again (DNS or `.well-known`, §5a);
- where signers disagree.

A reader who gives an unaffiliated key the same weight as a DNS-verified key
has made a choice. A network that made that choice for the reader would be
the thing this design avoids.

The honest limit: this makes poisoning attributable and visible, not
impossible. A reader that ignores provenance can still be poisoned. No
protocol fixes that.

### 8d. Order of work

1. Witness mesh across the four nodes. No server change. It closes split
   view. It has run since 2026-09-02 and worked both ways on 2026-09-05. Only
   `emem.dev` and `geo.qa` are nodes, so this is a witnessed pair, not a mesh.
2. Peer resolve (§4a). Not built. `emem-api-rest/src/lib.rs` reads
   `EMEM_PEERS`, but only to list peers in the `federation` block of
   `/.well-known/emem.json`. No read path asks a peer, and no crate has a
   `peer_resolve`. A declared peer and a queried peer are different things.
   The well-known file proves only the first.
3. Node identity in DNS. Done on 2026-09-05, both halves.
   `_emem-node.emem.dev` and `_emem-node.geo.qa` carry `v=emem1; k=<pubkey>`.
   Each matches the key its node signs with (checked again on 6 November
   2026).

   The record went up on 2026-09-02, and for three days nothing read it.
   `identify_signer` asks the peer's own web server for
   `/.well-known/did.json`. Alone, that proves only that the box agrees with
   itself.

   `verify_node_dns` in the witness job now resolves the TXT record for each
   peer. It compares the record with the `responder_pubkey_b32` of the STH it
   just verified. A mismatch is a finding, and the job does not co-sign. A
   missing record reads `unchecked` and does not block, because a witness that
   refuses every peer without a record witnesses nothing. The lookup uses DoH
   to the same resolver that `enlistment.rs` uses for `_emem-agent`. An
   operator debugs one resolver, and the job needs no DNS tools on the host.

   The check ran against controls first, because a check that passes is no
   evidence that it can fail:

   | Case | Result |
   |---|---|
   | the real key at `geo.qa` | confirms |
   | `emem.dev`'s key at `geo.qa` | `DNS MISMATCH`, blocks |
   | `emem.dev`'s key at `emem.dev` | confirms |
   | a domain with no record | `unchecked`, does not block |

   An attacker who wants to move a peer's key now needs both its web server
   and its DNS zone. Different people usually run those two systems.
4. Cross-node disagreement (§4c). A peer's facts land as a separate attester,
   so the existing contradiction scoring works without change.
5. Routing (§4d). Last, and only if read federation is not enough.

Write sharding (§4b) stays where §4b leaves it. Its premise was false, and the
plan must be derived again before anyone builds it.

### 8e. What each new node must publish before it counts as joined

There are two roles. The first version of this section had one role, and our
only peer failed it. `geo.qa` is a private deployment. Its `/emem/health`,
`/emem/.well-known/emem.json` and `/emem/openapi.json` answer 401, so nobody
can read its four registry CIDs. The old rule called it "not joined". That was
wrong: it is a good witness and has co-signed this log since September. A
node that holds no public data can be a better witness. So the bar depends on
what the node asks to be trusted for.

A witness attests that it saw a head. It must publish these, with no
credentials:

- `/v1/log/sth`, so a third party can check its own head;
- a stable `responder_pubkey_b32`, in the STH and in `/live`;
- `_emem-node.<domain> TXT "v=emem1; k=<52-char key>"`, so a third party can
  bind the key to the name without asking either node;
- `/.well-known/did.json`, so the signer resolves to a DID.

That is the whole list. It is short on purpose. Witnessing is the one job
that gets stronger the less the witness holds. A bar that demands a corpus
keeps out the parties whose independence is worth most. `geo.qa` meets this
bar and is joined as a witness.

A resolver also asks readers to fetch facts from it. It must also publish
what a reader needs to check those facts:

- the four registry CIDs (`algorithms_cid`, `bands_cid`, `schema_cid`,
  `sources_cid`), with no credential, because a fact means nothing without
  the registries it was signed against;
- `/openapi.json`, so a reader can list its surface;
- `/.well-known/emem.json` with a `federation` block that names the nodes it
  witnesses.

On 6 November 2026 emem.dev published everything on this list. It is still
not a resolver for other nodes, because read federation (§4a) is not built.
The role exists so the bar is written before someone needs it.

#### Check a node against this bar

Run this from any host. It needs no credential:

```sh
python3 scripts/verify_node.py https://geo.qa/emem
python3 scripts/verify_node.py --role resolver https://emem.dev
```

The script prints each requirement and the node's answer. It exits non-zero
when a node fails the witness bar. Run it before you add an origin to
`EMEM_PEERS`. On 6 November 2026 the first command printed "all 5 met: joined
as witness", and the second printed "all 8 met: joined as resolver". The
script checks only what a node publishes. It does not mean that peers fetch
facts from that node.

#### Start a witness node

`docker-compose.yml` at the repository root starts a node that meets the
witness bar. Its header comment lists three steps to join. Serve the node on
a public name over TLS. Publish the `_emem-node` TXT record. Run the witness
job. [Self-host](self-host.md) covers the container. Joining gives
mutual witnessing only. Nobody resolves facts across nodes yet.

---

*The README section "Where this is going" and
[examples/connect-and-evolve.md](https://github.com/Vortx-AI/emem/blob/main/examples/connect-and-evolve.md) are
companions to this page. The federation diagram is
`docs/diagrams/federation.png`.*

## 9. What the DePIN and open-protocol peers built that we did not

Read §8b first: no token, no chain, no consensus. This section covers what
physical-infrastructure (DePIN) networks and federated protocols learned
that needs none of those, and where emem is behind them. The "emem today"
claims were checked against the code on 2026-09-02, and the status notes
again on 6 November 2026. Each peer claim comes from a document in the
references at the end.

### 9a. DePIN with the token removed

A DePIN network meets three problems that a hosted service never meets. Most
of them pay for the answers with a token. The answers work without the token.

Proof that physical work happened. In Helium's Proof of Coverage, a
challenger picks a target from verifiable entropy (a block hash plus an
ephemeral key). The target broadcasts. Nearby hotspots witness the packet and
report time of arrival, signal strength and quality. The challenger submits
those receipts, and verification is a deterministic replay. Physics limited
gaming first (radio has a range), and later a witness-similarity denylist.
In emem the fact plane does this job: a band-typed measurement, signed by the
responder that made it, with the upstream named. emem lacks a witness
receipt. Only the responder signs that it served a fact. A dispute needs two
signatures on the same bytes, and today there is one.

Proof that custody continues. In Filecoin's WindowPoSt, each sealed sector
gets a challenge in a 24-hour proving period of 48 deadlines. A missed proof
is a fault that costs collateral. Without the collateral, the mechanism still
works: random challenges, sampled from a commitment, answered from the bytes.
emem's log is that kind of commitment. On 2026-09-02 nothing sampled from it.
The witness job now samples 4 leaves per run (§8a, §9e item 1).

Roles with different exposure. The Graph separates Indexers (stake and
serve), Fishermen (dispute with a Proof of Indexing) and Arbitrators
(decide). Slashing is 2.5% of self-stake. Without stake, the roles are still
real. emem has a responder (writes facts), attesters (write notes), witnesses
(co-sign heads), and a reputation leaderboard at `/v1/contributors` that no
code path reads. Since September the witness job audits served bytes against
the log, so witnesses now do part of the auditor role. emem has no working
disputer. A signed `disagrees_with` edge exists, and `memory_view` does not
show an inbound one.

The three jobs a token does, and what does them here:

| Token job | Web3 mechanism | emem today | Gap |
|---|---|---|---|
| Sybil cost on writes | stake (The Graph: 100,000 GRT minimum) | ladder T0 to T5 in `enlistment.rs`; 240 writes per minute per attester; DNS TXT and organisation vouching | code default is shadow mode; emem.dev enforces. On 2026-09-02 every key read T1, the operator's own agents too, so the shared entity space was closed to everyone (fixed, §9d) |
| Reward for serving | issuance (The Graph: 3% a year) | none; operators run a node because they need the data, as in the Nostr relay and ATProto PDS model | none needed at four nodes |
| Settlement | on-chain transfer | none | a payment rail without a token exists (§9b) |

### 9b. Tokenisation: what it settles, and the rail that replaced it

In 2026 an HTTP header settles what a token settled in 2021. x402 is an HTTP
402 flow:

1. The server answers `402` with a `PAYMENT-REQUIRED` header that carries the
   requirements.
2. The client retries with `PAYMENT-SIGNATURE`.
3. A third-party settlement service verifies and settles. It uses
   stablecoins on several chains and can extend to card rails.
4. The server answers with `PAYMENT-RESPONSE`.

A Linux Foundation body stewards x402. It has no native token and runs in
production. The official MCP registry lists x402-metered MCP servers next to
emem. Google's Agent Payments Protocol (v0.2, April 2026) supplies the human
mandate and carries x402 as an A2A extension.

emem already answers `402`: `ComputeQuotaExceeded` maps to `PAYMENT_REQUIRED`
in `lib.rs`. It is a quota error with a payment status. The header tells the
two apart. emem sends no such header, so no agent can mistake the quota error
for a payment request. That stays true until someone decides otherwise.

This page records a decision: reads are free at every tier, on every node,
and a self-hosted node never sees a price. Sybil pressure is real on two
surfaces: `SharedEntitySpace` (T3) and `FactPlane` (T4). Behind a flag that
ships off, the public node may answer those in the x402 shape. The URL of
the settlement service would come from the environment. None of this is built. The smaller step
came first: emem.dev enforces the ladder. On 2026-09-02 that showed that
enforcement without vouching locks out the operator's own agents, which all
read T1. The vouching document (§9c.3) turns enforcement into a policy and
not a lock-out.

Two surfaces promised what no code did. This project treats that kind of
drift as a false claim. `Cost.credits` in `crates/emem-fact/src/receipt.rs`
is a `u64` that is always 0. The `compute_quota_exceeded` error text told a
throttled caller that high-score attesters get larger quotas, and nothing
read `AttesterStats::score()` to widen anything. Tiers record checks and are
never scores, so the sentence was removed and the code stayed.

### 9c. The missing layers, each with the standard it should speak

1. Checkpoint interop. Not built. A C2SP checkpoint is three lines (origin,
   tree size in decimal, base64 root) plus signature lines of the form
   `— <name> base64(4-byte key id || signature)`. The key id is the first four
   bytes of SHA-256 over `name || 0x0A || 0x01 || pubkey`. A C2SP witness
   takes `POST /add-checkpoint` with an `old <size>` line, the consistency
   proof, a blank line and the checkpoint. It answers 200 with cosignature
   lines, 409 with the size it holds, or 422 when the proof fails. emem's
   witness surface is a JSON body over a private preimage. One problem: emem's
   tree hashes with BLAKE3, and a C2SP witness checks consistency per RFC 6962
   §2.1.2, which is SHA-256. An existing witness could co-sign emem's
   signature but could not check its proof. The fix is a second SHA-256 tree
   over the same entry bytes, one extra 32-byte hash per append. Then every
   witness in the Sigstore and Go ecosystems runs against emem with no new
   code. emem's job can then witness their logs.

2. SCITT receipts. Not built. RFC 9943 (June 2026) is the model emem grew
   into without the envelope. Its parts are a Transparency Service, Signed
   Statements (`COSE_Sign1`) and Receipts. A Receipt is COSE with a proof over
   a verifiable data structure. SCITT also has a Registration Policy and an
   append-only log. A Relying
   Party "MAY decide to verify only a single Receipt that is acceptable to
   them". The adapter: `Accept: application/cose` on `/v1/log/inclusion`
   returns a COSE receipt per draft-ietf-cose-merkle-tree-proofs. The same
   tree problem applies, because the registered algorithm is RFC 9162 over
   SHA-256. This waits for item 1. Then a supply-chain verifier that does not
   know emem checks an emem fact with the tools it already runs.

3. Node identity. Shipped on 2026-09-02. The agent ladder follows the NIP-05
   rule "identify, not verify". A DNS name maps to a key. The responder
   refuses redirects and private targets. Nodes now follow the same rule.
   `/.well-known/did.json` (`did:web:emem.dev`) lists the responder key and
   the witness key. Every DID and Verifiable Credential verifier can resolve
   emem's keys, with no new cryptography. The witness job reads a peer's
   `did.json`. It refuses to co-sign a head whose signer the domain does not
   list, and reports that as a finding. It reports a peer without the
   document as unchecked and still witnesses it. One gap remains. ATProto's
   DID documents separate a signing key from rotation keys. emem's revocation
   is only a doc comment in `key.rs`.

4. Portability. Not built. ATProto lets an account move to a new PDS
   "without the server's involvement", because a repository certifies itself.
   A signed commit names an MST root. The tree names records by CID. The
   whole repository exports as one CAR file. emem has `memory_bundle` (the
   caller picks the triples), `/v1/log/entries`, per-fact `GET` and the
   air-gapped container. It has no single file of "everything I wrote, with
   its inclusion proofs and the head", and no import. This feature lets an
   agent leave a node with its memory and its proofs. The plan has two
   parts. `GET /v1/agents/:pubkey/export` returns CBOR: the STH, the
   attester's entries and one inclusion proof for each. `emem import`
   verifies every proof again before it writes a byte.

5. Content addressing interop. Shipped: `cid_v1` sits beside `fact_cid` on
   recall facts. BLAKE3 is multihash `0x1e` in the multiformats table
   (draft). The CIDv1 of an emem fact is `0x01 0x55 0x1e 0x20` followed by the
   32-byte digest. It uses lower-case base32 with the `b` prefix. No new hash
   is computed.
   IPFS, Filecoin and ATProto tools can address emem facts directly. The
   `IpldConnector` stub stays unbound until an operator registers a
   blockstore.

6. Replication with proof of custody. Half built. The audit step runs in the
   witness job (§8a). `SegmentBackup` in `merkle_log.rs` is still a trait with
   no backend. The first one will copy a peer's `/v1/log/entries` into
   a local segment. With the audit step, this is WindowPoSt with no ZK and no
   collateral. A peer that dropped bytes fails within one run, and the
   evidence is a signed head plus a failed fetch.

7. Peer discovery without a DHT. Shipped as a file. Nostr puts `relays` in
   the NIP-05 record. ATProto puts the PDS endpoint in the DID document.
   emem puts `peers` and the witness data in the `federation` block of
   `/.well-known/emem.json`. A DHT stays refused until there are more nodes
   than a file can hold.

8. C2PA on rendered pixels. Later. `cell_scene_rgb` and `scene_png` make
   images. A C2PA 2.2 manifest would carry an image's provenance into
   browsers and newsrooms that do not speak emem. The `fact_cid` would be an
   assertion in it. The manifest needs three parts:
   - a `c2pa.hash.data` hard binding;
   - a `c2pa.actions` `created` entry with `digitalSourceType`;
   - `COSE_Sign1` over an X.509 key on the C2PA trust list. C2PA has no hardware capture
   attestation. emem's OS-trace substrate is the device half that it lacks.

### 9d. Acceptors: where emem is listed, where it is missing, where it drifted

Listed, checked on 6 November 2026:

| Channel | State |
|---|---|
| Official MCP registry, `io.github.Vortx-AI/emem` | 2.4.4 is latest; remote `https://emem.dev/mcp` |
| PyPI `ememdev` | 2.4.4 |
| npm `@vortxai/emem` | 2.4.4 |
| GHCR | `emem`, `emem-airgap`, `emem-encode` images |
| Docker MCP catalog, Dify marketplace | listed (Dify at 2.2.0 on 2026-09-05; not checked again) |

Missing:

- LlamaIndex. `sdks/llama-index-tools-emem` is at 2.4.4 and its tests run in
  CI. It is not on PyPI (404 on 6 November 2026) and not in
  `run-llama/llama_index`, where LlamaHub lists from. Publish it, then open
  the upstream PR.
- n8n. The node source is in `integrations/n8n` (`n8n-nodes-emem`). It is not
  on npm (404 on 6 November 2026). Zapier needs their platform, so n8n comes
  first.
- The C2SP and SCITT ecosystems. They can accept emem only after emem speaks
  their format (§9c items 1 and 2). After that, every running witness is a
  free auditor.

A one-command node was on this list. `docker-compose.yml` now exists (§8e).

Drifted. On 2026-09-05 this list had seven items. Fixes for four were in
source that day, and the image built later. On 6 November 2026 they are live:

| Item | State on 6 November 2026 |
|---|---|
| `/.well-known/emem-agents.json` served an empty list, because `config/` never reached the container and the handler turned "no file" into `{"agents": []}` | fixed: serves `count` 16, `source` `config`, `source_path` `config/emem-agents.json` |
| a missing vouching file looked the same as vouching for nobody | fixed: `count`, `source` and `source_path` are served beside `agents`, and a load failure warns at startup |
| the A2A card did not point back at the node identity | fixed: `responder` carries `did`, `did_document` and `federation` |
| MCP protocol version: for an unsupported client version, the header said `2025-03-26` and the body said `2025-11-25` | fixed: header and body both say `2025-11-25` |
| the MCP registry keeps old versions active | still open: 18 versions, 0.0.2 to 2.4.4, all `active`; mark the old ones deprecated from the publisher |
| Dify lags the other surfaces | still open on 2026-09-05; the plugin source is not in this repository, so `scripts/version_surfaces.py` cannot see it; vendor it under `integrations/` |
| the quota promise in the `compute_quota_exceeded` error text (§9b) | fixed: removed |

The empty vouching list had a dated risk. Evidence stands for 30 days. The
cached `T4_affiliated` tiers from 2026-09-02 would have fallen to T1 on
2026-10-02, while emem.dev enforced the ladder. The fix shipped as a fallback
path. The handler reads `EMEM_AGENTS_WELL_KNOWN` when it is set. Otherwise it
tries the relative `config/` path, then `$EMEM_DATA/emem-agents.json`. The image does not include a roster on
purpose. The document says that the operator of a domain vouches for these
keys. One roster in every self-hosted image would make each operator vouch
for keys they never chose.

The Claude connector directory needs a new submission. The compliance
material is in `docs/registries/anthropic-claude-connectors-submission.md`.

### 9e. Order of work, with the core kept first

The ranking is by what each item buys the four-node network per unit of
change. Nothing here touches the fact plane, the token scheme or the
preimages.

| # | Item | State on 6 November 2026 |
|---|---|---|
| 1 | Audit sampling in the witness job (`audit_custody` in `scripts/witness_peers.py`) | done 2026-09-02 |
| 2 | Enforce the ladder on `SharedEntitySpace` | on at emem.dev (`EMEM_ENLISTMENT_ENFORCE=1`); item 3 was the correctness fix that let its own agents write |
| 3 | `peers`, `witnesses`, `_emem-node` and `did:web` for the four nodes | code done 2026-09-02; DNS records exist and the witness job reads them (§8d); vouching document served (§9d) |
| 4 | Remove the quota promise; drop or wire `Cost.credits` | promise removed; `Cost.credits` stays 0 until a receipt-schema change is worth its own deploy |
| 5 | `cid_v1` | done, on recall facts |
| 6 | A compose file for a CPU replica, and the first `SegmentBackup` backend | compose file done; `SegmentBackup` not started |
| 7 | Namespace export and import | not started; medium to large; the portability keystone |
| 8 | C2SP checkpoint and `add-checkpoint`, with the SHA-256 shadow tree; then the SCITT receipt | not started; medium |
| 9 | Publish the LlamaIndex tool and open the upstream PR; vendor Dify; deprecate old registry versions | owner work, hours; not done |
| 10 | x402 on T3 and T4 writes, flag off | a decision before any code |
| 11 | C2PA on rendered scenes | later |

A survey of the live surface on 2026-09-05 added five fixes above item 6,
because each fixed something already claimed. All five are live on 6 November
2026. Three are in §9d: the vouching path and its count, the A2A card, and the
MCP version header. These are the other two:

- Witness independence. `head_is_witnessed` keeps its meaning, because peers
  read it. Beside it are `head_is_independently_witnessed`,
  `freshest_independent_witness_entries_behind`, `independent_witness_count`,
  `independent_cosignature_count` and `self_witness_pubkey_b32`, on the route
  and in the federation block. They count every stored row, not only the
  returned page. On 2026-09-05 the declared key held 82 of the newest 100
  rows.
- `live_perception.cameras_with_retained_clips` read upstream's
  `clips_retained`. Before, it counted only inside the returned camera page,
  which held `cameras_returned` of `camera_count` cameras (5 of 267 at
  Trafalgar Square).

Still refused: a token, a chain, consensus, a DHT at this size, and the word
"trustless".

References, all read on 2026-09-02:

- x402 <https://www.x402.org/> and <https://github.com/coinbase/x402>
- AP2 <https://ap2-protocol.org/>
- RFC 9943 <https://datatracker.ietf.org/doc/rfc9943/>
- COSE receipts <https://datatracker.ietf.org/doc/draft-ietf-cose-merkle-tree-proofs/>
- C2SP <https://c2sp.org/tlog-checkpoint>, <https://c2sp.org/tlog-witness>, <https://c2sp.org/signed-note>
- AT Protocol <https://atproto.com/guides/overview>, <https://atproto.com/specs/repository>
- NIP-05 <https://github.com/nostr-protocol/nips/blob/master/05.md>
- C2PA 2.2 <https://spec.c2pa.org/specifications/specifications/2.2/specs/C2PA_Specification.html>
- Helium PoC <https://github.com/novalabsxyz/devdocs/blob/master/blockchain/proof-of-coverage.md>
- Filecoin PoSt <https://spec.filecoin.io/algorithms/pos/post/>
- The Graph glossary <https://thegraph.com/docs/en/resources/glossary/>
- multicodec table <https://github.com/multiformats/multicodec/blob/master/table.csv>
- MCP registry <https://registry.modelcontextprotocol.io/v0/servers?search=emem>
