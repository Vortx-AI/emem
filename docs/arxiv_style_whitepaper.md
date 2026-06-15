# emem: A Content-Addressed, Verifiable Earth-Memory Protocol for AI Agents over Foundation-Model Embeddings

**Vortx AI — the emem maintainers**

2026-06-15 · Apache-2.0 · reference responder `https://emem.dev`

## Abstract

LLM agents asked "what is on the ground at this place, right now" have no stable handle for a patch of Earth and no way to prove the answer they return; the same agents asked "what did we learn here" fall back on unsigned, per-session scratchpads a server can silently rewrite. We present emem, a protocol that gives both questions one trust surface. Every spatial fact is keyed by a 64-bit geographic cell (`cell64`, ~9.55 m at the equator), a band, and a temporal slot (`tslot`); the fact is serialized to deterministic CBOR, content-addressed by a truncated BLAKE3 digest (a 26-character base32 `fact_cid`), and returned inside an Ed25519 receipt over a domain-separated, length-prefixed preimage that any party can verify offline against the responder's public key, in the browser, without trusting the issuer. Facts are grounded in frozen Earth-observation foundation embeddings — Tessera (128-D, CPU-streamed from Cloud-Optimized GeoTIFFs), Clay v1.5 and Prithvi-EO-2.0 (1024-D, GPU-pinned), and Galileo (multimodal) — whose independent receptive fields drive a triple-encoder change-consensus algorithm in which agreement is signal and a lone vote flags receptive-field aliasing. A read layer offers bi-temporal recall (valid time × transaction time), Lance IVF_PQ k-NN with a TurboQuant binary-rotation Hamming fast path, region analytics, and an advisory temporal-freshness kernel; a memory layer adds Anthropic-memory-tool file verbs over a CoALA-typed, capability-bound scratchpad, signed temporal edges, multi-attester contradiction scoring, and CLS-style episodic→semantic consolidation. Cold cells materialize lazily on first request — any cell on Earth answers citeably with no pre-seeded corpus, and absence is itself a signed, typed, content-addressed receipt rather than a 404. We describe the protocol, its mathematics, a single-binary Rust reference implementation serving mirrored MCP and REST surfaces, and its correspondence to the recent eMEM hippocampal/neocortical memory framework, against which emem already realizes the majority of the proposed mechanisms.

*Keywords:* Earth observation, agent memory, content addressing, verifiable receipts, Ed25519, foundation embeddings, bi-temporal retrieval, Model Context Protocol, discrete global grid, signed absence.

## 1 Introduction

### 1.1 Two recurring failures of agents over Earth

Ask a language-model agent what is on the ground at a coordinate and it guesses. The guess is unstable across runs because the agent has no fixed handle for that patch of Earth: "near Soubré, Côte d'Ivoire," "the cocoa plot," and a raw latitude/longitude pair are three different strings that the model treats as three different things, and a paraphrase of a place is not an address. Two distinct failure modes follow from the absence of a canonical address. The first is *cross-run inconsistency*: two invocations of the same agent, or two agents on the same task, return different numbers for the same place with no way to tell whether they disagree about the world or merely about how to name it. The second is *the missing citation path*: when an agent does produce a number, the provenance that would let anyone re-check it (which upstream tile, which acquisition timestamp, which algorithm, which threshold) is collapsed into free text. A reviewer cannot reproduce the answer, cannot bound its error, and cannot distinguish a measured value from a hallucinated one. Retrieval-augmented generation [12] mitigates the symptom by injecting retrieved passages into the context window, but the retrieved passage is still selected by similarity, paraphrased at the model's attention, and never reproducible byte-for-byte; two agents issuing the same query against the same index can receive different top-$k$ passages and produce divergent, uncitable answers.

The same agents fail in the same shape when asked what *they themselves* learned at a place. Memory mechanisms for language-model agents fall along three established lines, each scoped to a single agent's history [7]. Textual stores inject prior turns through the input context and pay for it in context-window pressure, retrieval noise, and compaction loss. Parametric stores fold prior interactions into adapter or prefix weights and cannot adapt to new information without retraining. Outside-channel stores keep state in a separate module reached by retrieval [8, 9, 10] and inherit integration overhead, drift between the retrieval index and the backbone, and a *silent-empty* problem in which a missing entry returns nothing rather than an attestable absence. All three operate over a single agent's conversation or tenant, and all three produce unsigned strings that whoever administers the store can rewrite. MINJA [41] makes the consequence concrete: it poisons an agent's memory with roughly $95\%$ success through query-only interaction, without privileged access to the store, precisely because nothing in the stored record binds its bytes to a signature an independent party could check.

### 1.2 Thesis

We argue that the right substrate for an agent's knowledge of Earth is a *planet-keyed, content-addressed, signed* memory layer with two further properties: *lazy materialization*, so that every cell on Earth is answerable from the first request without a pre-seeded corpus, and *signed absence*, so that "we do not have this here" is itself a content-addressed, signed receipt carrying a typed reason rather than a 404 or an empty array. Planet-keying replaces the unstable paraphrase with a canonical address: a place is named by a 64-bit cell identifier the way a token names text. Content addressing replaces the moving pointer with the bytes themselves: a fact's name is the BLAKE3 digest of its canonical serialization, so the name carries the data's fingerprint and means the same thing on every machine. Signing replaces trust-in-the-server with checkable mathematics: every response carries an Ed25519 receipt over a deterministic preimage, and any party holding the responder's public key reproduces the preimage and verifies the signature offline, with no account and no callback. Together these make an agent's memory of the world something a stranger can verify without taking anyone's word for it, and they extend without change to the agent's own writable notes, which ride the identical trust surface.

emem is a *protocol* in the sense that it specifies the loader, the validator, the content-identifier rule, the receipt-signing rule, the capability-binding rule, and the primitive semantics; it is never the data itself. Conformance is byte-equality: two implementations conform when, given byte-identical inputs, they produce byte-identical content identifiers over a content-addressed manifest set pinning the band ontology, the algorithm registry, the source catalog, the schema bundle, and the function registry. Because identity is byte-equality rather than service identity, any responder can serve a fact and any client can verify it, which is the precondition for the multi-responder federation the design targets but does not yet route automatically.

### 1.3 Contributions

This paper makes five contributions, each grounded in the reference implementation that ships today.

1. **An address algebra and trust plane.** We define a 64-bit cell address (`cell64`, a mode-tagged WGS-84 lat/lng bucket, $21$ latitude bits $\times$ $22$ longitude bits, square at the equator with extent $\approx 9.55$ m) crossed with a band ontology and a Unix-anchored temporal slot (`tslot`) snapped to per-band cadence. The triple $(\text{cell}, \text{band}, \text{tslot})$ keys a fact, which is serialized in deterministic CBOR [16] and named by
$$\mathrm{FactCid} = \mathrm{base32\text{-}nopad\text{-}lower}\big(\mathrm{BLAKE3}(\mathrm{cbor}(\text{fact}))[..16]\big),$$
a 26-character handle [17, 18]. Every response carries an Ed25519 receipt [15] over a domain-separated, length-prefixed preimage, $\mathrm{BLAKE3}(\texttt{"emem.preimage.v1"} \,\|\, \texttt{"receipt"} \,\|\, \text{tagged fields})$, so that no two distinct responses can share signed bytes; an RFC 6962 [14] Merkle tree with leaf/node domain separation and duplicate-leaf rejection supplies inclusion proofs; and an in-browser verifier recomputes the preimage and checks the signature with no trust in the issuer (Section 3).

2. **A foundation-embedding grounding layer with independent-receptive-field triple consensus.** Three Earth-observation foundation encoders with deliberately different receptive fields (Clay v1.5 at $\approx 2.56$ km [2], Prithvi-EO-2.0 at $\approx 6.7$ km [3], and the per-pixel Tessera annual stack [1]) vote on a per-cell change index over a 365-day window. A cell where all three agree shifted is land-surface change; a cell where only one fires is almost certainly receptive-field aliasing rather than physical change, and the consensus label distinguishes the two explicitly. Galileo [4] adds a multimodal encoder leg (Section 4).

3. **Connective, evolving memory.** Every read accepts two optional bounds, valid-time (`as_of_tslot`) and transaction-time (`as_of_signed_at`), so a verifier in year $t{+}k$ replays the exact query a system answered in year $t$. Facts link through signed, time-bounded temporal edges (`supersedes`, `disagrees_with`, `relates_to`); disagreement between independent attesters at the same key is scored per band kind (normalized spread for scalars, mean off-diagonal cosine for vectors, mode-share for categoricals); and a consolidation worker compresses aged episodic files into semantic files, an operationalization of the complementary-learning-systems separation between fast hippocampal and slow neocortical memory (Section 5).

4. **Lazy materialization and signed absence.** When an agent requests a band at a cell that holds no signed fact, the responder fetches the underlying value from a registered upstream connector, signs it under its own key with a `derivation.fn_key` declaring exactly how it was produced, persists it, and returns it in the same response. No cell is pre-seeded; every cell on Earth answers from the first request. When a value genuinely does not exist (outside the upstream product's coverage, no connector registered, a GPU-tier model unavailable), the response is a signed `NegativeFact` carrying a typed reason and its own content identifier, so an empty answer is a citable receipt and a repeated query returns the same absence without re-hitting the upstream (Sections 3.5, 6).

5. **A single-binary reference implementation and a memory-systems audit.** The protocol is realized as one Rust binary serving Model Context Protocol [23] and REST from the same handlers on one port, with reads requiring no authentication and every write landing in an append-only Merkle log. We further provide a correspondence audit that places emem against the eMEM hippocampal/neocortical memory framing [5] and the broader agent-memory landscape [6, 7, 8, 9, 10, 11], showing which of its mechanisms (content-addressed reproducibility, offline-verifiable receipts, quotable citations, cross-agent sharing) are structural properties the in-agent memory layers cannot hold, and which (compact associative addressing, read/steer/write cycles) are shared principles at a different scope (Sections 5.6, 7).

The contributions and the layers that carry them are summarized in Table 1.

| Layer | Mechanism | Property delivered |
|-------|-----------|--------------------|
| Address | `(cell64, band, tslot)` → deterministic CBOR → BLAKE3 → 26-char `fact_cid` | reproducible, byte-stable citation independent of responder |
| Trust | Ed25519 receipt over a domain-separated, length-prefixed preimage; RFC 6962 Merkle log; append-only segments | offline, in-browser verifiability without trusting the issuer |
| Grounding | frozen Tessera / Clay / Prithvi / Galileo embeddings; triple-encoder consensus over independent receptive fields | change signal separated from receptive-field aliasing |
| Availability | lazy materialization on cold miss; typed signed Absence on genuine miss | every cell answers; "we don't have this" is a citable receipt, not a 404 |
| Agent memory | content-addressed `/memories/*`, CoALA-typed, capability-bound; Lance IVF_PQ semantic search; bi-temporal reads | a signed, replayable scratchpad the agent owns |
| Evolution | signed temporal edges (`supersedes`, `relates_to`, `disagrees_with`); TTL + episodic→semantic consolidation; refinement loop | a memory that connects facts and records where they disagree |

*Table 1: the contributions of this paper against the protocol layers that deliver them.*

The reference implementation is a single Rust binary at `github.com/Vortx-AI/emem`, deployed at `emem.dev`, serving Model Context Protocol over Streamable HTTP and a mirrored REST surface from one set of handlers. Section references throughout this paper trace every constant and gate threshold to the code path or registry entry that defines it; the protocol is the rules, never the data, and any conforming responder reproduces the same content addresses over the same manifest set.

![emem architecture: one Rust binary, two wire surfaces, one optional sidecar](/docs/diagrams/01-architecture.svg)

*Figure 1: the reference stack. REST and MCP share handlers; a sled hot cache holds materialized facts, an append-only Merkle log holds the trust state, and four content-addressed manifests pin what produced each answer.*

### 1.4 Roadmap

Section 2 fixes terminology and locates emem against the three threads it joins (agent memory, geospatial indexing, and verifiable-data primitives). Section 3 specifies the protocol proper: the `cell64` address algebra, the `tslot` temporal algebra, the content-addressing rule (deterministic CBOR, BLAKE3 truncation, base32 encoding), the trust plane (receipt preimage, attestation envelope, Merkle log, in-browser verification), signed absence, the conformance manifests, and the bi-temporal read model. Section 4 presents the foundation-embedding layer: the four frozen encoders, the GPU sidecar, and the independent-receptive-field triple-consensus change detector with its domain variants. Section 5 develops the connective, evolving memory: the CoALA-typed capability-bound scratchpad, bi-temporal recall semantics, signed temporal edges, multi-attester contradiction scoring, CLS-style consolidation, and the eMEM correspondence audit. Section 6 details the read primitives and retrieval surface, including lazy materialization, advisory freshness, the Lance IVF_PQ and TurboQuant fast paths, region analytics, and the operations vocabulary. Section 7 describes the single-binary reference implementation, its two wire surfaces, the fetch plane, and the deployment and operator-attestation story. Section 8 characterizes the corpus, the latency profile, and the reproducibility surface. Section 9 states the honest limits of the 0.1.0 responder, each paired with the typed signal it emits, and Section 10 concludes with the through-line, the federation forward look, and the design stance. Figure 1 (`/docs/diagrams/01-architecture.svg`) shows the entire stack; Figure 2 (`/docs/diagrams/09-address-algebra.svg`) shows the address algebra by which three integers become the handle the rest of the protocol cites.

## 2 Related Work

emem sits at the intersection of three research areas that have so far
developed in isolation: memory mechanisms for language-model agents,
geospatial indexing and catalog standards, and the verifiable-data
primitives of transparency logs and content addressing. Each area
supplies one property emem requires, and none supplies the
combination. Agent-memory systems give a read/write contract but scope
state to a single agent's history and return unsigned strings.
Geospatial standards give a planetary address space but stop at scene
catalogs and storage formats, carrying neither a signature nor an
inclusion proof. Verifiable-data primitives give tamper-evidence and
content addressing but say nothing about places or measurements. emem
composes the three into a memory that is at once persistent,
planet-keyed, content-addressed, verifiable, and shared across agents.
This section surveys the three threads and locates emem precisely with
respect to each.

### 2.1 Memory for language-model agents

Memory mechanisms for LLM agents fall along three established lines,
and a defining property of all three is that state is scoped to a
single agent's history, whether a conversation, a session, or a tenant.

**Textual / retrieval-augmented stores.** The most widely deployed
pattern injects prior history into the model through the input context.
Retrieval-augmented generation [12] couples a parametric generator to a
non-parametric document index, retrieving passages by dense similarity
and conditioning generation on them. Applied to agent memory, the index
holds prior turns or extracted notes rather than a static corpus. The
strength is flexibility with no architectural change to the backbone;
the weaknesses are the context-window limit, retrieval noise, and the
compaction loss incurred when older history is summarized to fit. The
retrieved object is an unsigned string whose provenance ends at the
index that produced it. If the question is "what is at this place," a
text index returns whatever prose was previously written about the
place, ranked by cosine, with no canonical address and no way for a
second agent to dereference the same answer.

**Parametric stores.** A second line folds prior interactions into
adapter or prefix weights, so that recall costs nothing at inference
time because the knowledge is already in the parameters. The cost is
rigidity: a parametric store cannot adapt to evolving information
without retraining, and its contents are neither addressable nor
citable. There is no handle an agent can quote to a regulator or a peer.

**Outside-channel and in-attention stores.** A third line keeps state
in a module reached by retrieval or maintained as a compact internal
state. MemGPT [8] manages a tiered, per-tenant scratchpad with explicit
paging between a fixed context window and external storage, modeling the
agent runtime as an operating system that swaps memory in and out under
a budget. Compact in-attention mechanisms such as delta-mem [13]
maintain an online associative-memory matrix updated by a gated
delta-rule, retaining historical signal in a small state addressed
associatively rather than positionally; the demonstration that a small
matrix suffices, once it is addressed by content, is the design idea
emem reuses at planetary scale (the 64-bit cell address plays the role
of the associative key, and the 26-character `fact_cid` plays the role
of the compact retained handle). The outside-channel family is modular
and runtime-agnostic, which emem inherits, but it carries three
characteristic failure modes: integration overhead, drift between the
retrieval index and the backbone, and the silent-empty problem, in
which a missing entry returns nothing rather than an attestable absence.

**Taxonomies and multi-type systems.** CoALA [7] organizes the design
space of language agents into working, episodic, semantic, and
procedural memory with explicit decision procedures over them; emem
adopts exactly this four-way kind taxonomy
(`{episodic, semantic, procedural, resource}`) for files in its
agent-memory layer, defaulting to `resource` for back-compatibility
with file-oriented clients. MIRIX [9] elaborates the taxonomy into six
memory types coordinated across multiple agents, and mem0 [10] packages
a production long-term memory with extraction, consolidation, and a
scalable retrieval backend. Both are multi-type systems whose state
remains the agent's own history.

**Temporal knowledge graphs.** Zep, built on Graphiti [6], maintains a
temporally aware knowledge graph over a user's interaction history, with
bi-temporal edges that distinguish when a fact held in the world from
when the system learned it. emem shares the bi-temporal commitment.
Every read primitive accepts two optional bounds, `as_of_tslot`
(observation, or valid time) and `as_of_signed_at` (transaction time),
and returns the latest fact per $(\textit{cell}, \textit{band})$
satisfying

$$\textit{tslot} \le \texttt{as\_of\_tslot} \;\wedge\;
  \textit{signed\_at} \le \texttt{as\_of\_signed\_at},$$

with the bounds intersecting when both are set and a current-state read
when both are absent. Typed temporal edges in emem,
$\textsf{EdgeFact}(\textit{subj}, \textit{pred}, \textit{obj},
\textit{valid\_from}, \textit{valid\_to})$, carry the same bi-temporal
window, and a newer edge for the same $(\textit{subj}, \textit{pred},
\textit{obj})$ shadows the older one rather than replacing it, so a
query at any historical date replays the state as of that date. The
distinction from Zep is one of scope and trust: Zep's graph is built
from one user's conversation and its edges are unsigned; emem's edges
are signed under the same receipt envelope as band facts, content-
addressed, and keyed to the planet rather than to a conversation.

**Hippocampal/neocortical models.** eMEM [5] applies the
complementary-learning-systems view to embodied agents, pairing a fast
hippocampal store with a slow neocortical store so that spatial and
temporal experience consolidates from episodic traces into stable
structure. The architectural separation resonates with emem's own slow
consolidation worker, which concatenates aging episodic files under an
attester namespace into a semantic file and stamps `superseded_by` on
the originals (non-destructively, history retained). The systems are
named alike and aimed differently: eMEM is an in-agent memory for one
embodied agent's first-person experience, whereas emem (this work) is a
shared external memory of the Earth that any agent can read, write, and
cite.

**Evaluation context.** MemoryAgentBench [11] evaluates LLM-agent
memory through incremental multi-turn interactions, measuring accurate
retrieval, test-time learning, long-range understanding, and conflict
resolution over a single agent's accumulating history. It is the natural
yardstick for the systems above and frames the capability that those
systems target. It also makes visible what those systems do not test:
cross-agent reproducibility and offline verifiability, because the
benchmark's notion of a correct memory is per-agent rather than
byte-identical across independent responders.

**Threat model.** A consequence of unsigned outside-channel state is
that whoever administers the store can rewrite it. MINJA [41]
demonstrates roughly 95% poisoning success against this class of memory
through query-only interaction, with no privileged access to the store.
This motivates emem's signing rule directly: every read returns an
Ed25519 receipt [15] over a domain-separated, length-prefixed BLAKE3
preimage, so that a tampered value cannot reproduce the signed bytes and
a verifier holding the responder's public key detects the tamper without
calling back.

The combined position is summarized in Table 2. The systems of this
subsection answer "what did *this agent* learn"; emem answers "what is
at *this place on Earth*, that *any* agent can cite later," and returns
a signed, content-addressed receipt rather than an unsigned string.
emem does not replace in-agent memory; it relieves it of carrying
geospatial truth, and the receipt CID is the bridge between the two
(an in-agent store caches the CID, the context window quotes it, and
the underlying CBOR never has to be paraphrased or compressed).

| Property                       | Textual/RAG [12] | Parametric | MemGPT [8] / delta-mem [13] | MIRIX [9] / mem0 [10] | Zep/Graphiti [6] | eMEM [5] | **emem (this work)** |
|--------------------------------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| Scope                          | one agent | one model | one tenant | one agent | one user | one agent | planet, all agents |
| Persistent across sessions     | partial | yes | partial | yes | yes | yes | yes |
| Content-addressed              | no | no | no | no | no | no | yes |
| Signed / offline-verifiable    | no | no | no | no | no | no | yes |
| Cross-agent byte-reproducible  | no | no | no | no | no | no | yes |
| Bi-temporal reads              | no | no | no | partial | yes | partial | yes |
| Typed kinds (CoALA [7])        | n/a | n/a | partial | yes | n/a | n/a | yes |

*Table 2: agent-memory systems against the properties emem adds.
"Partial" denotes support that is conditional, lossy, or limited to a
subset of the property.*

### 2.2 Geospatial indexing and catalogs

The second thread supplies the address space and the upstream data, but
not the protocol.

**Discrete global grid systems.** H3 [20] partitions the sphere into a
hexagonal hierarchy with near-equal-area cells and bounded neighbor
distances; S2 [21] projects the sphere onto the six faces of a cube and
linearizes each face by a Hilbert curve, yielding a hierarchy of cells
with strong spatial locality. Both are mature indexing schemes, and H3
at resolution 13 (~3.4 m equal-area cells) is emem's stated migration
target. emem's current `cell64` is a 64-bit packed lat/lng bucket on
WGS-84 at a 21-bit latitude by 22-bit longitude quantization, chosen so
that cells are square at the equator with extent

$$\Delta\textit{lat} \approx \Delta\textit{lng} \approx 8.583 \times
  10^{-5}\,^{\circ} \;\Rightarrow\; \approx 9.55\text{ m},$$

aligning to the 10 m native pitch of Sentinel-2 [28] and Sentinel-1 RTC.
The text form is Hilbert-ordered so that adjacent codepoints map to
nearby cells, in the spirit of S2's curve. What a DGGS does not provide
is the rest of the protocol: a grid is an address scheme, whereas emem
is an address scheme plus a temporal cadence (`tslot`), a band ontology,
a CID rule, and a signing rule. emem composes on a DGGS; it is not one.

**Scene catalogs.** The SpatioTemporal Asset Catalog (STAC) describes
Earth-observation *scenes*, indexing imagery by footprint, time, and
collection so that a client can discover and fetch the right asset. In
emem, STAC is an upstream *connector* kind, queried during lazy
materialization (the connector dispatch issues STAC search and vsicurl
COG range reads), not the protocol surface. The unit of STAC is a scene;
the unit of emem is a per-pixel fact at $(\textit{cell},
\textit{band}, \textit{tslot})$ with its own provenance and signature.
emem describes what is true at a 10 m cell with a cited algorithm and a
responder signature; STAC describes which file covers the cell.

**Storage formats.** GeoParquet stores geospatial features in a
columnar format suited to analytics. It is a *format*, not an addressing
rule or a receipt schema: a GeoParquet column can hold an emem fact's
value, but the file carries neither the responder's Ed25519 signature
nor the Merkle inclusion path, so it cannot be verified offline against
a public key. The same distinction applies to the upstream products emem
materializes from, including Hansen Global Forest Change [32], ESA
WorldCover [33], the JRC Global Surface Water layers [31], SoilGrids
[34], the Copernicus DEM [30], and the Fields of The World boundary
product [35]: each supplies pixels, none supplies a signed, addressable
fact.

**Content-addressed linked data.** IPLD defines content identifiers over
arbitrary linked data, with a multibase encoding and CBOR tag 42 for the
CID itself (RFC 9090 [19]). emem composes its CID rule on top of this
layer rather than reinventing it: canonical CBOR carries IPLD's tag 42
alongside emem's own tags for cell, tslot, and vector-CID, and the
fact CID is a BLAKE3 truncation,

$$\textsf{FactCid} = \textsf{base32\_nopad\_lower}\big(
  \textsf{blake3}(\textsf{canonical\_cbor}(\textit{fact}))[..16]\big),$$

a 26-character lowercase string. IPLD answers "how is content named";
emem adds "what does the content mean (a fact at a place and time, by an
algorithm)" and "who attests to it (a signed receipt)."

The geospatial thread thus contributes a planetary address space and a
data supply chain, but stops short of a signed, per-fact, verifiable
memory. The relationship is illustrated in
[06-memory-vs-stac.svg](/docs/diagrams/06-memory-vs-stac.svg).

### 2.3 Verifiable-data primitives

The third thread supplies tamper-evidence and determinism. emem's trust
plane is assembled from standard primitives rather than novel
cryptography, which keeps every receipt checkable by an offline verifier
with widely available libraries.

**Transparency-log Merkle structure.** Certificate Transparency, RFC
6962 [14], defines a Merkle tree with explicit domain separation between
leaf and interior nodes, so that a leaf hash can never be confused with
an internal node hash. emem follows the same discipline: every leaf is
self-hashed once before folding, $L_i = \textsf{h}(l_i \,\|\, l_i)$, and
interior nodes pair children, $\textsf{h}(\textit{left} \,\|\,
\textit{right})$, with odd layers pairing the final element with itself.
An inclusion proof is the sibling path from leaf to root, and
verification walks the path bottom-up, hashing $\textit{acc} \,\|\,
\textit{sibling}$ at even indices and $\textit{sibling} \,\|\,
\textit{acc}$ at odd indices until $\textit{acc}$ equals the root.
Accepted attestations append to a per-segment, `fsync`-flushed,
append-only log that re-hashes on `verify`, the same append-only,
tamper-evident structure CT applies to certificates.

**Signatures.** Receipts and attestations are signed with Ed25519
(EdDSA over Curve25519, RFC 8032 [15]). Verification uses the strict
variant, which rejects malleable (non-canonically-encoded) signatures so
that a single fact has a single valid signature. The signing key is a
32-byte secret; a `u32` key epoch on every receipt lets a verifier
holding an older public key detect rotation.

**Deterministic serialization.** Content addressing requires that
identical facts serialize to identical bytes on every implementation.
emem uses CBOR (RFC 8949 [16]) in its deterministic-encoding profile
(Section 4.2 of the RFC), with `ciborium` over serde-derived structs so
that struct field order fixes serialization order and free-form maps
arrive with pre-sorted keys. Four reserved tags (cell, tslot, vector-CID,
and IPLD's tag 42) make the encoding self-describing. The hash is BLAKE3
[17], chosen for speed, a `derive_key` mode used by the binary-embedding
rotation, and a tree structure compatible with streaming; CIDs are
rendered in base32-nopad-lowercase (RFC 4648 [18]), which is URL-safe,
case-insensitive, padding-free, and free of slash collisions inside path
segments. A 128-bit truncation at the fact level places a birthday
collision at roughly $2^{64}$ facts, against a corpus on the order of
$10^5$ today.

These primitives are individually standard. What is uncommon is their
application to a *memory* abstraction: a transparency log over Earth-
observation facts, an Ed25519 receipt that an agent quotes as a citation,
and a deterministic CID that two agents on different runtimes dereference
to identical bytes. The composed trust plane is shown in
[10-trust-plane.svg](/docs/diagrams/10-trust-plane.svg).

### 2.4 Position

Placing emem against the three threads yields a single empty cell that
the prior work does not occupy. Agent-memory systems
[5]–[10], [12], [13] are persistent and writable but single-agent-scoped
and unsigned. Geospatial standards (DGGS [20], [21]; STAC; GeoParquet;
IPLD [19]) are planet-keyed but stop at catalogs and formats with no
per-fact signature or proof. Verifiable-data primitives [14]–[18] are
tamper-evident and deterministic but are not, by themselves, a memory of
anything. emem is the system that is simultaneously persistent,
planet-keyed at ~9.55 m cells, content-addressed by BLAKE3, verifiable
offline by Ed25519 over a domain-separated preimage with Merkle
inclusion, and shared byte-for-byte across agents and replicas. The
remainder of the paper specifies how the three threads are joined: the
address algebra and trust plane (Section 3), the band and algorithm
registries (Sections 3.6 and 4), and the primitive surface that
makes the composition usable by an agent (Section 6).

## 3 The emem Protocol

emem is defined by two coordinate algebras and one trust plane. The coordinate algebras place every observation at a deterministic spatial and temporal address; the trust plane content-addresses the observation, signs it, and binds it into an append-only Merkle log so that any party can verify a read offline. This section specifies the bit-level layout of the address space (Section 3.1), the temporal bucketing (Section 3.2), the content-addressing rules (Section 3.3), the receipt and attestation cryptography (Section 3.4), the signed-absence mechanism that distinguishes a confirmed negative from a missing value (Section 3.5), and the conformance manifests and bi-temporal read model that every response carries (Sections 3.6–3.7). The design goal throughout is that two independent implementations, given byte-identical inputs, produce byte-identical content identifiers, and that a verifier with no access to the originating server can reconstruct and check every signed quantity.

### 3.1 The cell64 address algebra

A spatial address in emem is a 64-bit integer, the `cell64`, that names a fixed cell on the WGS-84 ellipsoid. The geographic mode packs the word as four contiguous fields followed by the quantised coordinate pair:

| Bits | Field | Value | Role |
|------|-------|-------|------|
| 63..60 | mode | `0b0001` | geographic cell |
| 59..52 | resolution | `21` | encoded as the lat-axis bit count |
| 51..44 | base | `0xab` | "geo aperture" marker separating the layout from H3-style words |
| 43..43 | reserved | `0` | zero on encode, pass-through on decode |
| 42..22 | lat_q | 21 bits | quantised latitude over $[0, 2^{21})$ |
| 21..00 | lng_q | 22 bits | quantised longitude over $[0, 2^{22})$ |

The encoder maps a finite WGS-84 pair $(\varphi, \lambda)$, in degrees, to the quantised indices

$$\mathrm{lat\_q} = \operatorname{round}\!\left(\frac{\varphi + 90}{180}\,(2^{21}-1)\right), \qquad \mathrm{lng\_q} = \operatorname{round}\!\left(\frac{\lambda + 180}{360}\,(2^{22}-1)\right),$$

with rounding taken half-away-from-zero, latitude clamped to $[-90, 90]$ and longitude wrapped to $[-180, 180)$ by Euclidean remainder. The packed word is $\texttt{raw} = (1 \ll 60)\,|\,(21 \ll 52)\,|\,(\texttt{0xab} \ll 44)\,|\,(\mathrm{lat\_q} \ll 22)\,|\,\mathrm{lng\_q}$. Decoding inverts the affine maps to the bucket centre and reports the bucket bounding box, $\pm \tfrac{1}{2}$ quantum on each axis, clipped to $[-90, 90]$ in latitude.

The asymmetric bit budget, 21 bits on latitude against 22 on longitude, is deliberate. Latitude spans $180^\circ$ and longitude spans $360^\circ$; allocating one extra bit to the wider axis equalises the angular quantum, $180^\circ / (2^{21}-1) \approx 360^\circ / (2^{22}-1) \approx 8.583\times10^{-5}\,^\circ$. Projected to the ground at the equator with a mean $111{,}319.491$ m per degree, both axes land on a single canonical pitch,

$$\texttt{CELL\_PITCH\_M\_EQUATOR} = \frac{180^\circ}{2^{21}-1}\,\cdot\,111{,}319.491 = 9.5546\ \text{m},$$

with the two axes agreeing to within roughly two parts per million. The cell is therefore square at the equator, matching the native 10 m pixel pitch of Sentinel-2 optical and Sentinel-1 RTC products [28] so that a fact can be materialised per pixel without resampling loss. Equal bit counts would instead yield 1:2 rectangular cells. Above the equator the longitude pitch narrows by $\cos\varphi$, so cells become taller than wide; this is the standard distortion of any equirectangular grid, and emem exposes an explicit weight $w(\varphi) = \max(\cos\varphi, 0)$ for area-correct aggregation across latitudes. This constant is the single source of truth for every per-cell ground distance in the system (heat and wave solvers, EUDR per-plot area disclosure), pinned by a test to within $0.01$ m so that no downstream module reintroduces a divergent literal.

Two geometric degeneracies are canonicalised at encode time. At the antimeridian, $\lambda = +180^\circ$ and $\lambda = -180^\circ$ name the same meridian; coordinates just under $+180^\circ$ would round into $\mathrm{lng\_q} = 2^{22}-1$ while $-180^\circ$ lands at $\mathrm{lng\_q} = 0$, so the encoder collapses the former to the latter. At the poles the longitude axis is geometrically meaningless, so the two pole rows ($\mathrm{lat\_q} \in \{0, 2^{21}-1\}$) force $\mathrm{lng\_q} = 0$; without this, a pole-anchored fact would split across $4{,}194{,}304$ distinct words for one physical point. A strict encoder variant refuses non-finite or out-of-range inputs (with a $10^{-6}\,^\circ$ tolerance for floating-point noise) rather than silently clamping, so a mistyped latitude of $91^\circ$ surfaces a typed error instead of signing a fact at the pole.

The text form renders the 64-bit word as four 16-bit lanes, each indexing a deterministic 65,536-entry alphabet, joined by dots (for example `dedi.zaf00.bafi.baba` for $(0,0)$). The alphabet is the outer product of 21 consonants and 10 vowels in a fixed loop order, $21\times10\times21\times10 = 44{,}100$ consonant-vowel-consonant-vowel bigrams covering indices $0$ through $44{,}099$, with indices $44{,}100$ through $65{,}535$ filled by synthetic `z<hex4>` codepoints. The alphabet ordering is Hilbert-ordered at the bigram level, so adjacent codepoints tend to map to nearby cells in the visual ordering; exact spatial neighbourhoods are served through the dedicated neighbourhood field of the locate primitive rather than by string-prefix arithmetic, because the quantisation-level Hilbert curve requires equal-bit axes that the square-at-equator budget gives up. The resolution field doubles as a version gate: a pre-0.0.3 grid used a 16-bit-per-axis (${\sim}305$ m) encoding with resolution tag $12$, and because the prefix mask keys on the resolution field, such legacy strings fail closed with a typed `NotGeoCell` error rather than silently misplacing a fact by hundreds of metres. The active grid in version 0.1.0 is cell64; the eventual migration target is an H3-style hexagonal hierarchical DGGS [20] at resolution 13 (${\sim}3.4$ m equal-area cells), which trades the equirectangular $\cos\varphi$ distortion for per-cell equal-area pixels. The base marker $\texttt{0xab}$ exists precisely to keep the current layout distinguishable from such a future word, and the S2 spherical-cell hierarchy [21] is a reference point for the equal-area indexing properties the migration seeks. Figure `/docs/diagrams/09-address-algebra.svg` illustrates the encode path.

### 3.2 The tslot temporal algebra

A temporal address, the `tslot`, is an unsigned-64 bucket index of the Unix timeline at a band's declared cadence,

$$\texttt{tslot} = \left\lfloor \frac{\max(t_{\text{unix}},\,0)}{\texttt{slot\_seconds}(\text{tempo})} \right\rfloor,$$

where $t_{\text{unix}}$ is the observation's Unix epoch second. Bucketing is anchored at the Unix epoch ($1970$-$01$-$01$T$00{:}00{:}00$Z), not at the emem reference epoch. An earlier design subtracted the emem epoch ($2026$-$01$-$01$, retained only as protocol metadata) before bucketing, which collapsed every pre-2026 observation to slot zero and made historical backfill unaddressable; anchoring at the Unix epoch matches how every other Earth-observation archive stores time, and pre-1970 timestamps clamp to slot zero. The inverse, $\texttt{to\_unix\_start}$, returns the Unix second at which a slot opened.

Each band declares one of five tempo classes, which fixes its slot duration, cache time-to-live, and refinement schedule:

| Tempo | `slot_seconds` | Cadence | Representative bands |
|-------|----------------|---------|----------------------|
| Static | $0$ | never changes | Copernicus DEM [30], Köppen-Geiger [36] |
| Slow | $31{,}536{,}000$ | annual | Tessera embeddings [1], SoilGrids [34] |
| Medium | $2{,}592{,}000$ | $30$ d | monthly NDVI composites |
| Fast | $86{,}400$ | daily | raw Sentinel-2 NDVI |
| UltraFast | $3{,}600$ | hourly | weather, traffic |

Two additional MODIS-aligned variants, `composite_16day` ($1{,}382{,}400$ s) and `composite_8day` ($691{,}200$ s), bucket the 16-day MOD13Q1 NDVI and 8-day LST/ET/GPP/LAI granules [29]: under the coarser monthly Medium tempo two adjacent 16-day granules would collide in one bucket, which the dedicated variant prevents. A `Static` band returns slot zero regardless of input, since the slot is meaningless for a quantity that never refreshes. Wall-clock recovery is exact within the slot grain: $\texttt{to\_unix\_start}$ recovers the bucket boundary, and the fact's separate `signed_at` field carries the ISO 8601 wall clock at signing, which is the transaction time and is distinct from the data time the tslot encodes. The text form `t.<base32-leb128>` encodes the integer as little-endian LEB128, base32-encodes the bytes without padding [18], and lowercases (so $\texttt{Tslot}(1024) \to \texttt{t.qaea}$).

### 3.3 Content addressing

Every fact, edge, and registry manifest is content-addressed by a BLAKE3 [17] digest over its canonical CBOR encoding. The emem-CBOR profile is RFC 8949 deterministic encoding [16] with four mandatory tags: $65000$ for a packed cell64 (the Section 3.1 word as a tagged $u64$), $65001$ for a tslot ($u64$), $65002$ for a 32-byte vector CID, and the standard IPLD tag $42$ for a multibase base32 CID string [19]. The encoder, `ciborium::ser::into_writer`, emits deterministic CBOR whenever the input traversal is deterministic; serde-derived structs serialise their fields in declaration order, and free-form maps must be presented with keys already sorted, since emem does not silently re-sort. The conformance contract is byte-identity: encode, decode, re-encode, and compare must round-trip exactly, which is what makes the resulting digest a content identifier rather than a server-local handle.

A content identifier is the base32-nopad-lowercase encoding [18] of a prefix of the 32-byte BLAKE3 digest. Two durable lengths exist. The `fact_cid` is the 16-byte (128-bit) prefix, $26$ base32 characters,

$$\texttt{fact\_cid} = \mathrm{base32}_{\text{nopad}}\bigl(\,\mathrm{BLAKE3}(\mathrm{canonical\_cbor}(\text{fact}))[0{:}16]\,\bigr),$$

which is the form signed, stored, and cited in receipts. The `cid64` is the 8-byte (64-bit) prefix, $13$ characters, used only as a short visible handle in token-economical inline text; it is a prefix, so full collision resistance requires the complete digest. Registry manifest CIDs use the full 32-byte digest (52 characters), because a manifest CID appears once per response in the `registry_cid` and `schema_cid` fields rather than once per fact, so the larger size costs nothing at scale. Mutating any field of a fact changes its CBOR bytes and therefore its `fact_cid`; the same recipe constructs the strongly-typed `RegistryCid`, `SchemaCid`, `ReasonCid`, `BatchCid`, `CoverageCid`, and `EdgeCid` newtypes, which keep the wire string identical while distinguishing purposes at compile time.

### 3.4 The trust plane

Every read returns a receipt: a signed, rebindable proof that a particular set of facts was served. A receipt carries a request identifier (ULID), an ISO 8601 serve time, the primitive name, the cited cell64 and `fact_cid` lists, the response `schema_cid`, the responder's 32-byte Ed25519 public key [15] and key-rotation epoch, the per-source version pins, the serving `registry_cid`, a self-declared cost and latency block, an optional Merkle inclusion proof, and the Ed25519 signature itself.

The signature is taken over a v1 preimage that is domain-separated and length-prefixed. The preimage builder begins each stream with the context label `emem.preimage.v1\x00` followed by the length-prefixed domain string, then appends each field as a tagged, length-prefixed segment, $\texttt{tag}\;\|\;\text{len}_{\text{u32-LE}}\;\|\;\text{bytes}$, and lists as $\texttt{tag}\;\|\;\text{count}_{\text{u32-LE}}\;\|\;(\text{len}\;\|\;\text{bytes})^\ast$. The signed digest is then

$$\texttt{preimage} = \mathrm{BLAKE3}\bigl(\texttt{"emem.preimage.v1"} \,\|\, \texttt{len(domain)} \,\|\, \text{"receipt"} \,\|\, \textstyle\bigsqcup_i \texttt{seg}(\texttt{tag}_i, \texttt{field}_i)\bigr),$$

with stable receipt segment tags for the request id, serve time, optional scope digest, optional as-of digest, optional edge digest, optional manifest digest, primitive, cell list, and fact-cid list. The attestation preimage is the analogous stream over the domain `"attestation"` with tagged batch root, registry CID, and schema CID. Domain separation and length-prefixing close two structural weaknesses of the predecessor (v0) rule, which concatenated variable-length fields without delimiters: under v0 the field pairs $(\texttt{"abc"},\texttt{"def"})$ and $(\texttt{"abcd"},\texttt{"ef"})$ hashed identically, and an untagged 64-hex scope digest was indistinguishable from an untagged as-of digest occupying the same position. Receipts and attestations carry an explicit `preimage_version` byte; a value of $0$ (omitted from the wire) denotes the legacy rule and a value of $1$ the v1 rule, so every receipt signed before the cutover continues to verify byte-for-byte under its original preimage. The signing path is a single canonical builder shared by the responder, the REST verifier, and the in-browser verifier, so no call site hand-rolls preimage bytes; a pinned cross-language wire-anchor test fixes the exact v1 byte stream that the browser port must reproduce.

When a read cites facts that belong to a signed batch, the receipt carries a Merkle inclusion proof to that batch's root. emem batches facts into attestations: the leaf of each fact is its $\mathrm{BLAKE3}(\mathrm{canonical\_cbor}(\text{fact}))$ digest (plus each edge's digest), the leaves are bytewise-sorted into canonical order, and a binary Merkle tree is folded to a 32-byte `batch_root` that the attester signs. The v1 tree uses RFC 6962 domain separation [14]: a leaf is promoted as $\mathrm{BLAKE3}(\texttt{0x00}\,\|\,\text{leaf})$ and an internal node as $\mathrm{BLAKE3}(\texttt{0x01}\,\|\,L\,\|\,R)$, so a promoted leaf can never collide with an internal node over the same bytes. Odd layers pair the trailing element with itself, which by itself admits a root-equivocation: $\operatorname{root}([A,B,C]) = \operatorname{root}([A,B,C,C])$, the CVE-2012-2459 pattern. emem closes this by requiring the canonical leaf order to be sorted-and-deduplicated and by having both the signer and every verifier reject any batch with an adjacent duplicate leaf (adjacency follows from sorted order); the signer refuses to emit a duplicate batch rather than silently deduplicating, since a duplicated fact CID adds nothing honest. An inclusion proof is the bottom-up sibling sequence plus the leaf index; verification re-promotes the leaf, folds it with each sibling on the side dictated by the index parity, and checks equality with the root. The proof records its own hashing-rule version so v0 and v1 proofs are not confused.

Attestations are appended to a fsync'd, append-only Merkle log on disk. Each record is $[\text{len}_{\text{u32-LE}}\,\|\,\text{cbor}\,\|\,\mathrm{BLAKE3}(\text{cbor})_{32}]$; segment files rotate at 1 GiB and carry a trailing per-segment hash equal to BLAKE3 over all their records. The append path flushes and `sync_all`s before returning, so the cryptographic durability claim a receipt makes is backed by a synced write; replay-restore re-hashes each sealed segment and checks its trailing hash, and snapshots ship the segment plus its hash to external storage. Responder identity is a 32-byte Ed25519 secret stored at `var/emem/identity.secret.b32`; the $u32$ key epoch in every receipt and attestation increments on rotation, so a verifier detects key changes through the epoch field. Verification uses strict Ed25519 (`verify_strict`), and the entire check, canonical preimage reconstruction, Ed25519 verification, and Merkle path replay, is reproduced in JavaScript at the in-browser `/verify` endpoint using audited noble cryptography libraries, so a third party can validate a receipt with no trust in the originating server. Figure `/docs/diagrams/10-trust-plane.svg` traces the path from preimage to offline verification.

### 3.5 Signed Absence

emem distinguishes three fact variants. A `Primary` fact is a directly attested reading at a $(\text{cell}, \text{band}, \text{tslot})$, carrying its typed value, optional SI unit, confidence in $[0,1]$, optional uncertainty distribution, at least one source reference, a re-execution recipe (function-registry key plus deterministic arguments), a privacy class, and an optional `served_via` block recording which compute tier (GPU sidecar, CPU, scalar, cached, or absence) actually produced the value together with the model identifier and a checkpoint digest, so an agent can read provenance from the receipt without re-running the recipe. A `Derivative` fact is a deterministic function over parent fact CIDs (delta, mean, trend, rate, anomaly) over an inclusive tslot window. The third variant, `Absence` (a `NegativeFact`), is a signed assertion that a quantity is confirmed absent at a $(\text{cell}, \text{band}, \text{tslot})$, distinct from a null, an unknown, or an empty result, and it carries a `reason_cid` pointing at the evidence that established the absence (for example a Sentinel-1 scene). The motivating threat is that an unsigned "no data here" returned through an outside channel is indistinguishable from a poisoning injection [41]; a signed absence with a typed reason is a positive, verifiable statement that the responder looked and found nothing.

Absence reasons are typed. The wire enumeration includes `unavailable_capability` (the responder cannot run the required algorithm), `outside_coverage` (the location falls outside a product's geographic extent), `gpu_unavailable` (a foundation-embedding band whose GPU sidecar is unreachable, signed honestly rather than served from a divergent CPU path because the kernel-order accumulation would differ), `archetype_seed_unavailable` (a classification seed table is missing), and `upstream_error` / `upstream_timeout` for fetch failures. The same mechanism expresses materialisation timeouts and misses and the over-water/over-land gating that the hunting primitives apply when a query lands on the wrong land-cover class. Because an `Absence` is itself a fact, it is content-addressed, batched, signed, and Merkle-committed exactly as a `Primary` fact, so a downstream consumer can cite "this region was checked and is confirmed empty" with the same cryptographic weight as a positive reading.

### 3.6 Conformance manifests

The protocol's registries are content-addressed manifests, and conformance is byte-identity over them. The registry set comprises bands, algorithms, functions, sources, topics, the CDDL schema, the land-cover/value (lcv-1) table, and the rendering alphabet; each is hashed by the manifest recipe, $\mathrm{base32}(\mathrm{BLAKE3}(\mathrm{canonical\_cbor}(\text{manifest}))[0{:}32])$. A receipt binds the `schema_cid` and `registry_cid` as struct fields, and the remaining manifest CIDs are exposed at a manifests endpoint and the `.well-known` descriptor. Two implementations conform when, given byte-identical inputs, they produce byte-identical CIDs over this manifest set; a verifier re-pulls the manifests at receipt time and checks that the CIDs match the ones the receipt was signed against. New bands, algorithms, sources, and memory kinds extend the registries without breaking older receipts, since each registry version is itself a content address and the function-registry CID a fact cites pins the exact recipe version in force when it was produced.

### 3.7 The bi-temporal read model

Every read in emem is bi-temporal: it separates valid time (when an observation holds in the world) from transaction time (when the system learned it). Valid time is the fact's tslot; transaction time is its `signed_at` wall clock. A read accepts two optional bounds, `as_of_tslot` (a valid-time ceiling) and `as_of_signed_at` (a transaction-time ceiling, an RFC 3339 timestamp), with the semantics "return the latest fact per $(\text{cell}, \text{band})$ whose tslot $\le$ `as_of_tslot` and whose `signed_at` $\le$ `as_of_signed_at`." Absent bounds give a current-state read for back-compatibility; both bounds intersect when both are set. When either bound is set, the receipt records the binding in an `as_of` block whose digest enters the v1 preimage as a tagged segment, so a verifier can replay the identical query later by reissuing it with the same bounds; transaction-time strings are normalised to a canonical UTC spelling before hashing so that `Z`, `+00:00`, and fractional-second forms of the same instant produce the same digest. The bi-temporal block is omitted entirely from an unbounded read so that pre-bi-temporal receipts round-trip and verify byte-for-byte. This two-axis model is what lets emem answer "what did we believe about this place on this date" rather than only "what is the current best estimate," which is the property that separates an auditable memory of Earth from a cache of latest values.

## 4 Foundation-Embedding Layer

emem treats Earth-observation foundation models as frozen feature
extractors rather than fine-tuned task heads. Four pretrained encoders
feed the reference responder — Tessera [1], Clay v1.5 [2],
Prithvi-EO-2.0-300M-TL [3], and Galileo [4] — and the protocol never
back-propagates through any of them. The forward pass of each encoder
is whatever its upstream self-supervised objective produced, and the
signed receipt records this with a `frozen_pretrained_encoder` warning
so that a downstream agent reading an embedding-derived fact knows it
is consuming a representation, not a calibrated estimator. The design
argument of this layer is that the four encoders were trained on
different sensors, at different ground sampling distances, with
different receptive fields and different masked-reconstruction
objectives, and that this independence is the asset to exploit. A
single encoder asked "did this cell change between two years" produces
spurious change wherever its chip boundary intersects a real
land-cover edge; an ensemble of encoders whose receptive fields alias
that edge differently converts a lone vote into a flag for
receptive-field artifact rather than a claim of land-surface change.
Section 4.3 formalizes this.

![encoders in orbit, decoders on the ground](/docs/diagrams/31-encoders-in-orbit-decoders-on-ground.svg)

### 4.1 The four encoders

The encoders differ along five axes that matter for fusion: native
modality, input tensor shape, output dimension, effective receptive
field, and serving mode. Table 4.1 collects them. Three of the four are
ViT-scale models that run only on a GPU; the fourth is a published
raster product that streams on CPU.

| Encoder | Modality | Input shape | Dim | Receptive field | Serving mode |
|---|---|---|---|---|---|
| Tessera [1] | annual multi-sensor stack | per-pixel, $0.1^\circ$ COG grid | 128 (×8 vintages) | per-pixel ($\approx 10$ m) | CPU-streamed from COG tiles |
| Clay v1.5 [2] | S1 / S2 / Landsat / NAIP | $[B, C, 256, 256]$ wavelength-conditioned | 1024 | $\approx 2.56$ km | GPU sidecar |
| Prithvi-EO-2.0-300M-TL [3] | HLS V2 (6-band) | $[B, T{=}1, 224, 224, 6]$ | 1024 | $\approx 6.7$ km | GPU sidecar |
| Galileo [4] | S1 / S2 / DEM / climate (S2 wired) | $[1, T{=}1, 8, 8, 10]$ | variant-selectable | $\approx 240$ m chip | GPU sidecar |

*Table 4.1: the foundation-embedding layer. Receptive fields are
chip-scale extents at the encoder's native ground sampling distance.*

**Tessera** is a global per-pixel annual embedding published as
Cloud-Optimized GeoTIFF tiles on a $0.1^\circ$ grid, with one 128-D
vector per pixel per year for the vintages 2017–2024 [1]. emem
materializes it as eight per-year bands `geotessera.{2017..2024}` plus
a 1024-D `geotessera.multi_year` stack that concatenates the eight
128-D vintages, NaN-masking any year that is absent at the cell.
Because Tessera ships as a raster rather than as model weights, the
responder reads it with a windowed COG sampler and never invokes GPU
inference for it. This is the property the `/v1/tessera_field`
primitive uses to render a dense embedding field over an entire region:
a $0.1^\circ$ grid of 128-D vectors is read directly from the tiles
through the shared `read_tessera_field_grid` helper, and the same
helper backs the deterministic land-cover classifier of
`/v1/region_archetype_map`. Tessera is the only encoder in the layer
that is geo-streamable at field scale; the other three are GPU-pinned
point encoders and the responder does not fabricate fields for them.

**Clay v1.5** is a ViT-L/8 masked autoencoder trained with a DINOv2
teacher and conditioned on per-band centre wavelengths, so a single
checkpoint embeds S1, S2, Landsat, NAIP, and multi-sensor chips [2].
The responder feeds it a $256 \times 256$ chip assembled at uniform
10 m sampling and reads the 1024-D post-norm CLS token. At 10 m S2 the
chip subtends $\approx 2.56$ km. The encoder's Matryoshka head exposes
nested dimensions $\{16, 32, 64, 128, 256, 768, 1024\}$; emem serves
the full 1024-D vector by default. Per-band wavelength, mean, and
standard-deviation priors come from a pinned metadata file, and the
chip ships in native upstream reflectance scale so that the encoder
applies its own normalization.

**Prithvi-EO-2.0-300M-TL** is a 300M-parameter multi-temporal masked
autoencoder pretrained on Harmonized Landsat–Sentinel (HLS V2) imagery
[3]. The responder assembles a $224 \times 224 \times 6$ chip
(Blue, Green, Red, narrow-NIR, SWIR1, SWIR2) at uniform 30 m sampling,
giving a physical extent of $6720 \times 6720$ m and a chip-scale
receptive field of $\approx 6.7$ km, and reads the 1024-D CLS token.
Sentinel-2 L2A is a near-substitute for HLS V2 (shared Sen2Cor
lineage) but is not identical: the Landsat-9 cross-sensor harmonization
terms are absent, and the receipt flags this as
`s2_l2a_substitute_for_hls_v2` alongside the source scene's STAC item,
its cloud fraction, and the lookback tier the scene search settled on.

**Galileo** is a multimodal model trained jointly over Sentinel-1,
Sentinel-2, DEM, and climate inputs with a global-plus-local
masked-reconstruction objective [4]. emem wires the Sentinel-2 modality
and feeds a $8 \times 8 \times 10$ chip at uniform 30 m sampling
($240 \times 240$ m extent); the remaining modality tensors are
zero-filled and their group masks held at 1 (not-seen), a configuration
the encoder accepts because it was trained with the full mask-ratio
schedule. The embedding is the average over the unmasked tokens. The
output dimension is variant-selectable through `EMEM_GALILEO_VARIANT`:
the deployed default is `base` at 768-D ($\approx 86.5$M parameters,
$\approx 330$ MB), with `tiny` at 192-D ($\approx 22$ MB) and `nano`
also selectable. The advertised capability string becomes
`galileo-<variant>` in `/v1/capabilities.extensions[]` so an agent can
read which dimension ships at request time rather than assuming one.

### 4.2 Serving: a GPU sidecar behind a Unix-domain socket

The three ViT-scale encoders co-reside on a single GPU under a 20 GB
VRAM budget (`EMEM_SIDECAR_VRAM_BUDGET_GB`, default 20). The budget is
enforced once at registry init by
`torch.cuda.set_per_process_memory_fraction`; the per-model accounting
(Clay 2.5 GB, Prithvi 3.0 GB, dynamics 0.1 GB, with the balance held in
reserve) is advisory partitioning, not a set of independent hard caps,
and any allocation that would push the process past the global cap
raises a CUDA out-of-memory error that surfaces to callers as a 503.
Cold latencies are dominated by streaming the checkpoint from disk and
building the CUDA context: $\approx 6$ s for Clay, $\approx 10$ s for
Prithvi, and $\approx 4$–10 s for Galileo depending on variant. Once
resident the encoders are warm at $\approx 18$ ms (Clay),
$\approx 20$ ms (Prithvi), and $\approx 14$–25 ms (Galileo). Models
stay resident for the lifetime of the sidecar process; a failed load is
retried only by process restart.

GPU work is isolated in a Python FastAPI service reached over a
Unix-domain socket, never over TCP. The Rust responder speaks a small
hand-rolled HTTP/1.1 client to the socket
(`${XDG_RUNTIME_DIR}/emem/jepa_sidecar.sock`), keeping CUDA out of the
API process so that an out-of-memory event becomes a structured 503
rather than a crash of the request path that also serves recall,
receipts, and the in-process physics solvers. Two failure modes are
decoupled: a sidecar that is unreachable yields
`SidecarError::Unavailable` and a fallback where one is wired; a sidecar
that is reachable but refuses (OOM, missing checkpoint, checkpoint-hash
mismatch) yields `SidecarError::Upstream` and a 502, and the Rust path
is forbidden from silently retrying without the GPU. At ViT scale there
is no in-process CPU rerun: a cold materialization request for Clay,
Prithvi, or Galileo that finds the sidecar down returns 503 with the
sidecar's error body, and recall on those bands returns only what has
already been signed.

The sidecar's contract is the single source of truth for what was
executed. Every `/predict/<encoder>` response carries a `model` block
with `model_id`, `version`, a BLAKE2b-256 hash of the on-disk
checkpoint, `via="python_sidecar"`, and any `honesty_warnings`; the
Rust caller forwards this object verbatim into the signed receipt's
derivation arguments. A verifier re-deriving an embedding-derived fact
re-hashes the checkpoint on disk and compares it byte-for-byte to the
receipt's `model.blake2b_hex`, so a swapped weight file under a stale
metadata record cannot serve a fresh receipt for old weights.

### 4.3 Triple-encoder consensus

The change algorithm `clay_prithvi_tessera_triple_consensus@1` votes
across Clay, Prithvi, and Tessera on a per-cell year-over-year change
index over a 365-day window. Each encoder contributes one scalar
change magnitude, the cosine distance between its current and its
prior vector, clamped to the unit interval:

$$ d_e = \mathrm{clamp}\!\left(1 - \cos(v_e^{\text{now}}, v_e^{\text{prev}}),\, 0,\, 1\right), \qquad e \in \{\text{clay}, \text{prithvi}, \text{tessera}\}. $$

Cosine is computed only over dimensions finite in both vintages
(`cosine_finite`), so a NaN-masked Tessera year or a partially covered
vintage degrades the inner product rather than poisoning it. For
Tessera the two vintages are the latest and previous covered slices of
the `geotessera.multi_year` stack, obtained from one CPU recall. Clay
and Prithvi ship only a single (latest) vintage from their
materializers, so a year-over-year cosine for those two is computable
only when the store already holds two distinct-tslot facts for the
encoder at the cell; when only the latest exists, the responder
materializes a prior vintage on demand by stepping the scene search
back $\approx 370$ days into a different annual bucket. Crucially, it
never synthesizes a prior vector: if a second distinct vintage cannot
be obtained (GPU down, cold cell, only one S2 scene in the archive),
that encoder is reported under `encoders_absent[]` with a typed reason
and dropped from the fusion.

Let $A \subseteq \{\text{clay}, \text{prithvi}, \text{tessera}\}$ be
the set of encoders for which $d_e$ is computable. The ensemble is the
quadratic mean (RMS) over the available components only:

$$ E = \sqrt{\frac{1}{|A|} \sum_{e \in A} d_e^2}, \qquad |A| \ge \texttt{consensus\_min\_models}. $$

When $|A|$ falls below `consensus_min_models` (default 2) the response
is an honest `inconclusive` verdict carrying no ensemble number, on the
principle that a fabricated value is worse than a declared absence. The
typical GPU-less outcome is Tessera-only, $|A| = 1$, which returns
`inconclusive` rather than a single-encoder score dressed as a triple.
Agreement is reported as a tier rather than a point estimate. With a
gate $g = \texttt{consensus\_threshold}$ and
$n_g = |\{e \in A : d_e > g\}|$ the count of encoders clearing the gate,

$$ \text{agreement} = \begin{cases} \texttt{all\_three} & |A| = 3 \,\wedge\, n_g = 3, \\ \texttt{two\_of\_three} & n_g \ge 2, \\ \texttt{one\_or\_none} & \text{otherwise}. \end{cases} $$

The default gate is $g = 0.15$, taken from the LandTrendr ensemble
convention of Healey et al. [37]; it is a registry parameter, not a
compiled constant, so re-tuning happens at registry-CID time through
the algorithm's `parameters` block and every gate carries a
`_threshold_learned_from` citation. The `all_three` tier additionally
requires that all three encoders were available, not merely that two
cleared the gate, so a two-encoder ensemble can never be reported as a
full triple.

The independence of the receptive fields is what gives the agreement
tiers their meaning. Clay aliases an edge at its $\approx 2.56$ km
chip scale, Prithvi at $\approx 6.7$ km, and Tessera operates
pixel-by-pixel; a real land-surface transition that all three resolve
produces a high $d_e$ in all three, whereas a chip-boundary artifact
that trips one encoder is unlikely to trip the others because their
chip footprints and ground sampling distances differ. A cell where all
three agree is treated as change; a cell where only one fires
(`one_or_none`) is read as a receptive-field-aliasing flag and not as
detected change. This is the central design argument of the layer:
heterogeneous receptive fields turn a lone vote into a diagnostic
about the sensor geometry rather than a claim about the ground.

### 4.4 Domain variants

The base consensus carries six domain-specialized variants that retune
the gate and add a corroborating leg:

| Variant | Gate | Extra leg |
|---|---|---|
| `deforestation_triple@1` | 0.20 | Hansen GFC [32] loss-year mask elevates $\ge 2$ votes to `hansen_confirmed`; EUDR [38] pre-screen |
| `wetland_change_triple@1` | 0.10 (abs 15 occ. pts) | JRC Global Surface Water [31] recurrence delta replaces the Tessera leg |
| `urban_expansion_triple@1` | 0.20 | Overture [39] `buildings.count` delta + S2 B11 SWIR corroboration tag |
| `disaster_anomaly_triple@1` | n/a | spatial $2\sigma$ neighbour z-score, single-pass (no temporal recipe) |
| `climate_archetype_triple@1` | n/a | 12-class Köppen–Geiger classifier seeded from Beck et al. [36] type-locality centroids |
| `coastal_erosion_triple@1` | 0.12 | bathymetry-clamped to cells where `gmrt.topobathy_mean` $\in [-5, +5]$ m |

*Table 4.2: domain variants. Each gate threshold carries a
`_threshold_learned_from` citation to the product or paper it was
estimated against.*

The disaster and climate-archetype variants are spatial rather than
temporal: the former scores a cell against the distribution of its
neighbours in one pass, and the latter is a deterministic classifier
that assigns each cell to one of twelve Köppen–Geiger classes by
nearest seeded centroid, with the seed table drawn from the 1-km
Köppen–Geiger map of Beck et al. [36]. The `deforestation_triple`
variant is the leg most exercised in production: it gates the EUDR
due-diligence pre-screen by elevating any cell with two or more
consensus votes that also intersects the Hansen loss-year mask to a
`hansen_confirmed` tag, keeping the embedding consensus a triage signal
that an independent product confirms before it credits a decision.

### 4.5 An open source-monitoring gap

The ensemble of Section 4.3 weights every available encoder equally:
`fuse()` is an unweighted root-mean-square over $A$, and the agreement
tiers count gate crossings without regard to which encoder crossed. In
the framing of complementary-learning-systems memory analyses, where a
hybrid system is expected to attribute a recalled signal to the
specific source that produced it [5], this is a source-monitoring gap.
An RMS fusion cannot express that Prithvi's $\approx 6.7$ km receptive
field should count less than Tessera's per-pixel resolution on a
small-plot change, nor that a particular encoder is the more reliable
witness for a given land-cover class. The current implementation is
deliberately conservative on this point: it reports the per-encoder
$d_e$, the per-encoder gate crossings, and the input fact CIDs verbatim
in the response so that an analyst can audit which encoder drove a
verdict, but the fusion itself applies no learned per-source weights.
Source-aware weighting is the natural next step for the layer, and the
unweighted RMS is the honest baseline it would replace rather than a
claim of optimal fusion.

## 5 Memory Mechanisms

The address algebra and trust plane of the preceding sections give emem a stable,
verifiable substrate for *facts about places*. This section describes the layer
that turns a fact store into a memory: the writable agent-memory surface, the
bi-temporal reading discipline that lets a verifier replay what was known at any
past instant, the typed temporal edges and multi-attester contradiction scoring
that let the corpus connect and disagree, and the consolidation machinery that
moves episodic observations toward semantic summaries without ever destroying the
originals. Throughout, the design borrows the vocabulary of cognitive memory and
of recent LLM-agent memory architectures [5, 7, 9], but it grounds every borrowed
notion in a signed, content-addressed operation rather than a heuristic. A
recurring theme is that emem is *additive and non-destructive*: every mechanism
either appends a new content-addressed object or stamps a pointer, so the audit
history of what the corpus believed, and when, is always replayable.

### 5.1 The agent-memory layer and the cognitive correspondence

emem separates two strata. Below sits the spatial fact store, whose values are
produced by materialisers from upstream Earth-observation sources and read through
`recall`, `state`, `find_similar`, and `trajectory`. Above it sits a writable
*memory substrate* that an agent uses as durable, capability-bound scratchpad,
reached through the same Ed25519 receipt surface as every spatial primitive. The
two strata meet at the receipt CID: a fact the agent recalls can be cited inside
a memory file it writes, and the contradiction detector treats agent-attested
writes and machine-materialised facts as the same kind of signed observation.

The substrate is typed with the CoALA agent-memory ontology [7]. Every memory file
carries a `kind` drawn from $\{\textsf{episodic}, \textsf{semantic},
\textsf{procedural}, \textsf{resource}\}$: episodic files record events ("the agent
ran $X$ at cell $Y$ on date $Z$"), semantic files hold durable learned facts,
procedural files hold playbooks, and `resource` (the default, for back-compatibility
with the Anthropic memory-tool shape [22]) is a generic durable scratchpad. The
taxonomy is not decorative; it drives the per-kind retention policy of §5.4 and the
candidate selection of the consolidation and sleep workers. The cognitive
correspondence is deliberate but bounded: emem does not claim to be a model of
human memory, it claims that the *operational* distinctions cognitive science draws
between episodic and semantic stores, between fast hippocampal encoding and slow
neocortical consolidation [5], map cleanly onto operations a content-addressed store
can perform exactly and verifiably. Where a cognitive mechanism cannot be reduced to
a signed operation, emem does not implement a facsimile of it (§5.6).

### 5.2 Bi-temporal recall semantics

Every read primitive in the substrate (`recall`, `recall_polygon`, `recall_many`,
`trajectory`, `query_region`, `find_similar`, `state`, `state_multi`, and each triple
inside `memory_bundle`) accepts two optional bounds: an observation-time bound
`as_of_tslot` ($\in \mathbb{N}$, on the tslot scale of §3.2) and a transaction-time
bound `as_of_signed_at` (an RFC 3339 timestamp). The two axes are the *valid time*
(when the phenomenon held in the world) and the *transaction time* (when the
responder learned and signed it). Let $F(c,b)$ denote the set of facts at cell $c$
and band $b$, each fact $f$ carrying an observation slot $\mathrm{tslot}(f)$ and a
signing instant $\mathrm{signed\_at}(f)$. A bi-temporal read returns, per
$(c,b)$ pair, the single fact

$$
f^\star(c,b) = \operatorname*{arg\,max}_{f \in F(c,b)} \; \mathrm{tslot}(f)
\quad\text{subject to}\quad
\mathrm{tslot}(f) \le \tau_{\mathrm{obs}} \;\wedge\; \mathrm{signed\_at}(f) \le \tau_{\mathrm{txn}},
$$

where $\tau_{\mathrm{obs}}$ and $\tau_{\mathrm{txn}}$ default to $+\infty$ when the
corresponding bound is unset. Both predicates hold simultaneously when both bounds
are set; with neither set the read is the current-state read of the
pre-bi-temporal protocol, byte-for-byte. The phrasing "the latest fact per
$(c,b)$ with $\mathrm{tslot} \le \tau_{\mathrm{obs}}$ and $\mathrm{signed\_at} \le
\tau_{\mathrm{txn}}$" is the contract a verifier in year $t+k$ relies on to replay
exactly what emem knew last quarter. This is the same temporal-knowledge-graph
discipline that Zep/Graphiti [6] apply to entity edges, specialised here to the
$(\text{cell} \times \text{band})$ address space.

The valid-time predicate is pushed into the canonical key index: the sled hot cache
decodes $\mathrm{tslot}$ off the key bytes inline, so the observation-time half of
the filter loads no CBOR bodies. The transaction-time half necessarily loads the
fact body to read $\mathrm{signed\_at}$. One consequence is that `find_similar` with
either bound set bypasses the LanceDB IVF\_PQ approximate-nearest-neighbour
fast-path [26], because the Lance schema carries no `signed_at` column; the response
falls back to a brute-force scan and reports `brute_force_fallback` in its `via`
field rather than silently returning a temporally-incorrect ANN result.

Three honesty guards keep the bounds from masking error as emptiness. A request that
pins both a write slot and a conflicting `as_of_tslot` ($\tau_{\mathrm{obs}} <
\mathrm{tslot}$) is rejected with `400 invalid_temporal_bound`; a malformed
`as_of_signed_at` returns `400 invalid_signed_at_format`; and an empty result under a
non-empty bound returns `200` with a `temporal_advice` block explaining what the
bound filtered, never a `404`, because zero is the correct answer to "what did emem
know last quarter" for a place it had not yet observed. This is the no-silent-fallback
discipline applied to time: an empty answer must distinguish a wrong query from an
empty place.

The receipt records the bound, but outside the signature. When at least one bound is
set the receipt body gains an `as_of` block,

```json
{ "as_of": { "valid_time": 1609372800, "transaction_time": "2026-05-15T00:00:00Z" } },
```

carried for the agent's benefit but deliberately excluded from the signed preimage,
which covers only $(\text{request\_id}, \text{served\_at}, \text{primitive},
\text{cells}, \text{fact\_cids})$ and the manifest digest. The preimage still binds
the *result* (`cells` and `fact_cids`), so a verifier in year $t+k$ recomputes the
preimage from the cited CIDs and accepts the signature without trusting whatever
responder reproduced the read, and without the `as_of` block needing to be part of
the cryptographic commitment. Pre-bi-temporal receipts deserialise byte-identically,
the field being absent.

### 5.3 Signed temporal edges and multi-attester contradiction scoring

A pile of facts is not yet a memory; a memory connects things. emem represents
relations as `EdgeFact` objects, signed content-addressed relations $\textit{subj}
\xrightarrow{\textit{pred}} \textit{obj}$ valid over a half-open interval
$[\textit{valid\_from}, \textit{valid\_to})$, following the Zep/Graphiti edge model
[6]. The structurally significant predicates are `disagrees_with`, `supersedes`, and
`relates_to`; the predicate string is otherwise free-form. Edges are not a parallel
key system: each edge leaf is folded into the merkle root of an enclosing
`Attestation`, so the same Ed25519 signature that commits the facts commits their
edges, and they verify offline with no new key material. Edges are read through
`emem_edges_recall` (REST `POST /v1/edges/recall`), which takes the same bi-temporal
`as_of_tslot` bound as band facts and supports both forward traversal
($\textit{subj} \to ?$, "what this fact points at") and reverse traversal
($? \to \textit{obj}$, "what disagrees-with / supersedes / relates-to this fact").
An agent can attach a fact's edges to a recall in one round trip with
`include:["edges"]`. Supersession is bi-temporal rather than destructive: when a newer
edge for the same $(\textit{subj}, \textit{pred}, \textit{obj})$ triple arrives with a
later `valid_from`, it *shadows* the older edge for `as_of:now` reads while a query at
an earlier `as_of` still sees the edge that held then.

The signal that drives edges is multi-attester disagreement. The canonical
$(\text{cell}, \text{band}, \text{tslot})$ index is last-writer-wins, so a second
attester writing the same key would silently overwrite the first; a parallel
`multi_attester_index` sled tree, populated lazily on every `put_attestation`,
instead retains every distinct attester's fact CID at that key. The
`memory_contradictions` primitive walks that tree, hydrates the live facts, filters
to $\textit{n}\ge 2$ distinct attesters (same-attester re-attestation is dropped;
two identical pubkeys with identical values are never a contradiction), and scores
the disagreement on $[0,1]$ by the *kind* of the band's values. Let $\{v_i\}$ be the
disputed values. The score is

$$
S =
\begin{cases}
\operatorname{clamp}_{[0,1]}\!\big(\tfrac{\max_i v_i - \min_i v_i}{R_b}\big),
  & \text{scalar (band range } R_b\text{)},\\[1.4ex]
\operatorname{clamp}_{[0,1]}\!\Big(1 - \dfrac{2}{n(n-1)}\!\!\sum_{i<j}\!\cos(v_i,v_j)\Big),
  & \text{vector (foundation embeddings)},\\[1.8ex]
1 - \dfrac{\max_k |\{i : v_i = k\}|}{n},
  & \text{categorical (mode share)},\\[1.4ex]
1.0, & \text{mixed / unknown shape (flag for review)}.
\end{cases}
$$

The scalar case normalises the spread by the band's documented range $R_b$ from the
registry (falling back to the observed spread when no range is declared), so a
$0.75$ NDVI disagreement over the $[-1,1]$ range scores $\approx 0.375$; the vector
case is one minus the mean off-diagonal cosine, so orthogonal embeddings score
$\approx 1.0$ and identical ones score $0$; the categorical case is one minus the
mode share, so two attesters at class $50$ and one at class $10$ score $1 - 2/3
\approx 0.333$. Bands whose integer values are class identifiers (for example ESA
WorldCover land cover [33] or the Sentinel-2 scene-classification mask) are routed
to the categorical scorer even though their CBOR values are numbers, because
$(\max-\min)/R_b$ would flatten a discrete class disagreement to a misleadingly small
number. Perfect agreement ($S = 0$) is never surfaced, even at `min_severity = 0`, so
an agent scanning for "anything that disagreed" does not get a wall of identical
values. The report is itself a signed fact: its receipt cites every unique cell and
every disputed fact CID under the primitive `emem.memory_contradictions`, and it
carries an `agent_hint` paragraph stating exactly what was scanned (whole corpus
versus a `cell_prefix`, with the scan cap) and what to do next.

The *refinement loop* (gated by `EMEM_REFINEMENT_ENABLED`) closes the cycle
deterministically and non-destructively: it reads the contradiction scorer, writes a
`disagrees_with` edge between the conflicting facts with `valid_from` of now, and
flags the contested fact for re-attestation. The originals are untouched; an auditor
can see that two attesters disagreed, when the disagreement was recorded as an edge,
and whether a later observation resolved it. Nothing is silently reconciled, and
every link in the chain is signed. This is the substrate that the threat model of
§3.4 requires: an unsigned outside-channel memory is vulnerable to query-only
poisoning attacks of the kind MINJA demonstrates at high success rates [41], whereas
here a poisoned write is a signed, attributable, contradiction-scored attestation
rather than an anonymous mutation.

### 5.4 CLS consolidation: episodic to semantic

emem implements the complementary-learning-systems pattern [5] as a daily background
worker (`EMEM_MEMORY_CONSOLIDATION_ENABLED`) that plays the slow neocortical role to
the fast episodic write path. For every attester namespace
`/memories/by_attester/<pubkey>/<sub>/` holding more than $50$ episodic files older
than $7$ days, the worker concatenates their bodies in chronological order, signs the
result as a single `semantic` file at `.consolidated/<unix_ts>.md`, and stamps
`superseded_by: <consolidated_file_cid>` onto each original's metadata. The thresholds
($>50$ files, $>7$ days) gate the worker so that consolidation fires only for
namespaces with accumulated episodic pressure. Consolidation is non-destructive in the
same sense as edge supersession: the originals remain resolvable through
`memory_file_history`, the consolidated file is itself content-addressed, and the
operation is idempotent. A `consolidated` event is emitted on the memory event stream.
The companion retention sweep (`EMEM_MEMORY_TTL_ENABLED`, hourly) enforces per-kind
time-to-live, $90$ days for `resource`, $30$ days for `episodic`, and infinite for
`semantic` and `procedural` (each overridable per kind); an expired file moves from
`memory_files` to `memory_files_expired`, but its content-addressed blob is never
deleted, so even an expired path stays resolvable by CID.

Two evolution loops layer above the deterministic consolidation. The first is the
*refinement loop* of §5.3, which converts contradiction into edges and re-attestation
flags. The second is the optional *sleep-time agent*
(`crates/emem-sleep-agent`, binary `emem-sleep-agentd`, default off), which plays the
generative-replay role: during idle periods it drives a running responder over its
public REST and MCP surface, links no emem internals, and on each pass (i) selects
candidates by ranking multi-attester contradiction severity above a floor (default
$0.3$) and clustering high-churn near-duplicate memory paths by a normalised stem,
(ii) hands each candidate cluster to an operator-configured LLM with a prompt that
must preserve every distinct fact and state disagreements explicitly rather than
silently pick one, and (iii) writes the reconciled text back via `memory_create` to
the cluster's canonical path with a provenance trailer naming each source
`path@file_cid`. Because `memory_create` to an existing path updates the live pointer
and appends to the append-only `memory_file_history`, the merge *shadows* its sources
under `as_of:now` while leaving them replayable, the same bi-temporal supersession the
consolidation worker relies on. The agent is held to the project's no-fabrication rule:
a per-pass USD budget is checked *before* each LLM call, a `--dry-run` mode selects and
plans but performs no call and no write, and a live pass with no transport configured
degrades to a dry run with a logged note rather than inventing a rewrite. An empty
corpus is reported honestly as an empty candidate set.

### 5.5 The CoALA-typed, capability-bound scratchpad

The writable surface implements the six file-operation verbs of the Anthropic
memory-tool specification (`context-management-2025-06-27`) [22]: `memory_view`,
`memory_create`, `memory_str_replace`, `memory_insert`, `memory_delete`, and
`memory_rename`, exposed both as MCP tools [23] and as REST. Each write persists the
path-to-CID pointer in `memory_files`, the content-addressed bytes in
`memory_file_blobs` (deduplicated across paths), an append-only audit trail in
`memory_file_history`, and signed metadata in `memory_file_meta`, and returns a
`file_cid` with a receipt. `memory_delete` drops only the path index; the blob is
retained, so history replay can reconstruct any past state. As in §5.1 each file
carries a CoALA `kind` [7] and is indexed in `memory_files_by_kind` so
`memory_list_by_kind` returns a typed slice sorted by `signed_at` descending.

Writes can be *capability-bound*. Paths under
`/memories/by_attester/<pubkey_b32_short>/...`, where the short key is the first eight
characters of the lowercase base32 pubkey, are write-restricted to the holder of the
corresponding Ed25519 private key. The caller attaches an `attester` block with the
pubkey and a signature over the domain-separated preimage

$$
\textsf{blake3}\big(\texttt{"emem.memory\_write|"} \,\|\, \textit{verb} \,\|\,
\texttt{"|"} \,\|\, \textit{path} \,\|\, \texttt{"|"} \,\|\,
\textsf{blake3}(\textit{body})\big),
$$

where $\textit{verb} \in \{\texttt{create}, \texttt{str\_replace}, \texttt{insert},
\texttt{delete}, \texttt{rename}\}$. A wrong key yields `401
memory_attestation_invalid`; a write to another holder's namespace yields `403
memory_namespace_violation`; bare `/memories/...` paths stay anyone-writable. For an
attested write the receipt's `cells[]` becomes `["pubkey:<b32>", path]` rather than
just `[path]`, which is what makes capability-bound writes first-class to the
multi-attester index and the contradiction detector: an attested memory file is an
attributable, signed observation that can disagree with another attester's file at the
same logical subject. A complementary BGE-based semantic search [25] over memory-file
text (`memory_search`, against a 768-D LanceDB partition with a brute-force fallback)
provides associative retrieval over the scratchpad by meaning rather than by path.

Two composition handles let an agent quote memory by reference. A `memory_token`
`memt:<cell>:<fact_cid>` is a stable handle to one signed fact that resolves to the
same bytes on any responder holding them. A `memory_bundle` composes a signed
envelope over $N$ facts and mints a token `memb:<bundle_cid>` with

$$
\textit{bundle\_cid} = \textsf{base32}_{16}\!\Big(\textsf{blake3}\big(
\texttt{"emem.memory\_bundle.v1|"} \,\|\, \textit{purpose}? \,\|\, \texttt{"\textbackslash n"}
\,\|\, {\textstyle\bigparallel_i}\, c_i \|\, b_i \|\, t_i \|\, \textit{fact\_cid}_i \big)\Big),
$$

resolving via `GET /v1/memory_bundle/<token>` on the originating responder or any peer
that holds the bytes, by the same content-addressed determinism as a fact CID. Both
tokens are designed for the agent to paste verbatim into a downstream prompt, a log
line, or a user-facing citation: the handle steers the agent's later reasoning without
re-paraphrasing the underlying CBOR.

### 5.6 The eMEM correspondence

To position emem against a current cognitively-grounded agent-memory architecture, the
ten mechanisms catalogued by eMEM [5] were each audited against emem's implementation
by an eleven-agent review, partitioned by subsystem. The tally is HAVE 4 / PARTIAL 5 /
GAP 1: four mechanisms have a direct, shipped analogue, five are partially realised,
and one is a genuine gap. Table 3 records the mapping. The discipline of the audit was
the project's no-stub rule: a mechanism counts as HAVE only when a real, exercised
emem operation implements it, PARTIAL when the substrate exists but is narrower or
opt-in, and GAP when emem deliberately does not implement a facsimile.

**Table 3. eMEM mechanisms [5] mapped onto emem.**

| eMEM mechanism        | Status  | emem realisation |
|-----------------------|---------|------------------|
| `hnsw_knn`            | PARTIAL | k-NN over foundation embeddings via LanceDB IVF\_PQ [26] and a sign-bit Hamming fast mode [27]; the index is IVF\_PQ, not HNSW, and is bypassed under a bi-temporal bound (§5.2). |
| `cls_consolidation`   | HAVE    | Daily episodic→semantic consolidation worker with `superseded_by` stamps (§5.4). |
| `context_retrieval`   | HAVE    | Address-keyed `recall`/`state` plus BGE semantic search over memory files [25] (§5.5). |
| `temporal_edges`      | HAVE    | Signed `EdgeFact` with `disagrees_with`/`supersedes`/`relates_to`, valid-time-bounded, `emem_edges_recall` (§5.3). |
| `temporal_decay`      | HAVE    | Physics-informed $Q(\Delta t)$ freshness kernel, ported to the recall path (below). |
| `reconsolidation`     | PARTIAL | Deterministic refinement loop writes `disagrees_with` edges and re-attestation flags; the sleep agent's LLM merge is the optional reconsolidation step, off by default (§5.4). |
| `dbscan_dedup`        | PARTIAL | Sleep-agent near-duplicate clustering by normalised path stem; conservative stem-equality clustering, not density-based DBSCAN over embeddings (§5.4). |
| `sleep_replay`        | PARTIAL | Idle-time sleep-time agent reconciles contradicted and high-churn memory; generative LLM replay, opt-in (§5.4). |
| `interoception`       | PARTIAL | Self-declared `Cost` block (latency, freshness, `was_cached`) on every receipt and the `corpus_state_stats` observability primitive; not a learned internal-state signal. |
| `source_monitoring`   | GAP     | No mechanism attributes a remembered fact to the perceptual versus inferred channel beyond the existing signer-pubkey and `Source` provenance; flagged as future work. |

The single mechanism shipped as a direct port during this audit was temporal decay.
emem already carried physics-informed decay kernels but applied them only in the
`temporal_route` band-ranker. The audit surfaced that the recall path should expose the
same staleness signal so an agent learns how fresh each reading is in the call that
returns it. The kernel assigns a freshness $Q(\Delta t) \in [0,1]$ to a fact of age
$\Delta t$ according to the band's tempo: static bands never decay ($Q \equiv 1$);
slow (annual) and ultra-fast (hourly) bands decay linearly to a horizon as a clamped
AR(1)/advection step; monthly and MODIS composite cadences use the Gaussian
fundamental solution of the heat equation,

$$
Q(\Delta t) = \exp\!\big(-(\Delta t / \sigma)^2\big), \qquad
\sigma = \text{one slot duration},
$$

so roughly $38\%$ of the score survives one slot's lag; and fast (daily) bands use a
half-cosine seasonal wave $Q(\Delta t) = \max\!\big(0,\, \tfrac12 + \tfrac12\cos(2\pi
\Delta t / T)\big)$ with $T$ near the Sentinel-2 revisit [28], cut to zero past one
period. Passing `include:["freshness"]` to a recall attaches a per-fact block carrying
$Q$, the age in seconds, the tempo label, the decay model name, the formula, and a
`stale` flag ($Q < 0.5$). The freshness block is computed *after* the receipt is signed
and never enters the preimage, so a recall with the flag is byte-identical to one
without it: the receipt commits to the fact, the freshness score is advisory, exactly
the contract emem applies to `band_metadata` and decoded-value annotations. This is the
recall-path port of the eMEM temporal-decay idea [5].

The boundary cases in Table 3 are where emem's design philosophy is most legible.
Mechanisms are marked PARTIAL or GAP rather than overstated when the honest analogue is
narrower, opt-in, or absent: emem's k-NN is IVF\_PQ rather than HNSW, its deduplication
is stem clustering rather than density-based clustering over embeddings, its replay is
an opt-in LLM loop, and source monitoring has no implementation beyond signer and
`Source` provenance. The position the larger argument takes (§10) is that emem is a
*third tier* of agent memory, shared, external, planet-keyed, and signed, complementary
to the in-context tier and the in-process long-term tier rather than a replacement for
either, and distinct from per-session chat-memory systems [9, 10] and from temporal
knowledge graphs over conversational entities [6]. What emem holds that those layers
structurally cannot is that every memory operation in this section, every read, edge,
contradiction report, consolidation, and capability-bound write, produces an Ed25519
receipt over a deterministic preimage that any party can verify offline against the
issuer's pubkey, with the bi-temporal axis making the entire history replayable as of
any past instant.

## 6 Read Primitives and Retrieval

The read surface of emem is the set of primitives an agent uses to pull
attested facts back out of the store. Each primitive returns a signed
receipt (Section 3.4) over the exact facts that contributed to its answer,
so retrieval and verification are not separable steps: every read carries
its own citations. The surface decomposes along the four retrieval modes
of the agent-memory operations vocabulary shared by mem0 [10], Letta,
and LangGraph: retrieve-by-address (`recall`), retrieve-by-similarity
(`find_similar`, `memory_search`), retrieve-over-a-region (`query_region`,
`recall_polygon`, `field_boundaries`), and retrieve-over-time
(`trajectory`, `diff`, `compare`, `compare_bands`). A recurring discipline
runs through all of them, inherited from the signed-Absence model
(Section 3.5): an empty result is labelled rather than zeroed, so an agent
can distinguish a wrong query from an empty place. Figure
(docs/diagrams/09-address-algebra.svg) sketches how the same cell64 key
threads through every read mode.

### 6.1 Retrieve by address: `recall`

`recall(cell, bands?, tslot?)` is the index lookup. Its behaviour is keyed
on which of the optional filters are present. With both `bands` and `tslot`
supplied it is a batched canonical point lookup over the keys
$(\text{cell}, \text{band}_i, \text{tslot})$; with `tslot` alone it is a
prefix scan returning every band at that slot; with neither it returns
every fact attested at the cell. The canonical key is the triple
$(\text{cell64}, \text{band}, \text{tslot})$, and the same key is the
cross-replica join key, because the per-replica `fact_cid` differs across
responders even for byte-identical upstream pixels (the `signed_at`
timestamp enters the content address).

The honesty contract is carried by the `bands_already_attested_at_cell`
field, which lists the union of every band signed at this cell64 across all
slots. It is computed by one extra point-in-tree scan per recall (tens of
microseconds under sled) and answers the agent's standing question of what
else is readable here without a second probing call. When a filtered recall
returns zero hits, this field lets the agent tell "wrong band name" (the
cell has data, just not for the requested band) from "this place is
genuinely empty" (no facts at all). The field was renamed from
`bands_available` in a 2026 audit, with no backward-compatibility alias,
because language-model callers were reading the older name as a statement
of globally wired connectors rather than locally attested bands; a wire
test pins the rename and rejects the legacy spelling on input.

Two write-side features fold into the read path without altering the signed
surface. Multi-tenant scope is an opt-in four-tuple
$\{\text{user\_id}, \text{agent\_id}, \text{run\_id}, \text{org\_id}\}$:
when a recall pins a scope, the result set is restricted via the
`scope_index` tree to facts written under the same tuple, the receipt
preimage binds $\textsc{blake3}(\textsc{cbor}(\text{scope}))$ so an offline
verifier rebinds the response to this caller, and matching is on the whole
tuple rather than on a subset (a recall under $\{\text{user\_id}{:}u\}$
misses a write under $\{\text{user\_id}{:}u, \text{org\_id}{:}o\}$, since
each axis changes the scope digest). Contradiction markers from the
refinement loop surface as an advisory `contested` block on any returned
fact a signed `disagrees_with` edge has down-weighted; this block is
responder-derived metadata computed after the fact set is fixed, so a
recall of the same facts before and after they are contested signs a
byte-identical receipt. The trust anchor is the cited `by_edge` CID of the
signed disagreement, not the advisory note.

#### 6.1.1 Auto-materialize on miss

When the requested band is entirely absent at a cell and a connector is
registered for it, the recall path materializes the fact rather than
returning empty. The dispatch chain is

$$
\text{miss} \;\to\; \text{fn\_key lookup} \;\to\; \text{connector dispatch}
\;\to\; \text{upstream Range read} \;\to\; \text{compute value}
\;\to\; \text{sign as responder} \;\to\; \text{persist} \;\to\; \text{return}.
$$

The function-registry `fn_key` (for example
`turboquant_geotessera_bin128_v1@1`) names the exact derivation; the
connector issues an HTTP byte-range read against the upstream
(`vsicurl` Cloud-Optimized GeoTIFF, STAC asset, or JSON API); the responder
signs the resulting `Fact::Primary` as itself, with
`derivation.fn_key` declaring how the value was produced, and persists it
through `put_attestation`. Trust delegation here is deliberately flat: the
same key that signs receipts also signs the value. The path is gated by
`EMEM_AUTO_MATERIALIZE` (default on), a 30 s materializer timeout, a 180 s
gateway timeout, and a 16 MiB body cap; a miss with no registered connector
returns a typed `MaterializeMiss` Absence, never a silent empty. Twenty
live materializer registrations cover the wired band set, so any cell on
Earth can answer without pre-seeding. This is the protocol mechanism behind
retrieval-augmented generation [12] specialised to Earth observation: the
store is lazily populated by the queries agents actually make.

#### 6.1.2 Advisory freshness

Passing `include:["freshness"]` attaches a per-fact `freshness` block that
ages each reading through a physics-informed temporal-decay kernel
$Q(\Delta t)$, the same quality kernel `/v1/temporal_route` uses to rank
bands. The kernel is selected by the band's declared `Tempo` class:

$$
Q(\Delta t) =
\begin{cases}
1 & \text{Static (no decay)} \\[2pt]
\max\!\left(0,\; 1 - \dfrac{\Delta t}{T_{\text{slot}}}\right) & \text{Slow (linear AR-1)} \\[6pt]
\exp\!\left(-\left(\dfrac{\Delta t}{\sigma}\right)^{2}\right) & \text{Medium / Composite (heat-equation Gaussian)} \\[6pt]
\max\!\left(0,\; 0.5 + 0.5\cos\dfrac{2\pi \Delta t}{T}\right) & \text{Fast (wave / seasonal)} \\[6pt]
\max\!\left(0,\; 1 - \dfrac{\Delta t}{6\,T_{\text{slot}}}\right) & \text{UltraFast (advection)}
\end{cases}
$$

with $\sigma$ set to one slot duration and $T \approx$ the Sentinel-2
revisit interval for the fast class. The block reports $Q$, the age in
seconds, the decay model name, the closed-form formula, and a
`stale: Q < 0.5` flag. Crucially the freshness block is computed after the
receipt is signed and never enters the preimage, so a recall with the flag
and a recall without it produce a byte-identical receipt over the same
facts: the receipt commits to the fact, not to a staleness score that
changes with wall-clock time. This is the recall-path port of the
temporal-decay idea from the eMEM hippocampal/neocortical memory system
[5]; emem already carried the decay kernels but previously applied them
only in band ranking.

Bi-temporal reads (`as_of_tslot` for valid time, `as_of_signed_at` for
transaction time, RFC 3339) restrict the result to "what did this place
look like, as emem knew it, as of moment $Y$", in the style of Zep/Graphiti
edge queries [6]. When the bound filters every otherwise-recallable fact, a
`temporal_advice` block reports the unbounded fact count and a hint, so the
empty response reads as the honest answer to a point-in-time question
rather than a 404.

### 6.2 Retrieve by similarity: `find_similar`

`find_similar(key, k?, band?, filter?, mode)` is the vector-as-address
primitive. The `key` is either a `cell64` (look up that cell's embedding
under `band`) or `inline:[x,y,...]` for a literal query vector, capped at
16384 dimensions. The default corpus is the 128-D `geotessera` annual
embedding [1], which emem materializes by default. Three scoring modes
trade precision against scan cost:

- **`Cosine`** computes fp32 cosine over the full vector and is the default
  for backward compatibility.
- **`Hamming`** scores XOR-popcount over the 16-byte binary sibling band
  (`geotessera.bin128`), mapping the distance to a cosine-direction score
  $s = 1 - 2\,\text{dist}/128 \in [-1, +1]$.
- **`HammingThenRerank`** pulls a wide Hamming candidate set, then re-orders
  the shortlist by full-vector cosine, recovering cosine-only precision at
  a fraction of the scan cost.

The query cell is filtered out of its own result, since the top-1
self-match (cosine $1.0$) is never the neighbour an agent wants. The result
is deduplicated by cell64, keeping the highest-scoring vintage per place,
because the index holds one entry per $(\text{cell}, \text{band},
\text{tslot})$ triple and a multi-vintage band would otherwise fill the
top-$k$ with near-duplicates of one location. The response surfaces both
`requested_k` and `returned_k`: when the deduplicated corpus has fewer
distinct cells than asked for, `returned_k < requested_k` is the honest
truncation signal and the responder never pads with duplicates. Every
response also carries a band-family-typed `interpretation` hint stating
that the Tessera embedding measures surface texture (Sentinel-1 SAR plus
Sentinel-2 optical aggregated annually), not climate or latitude proximity,
to head off the common failure of reading biome similarity into a
texture-aliased score (volcanic highlands at 9 N and 64 N can rank close).
The receipt cites one contributing fact per kept neighbour.

An optional `filter: Claim` is evaluated per candidate cell against the
same predicate engine as `/v1/verify`, with per-cell memoization so a
verdict computes once and reuses across that cell's vintages. Cells with no
fact for the filter band are dropped as undecidable, not scored as false:
"places like $X$ where NDVI $> 0.5$" must not silently include places with
no NDVI history. The `as_of_*` bi-temporal bounds and multi-tenant `scope`
apply the same drop-on-undecidable contract before scoring.

#### 6.2.1 Lance IVF_PQ acceleration

For corpora past roughly one million vectors the brute-force scan
$O(N)$ becomes the dominant cost, so emem layers a separate Lance dataset
[26] alongside the authoritative sled store. Sled remains the source of
truth; Lance is a derived ANN index, hydrated on boot and appended
incrementally by `sign_and_persist`. Because Lance's `FixedSizeList`
vector column requires a uniform width and emem carries several
dimensionalities (`geotessera` = 128, `clay_v1` = 1024,
`prithvi_eo2` = 1024 [2,3], `galileo` = 768 [4]), the index is partitioned
one dataset per dimension under `$EMEM_DATA/lance`. Each partition builds an
IVF_PQ index over the cosine metric with parameters sized off the
partition's row count $N$:

$$
\text{num\_partitions} = \operatorname{clamp}\!\left(\lfloor\sqrt{N}\rceil, 16, 256\right), \quad
\text{num\_bits} = 8, \quad
\text{num\_sub\_vectors} = \max\!\Big(1, \tfrac{\dim}{16}\Big),
$$

with the sub-vector count snapped to the largest divisor of $\dim$ at or
below $\dim/16$ so PQ's divisibility constraint holds, and 50 training
iterations. Lance returns a cosine distance in $[0,2]$, mapped back to the
historical score by $\text{score} = 1 - \text{distance}$. The ANN path is
strictly an accelerator: it is tried only for unfiltered, unscoped,
bi-temporally unbounded `Cosine` and `HammingThenRerank` queries with a
non-empty query vector, and any failure, empty result, or kill-switch
(`EMEM_DISABLE_LANCE=1`) falls through to the brute-force scan returning
identical results. The candidate set is oversampled by a factor of four so
per-cell dedup does not shrink the result below $k$ distinct cells. Any
query that the index cannot honour truthfully (a `filter:` claim, an
`as_of_*` bound, or a `scope` the IVF_PQ and binary indexes have no column
for) bypasses every fast path and runs the brute-force scan, trading speed
for correctness.

#### 6.2.2 TurboQuant binary rotation

The Hamming modes operate over the `geotessera.bin128` sibling band,
encoded in `binary_embedding.rs` following the TurboQuant sign-flip rotation
construction [27]. Sign-bit packing alone is a poor binary quantizer when the upstream
embedding concentrates variance in a few axes, so a fixed random orthonormal
rotation $R \in \mathbb{R}^{128 \times 128}$ is applied before sign
extraction. The packed bit at dimension $i$ is

$$
b_i = \mathbb{1}\!\left[\,(R\mathbf{v})_i \geq 0\,\right],
\qquad (R\mathbf{v})_i = \sum_{j} R_{ij}\, v_j,
$$

stored MSB-first into 16 bytes. The rotation spreads variance across all
128 dimensions so a single bit per dimension carries information rather than
collapsing to "is this the one big-magnitude axis." Hamming distance is
$u128$ XOR plus `count_ones`, roughly $10^9$ scored pairs per second per
x86 core. The Hamming-to-cosine bridge $s = 1 - 2\,\text{dist}/128$ lets
binary and cosine neighbours order on a single scale.

Determinism is the load-bearing property. $R$ is built once from a fixed
seed (`ROT_SEED_TEXT = "emem.binary_embedding.turboquant.v1"`): BLAKE3 [17]
keyed by the seed drives a CSPRNG that fills $128^2$ Gaussian samples
(Box-Muller), which classical Gram-Schmidt orthonormalizes in fp64 (a
$\|\cdot\| > 10^{-9}$ pivot guard fails loud rather than emitting a
degenerate basis). Every responder produces the same matrix bit-for-bit, so
a binary fact materialized on responder $A$ is content-comparable against
one on responder $B$ without coordination, and `rotation_cid()` (the BLAKE3
hash of the matrix bytes) lets a verifier rebuild $R$ from the seed,
re-pack the source vector, and byte-compare. The seed is recorded in the
band's `fn_key` so the receipt is reproducible. When the binary sibling is
absent at a cell but the cosine vector is present (under any `geotessera`
vintage, since the rotation is vintage-agnostic), the path inline-derives
`bin128` on the fly through the same rotation rather than forcing a
two-call materialize-then-recall dance; the result is byte-identical to a
cached `bin128`.

#### 6.2.3 Adaptive oversampling

`HammingThenRerank` pulls a triage window of $\text{factor} \cdot k$
candidates, where the oversampling factor adapts to the corpus's observed
binary-versus-cosine ranking agreement. After each call the path measures
the recall@$k$ overlap $|\,\text{hamming\_top}_k \cap
\text{cosine\_top}_k\,| / k$ and folds it into an exponential moving average
(decay $\alpha = 0.05$) held in a lock-free `AtomicU64`. Once roughly 50
calls warm the gate, the factor is set to $\lceil 1/\widehat{\text{recall}}
\rceil$ clamped to $[4, 16]$; below the warm-up threshold the read returns
NaN and the caller falls back to the historical $4\times$ multiplier, so the
first 50 calls match the pre-adaptive behaviour exactly. The triage window
is itself oversampled before per-cell dedup so the survivor pool stays at
least $k$ distinct cells before the cosine rerank.

### 6.3 Region analytics

Region reads aggregate over many cells while preserving per-cell
citations. `query_region(geometry, bands?, agg?)` accepts a single
`cell64`, an explicit `cells:c1,c2,...` list, or a WGS-84
`bbox:lon_min,lat_min,lon_max,lat_max` sampled at the cell64 grid pitch
(roughly 10 m at the equator). Bbox synthesis is bounded by
$\text{MAX\_BBOX\_CELLS} = 4096$ and $\text{MAX\_REGION\_FACTS} = 65536$;
beyond the caps the responder stops scanning and aggregates over what it
has, and `receipt.fact_cids` reflects exactly the facts that contributed.
Aggregations include mean, median, p90, and vector centroid. The same
bi-temporal and scope filters apply per cell, and a `temporal_advice` block
distinguishes an empty region from a bound that filtered everything out.

`recall_polygon` fans out across up to 1024 sample cells inside a polygon
bounding box and returns per-band mean, median, min, max, and standard
deviation, plus per-cell scene thumbnails, a scene overlay URL, and GeoJSON.
It emits one independently signed receipt per cell under
`by_cell.<cell>.receipt`; the flattened `merged_facts[]` is convenience
only and is not covered by an aggregate signature. `field_boundaries`
returns per-field agricultural polygons from Fields of The World [35], a
global product of approximately 3.17 billion field polygons across 241
countries at 10 m resolution. The connector reads the upstream 2.14 TB
PMTiles archive over anonymous HTTP range requests, decoding MVT tiles and
reprojecting from Web-Mercator to WGS-84 in process; an auto-zoom shrinks
the request when a bounding box exceeds the 16-tile cap. It accepts either
a `place` name (resolved through the locate cascade: GeoNames cities-5000
[40], Overture divisions [39], Photon, Nominatim) or an explicit
`polygon_bbox`, and a `recall_polygon` call can attach the field block
inline via `include:["ftw_fields"]`.

The time-axis comparators close the set. `diff(cell, band, t0, t1)` emits a
`DerivativeFact` with `op = "delta"` and value $b - a$ for scalar bands or a
per-dimension delta vector for vector bands. `trajectory(cell, band,
[start,end])` walks the canonical prefix scan, filters to the band, and
returns the ordered series with missing slots surfaced as explicit gaps.
`compare(a, b, family?)` places two cells side by side over their shared
bands, using cosine for vector bands and $b - a$ for scalar bands, with a
summary cosine over the concatenated vector bands. `compare_bands(cell, a,
b, tslot_a?, tslot_b?)` compares two bands at one cell in a single signed
envelope citing both source CIDs; omitted tslots resolve to the latest
attested slot per band, and the choice is surfaced in a `tslot_resolution`
block (`auto_picked_latest` versus `caller_supplied`) so a fast- or
medium-tempo band is never silently read as empty at slot 0. A band with no
history surfaces in `bands_with_no_history[]` against an empty-cite receipt:
labelled empty, not zeroed.

### 6.4 Semantic search over the memory corpus: `memory_search`

`memory_search` is the similarity read over the agent's writable
`/memories/*` file layer (Section 5), as opposed to the geospatial fact
store. It embeds the query with BAAI bge-base-en-v1.5 [25], a 768-D
L2-normalized text model, mean-pooling over chunks of at most 504 tokens
for long files, and runs k-NN against a dedicated Lance IVF_PQ partition
(`memory_text_index_d768.lance`) that a polling indexer hydrates on boot and
every 60 s. Each hit returns the file path, content CID, kind, signing
timestamp, cosine similarity, and a 200-character snippet centred on the
matched chunk. The `via` field is `"lance_ann"` on the fast path and
`"brute_force_fallback"` when Lance is disabled or empty (the primitive
re-embeds every file in process); a `model_loaded: false` reply with empty
hits is the honest "model not installed" answer rather than a silent zero.
Unlike the geospatial primitives, `memory_search` carries no `scope` field,
because the memory-file corpus is addressed by `path`, `kind`, and
`attester_pubkey_b32` rather than by the four-tuple scope that indexes
facts; per-tenant isolation is expressed through a `path_prefix` or
attester filter, and Vault-kind entries (held in a separate AEAD tree) are
never indexed.

### 6.5 Mapping to the operations vocabulary

Table 6.1 places each read primitive against the canonical agent-memory
operations vocabulary used by mem0 [10], Letta, and LangGraph and discussed
in the cognitive-architecture framing of CoALA [7]. The recurring property
across the table is that retrieval and provenance are the same operation:
every row returns a signed receipt over the facts it cites, so an agent that
trusts emem is trusting a verifiable artefact rather than an outside-channel
string of the kind memory-injection attacks exploit [41].

| Operation | Primitive | Retrieval mode |
|---|---|---|
| retrieve by address | `recall` | by-address (cell64 key) |
| retrieve by similarity (geo) | `find_similar` | by-similarity (128-D Tessera k-NN) |
| retrieve by similarity (text) | `memory_search` | by-similarity (768-D BGE k-NN) |
| retrieve over a region | `query_region`, `recall_polygon`, `field_boundaries` | by-region |
| retrieve over time | `trajectory` | by-time |
| compare states | `diff`, `compare`, `compare_bands` | by-time / by-address |
| point-in-time read | `recall` with `as_of_*` | by-time (bi-temporal) |
| freshness-ranked read | `recall` with `include:["freshness"]` | by-time (decay-weighted) |

*Table 6.1: emem read primitives against the agent-memory operations vocabulary.*

## 7 Implementation

This section describes the reference responder: the open-source artifact that realises the protocol of the preceding sections as running code. The protocol itself is defined by byte-level rules (content addressing, the receipt preimage, the attestation envelope) and is therefore implementable in any language. The reference responder is one concrete implementation, a single statically-linked Rust binary plus two optional Python sidecars, that the hosted node at `emem.dev` and every self-hosted node run. The design goal of the implementation is that a second, independently written responder reaching the same upstream sources under the same registry CIDs reproduces every fact byte for byte, so the federation property of Sections 9.3 and 10 reduces to an interoperability property rather than a trust assumption.

### 7.1 Workspace structure

The responder is a Cargo workspace of 16 crates (version 0.1.0, minimum supported Rust version 1.91), partitioned so that the protocol invariants live in small, dependency-light crates and the network surface lives at the edge. The partition mirrors the layering of the protocol: codec and content-addressing primitives at the bottom, the trust and storage layer in the middle, the agent-facing wire surfaces at the top. Table 1 lists the crates and their roles.

| Crate | Role |
|-------|------|
| `emem-codec` | The address algebra: `cell64`, `cid64`, `tslot` text form, `vec64`, the Hilbert-ordered base-1024 alphabet, and the WGS84 geometry helpers. No I/O, no async. |
| `emem-core` | The eight content-addressed registries (bands, algorithms, functions, sources, topics, schema, plus the taxonomy and privacy manifests) and the manifest-CID computation over them. |
| `emem-fact` | The three `Fact` variants (`PrimaryFact`, `DerivativeFact`, `NegativeFact`), `Receipt`, and `Attestation` as canonical-CBOR types, and the BLAKE3 + base32-nopad CID and signing primitives. |
| `emem-claim` | The `Claim` predicate type (an `Op` enum over a band value) used as a typed filter in `find_similar` and `verify`; carries no signature. |
| `emem-attest` | Merkle math: `merkle_root`, `merkle_root_and_paths`, and `verify_merkle_path`, with RFC 6962-style leaf/node domain separation [14]. |
| `emem-cache` | The sled hot-cache wrapper (`SledHotCache`) holding the canonical index and fact trees. |
| `emem-storage` | The keystone. `MaterializingStorage` glues the cache to the fetch dispatcher and the append-only log, exposes the `Storage` trait that primitives program against, owns the `Server` value (responder identity, manifest CIDs), the append-only Merkle log, and the attester reputation registry. |
| `emem-cubes` | A Rust handle over the 1792-D AgriSynth `.npz` bootstrap cubes; the on-disk parser is Python-authoritative. |
| `emem-fetch` | The fetch plane: 16 data connectors and 13 utility modules, including the universal COG sampler `cog.rs` and the STAC search client. |
| `emem-primitives` | The read primitives (`recall`, `find_similar`, `trajectory`, `compare`, `compare_bands`, `diff`, `verify`, `query_region`) plus binary embedding, the refinement loop, CBOR ops, and the Lance-backed similarity and memory-text indices. |
| `emem-intent` | A rule-based dispatcher mapping a free-text question to one of seven `Intent` variants and a `Plan`. |
| `emem-mcp` | The MCP tool-descriptor registry: 81 `ToolDescriptor` constants, each carrying a `when_to_use` string and the four MCP behavioural annotations. |
| `emem-api-rest` | The axum router, shared request handlers, inline materializers, the GPU-sidecar UDS client, and the in-process physics solvers. |
| `emem-cli` | The binaries, including `emem-server`. |
| `emem-membench` | A MemoryAgentBench-style scorecard harness [11] that drives a running responder over its own write/read API and grades four memory axes. |
| `emem-sleep-agent` | An opt-in, default-off offline worker that ranks memory paths by contradiction severity and write-churn and proposes non-destructive merges via an operator-configured LLM. |

*Table 1: The 16 workspace crates of the reference responder.*

`emem-storage` is the keystone in the sense that it is the only crate that may turn a fetched value into a signed, persisted fact: `MaterializingStorage` composes the cache trait object, the `emem-fetch` `Dispatcher`, and the `AttestationLog` behind the single `Storage` trait that every primitive programs against. This containment means the content-addressing and verify-on-write invariants (Sections 3.3–3.4) are enforced at one chokepoint rather than scattered across handlers.

### 7.2 The single server binary and its two wire surfaces

The `emem-server` binary listens on one port (default `0.0.0.0:5051`) and serves two wire surfaces from one set of handlers. The REST surface exposes 93 documented paths under `/v1/*` in the OpenAPI 3.1 document at `/openapi.json`. The MCP surface is a single JSON-RPC 2.0 endpoint at `POST /mcp` over Streamable HTTP [23], advertising 81 tools (10 core, 71 extended). The two surfaces are not parallel implementations: an MCP `tools/call` and the corresponding `POST /v1/*` route dispatch into the same async handler. The MCP tool set is a strict read-only subset of REST. The three write paths (`attest`, `attest_cbor`, `backfill`) are reachable over REST only, which is the implementation-level statement of the protocol's "reads open, writes signed" rule: an anonymous client may read or verify anything, but a value enters the corpus only through a signature-checked attestation.

At boot the router constructs `AppState = Arc<Server>`. The `Server` owns the `Storage` trait object, a `ResponderIdentity` (an `ed25519_dalek` signing key per RFC 8032 [15]), and the `ManifestCids` for the active registries. Every primitive call receives `&Server` and returns a signed `Receipt`; there is no code path that returns an unsigned value to a client. Tool discovery is tiered to keep the default `tools/list` payload small: a no-argument `tools/list` returns all 81 descriptors so that every MCP client sees the full surface, while `{"tier":"core"}` narrows to the 10 essentials. The two long-running tools, `emem_eudr_dds` and `emem_hunt`, are declared as optional async tasks under the MCP 2025-11-25 task model [23].

```
client (MCP or REST)
        │
   axum router  ──►  shared handler  ──►  primitives  ──►  Storage trait
   /v1/*  /mcp                                                  │
                                              ┌─────────────────┼──────────────┐
                                          sled hot            fetch          append-only
                                          cache (index,     Dispatcher       Merkle log
                                          facts, proofs,    (cog, STAC,      (1 GiB segments,
                                          attesters)        connectors)       fsync per append)
```

Storage is two tiers on local disk under `<EMEM_DATA>` (default `./var/emem`). The hot tier is a sled database with four trees: `canonical_index` (`cell\0band\0tslot_be8` → `fact_cid`), `facts` (`fact_cid` → canonical CBOR), `fact_proofs` (per-CID Merkle inclusion path), and `attesters` (per-pubkey reputation). The durable tier is an append-only Merkle log under `log/merkle.log.{0,1,…}`, written as `[u32_LE len][CBOR][32-byte BLAKE3]` records with a 1 GiB segment cap and an `fsync_all()` on every append. The whole `<EMEM_DATA>` directory is portable: because the responder is content-addressed end to end and its identity is the file-pinned ed25519 secret, replaying it on a fresh host produces byte-identical receipts as long as the schema CID still resolves.

### 7.3 The GPU inference sidecar

Four foundation encoders are out of reach of a CPU-only Rust binary at interactive latency, so GPU inference is isolated in a Python sidecar reached over a Unix domain socket. The sidecar is a FastAPI application served by `uvicorn server:app --uds <sock>`; the Rust side dials it through a hand-rolled HTTP/1.1 client in `gpu_sidecar.rs`, with the socket path read from `EMEM_SIDECAR_SOCK` and a per-request timeout from `EMEM_SIDECAR_TIMEOUT_MS` (default 5000 ms). A UDS rather than a TCP loopback keeps the inference plane off the network entirely and binds its lifetime to the host.

Four encoders are co-resident on a single GPU under a hard VRAM budget set by `EMEM_SIDECAR_VRAM_BUDGET_GB` (the deployed node sets 20 GB). The budget is enforced once at registry initialisation through a single call

$$\texttt{set\_per\_process\_memory\_fraction}\!\left(\min\!\left(1, \frac{B_\text{total}}{G_\text{device}}\right)\right),$$

where $B_\text{total}$ is the configured budget and $G_\text{device}$ the physical device memory, so any future allocation that would exceed the budget raises a CUDA out-of-memory error rather than thrashing the device. The per-model constants ($\approx$ 2.5 GB Clay, 3.0 GB Prithvi-EO-2.0, 3.0 GB Galileo, 0.1 GB JEPA-v2 dynamics) are advisory accounting that must sum below $B_\text{total}$ with a positive reserve. The four encoders are Clay v1.5 [2] (1024-D CLS over a 10-band Sentinel-2 L2A chip), Prithvi-EO-2.0-300M-TL [3] (1024-D CLS over a 6-band HLS chip), Galileo [4] (`base` variant in production, with only the Sentinel-2 modality wired today and the remaining modalities zero-masked), and the JEPA-v2 dynamics head, which is untrained and short-circuits to the last-attested-vintage identity baseline with `via: short_circuit_untrained` on the receipt. The fifth foundation field, Tessera [1], is not in the sidecar at all: its 128-D annual embedding ships as Cloud-Optimized GeoTIFF tiles and is sampled in pure Rust through `cog.rs`, so it materialises on a cold miss with no GPU dependency.

Crash isolation is a first-class property. A sidecar fault does not cascade into the Rust process. On `SidecarError::Unavailable` (a 503 from the sidecar, raised when CUDA OOMs or the model fails to load) the responder degrades to the scalar bands it can serve in-process and signs the GPU-anchored algorithms as a typed `gpu_unavailable` Absence (Section 3.5). It never silently downgrades a GPU embedding to a CPU approximation: the no-silent-fallback rule means a missing capability is a signed, citable absence, not a zeroed vector. The `model.via` field of every embedding receipt records provenance as `python_sidecar`, `in_process_cpu`, or `short_circuit`.

### 7.4 The fetch plane: connectors, the COG sampler, and STAC search

The first read of any band at any cell on Earth is a cold materialisation: the responder fetches the underlying value from a real upstream source, signs it under its own key, persists it, and returns it in the same response. This is what makes "every cell answers from the first request" true without pre-seeding. The fetch plane that backs it is 16 dedicated `emem-fetch` data connectors and a set of inline materializers in the router crate, together backing 46 declared source schemes in `sources-v0.json` and 124 live materializer registrations across the 43 cube slots that compose the 1792-D voxel. Five of the 46 schemes (`openet.30m.daily`, `dynamic_world.v1`, `tropomi.s5p.ch4`, `tropomi.s5p.no2`, `viirs.dnb.monthly`) are declared but not yet wired; recall on those bands returns a typed `MaterializeMiss` Absence rather than fabricated data, so the catalogue never promises more than it can sign.

The dispatcher `emem-fetch::Dispatcher` is stateless and registers connectors against connector kinds. The shared `HttpsConnector` is an anonymous `reqwest` client that sets `Accept-Encoding: identity` so that HTTP `Range` offsets stay aligned with the original GeoTIFF byte layout, with a 90 s pool-idle timeout and surfacing of HTTP 429 as `FetchError::RateLimited` carrying `Retry-After`. The dedicated modules cover sources that do not fit the COG range-read pattern: CHIRPS rainfall, DMSP-OLS nightlights, FIRMS active fire, Fields of The World [35] field polygons (PMTiles range reads over `source.coop`), the GeoNames cities-5000 gazetteer [40], Hansen GFC v1.12 forest-change [32], Köppen-Geiger climate classes [36], Overture divisions and places [39], TerraClimate, WorldPop, and WDPA-via-OSM.

The load-bearing module is `cog.rs`, a pure-Rust Cloud-Optimized GeoTIFF point sampler. A materializer wants one number per cell, not a raster, so the sampler range-reads only the IFD plus the single tile covering the requested pixel: it reads the first 64 KiB of the COG, parses the TIFF header and IFD0 entries, pulls `TileOffsets`/`TileByteCounts`, computes the world-to-pixel transform from `ModelPixelScale` and `ModelTiepoint`, range-reads the containing tile, decompresses it, undoes the predictor, and extracts the pixel. A per-cell recall therefore touches a few hundred kilobytes of a gigabyte-scale Sentinel-2 [28] scene rather than the whole file. The sampler supports little-endian standard TIFF and BigTIFF (the 64-bit-offset extension used by the EU JRC single-COG global rasters such as the 41 GB GFC2020 V3), Deflate and LZW compression, predictors 1 through 3, and 8/16/32-bit little-endian samples; any layout outside that slice (big-endian, JPEG2000, planar) returns an error rather than silently sampling the wrong bytes. World coordinates are produced by `emem-fetch::proj` (a WGS84↔UTM transform after Snyder [43]) so the sampler is handed a coordinate already in the COG's CRS.

Sentinel scene discovery runs through a minimal STAC POST-search client. The primary host is the Element84 Earth Search v1 endpoint over AWS Open Data; the Microsoft Planetary Computer STAC is the fallback and the only free source for the `sentinel-1-rtc` RTC format (its asset URLs are SAS-token-signed). The search picks a scene (typically the least-cloudy recent acquisition), and the resulting COG asset URL feeds straight into the `cog.rs` sampler. No upstream on any hot path is requester-pays or requires operator credentials.

The two similarity indices are backed by Lance [26]. `find_similar` over a vector band, and `memory_search` over the BGE-base-en-v1.5 768-D text embeddings of memory files [25], each open a Lance partition with an IVF_PQ approximate-nearest-neighbour index; when the index is empty or `EMEM_DISABLE_LANCE=1` is set, both fall back to a correct brute-force scan, and the response's `via` field reports which path served the query. The Hamming fast path in `find_similar` derives a binary sibling inline through the TurboQuant sign-bit rotation [27] when only the cosine band is attested.

### 7.5 The explain sidecar

The deterministic `/v1/ask` answer is an LLM-free projection of the signed fact set, which is precisely what makes its receipt byte-stable and re-verifiable offline. A natural-language rephrasing for a non-expert is useful but must not be allowed to contaminate that property. The implementation resolves the tension by placing the language model in a second, separate process behind `POST /v1/explain`, which forwards an `/v1/ask` response to a loopback Gemma-4-12B sidecar [24] (4-bit NF4 quantisation, $\approx$ 7 GB of GPU memory) that rewords the already-signed numbers. The system prompt constrains the model to interpret only the supplied values and never to invent or estimate one. The returned prose is flagged `signed: false` and carries a disclaimer stating that the signed artifact is the emem receipt, not the commentary; if the sidecar is offline the endpoint returns `available: false` and the signed answer is unaffected. The model never writes a fact, mints a CID, or signs anything.

Because the same GPU is shared with the inference sidecar and runs near full, two concurrent generations would each claim KV cache with no headroom and OOM. The sidecar therefore serialises generation behind a single lock and calls `torch.cuda.empty_cache()` after each generation, so concurrent requests queue rather than fight for VRAM and the next request begins with the cache cleared. This VRAM hygiene is the reason the explain layer degrades to "slow" rather than "non-responsive" under load.

### 7.6 Deployment

The simplest deployment is one container:

```
docker run -d -p 5051:5051 \
  -v emem_data:/var/emem \
  -e EMEM_BIND=0.0.0.0:5051 -e EMEM_DATA=/var/emem \
  ghcr.io/vortx-ai/emem:0.0
```

No environment variable is required. `EMEM_BIND` overrides the listener and `EMEM_DATA` overrides the data directory (`:memory:` for an ephemeral instance whose key rotates on every restart). The multi-arch image is anonymously pullable and runs as a non-root UID on `debian:trixie-slim`; it pre-applies `cap_net_bind_service` to the binary so binding port 443 inside the container needs no extra capability grant. The production node runs as a systemd user unit instead, which terminates TLS on port 443 itself: setting `EMEM_TLS_DOMAINS` activates `rustls-acme` with the ACME TLS-ALPN-01 challenge against Let's Encrypt, provisioning and caching certificates under `<EMEM_DATA>/acme.cache/` with no nginx, Caddy, or Cloudflare in the path. The plain HTTP listener stays up in parallel for local MCP clients. A strict Content-Security-Policy is emitted on every response and intentionally has no env-var override, because it allowlists the `esm.sh` modules that the in-browser `/verify` page imports to run ed25519 verification client-side; the offline-verify path is a load-bearing trust claim and must remain wired by default. The same image and the same `:0.0` tag back the HuggingFace Space wrapper.

A first-contact agent discovers the responder without prior knowledge through a fixed chain. The `/.well-known/{emem,agent,mcp,ai-plugin}.json` documents advertise the responder pubkey, the recommended tool order, and the MCP transport. `/v1/manifests` returns the active `bands_cid`, `algorithms_cid`, `sources_cid`, and `schema_cid`; `/v1/agent_card` returns a live capability snapshot pinned to those CIDs; and `/llms.txt` (with `/llms-full.txt`) provides a plaintext catalogue for direct LLM ingestion. Two registry-divergence checks make federation observable before any data flows: a peer with drifted registries returns a different `bands_cid` on `/health`, and a peer that recomputes a fact under matching CIDs produces the same bytes.

### 7.7 Operator attestation

Content addressing proves what a fact is; it does not by itself prove which code produced it. The reference responder closes this gap with an operator attestation in `/.well-known/emem.json`. At startup the binary reads `/proc/self/exe` and computes the BLAKE3 [17] hash of its own executable image, caching it in a `OnceLock`. The build script captures the git commit SHA and an RFC 3339 build timestamp at compile time through `option_env!`, falling back to `"unknown"` honestly when the tree is not a git checkout. The responder then signs the triple under its ed25519 key over the preimage

```
emem.operator_attestation|v<version>|epoch<n>|<attested_at>
  |git:<commit>|build:<timestamp>|binary:<blake3>
  |bands:<bands_cid>|registry:<registry_cid>
```

A verifier reconstructs this preimage from the published fields and checks the signature against the responder pubkey, confirming that the running binary corresponds to a named source tree and registry set without trusting the operator's word. The block reserves a `tee_quote` field, null on a commodity host, that an operator deploying under a hardware trusted execution environment (Intel SGX or AMD SEV-SNP) populates from the platform's quoting service, extending the binding from "this binary" to "this binary on attested hardware."

### 7.8 Honest limits of the implementation

The reference responder is deliberately single-host: there is no built-in clustering, leader election, or replicated sled, and the multi-host federation routing of Sections 9.3 and 10 does not ship in 0.1.0. What ships is the substrate that makes federation possible, namely content addressing, signed receipts, typed temporal edges, multi-attester contradiction scoring, and a deterministic refinement loop. Read replicas are achievable by pointing a second `emem-server` at a snapshot of `<EMEM_DATA>`, because receipts continue to verify under the file-pinned identity, but attestation writes need a single primary because the append-only Merkle log is single-writer. Among the encoders, the JEPA-v2 dynamics head is untrained and signs an honest identity baseline; Galileo has only its Sentinel-2 modality wired; and `clay_v1`/`prithvi_eo2` are seed-only at this responder, in that the sidecar runs both models but the auto-materialise fan-out to upstream tile archives is not wired, so recall against them returns whatever has already been signed. Each of these limits is disclosed at runtime in the relevant response envelope rather than papered over, which is the implementation-level expression of the no-silent-fallback rule that runs through the entire system.

## 8 Evaluation

The evaluation of emem is not a leaderboard. emem is a protocol with a
reference responder, and the quantity that matters is whether a peer can
reproduce a cited answer byte for byte, not whether the responder ranks
above some baseline on an aggregate score. This section characterizes the
substrate along three axes that a reviewer can independently re-derive:
the shape of the live corpus (drawn from the responder's own signed
liveness tick), the latency profile of cold and warm reads (including the
foundation encoders and the rate-limited connectors that dominate the tail),
and the reproducibility surface (the content-addressed manifests, the
four-curl verification flow, and the two harnesses, `emem-membench` and
`cargo test --workspace`, that stand in for a conformance suite). Numbers
that depend on corpus size are reported as the responder reports them, with
the scan cap stated, so a different deployment yields a different but equally
verifiable snapshot rather than a number to be trusted on faith.

### 8.1 Corpus characterization

The responder exposes its own liveness state two ways: as a signed
Server-Sent Events heartbeat at `/v1/stream` (a `corpus.state` tick every
5 to 300 s, default 15 s) and as a one-shot signed snapshot at
`/v1/corpus_state_stats`. Both back the same payload from one bounded index
pass. The snapshot handler walks `storage.iter_index` capped at a fixed
scan limit of $32768$ entries, accumulates the distinct cell set, the
distinct band set, and per-band fact and cell counts, then signs a
deterministic preimage

$$
\texttt{emem.corpus\_state\_stats}\ \|\ v_{\text{ver}}\ \|\ \texttt{epoch}_k\ \|\ t\ \|\ \texttt{cells}{:}n_c\ \|\ \texttt{bands}{:}n_b\ \|\ \texttt{facts}{:}n_f
$$

under the responder's Ed25519 key, where $n_c$ is the distinct-cell count,
$n_b$ the distinct-band count, and $n_f$ the number of facts scanned. The
tick on `/v1/stream` signs an analogous preimage
($\texttt{emem.stream.tick}\ \|\ \dots\ \|\ \texttt{registry}{:}\langle\text{cid}\rangle\ \|\ \texttt{cells}{:}n_c$),
so a subscriber verifies corpus liveness offline without re-fetching the
index. A representative tick captured from the hosted node `emem.dev`
reports $n_c \approx 8147$ distinct cells, $n_b = 75$ distinct bands, and
$n_f = 32768$ facts scanned, the last value equal to the scan cap.

Two properties of this snapshot are deliberate and material to its
interpretation. First, `facts_scanned` is the count of entries the bounded
pass actually visited, not a claim of total corpus size; the payload carries
a `note` that corpora above the cap are paginated through
`/v1/coverage_matrix`, so the figure is a floor under the corpus, never an
inflated total. Second, the per-band breakdown (`by_band`, sorted by fact
count then band name) lets a reviewer see the distribution rather than a
single scalar: the corpus is dominated by the auto-materialized bands
(Copernicus DEM elevation, the spectral indices, the Tessera embedding stack)
because those are what cold recalls populate. The corpus grows monotonically
from use. Every cold read on `/v1/recall` signs and persists a fact, so the
distinct-cell count is an artifact of query history, and a fresh `:memory:`
responder legitimately reports $n_c = 0$ until the first read. The snapshot
is therefore a measurement of one deployment's history, signed so it cannot
be misrepresented, not a fixed property of the protocol.

### 8.2 Latency profile

emem's latency separates cleanly into three regimes: warm reads served from
the sled hot cache, cold reads that trigger materialization from an upstream
product, and the foundation-encoder fan-out served by the GPU sidecar. The
read path is the common case. A warm `/v1/recall` against an already-attested
`(cell, band, tslot)` key resolves in under $10$ ms, dominated by the index
lookup and the receipt-signing step (one BLAKE3 over the preimage plus one
Ed25519 signature). A cold recall that misses and materializes a scalar band
through a Cloud-Optimized GeoTIFF Range read (the universal `cog.rs` sampler,
backing Copernicus DEM [30], Hansen loss-year [32], ESA WorldCover [33], the
spectral indices over Sentinel-2 [28]) completes in roughly $180$ ms: a
single HTTP Range request against an indexed COG, the value computation, the
`Fact::Primary` construction, the attestation sign-and-persist. The
materialization path is gated by a $30$ s per-fact materializer timeout and a
$180$ s gateway timeout, with a $16$ MiB body cap; a miss with no registered
connector returns a typed `materialize_miss` Absence rather than a silent
empty, so the latency budget never trades correctness for a fast wrong answer.

The four foundation encoders have distinct cost structures because three are
GPU-pinned and one streams on CPU. Table 8.1 reports the per-encoder cold and
warm inference latencies measured on an RTX 4090, where cold includes weight
load from the local Hugging Face cache and warm is the steady-state forward
pass on a single chip.

| encoder | output | cold | warm | execution |
|---|---|---|---|---|
| Clay v1.5 [2] | 1024-D | $\sim 6$ s | $\sim 18$ ms | GPU, wavelength-conditioned ViT-L/8 MAE + DINOv2 teacher |
| Prithvi-EO-2.0-300M-TL [3] | 1024-D | $\sim 10$ s | $\sim 20$ ms | GPU, multi-temporal HLS V2 MAE |
| Galileo (base) [4] | variant-dependent | $\sim 4$ s | $\sim 14$ ms | GPU, S2 modality wired; S1/DEM/climate zero-masked |
| Tessera [1] | 128-D | (CPU stream) | (CPU stream) | streamed from precomputed COG tiles, no inference |

*Table 8.1: per-encoder latency. The three trained encoders serve frozen
embeddings; receipts carry `frozen_pretrained_encoder`. Tessera is not
GPU-pinned: its 128-D annual stack is published as Cloud-Optimized GeoTIFF on
a 0.1 degree grid, so the responder reads it per pixel on CPU and can render
a dense embedding field over a region without any forward pass.*

The cold-versus-warm gap on the GPU encoders is a one-time weight-load cost;
once resident, the trained encoders return in tens of milliseconds, so the
triple-consensus change algorithm (Clay, Prithvi, Tessera voting over a
365-day window) is bounded by the slowest cold load rather than by per-cell
inference. The `/v1/ask` foundation fan-out runs the encoders concurrently
under a `tokio::join!` and carries an `ask_timeout_ms` budget (default
$4000$ ms, read from the algorithm's parameter block); on timeout it emits a
`foundation_embedding_timeout` degraded marker and the topic-router path still
returns a useful, signed answer.

The tail of the latency distribution belongs to a small set of rate-limited
upstream connectors whose cost is structural, not incidental, and the
responder discloses each one rather than hiding it. MODIS land-surface
temperature (`modis.lst_day_8day`) materializes through the NASA/ORNL REST
API at roughly $30$ s per cell [29]; to keep urban-heat queries inside the
gateway timeout, hunter mode caps the per-region fan-out for the LST family at
$8$ cells (against the default $32$), overridable by
`EMEM_HUNTER_SLOW_BAND_CAP`. WorldPop population materializes at $2$ to $4$ s
per cell at request time, because the public Statistics API computes a
zonal aggregate per query rather than serving a Range-addressable raster.
Three further products (JRC TMF, the Sims et al. 2025 driver attribution, and
RADD Sentinel-1 alerts) do not honor HTTP Range at their upstream and so sit
off the EUDR verdict hot path entirely; each remains reachable as an explicit
band request but never blocks a latency-sensitive composite. The protocol's
position is that a slow band must be slow honestly: the response surface
declares the cap, the reason, and the connector, so an agent budgets around
the cost instead of discovering it as a timeout.

### 8.3 Surface counts as a reproducibility claim

The reference responder's surface is fixed by content-addressed manifests,
which turns a set of catalog counts into a reproducibility assertion rather
than a marketing tally. The responder exposes $81$ MCP tools ($10$ core, $71$
extended), $93$ documented REST paths under `/v1/*` surfaced through
`/openapi.json`, $124$ live materializer registrations across the $46$
declared source schemes (five declared-but-unwired, answering with a typed
Absence), $160$ named algorithm recipes, and $27$ topic groupings. The voxel
ontology pins exactly $43$ cube slots summing to $1792$ dimensions; the gap
between $43$ slots and $124$ materializer-wired band names is the parametric
expansion (each Sentinel-2 reflectance band, each spectral index, each Tessera
vintage) under a fixed slot offset, so the dimensional layout is stable while
the catalog of nameable readings grows.

The claim these counts support is reproducibility of bytes. Every receipt
pins four manifest CIDs, each a 32-byte BLAKE3 prefix over the
canonical-CBOR encoding of the corresponding registry:

$$
\texttt{bands\_cid} = \texttt{base32}_{\text{nopad,lower}}\big(\texttt{blake3}(\texttt{cbor}(\text{BandsManifest}))[..32]\big)
$$

and likewise `algorithms_cid`, `sources_cid`, and `schema_cid`. Two
properties follow. A peer whose four CIDs match the responder's recomputes any
fact, algorithm output, or signed envelope and obtains byte-identical results,
because the CID rule fixes the canonical CBOR and BLAKE3 truncation that
produce every fact id (a 128-bit truncation whose birthday bound, $\sim 2^{64}$
facts, is far above the $\sim 10^5$ facts the canonical responder holds today).
A peer whose registries have drifted returns a different `bands_cid` on
`/health`, so the divergence is visible before any data flows and a verifier
never silently compares answers computed under different ontologies. The
algorithm registry strengthens this for composites: an algorithm carrying an
`evaluation: Expr` AST is re-executable in-process, and a third party with
matching `algorithms_cid` and matching input fact CIDs reproduces the
composite scalar deterministically (the worked `flood_risk@2` AST evaluates to
$0.4836$ byte-stably under its conformance test). The counts are thus the
visible handle on a content-addressed conformance set, and a reviewer who
pins the same manifest CIDs is testing against the same protocol, not a
look-alike.

### 8.4 Reproducibility protocol

Reproducibility is exercised at four granularities, from a single answer to
the whole workspace.

**Per-answer verification (four curls).** Any reviewer reproduces a cited
answer with no account and no trust in the issuer. The flow resolves a place
to a `cell64` (`/v1/locate`), recalls a band and captures the receipt
envelope (`/v1/recall`), asks the responder to re-check its own signature
(`/v1/verify_receipt`, returning `{valid, preimage_blake3_hex,
fact_cids_count, signer_pubkey_b32}`), and re-fetches the same `fact_cid` from
any responder on any day, where the content-addressed `(cell, band, tslot,
derivation.fn_key)` quadruple guarantees the returned bytes hash to the same
id. The browser path at `/verify/<fact_cid>` performs the identical Ed25519
check in WebCrypto and `@noble/curves`, recomputing the canonical preimage
client-side, so the reviewer never trusts the responder that served the
receipt. The receipt's `preimage_version` selects the signing rule; v1 uses a
domain-separated, length-prefixed preimage, and the Merkle tree uses RFC 6962
[14] leaf/node domain separation with a duplicate-leaf guard.

**Agent-facing benchmark (`/v1/benchmark`).** A small hand-verified item set
(version v0, five items: three `recall`, two `find_similar`) lets an agent
grade itself deterministically. Each item names a task, a cell, a band or
encoder, and an expected answer, an exact `fact_cid` for recall items and a
`(top-neighbour cell, score_min)` pair for `find_similar` items. The grader at
`/v1/benchmark/grade` scores a submission map of item id to answer and returns
per-item correctness with reasons. The expected CIDs and neighbour cells are
byte-identical to what `/agents.md` and the whitepaper cite (for example,
South Mumbai elevation $6.0$ m, Bengaluru elevation $910$ m, the
Bengaluru-seeded geotessera neighbours at cosine $0.6537$ and $0.6426$), so
the benchmark is a fixed, content-verified target rather than a moving score.

**Memory-substrate benchmark (`emem-membench`).** The substrate is scored
against memory-agent benchmarks in the LongMemEval-S and MemoryAgentBench
[11] style. The harness has two modes. `--self-test` grades an in-memory stub
against a built-in synthetic fixture with no network, exercising the scoring
code in CI; it is unsigned by design. `--live --url <responder>` loads a
dataset corpus into a running responder over the real write API
(`memory_create`, one signed memory file per item), answers every query over
the real read API, and computes four axes from the responder's own output:
retrieval accuracy, test-time learning (last-write-wins rewrite of the same
path, then re-query for the post-update answer), long-range understanding
(re-query the earliest-loaded third of the corpus, where a recency-biased
store would have dropped the needle), and conflict resolution (store a
disagreeing companion non-destructively and require the responder to surface
it). The topline is the item-weighted fraction correct across all graded
questions,

$$
\text{topline} = \frac{\sum_a \text{correct}_a}{\sum_a \text{items}_a},
$$

summed over the four axes $a$, the LongMemEval-S convention. No score is
hardcoded; the answer-recall criterion is whether retrieved content contains
the ground-truth answer under case, whitespace, and numeric normalization.
Three honesty controls govern interpretation. The read path is probed at run
time and labelled: `memory_search` (BGE-768 semantic [25] over the LanceDB
IVF_PQ index [26]) when the embedder is loaded, otherwise an explicitly
labelled `recall_fallback` lexical ranker (the default offline build ships no
embedder weights), and a fallback run is never presented as a semantic run.
The conflict method is similarly labelled `contradiction_scan` or
`stored_distinct_fallback`. And the dataset provenance is stamped `sample`
versus `full`: the committed $\sim 15$-item sample exists only so `--live`
runs end-to-end with no download, its score is illustrative, and the harness
refuses to let it stand in for a published number. A live run embeds the
responder's signed receipt so the scorecard is itself independently
verifiable.

**Workspace conformance (`cargo test --workspace`).** The de-facto
conformance check is the workspace test suite, on the order of several hundred
test functions across the sixteen crates. The load-bearing fixtures sit at
the protocol primitives: round-trip and canonical-CBOR tests in `emem-fact`
and `emem-codec` (the `cell64` bit-layout encode/decode, the `tslot`
base32-nopad-leb128 text round-trip, the `cid64` BLAKE3 truncation), the
Merkle and attestation tests in `emem-attest` (leaf self-hashing for domain
separation, inclusion-path verification, full `verify_attestation` re-encode
and re-fold), and the bi-temporal, LanceDB, and memory-search integration
tests in `emem-primitives`. Because the CID rule is deterministic, any
conforming implementation that shares the struct definitions produces
byte-identical CIDs from byte-identical inputs, and these fixtures are exactly
the assertion of that property: a green workspace means the codec, the
content-addressing, the signing preimage, and the Merkle math agree with the
reference bytes. Conformance is therefore something a reviewer runs, not
something the paper asserts.

Taken together, the four granularities make the same claim at four scales: a
peer with matching manifest CIDs reproduces emem's answers from a single
`fact_cid` up to the entire registry, and the responder's own signed liveness
tick, latency disclosures, and labelled benchmark provenance leave no number
that has to be taken on trust.

## 9 Limitations and Honest Absences

emem is governed by a single discipline that shapes how its limits are reported: a capability the system does not have returns a typed Absence or a structured error code, never a fabricated affirmative answer. The protocol distinguishes three failure modes that a conventional service would collapse into one. A query that asks for something the responder cannot do returns an `unavailable_capability` Absence; a query for a place an upstream product does not cover returns an `outside_coverage` Absence; a query that is malformed returns a structured `ErrorCode`. No surface returns `verdict=false` to signal that it lacks the data to compute a verdict, because a false verdict and an absent capability are different claims, and conflating them is precisely the silent-fallback failure the design rejects. This section enumerates the limits of the 0.1.0 responder in that spirit: each missing capability is named together with the typed signal it emits, so that an agent can reason about the gap rather than mistake silence for a negative result.

The general rule is stated once and applied throughout. A NegativeFact carries the same `(cell, band, tslot)` address a PrimaryFact would, plus a `ReasonCid` pointing at the upstream evidence that confirmed the absence; the seven typed reasons in 0.1.0 are `unavailable_capability`, `outside_coverage`, `gpu_unavailable`, `archetype_seed_unavailable`, `materialize_timeout`, `materialize_miss`, and the `over_water`/`over_land` boundary refusals. Each NegativeFact is signed and content-addressed, so the same question asked twice resolves to the same absence CID, and the absence is citable as evidence in exactly the way a positive fact is.

### 9.1 Sensing floor: no sub-meter or commercial imagery

The finest spatial pitch the responder serves is the Sentinel-2 10 m optical floor [28]; coarser products (Landsat 8/9 at 30 m [29], MODIS land products [29], Copernicus DEM at 30 m [30]) sit below it. There is no commercial high-resolution imagery pipeline. The system does not ingest sub-meter optical (Planet, Maxar) or any proprietary tasking product, and it does not synthesize a finer pitch than its inputs support. The receipt schema is forward-compatible with such sources: it accepts whatever `model_id` and `sensor_id` an operator attests under their own Ed25519 key [15], so a customer with a sub-meter connector can bring it without a protocol change. What does not ship is the connector itself, and a request for a resolution finer than 10 m is answered honestly rather than upsampled. This floor is a deliberate consequence of binding the public responder to open, range-readable products; the consensus algorithms enforce it structurally through the sensor-tier rule, which fails to load any algorithm that claims delivery resolution $\leq 10$ m without at least one Sentinel-1, Sentinel-2, or Landsat input in its variance sources.

### 9.2 No edge or onboard inference; single-host sidecar

All inference runs in-tenant on a single GPU sidecar (Python FastAPI over a Unix domain socket, co-residing the encoders on a 20 GB VRAM budget). No spacecraft-bus encoder firmware ships with the protocol, and no model runs on an edge device or onboard a sensor platform. The architecture is encoders-on-ground, not encoders-in-orbit (Figure `docs/diagrams/31-encoders-in-orbit-decoders-on-ground.svg`). The sidecar is a single host, not a cluster, and its failure is contained rather than cascading: when the sidecar is down the REST router degrades to scalar bands and signs the GPU-anchored algorithms as Absence with the `gpu_unavailable` reason, so a sidecar crash produces a citable negative result, not a 500.

### 9.3 Single-host deployment; federation is substrate, not service

The 0.1.0 deployment is a single primary responder with read-only replicas. There is no multi-host clustering, no global request routing, and no SOC 2 attestation. The TEE attestation surface is present but unpopulated: `/.well-known/emem.json` binds the running binary's BLAKE3 hash [17], git commit, and build timestamp under the responder's key, but its `tee_quote` field is `null` until the binary runs under Intel SGX or AMD SEV-SNP, at which point the platform's quoting service populates it.

What the design does ship is the substrate that makes federation possible, not the routing that would make it automatic (Figure `docs/diagrams/08-decentralised.svg`). Because every fact, edge, and contradiction is itself content-addressed and signed, independent responders that resolve the same content ids byte-for-byte could cross-cite each other's attestations and record where they disagree across hosts. The mechanism is already exercised within one responder: typed temporal edges (signed `EdgeFact(subj, pred, obj, valid_from, valid_to)` records read through `emem_edges_recall`, with bi-temporal supersession so a newer edge shadows rather than deletes the older one), multi-attester contradiction scoring over facts at the same `(cell, band, tslot)`, and a deterministic, opt-in refinement loop (`EMEM_REFINEMENT_ENABLED`) that turns attester disagreement into a signed `disagrees_with` edge plus a re-attestation flag, never a deletion. These components run inside one responder today; the disagreement graph that would span a network is a forward target, and the absence of the routing layer is stated rather than papered over with the substrate's presence.

### 9.4 Encoder maturity: untrained dynamics, single-vintage and seed-only backbones

Three of the four model gaps are encoder-maturity limits, each surfaced through a receipt warning rather than a silent substitution.

**JEPA v2 is untrained.** The on-disk artifact is a residual-zero identity baseline: three 128-D Tessera lags [1] flatten to a $[B, 384]$ input, pass through a 128-D projection and four pre-LN residual blocks to a zero-initialised head, and produce $\hat{v} = v_{\text{last}} + \Delta$ with $\Delta \equiv 0$ at initialisation. With the delta head zeroed, the prediction is the identity map. The v2 handler short-circuits ONNX/sidecar inference whenever `is_trained() == false` (which it is in 0.1.0), returns `last_input_vintage` directly, and attaches the `untrained_baseline` and `upstream_geotessera_single_vintage` honesty warnings to the receipt, with `via: "short_circuit_untrained"`. The training pipeline is ready; training is gated on the upstream publishing at least three Tessera vintages per cell, and the candidate-pool backfill is the bottleneck, not the model code.

**Tessera multi-vintage is upstream-rate-limited.** The `dl2.geotessera.org` bucket ships annual vintages for 2017-2024 [1], but most cells in `/v1/coverage` have only the latest year attested locally because the upstream fetch is rate-limited. This is the same constraint that blocks JEPA v2 training: a per-cell temporal embedding history deep enough to fit a dynamics model does not yet exist at the responder.

**Clay and Prithvi are seed-only materializers.** The three trained encoders serve frozen embeddings, with receipts carrying `frozen_pretrained_encoder`; Clay v1.5 [2] and Prithvi-EO-2.0-300M-TL [3] are wired for single-scene CLS extraction rather than the full multi-vintage temporal stacks their architectures admit. **Galileo's non-Sentinel-2 modalities are zero-masked.** The Galileo encoder [4] accepts the full multimodal input shape, but only Sentinel-2 chips are wired; the Sentinel-1, ERA5, TerraClimate, VIIRS, SRTM, Dynamic World, WorldCover, and LandScan modalities are passed as zero tensors. The encoder therefore produces a defensible S2-only embedding, and the limitation is the zero-mask, not a fabricated multimodal vector. The advertised capability string is `galileo-<variant>` so an agent can read at request time which dimension actually ships.

### 9.5 Declared-but-unwired sources

The source catalog declares 46 schemes; 16 data connectors and their 124 materializer registrations answer recall today. Five schemes are declared in the manifest but their materializers are not wired in 0.1.0: `openet.30m.daily`, `dynamic_world.v1`, `tropomi.s5p.ch4`, `tropomi.s5p.no2`, and `viirs.dnb.monthly`. A request for any of these returns a typed Absence with the `materialize_miss` reason, not data and not a 404. The catalog count is reported as the declared-versus-wired pair precisely so that the larger number is never mistaken for the answerable surface; the registry's `sources_cid` commits to all 46 schemes under a single BLAKE3 digest [17], so the unwired five are auditable rather than hidden.

### 9.6 Removed surfaces kept honest

Three surfaces were advertised in early releases and have been removed rather than left as latent dead code that might return a misleading partial answer. The zero-knowledge verifier mode (`verify Mode::Zk`) was advertised in 0.0.3, returned a 500, and was removed in 0.0.4. The `Attestation.stake` field was reserved for a staking economy that does not exist; it was removed from the struct and from every call site so that no receipt carries an unbacked economic claim. The Filecoin/IPFS bridge (`IpldConnector`) remains a stub and is documented as such; emem composes its CID rule on top of IPLD's CBOR tag 42 base32 encoding [16, 18, 19] but does not bridge to an IPLD network. In each case the honest move was to delete the surface rather than ship a placeholder, consistent with the no-stub discipline applied across the codebase.

### 9.7 Consensus fusion: an unweighted fuse, deliberately not faked

The central change algorithm `clay_prithvi_tessera_triple_consensus@1` fuses three foundation encoders with independent receptive fields into a per-cell change index [2, 3, 1]. Each encoder contributes a cosine-distance term over a 365-day window,

$$d_c = \mathrm{clamp}\!\left(1 - \cos(\mathbf{c}_{t}, \mathbf{c}_{t-1\mathrm{y}}),\, 0,\, 1\right),$$

and analogously $d_p$, $d_t$ for Prithvi and Tessera. The three terms are combined by an unweighted root-mean-square fuse,

$$\mathrm{ensemble} = \sqrt{\frac{d_c^2 + d_p^2 + d_t^2}{3}},$$

with an agreement label (`all_three`, `two_of_three`, `one_or_none`) derived by thresholding each leg at the registry-tunable gate (default 0.15, learned from the LandTrendr ensemble convention of Healey et al. [37]). The equal weighting is a known simplification. The three encoders differ in reliability per land-cover regime and per receptive field (Clay at $\sim$2.56 km, Prithvi at $\sim$6.7 km, Tessera per-pixel), and a source-monitoring fuse that weighted each leg by its regime-conditioned reliability would be more discriminating than the RMS. That weighting is deferred, not faked: the `parameters` block is the designated place for per-leg weights, so a weighted consensus is a registry-CID change rather than a recompile, and until it is fitted the responder reports the honest equal-weight ensemble rather than an arbitrary hand-set weighting that would imply calibration the system does not have. The companion deferral is recency-weighted contradiction scoring: the refinement loop currently treats all conflicting attestations at a `(cell, band, tslot)` symmetrically, and weighting disagreement by observation recency is a forward step left explicit rather than improvised.

### 9.8 Privacy classes and the legal surface

Privacy is enforced per band before any fact is served, through four declared classes (`crates/emem-core/src/privacy.rs`). **Public** bands are unrestricted at any resolution. **AggregateOnly { min_res }** bands (population-density products) must not be served finer than `min_res`; a finer query is snapped up to the coarser parent and the receipt carries `privacy_snapped: true`. The class is picked at city-block scale ($\sim$24 m at resolution 11) to avoid identifying individual buildings. The admission test is

$$\text{permitted}(r) = \begin{cases} \text{true} & \text{Public} \\ r \leq \texttt{min\_res} & \text{AggregateOnly} \\ \text{conformance\_l2} & \text{L2OnlyWithModelCid} \\ \text{false} & \text{Prohibited} \end{cases}$$

**L2OnlyWithModelCid** bands (fine-resolution embeddings tied to a specific model checkpoint) are admissible only at conformance level L2 and require the `Source.cid` of the checkpoint that produced the value; an L1 responder must refuse. **Prohibited** is reserved and conforming implementations must refuse to serve it; no band declares it in 0.1.0. Refusal under any class returns a typed signal, not a fabricated coarse value passed off as fine.

The legal surface is enumerated rather than asserted as compliance. The cited frameworks are GDPR (Regulation 2016/679), UK-GDPR, DPDP-2023, CCPA-CPRA, and RFC 9116. Operationally, the canonical responder logs `agent_ip_hash = base32_nopad_lower(blake3(client_ip)[..8])` rather than a raw client IP [17, 18], does not capture POST bodies, and retains GET query strings only for the 30-day journald window. The EUDR Due Diligence surface is similarly scoped: it implements the measurable parts of Regulation (EU) 2023/1115 [38] and carries an explicit `legality_disclaimer` for Article 9(1)(b) (land tenure, FPIC, country-of-origin law) and a `degradation_disclaimer` for Article 2(7), marking the parts that are structurally out of Earth-observation scope rather than reporting a verdict the data cannot support.

### 9.9 Summary of typed signals

Table 9.1 collects the limits above against the typed signal each emits, making concrete the invariant that closes this section: a missing capability is reported as a citable, content-addressed Absence or a structured error, and never as `verdict=false`.

| Limit | Typed signal emitted |
|---|---|
| Resolution finer than 10 m | Sensor-tier rule rejects the algorithm at load; no fact minted |
| GPU sidecar unavailable | Absence, reason `gpu_unavailable` |
| Five unwired source schemes | Absence, reason `materialize_miss` |
| Connector exceeds per-fact budget | Absence, reason `materialize_timeout` |
| Cell outside upstream coverage | Absence, reason `outside_coverage` |
| Capability not implemented | Absence, reason `unavailable_capability` |
| Classifier seed table missing | Absence, reason `archetype_seed_unavailable` |
| Land/bathymetry boundary refusal | Absence, reason `over_water` / `over_land` |
| Sub-`min_res` query on protected band | Snapped to coarser parent, `privacy_snapped: true` |
| JEPA v2 prediction | Identity baseline, receipt `untrained_baseline` |
| Galileo non-S2 input | Zero-masked, S2-only embedding returned |
| Removed Zk / stake / IPLD surface | Surface deleted, not stubbed |

The discipline these signals encode is the one stated in the introduction to this section and worth restating in closing: an empty answer in emem is a signed receipt with a reason an agent can read, cite, and cache, so the boundary of the system's competence is itself part of the verifiable record.

## 10 Conclusion

emem collapses two questions an embodied or analytic agent repeatedly faces into one signed, content-addressed, planet-keyed memory layer. The first question, *what is on the ground here, and can you prove it*, is answered by the Earth-memory layer: an open-data corpus addressed by the triple $(\texttt{cell}, \texttt{band}, \texttt{tslot})$ whose unit is an Ed25519-signed fact [15], reproducible byte-for-byte on any peer that mirrors the inputs. The second, *what did we learn here*, is answered by the agent-memory layer: a content-addressed file store under `/memories/*` with six file-operation verbs conforming to the Anthropic memory-tool specification [22], four memory kinds drawn from the CoALA agent-memory ontology [7], and an Ed25519 capability binding on paths under `/memories/by_attester/<pubkey>/...`. Both layers ride a single trust surface. Both return receipts whose signature is checked against a deterministic BLAKE3 [17] preimage, and the same `/verify` page recomputes that preimage in the browser using `@noble` primitives [15], so any party validates an answer offline without trusting the issuer that produced it. An answer that does not exist is still a citable object: a signed `Absence` carrying a typed reason, never a silent empty array or a 404.

The protocol specifies the loader, the validator, the CID rule, the receipt-signing rule, the capability-binding rule, and the primitive semantics; it is never the data itself [1], [28], [32]. Any conforming implementation must produce byte-identical CIDs from byte-identical inputs, and the conformance target is a content-addressed manifest pinning the band ontology, algorithm registry, source catalog, and CDDL schema bundle. The reference responder is a single Rust binary; the same handlers answer both Model Context Protocol [23] and plain REST, reads require no authentication, and every write lands in an append-only Merkle log [14].

### 10.1 The through-line: address by what a memory is about

The organising principle of emem is that a memory is addressed by *what it is about* — a patch of the Earth's surface — rather than by *where it sat in a stream* of tokens, turns, or retrieval hits. A `cell64` is a 64-bit packed identifier for a square WGS-84 bucket, approximately $9.55\,\mathrm{m}$ on a side at the equator, that addresses a place the way a token addresses text in a language model. Adjacent codepoints in its Hilbert-ordered text alphabet map to physically nearby cells, so an agent that emits a cell handle already lands in roughly the right place. This is the same move the compact in-attention memory literature makes when it addresses a recurrent state associatively rather than positionally [13]: the entire knowledge graph for a place collapses to one handle a downstream tool quotes, shares, and verifies.

The bibliographic primitive that carries this principle across tiers is the 26-character `fact_cid`,

$$\texttt{fact\_cid} = \mathrm{base32\_nopad\_lower}\big(\,\mathrm{blake3}(\mathrm{canonical\_cbor}(\texttt{fact}))[0..16]\,\big),$$

where the canonical CBOR encoding is RFC 8949 deterministic [16] with four domain tags, and the 128-bit truncation gives a birthday-collision bound near $2^{64}$ against a corpus presently at order $10^5$ facts. Because the name is the BLAKE3 fingerprint of the bytes, changing one byte changes the name, and the name therefore proves the bytes. This single handle is what bridges an agent's internal memory tiers and the shared external memory: a runtime's short-term context window (tier 1) quotes the CID verbatim, its in-process long-term store (tier 2) caches it, and the planet-keyed shared layer (tier 3) dereferences it — the underlying CBOR never needs paraphrasing or recompression to cross a tier boundary [7], [8], [10]. Two agents on different runtimes, in different processes, with no shared state, paste the same CID and pull the same bytes. Composite citations extend the same algebra: `memt:<cell64>:<fact_cid>` names one fact at one place, and `memb:<bundle_cid>` names a signed envelope over $N$ facts at $N$ places. The properties the in-agent memory layers structurally cannot hold — byte-equality reproducibility rather than similarity-up-to-recall, offline verifiability against an issuer pubkey, and verbatim citation to a user, a regulator, a competing agent, or the agent's future self — follow directly from making the address content-derived and the receipt signed [5], [6], [9], [11], [12], [41].

![cell + band + tslot → canonical CBOR → blake3 → 26-character base32 CID](/docs/diagrams/09-address-algebra.svg)
*Figure 10.1: address algebra. Three integers become one 26-character handle that the rest of the protocol, and any agent reasoning chain on top of it, cites.*

### 10.2 Forward look

The deliberate end-state is federation. Every fact is content-addressed and signed, so any responder serves it and any client verifies it offline without trusting the source. The single hosted responder plus self-hosted nodes that ship today are the substrate for a network of independent responders that resolve the same content ids byte-for-byte, cross-cite each other's attestations, and record where they disagree. The trust model is *trust the signature, not the server*: a receipt minted by responder $A$ in one year verifies against the same pubkey on a self-hosted replica $B$ that never spoke to $A$, and a content id means the same bytes everywhere. Multi-host routing does not ship in the current release; what ships is the machinery that makes it possible — content addressing, signed receipts, typed temporal edges (`supersedes`, `disagrees_with`, `relates_to`), the multi-attester contradiction index scored per band kind, and a deterministic refinement loop that re-derives a fact when a newer attestation or a `disagrees_with` edge lands.

![many responders, one address space; each signs under its own key, all resolve the same id, disagreement is recorded](/docs/diagrams/08-decentralised.svg)
*Figure 10.2: the federation end-state. One address space, many independent signers, byte-identical resolution, recorded disagreement; the client trusts the signature instead of the server.*

Four further directions extend the substrate without disturbing the address rules:

| Direction | Present state | Target |
|-----------|---------------|--------|
| Trained JEPA v2 dynamics | residual-zero identity baseline; receipt carries `untrained_baseline` | learned transition $\hat{v}_{t+1} = v_t + \Delta(v_{t-2}, v_{t-1}, v_t)$, gated on multi-vintage Tessera [1] |
| Weighted multi-encoder consensus | equal-weight $\ell_2$ vote across Clay, Prithvi, Tessera [2], [3], [4] | receptive-field-aware weighting via the `WeightedBlend` AST node |
| H3 hex migration | square `cell64`, non-square poleward | equal-area H3 DGGS at resolution 13 ($\approx 3.4\,\mathrm{m}$) [20], [21] |
| Encoder / modality coverage | Galileo S2-only; S1/DEM/climate zero-masked [4] | wired connectors and chip fetchers per modality |

The JEPA v2 head exists, signs honestly, and short-circuits to the last attested vintage until the dynamics predictor is trained; its training is bottlenecked on upstream Tessera publishing three or more annual vintages per cell [1], not on the model. The triple-encoder consensus today computes

$$\textrm{ensemble} = \sqrt{\tfrac{1}{3}\big(d_c^2 + d_p^2 + d_t^2\big)}, \qquad d_\bullet = \mathrm{clamp}\big(1 - \cos(v_\bullet^{\,\mathrm{now}}, v_\bullet^{\,\mathrm{prior}}),\, 0,\, 1\big),$$

an equal-weight aggregation across Clay's $\approx 2.56\,\mathrm{km}$, Prithvi's $\approx 6.7\,\mathrm{km}$, and Tessera's per-pixel receptive fields whose independence is the entire point of the vote: agreement is land-surface change, a lone firing encoder is receptive-field aliasing [37]. The weighted generalisation replaces the uniform $\tfrac{1}{3}$ with receptive-field-aware coefficients expressed in the existing `WeightedBlend` AST node, retuned at registry-CID time rather than by recompilation. The H3 migration changes the spatial primitive to an equal-area hexagonal hierarchy while leaving the CID, receipt, and capability rules untouched, requiring only a new band-ontology manifest CID and an in-flight-fact reconciliation story [20]. Broader coverage means wiring the remaining Galileo modalities (S1, DEM, ERA5, and the rest) and the declared-but-unwired source schemes, each of which answers with a typed Absence until its connector and chip fetcher land.

### 10.3 Design stance

emem is a protocol, not a service. The hosted node at `https://emem.dev` is one conforming responder among many possible ones; the same binary self-hosts with a single container invocation, and the contract is the address rules and the signing rules, not the deployment. The default build runs entirely on open data — Copernicus DEM [30], JRC Global Surface Water [31], Hansen Global Forest Change [32], ESA WorldCover [33], SoilGrids [34], Overture divisions [39], GeoNames [40], Fields of The World [35], Köppen-Geiger climate seeds [36], and Tessera [1] — with no API keys, no operator credentials, and no SaaS lock-in. Honesty is a first-class feature rather than an afterthought: a missing measurement is a signed, content-addressed `Absence` carrying a typed reason an agent can read and cache, an empty result distinguishes a wrong query from an empty place, and the catalog never promises more than it can sign. The unsigned outside-channel memory that current agent practice relies on is rewritable by whoever administers it, a class of state against which query-only poisoning attacks succeed at high rates [41]; emem's response is that every byte an agent reads or writes carries a signature checkable offline, so the memory becomes more trustworthy the more agents read and write against it.

## References

[1] Tessera Project. "Tessera: a global per-pixel annual Earth-observation embedding (geotessera)." Cambridge / geotessera, 128-D annual embedding stack 2017-2024, published as Cloud-Optimized GeoTIFF tiles. https://github.com/ucam-eo/geotessera

[2] Clay Foundation. "Clay v1.5: an open foundation model for Earth." Wavelength-conditioned ViT-L/8 MAE with DINOv2 teacher, 1024-D, Apache-2.0. https://github.com/Clay-foundation/model

[3] Jakubik, J., et al. "Prithvi-EO-2.0: a versatile multi-temporal geospatial foundation model for Earth observation applications." IBM/NASA, Prithvi-EO-2.0-300M-TL, 2024. arXiv:2412.02732. https://arxiv.org/abs/2412.02732

[4] Tseng, G., et al. "Galileo: learning global and local features in pretrained remote sensing models." NASA Harvest, multimodal (S1/S2/DEM/climate) geospatial foundation model, 2025. arXiv:2502.09356. https://arxiv.org/abs/2502.09356

[5] eMEM: A Hybrid Spatio-Temporal Memory System for Embodied Agents (hippocampal/neocortical CLS memory). arXiv:2606.03374, 2026. https://arxiv.org/abs/2606.03374

[6] Rasmussen, P., et al. "Zep: a temporal knowledge graph architecture for agent memory (Graphiti)." arXiv:2501.13956, 2025. https://arxiv.org/abs/2501.13956

[7] Sumers, T., Yao, S., Narasimhan, K., Griffiths, T. "Cognitive Architectures for Language Agents (CoALA)." Transactions on Machine Learning Research, 2024. arXiv:2309.02427. https://arxiv.org/abs/2309.02427

[8] Packer, C., et al. "MemGPT: Towards LLMs as Operating Systems." arXiv:2310.08560, 2023. https://arxiv.org/abs/2310.08560

[9] MIRIX: Multi-Agent Memory System for LLM-Based Agents (six memory types). arXiv:2507.07957, 2025. https://arxiv.org/abs/2507.07957

[10] Mem0: Building Production-Ready AI Agents with Scalable Long-Term Memory. arXiv:2504.19413, 2025. https://arxiv.org/abs/2504.19413

[11] Hu, Y., et al. "Evaluating Memory in LLM Agents via Incremental Multi-Turn Interactions (MemoryAgentBench)." arXiv:2507.05257, 2025. https://arxiv.org/abs/2507.05257

[12] Lewis, P., et al. "Retrieval-Augmented Generation for Knowledge-Intensive NLP Tasks." NeurIPS, 2020. arXiv:2005.11401. https://arxiv.org/abs/2005.11401

[13] Lei, J., Zhang, D., Li, J., et al. "delta-mem: Efficient Online Memory for Large Language Models." arXiv:2605.12357, 2026. (Compact in-attention associative memory.)

[14] Laurie, B., Langley, A., Kasper, E. "Certificate Transparency." RFC 6962, IETF, 2013. (Merkle leaf/node domain separation.) https://www.rfc-editor.org/rfc/rfc6962

[15] Josefsson, S., Liusvaara, I. "Edwards-Curve Digital Signature Algorithm (EdDSA)." RFC 8032, IETF, 2017. (Ed25519.) https://www.rfc-editor.org/rfc/rfc8032

[16] Bormann, C., Hoffman, P. "Concise Binary Object Representation (CBOR)." RFC 8949, IETF, 2020. (Deterministic encoding, Section 4.2.) https://www.rfc-editor.org/rfc/rfc8949

[17] O'Connor, J., Aumasson, J.-P., Neves, S., Wilcox-O'Hearn, Z. "BLAKE3: one function, fast everywhere." 2020. https://github.com/BLAKE3-team/BLAKE3-specs

[18] Josefsson, S. "The Base16, Base32, and Base64 Data Encodings." RFC 4648, IETF, 2006. (base32-nopad.) https://www.rfc-editor.org/rfc/rfc4648

[19] Bormann, C. "Concise Binary Object Representation (CBOR) Tags for Object Identifiers / IPLD multibase 'b'." RFC 9090 / IPLD CID. (Tag 42, base32 multibase.) https://www.rfc-editor.org/rfc/rfc9090

[20] Uber Technologies. "H3: A Hexagonal Hierarchical Geospatial Indexing System." (Migration target DGGS, resolution 13.) https://h3geo.org

[21] Google. "S2 Geometry Library: spherical cell hierarchy for geographic indexing." https://s2geometry.io

[22] Anthropic. "Memory tool (context-management-2025-06-27): file-op verbs for agent memory." Claude developer documentation, 2025. https://docs.anthropic.com

[23] Anthropic. "Model Context Protocol (MCP): Streamable HTTP, JSON-RPC 2.0, specification revision 2025-11-25." https://modelcontextprotocol.io

[24] Google DeepMind. "Gemma 4: open models (gemma-4-12b-it)." Model release, 2026. (Explain sidecar.) https://ai.google.dev/gemma

[25] Xiao, S., et al. "C-Pack / BAAI bge-base-en-v1.5: 768-D English text embeddings." 2023. arXiv:2309.07597. (memory_search and topic routing.) https://arxiv.org/abs/2309.07597

[26] LanceDB. "Lance columnar format and IVF_PQ approximate-nearest-neighbor index." https://lancedb.github.io/lance

[27] Zandieh, A., et al. "TurboQuant: Online Vector Quantization with Near-optimal Distortion Rate." Google Research, 2025. arXiv:2504.19874. (Randomized sign-flip rotation for binary / scalar quantization without a codebook.) https://arxiv.org/abs/2504.19874

[28] Drusch, M., et al. "Sentinel-2: ESA's optical high-resolution mission for GMES operational services." Remote Sensing of Environment 120:25-36, 2012. (10 m optical; Copernicus Open / MPC.)

[29] Justice, C.O., et al. "The Moderate Resolution Imaging Spectroradiometer (MODIS): land remote sensing for global change research." IEEE TGRS 36(4):1228-1249, 1998. (MOD11/MOD13/MCD64A1 connectors.)

[30] European Space Agency / Airbus. "Copernicus DEM (GLO-30): 30 m global digital elevation model." ESA, 2021. https://doi.org/10.5270/ESA-c5d3d65

[31] Pekel, J.-F., Cottam, A., Gorelick, N., Belward, A.S. "High-resolution mapping of global surface water and its long-term changes." Nature 540:418-422, 2016. (JRC Global Surface Water.) https://doi.org/10.1038/nature20584

[32] Hansen, M.C., et al. "High-resolution global maps of 21st-century forest cover change." Science 342:850-853, 2013. (Hansen GFC v1.12 loss-year.) https://doi.org/10.1126/science.1244693

[33] Zanaga, D., et al. "ESA WorldCover 10 m 2021 v200." ESA / CC-BY-4.0, 2022. https://doi.org/10.5281/zenodo.7254221

[34] Poggio, L., et al. "SoilGrids 2.0: producing soil information for the globe with quantified spatial uncertainty." SOIL 7:217-240, 2021. https://doi.org/10.5194/soil-7-217-2021

[35] Kerner, H., et al. "Fields of The World (FTW): a global benchmark and product of ~3.17 billion agricultural field boundaries, 241 countries, 10 m, CC-BY-4.0." source.coop PMTiles archive, 2024. https://fieldsofthe.world

[36] Beck, H.E., et al. "Present and future Koppen-Geiger climate classification maps at 1-km resolution." Scientific Data 5:180214, 2018. (climate_archetype seed centroids.) https://doi.org/10.1038/sdata.2018.214

[37] Healey, S.P., et al. "Mapping forest change using stacked generalization: an ensemble approach." Remote Sensing of Environment 204:717-728, 2018. (Triple-consensus threshold provenance.) https://doi.org/10.1016/j.rse.2017.09.029

[38] European Union. "Regulation (EU) 2023/1115 on deforestation-free supply chains (EUDR)." Official Journal L150, 2023; application deferred via Reg. 2024/3234 and 2025/2650. https://eur-lex.europa.eu/eli/reg/2023/1115

[39] Overture Maps Foundation. "Places, buildings, transportation, and divisions themes (divisions/division_area)." ODbL / CDLA-Permissive, 2024. https://overturemaps.org

[40] GeoNames. "cities-5000: 68,581 populated places with population >= 5,000, CC-BY-4.0." https://www.geonames.org

[41] MINJA: Memory Injection Attack on LLM agents demonstrating ~95% poisoning success through query-only interaction. NeurIPS, 2025. (Motivating threat model for unsigned outside-channel memory.)

[42] Oke, T.R. "Boundary Layer Climates," 2nd ed. Methuen, 1987. (Urban thermal diffusivity for /v1/heat_solve FTCS solver.)

[43] Snyder, J.P. "Map Projections: A Working Manual." USGS Professional Paper 1395, 1987. (UTM in emem-fetch::proj.)
