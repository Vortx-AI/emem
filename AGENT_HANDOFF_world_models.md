# Handoff: what emem needs to be flawless for agents building world models

**From:** the agent in `/home/ubuntu/navigatable_worlds` (built the 4D Gaussian-splat worlds at
emem.dev/splats from emem's signed facts), 2026-07-14.
**To:** the agent working in `/home/ubuntu/emem`.
**Status:** proposal + evidence. I have changed **nothing** in this repo outside
`crates/emem-api-rest/src/lib.rs`'s `/splats` router (Range/CSP/mime — separate work, already
discussed with the owner). Everything below is yours to accept, reject, or reshape.

Every claim here is cited to `file:line` or to a measurement I actually ran. Where I could not
determine something I say so. **Please re-derive anything before acting on it** — I am reporting
from outside your codebase.

---

## The one-sentence thesis

> **emem computes rich intermediate state, then collapses it at the last mile. For a world model,
> the intermediate IS the product.**

Three independent instances below, all with the same shape: the hard part is already done and
correct; the value is destroyed one function before the API boundary. This is why I ended up
bypassing emem for the single most important input to our farm world — not because emem was wrong,
but because the thing I needed existed inside it and had no way out.

---

## P0 — There is no native-resolution raster for an AOI. The primitive already exists.

**What I hit.** NDVI needs B08. `build_cell_scene_rgb` (`lib.rs:37641`) reads only B04/B03/B02
(`lib.rs:37675-37692`) and returns a **256×256 8-bit** PNG/RGB plane after a 2–98th percentile
stretch + gamma 1/2.2 (`lib.rs:37746-37789`) — non-invertible for science. Every other route returns
per-cell scalars. Our 2 km AOI holds **40,040 cells at 10 m**; `recall_polygon`'s `max_cells` hard
limit is **1024** (`lib.rs:11097-11109`). So per-cell recall cannot express the query at all.

I therefore wrote `fetch_ndvi_raster.py` against **your own upstream** — STAC + COG windowed reads —
and got a clean 200×200 @ 10.00 m/px. emem was cut out of its own pipeline.

**The primitive is already there.** `cog::sample_window` (`crates/emem-fetch/src/cog.rs:1360`)
returns a native-res row-major `Vec<f64>`. It has 5 call sites and **every one destroys it**:
- `lib.rs:37712/37715/37726` → crushed to 8-bit RGB.
- `lib.rs:41630` → `let _ = …`, cache warming only.
- `lib.rs:41771` → collapsed to one scalar fact per cell.
- `clay_chip.rs:143`, `prithvi_chip.rs:151`, `galileo_chip.rs:150` → resampled and POSTed to the GPU
  sidecar; only an embedding vector returns (`lib.rs:25936-25942`).

**Proposal.** `POST /v1/band_raster { bbox|polygon, band, tslot|datetime, max_px }` → the f64 array
(or f32/CBOR) + CRS/transform/nodata + the same signed receipt a fact gets. Cap by pixel count, not
cell count. This is plumbing, not new science — `sample_window` already does the work, and
`build_cell_scene_rgb` already proves the STAC→COG→window path.

**Why it matters more than it looks.** A world model is a *field*, not a set of points. Every
agent that needs a field currently either (a) reimplements your COG stack against your upstream, or
(b) silently accepts a 125 m grid and calls it 10 m. Both are worse for emem than a raster endpoint.

---

## P1 — Your NDVI is correct, but for a reason you don't record and cannot check

**I tried to report this as a bug and the measurement stopped me.** Please read this one carefully;
the conclusion is the opposite of where I started.

The math (`lib.rs:29782-29789`) applies **no** BOA offset:
```rust
"index_ndvi" => {
    let nir = samples[0] * 1e-4;
    let red = samples[1] * 1e-4;
```
Sentinel-2 L2A baseline ≥ 04.00 (2022-01-25+) encodes reflectance with a **−1000 DN offset**. NDVI is
scale-invariant but **not** offset-invariant. Repo-wide there are **zero** hits for
`BOA_ADD_OFFSET` / `processing_baseline` / `add_offset`(S2), and `StacItem`
(`crates/emem-fetch/src/stac.rs:31-44`) has no field for the baseline — so nothing downstream
*could* branch on it.

**But you are right anyway, because Element84 ships harmonised DNs.** I measured it — same pixel
(32.5628, 76.9569), same scene dates, Element84 vs Microsoft Planetary Computer:

| date | baseline | MPC B04 | E84 B04 | diff | emem's math on E84 | MPC offset-corrected |
|---|---|---|---|---|---|---|
| 2024-05-18 | 05.10 | 2562 | 1556 | **+1006** | +0.1925 | **+0.1926** |
| 2024-05-23 | 05.10 | 2463 | 1459 | **+1004** | +0.2274 | **+0.2280** |
| 2024-06-02 | 05.10 | 2084 | 1084 | **+1000** | +0.3333 | **+0.3348** |
| *control* 2021-06-11 | 02.12 | — | — | **+113 ≈ 0** | — | — |

Post-baseline: E84 = MPC − 1000. Pre-baseline control: diff ≈ 0. Your raw `×1e-4` on Element84
reproduces offset-corrected MPC NDVI to ~4 dp. **The offset is a property of the UPSTREAM, not of
"Sentinel-2".**

**The risk is that this invariant is invisible and unenforced:**
1. `crates/emem-core/data/sources-v0.json:469-471` **declares an MPC `sentinel-2-l2a` provider.** I
   could not find a code path that routes S2 through it (all live routing is `STAC_ELEMENT84_V1`,
   `stac.rs:21`, used at `lib.rs:29620-29629`) — but the day someone wires that failover, **every
   NDVI silently biases toward zero**, worst over dark/wet targets, and no test fails.
2. The bucket isn't pinned — you follow whatever Element84 puts in `assets[].href`. If they
   re-collection to raw DNs, you follow silently.
3. The docstrings at `lib.rs:4687` / `:29225` state "reflectance scale = 1e-4" as a universal fact.
   It's true *for Element84*. That sentence is the trap.

**Proposal (cheap, high leverage):**
- Parse `s2:processing_baseline` into `StacItem` — you can't assert what you don't carry.
- One assertion at the offset decision point, keyed on **provider**, not on the band.
- A regression test pinning "E84 post-baseline B04 ≈ MPC B04 − 1000" so a provider swap fails loudly.
- Reword the docstring: *"×1e-4 with no offset — correct **because Element84 harmonises**; MPC serves
  raw ESA DNs and would need −1000 for baseline ≥ 04.00."*

*(Full disclosure: my own pipeline had this exact bug the other way round — MPC without the offset,
bias −0.095. **emem's signed facts are what caught it.** Cross-checking against emem worked exactly
as designed; that's the strongest endorsement of the fact model I can offer. I'd like the invariant
pinned so it keeps working.)*

---

## P2 — You read the pixel-level SCL, sign it, then tell agents to use a scene-level proxy

`s2_pick_clear_scene` does a real per-pixel SCL probe before paying for the value read
(`lib.rs:29505-29527`) — genuinely good, and better than the scene-level filter most tools use.
The class is even signed into the fact's provenance args (`lib.rs:30103-30116`, position 9), with a
thoughtful comment about letting a verifier audit the confidence derivation.

**But no API response surfaces it.** I called `/v1/ndvi` at our farm just now. I got
`confidence: 0.95`, and `band_metadata.pitfalls` told me:

> *"Cloud / shadow flips signs misleadingly — always co-check `weather.cloud_cover` and the scene's
> recorded `cloud_pct` in the receipt."*

You are advising a **scene-level** proxy while holding the **pixel-level** truth. That's the thesis
again.

**It matters concretely.** The reject set is `{0,1,8,9,10}` (`lib.rs:29483-29485`), and confidence
(`lib.rs:30053-30068`):
```rust
Some(4) | Some(5) | Some(6) | Some(11) => 0.95, // veg / soil / water / snow — clear
Some(2) | Some(3) => 0.65,                      // cast shadows / cloud shadows
```
**Snow (11) is signed at 0.95 "clear."** Radiometrically fair — it *is* a clear pixel. But
`indices.ndvi` is a *vegetation* index. Our farm is Pattan Valley, Lahaul: snow **Nov–Apr**. An agent
asking "NDVI in January" gets `0.95` confidence on a number that measures snow. It cannot tell,
because the class it would need is buried at `args[9]`.

I hit the general form of this hard enough to build three statistical gates against it — my farm
verdict confidently called **bare rock (NDVI −0.055) "prospered"** until I added a vegetation gate.
Every downstream agent will rebuild that gate, badly, from the value alone.

**Proposal — surface, don't reject.** Rejecting loses information; the consumer should decide.
Add a first-class, documented field on the fact/response:
```json
"surface_class": { "scl": 11, "label": "snow_ice", "vegetation_valid": false }
```
Then `confidence` can keep meaning *"is this radiometrically clean"* (which is what it correctly
means today) and `vegetation_valid` carries *"is NDVI meaningful here"* — two different questions
currently collapsed into one number. Update `pitfalls` to point at the pixel-level class you already
have, not at `weather.cloud_cover`.

Also worth a look: `s2_search_with_fallback` (`lib.rs:29373`), used by the Clay/Prithvi/Galileo chip
paths, does **no** SCL check at all — scene-level cloud only. So the foundation-model embeddings are
built on a weaker gate than the scalar facts. I did not verify the downstream impact.

---

## P3 — The S2 path can't meet the 40 s gateway timeout, and this is why cold NDVI polygons 504

I spent real time fighting 504s and built checkpoint/resume around them. It isn't load — it's
structural:

- Gateway timeout: **40 s** (`lib.rs:1424-1441`, applied `:1279-1282`).
- `materializer_timeout_secs()` = **14 s** default (`lib.rs:1542-1548`) — the intended guard — is
  applied at `:25259`, `:26258`, `:27862`, `:28055`, `:41768` but **NOT to the S2 path**:
  `lib.rs:36005` is a bare `await` (same at `:35341`, `:41111`).
- So a cold `indices.ndvi` is bounded only by the reqwest client timeout: **90 s per request**
  (`lib.rs:34051-34057`) — already > the 40 s gateway.
- Worst case ≈ **31 sequential round trips** for ONE cell: up to 3 STAC searches
  (`:29593-29634`) + up to 12 SCL probes × ~2 range reads, strictly sequential (`:29641-29643`)
  + 2 assets × 2 reads (`:29742-29759`).
- `recall_polygon` then fans 64 cells at concurrency 16 (`:11199-11200`) = 4 waves of that.
- `prewarm_polygon_static_cog_bands` explicitly no-ops for STAC-driven bands (`:41590-41592`), so
  NDVI gets no prewarm.

**A cold 64-cell NDVI polygon cannot finish inside 40 s.** It is not a tuning problem.

**Proposal:** apply `materializer_timeout_secs` to the S2 arm; parallelise the SCL candidate probe
(it's an independent read per candidate); consider a partial-result response (`ready` + `pending`)
so a 504 becomes a resumable answer instead of a total loss. An agent that gets 40 of 64 cells plus
"these 24 are materializing" can proceed; a 504 teaches it to stop using emem.

---

## P4 — the agent-to-agent memory channel refused me, and I'm your ideal user

I tried to leave this note through emem's own `memory_create` — the channel your MCP instructions
advertise: *"Write durable agent notes with the memory_* file verbs and cite them the same way."*

```
tool error (-24): unattested `create` is refused by this responder's memory-write policy;
supply an `attester: {pubkey_b32, sig_b32}` block (see /memories/by_attester/<pubkey8>/...),
or the operator can relax the policy.
```

(`EMEM_MEMORY_REQUIRE_ATTESTER=1` in the emem-server unit.) Requiring attestation is a *good*
default — I'm not asking you to drop it. The problem is that an agent cannot satisfy it from the
tool contract alone. `sig` is specified as signing
`blake3("emem.memory_write|create|path|body_hash")`, which leaves open:

- is `body_hash` the raw blake3 digest or its hex/base32 text?
- do I sign the digest **bytes** or that string?
- which base32 alphabet/padding is `_b32`? (RFC4648 lowercase, unpadded?)
- must `pubkey_b32` be pre-registered, or is any keypair accepted?
- the path must be under `/memories/by_attester/<pubkey8>/…` — `<pubkey8>` being which 8 chars of
  which encoding?

I stopped rather than brute-force guesses against a production memory store. So: **the durable
agent-memory feature is, in practice, unreachable for a fresh agent** — which is precisely the
"tough to get value out of emem" the owner is asking about. This note lives in a plain repo file
instead, which is exactly the outcome the feature exists to prevent.

**Proposal (any one of these fixes it):**
- A worked example in the tool description: 6 lines of Python producing a valid `attester` block.
- An `emem_memory_attest_helper` verb, or accept a first-write trust-on-first-use pubkey.
- Make the error *actionable*: return the exact preimage string the responder expects, so a caller
  can sign it without reverse-engineering. Right now it names the shape but not the bytes.
- Ship a signing helper in the MCP client alongside the verb.

Cheapest real test: hand a fresh agent the tool list and ask it to save a note. Watch where it stops.

---

---

# Part 2 — make the TOKEN the unit of processing

Everything above is about single calls. This part is about the shape of the whole pipeline, and it
is the more important half. Written after the owner asked why building a world on emem is slow,
manual, and hard to keep honest.

## The diagnosis, measured on our own build

```
real/cellfacts_farm.json        fact_cid x64     <- we DO harvest emem tokens
world_farm/manifest.web.json    fact_cid x0      <- the SHIPPED world carries NONE
                                receipt  x0         responder_pubkey x0
```

**We collect 64 signed fact_cids during the build and throw every one away.** The published world
carries our own `root_cid` + ed25519 signature and not a single emem token. So a world whose entire
pitch is *"don't trust me, resolve the token"* ships as *"trust me."* The provenance chain breaks
exactly at the manifest boundary — and it breaks because **there is no token that can name what a
world is made of.** A world is a FIELD over an AOI across time. `emem:fact:<cell64>:<fact_cid>` names
one scalar at one cell at one tslot. You cannot build a 200×200×26 cube out of 1.04 M of those.

So the token DB does not exist yet, and it is not for lack of trying: **the type is missing.**

## The wishes

### W1 — a token that names a FIELD (the keystone; everything else hangs off it)
```
emem:raster:<aoi_cid>:<band>:<tslot>          one array
emem:cube:<aoi_cid>:<band>:<tslot_range>      a time series of arrays
```
Resolves to bytes + CRS/transform/nodata + the signed receipt. `aoi_cid` content-addresses the
geometry so the same AOI always yields the same token. This is P0 with a name: without it a world
model cannot be token-anchored *at all*, and every agent falls back to URLs and "trust me".

### W2 — let agents REGISTER derivations (this is what makes it a DAG, not a list)
You already have the concept: facts carry `derivation_fn_key` (`sentinel2_l2a_indices_ndvi@1`).
It's just closed. Open it:
```
POST /v1/derive { fn: "same_doy_ndvi_delta@1", inputs: [<token>...], bytes|value, code_cid }
  -> emem:derived:<cid>   (signed; lineage resolves transitively to YOUR primary facts)
```
Then `zones.json`, `plots.json`, the verdict raster, and the splat field itself each become a token
whose ancestry terminates in emem-signed measurements. A verifier walks the DAG instead of taking
our word. **This is the single change that would turn our manifest from a claim into a proof.**
Our worlds already bake a per-splat provenance channel + `layer_index` bands
(measured|interpolated|synthesized) — that is a lineage graph with no token to point at.

### W3 — the token DB: batch resolve + content-addressed cache
- `POST /v1/resolve_many { tokens: [...] }` — one round trip, not N. Rebuilds touch thousands.
- Tokens are content-addressed, so **cache-forever is safe by construction**. Say so, and give the
  bytes an immutable `cache-control` + strong ETag. Then a client-side token DB is trivial, rebuilds
  are offline and instant, and emem gets cited *more* because citing costs nothing.
- Today a rebuild re-fetches everything because nothing is addressable as a stable artifact. That is
  the whole "why is this slow" answer.

### W4 — live: subscribe to a token, don't poll it
Our `isr_watch` polls on a stage-aware cadence (5–30 days) because there is no other option, so we
are always somewhere between wasteful and late.
```
emem:latest:<aoi_cid>:<band>  -> resolves to the current token + an SSE/webhook when it CHANGES
```
A world that subscribes rebuilds within minutes of a new scene landing and does nothing in between.
That is "live" for a fraction of the current cost.

### W5 — same-DOY as a FIRST-CLASS temporal op (algorithmic correctness)
`emem_diff` compares two arbitrary tslots with **no phenology guard**. I tried
`diff(19723, 20634)` — 911 days apart. If those land on different days-of-year the delta is
meaningless for any seasonal band, and *nothing in the API says so*.

This is not hypothetical: our farm confidently reported **"4 prospered / 0 stressed"** purely because
the current season's latest frame was DOY 176 while the baseline year had no frame past DOY 135 — so
every year was compared at the wrong phenological stage and every field looked improved. The fix was
per-year interpolation to the SAME day-of-year, excluding years that cannot be bracketed.

**Every agri agent will hit this and most will ship the bug.** Ask:
`POST /v1/compare_same_doy { cell|aoi, band, doy, years[] }` → per-year values interpolated at the
same DOY, with unbracketable years EXCLUDED and said so. Make the biased comparison the one you have
to ask for.

### W6 — retries you can actually write code against
Three different failure shapes, three different correct responses, no way to tell them apart:
- `recall` **auto-materializes** on a miss. `diff` does **not** — it returns
  `CidNotFound: no fact at tslot_a=19723`, and never mentions that `/v1/backfill` is the fix. Same
  conceptual op ("get me the value"), opposite semantics, no signpost.
- A cold polygon 504s (P3) and the whole request is lost — no partial, no resume.
Ask: every error carries `retryable: bool`, `retry_after_s`, and `remedy` (the exact call that would
fix it — `/v1/backfill {...}`). Long jobs return a **resume token**: `{ready: [...], pending: <token>}`.
An agent that can resume will retry correctly; an agent facing an opaque 504 just gives up on emem.

### W7 — answer the question I asked, not the topic I mentioned
I asked `intent{ask}`: *"which tool gives me the 10 m NDVI **array** — not per-cell scalars?"*
I got: *"At the requested cell: greenness (NDVI) 0.89"* — plus 7 algorithms I never asked for and
~8 KB of JSON. The router matched topics (`optical_raw_reflectance` 0.67, `scene_classification`
0.67) and answered the **topic**. There is no meta surface: every road ends at a scalar at a cell.

Also broken: `intent{did_change}` builds a plan and **cannot call its own primitive** —
```
"args": {"tslot_a": "19723", "tslot_b": "20634"}    <- planner stringified them
"error": "invalid type: string \"19723\", expected u64"
```
I passed integers. The planner is the tool that exists to solve "what do I call?", and it fails on
its own generated args. **Fix that first — it is a type coercion, and right now the discovery story
is a broken planner in front of a router that answers a different question.**

Ask, in order: (a) fix the planner's arg types + add a regression test over every intent variant;
(b) let `ask` return a **plan** (`"you want /v1/band_raster; here is the exact call"`) when the
question is about capability rather than a place; (c) index routes by the SHAPE of the answer —
scalar | raster | timeseries | plan — because "what tool do I use" is nearly always a question about
shape, and topic-matching cannot answer it.

### W8 — samples, demos, comparisons (the thing that makes all the above usable)
- **One worked example per verb**, in the tool description, with real values. `memory_create`'s
  attester block (P4) is the sharp case: the shape is documented, the bytes are not, so the feature
  is unreachable.
- **One end-to-end demo that builds something**: AOI → cube token → derived token → verify. If that
  demo existed I would have found `sample_window` on day one instead of reimplementing your COG
  stack.
- **A golden set**: a fixed AOI + date with published expected values, so an agent can prove its
  pipeline agrees with emem before trusting it. I did this by hand and it caught a real bug in my
  code (P1). It should not have taken a bespoke script — it should be one call.

## Why this is worth it for emem, not just for us

Every gap above pushed us **away** from emem and toward its upstream: no raster token → we read the
COGs ourselves; no derivation registry → we self-sign; no subscribe → we poll; no batch resolve → we
cache nothing. We are your most committed user and our shipped artifact cites **zero** emem tokens.
That is the whole story in one number.

Fix W1–W3 and the next set of worlds are token-anchored end to end: every splat traceable to a
signed measurement, every derived layer a resolvable claim, rebuilds instant and offline, and the
provenance story becomes checkable by a stranger instead of asserted by us.

---

# Part 3 — the game changers (written after USING what you just shipped)

Addendum, same author, a few hours later. You moved fast: `/v1/derive`, discovery-by-shape, and the
bounded S2 path all landed while I was still writing Part 1. So I stopped theorising and used them.
Everything below is grounded in a real call, not a wish.

## First, what you nailed — because it matters that you hear it

I minted **the first emem token our pipeline has ever produced**:
```
emem:fact:defi.zb572.towe.zae65:52skvxb3welhimddit4m2snbonrudgahohyuzqunanq7bq2q252q
```
A same-DOY NDVI delta whose three parents are real signed Sentinel-2 facts — the exact fix for the
"we ship zero tokens" finding in Part 2, done live against your `/v1/derive`. Two things you got
exactly right:
- **`derive` closes W2.** Lineage terminates in your signed measurements. Our manifest can now be a
  DAG instead of a "trust me."
- **The 401 `how_to_sign` closes P4 and W8 in one stroke.** `sign_this.digest_hex` + a copy-paste
  `worked_example` + the full CBOR byte-rules — the channel that *refused me last session* is now
  self-teaching. I signed and submitted with zero reverse-engineering. This is the single best
  agent-DX improvement in the codebase; do it to every attested verb.

Now the three things that would turn "correct" into "nobody else can do this".

## GC-1 — make `code_cid` RUNNABLE. Reproducible > signed. This is the whole moat.

Today `code_cid` is "recorded, never fetched or run", `provenance_class` is forced to
`model_output`, and the response says plainly: *"NOT that the value is true."* So my same-DOY delta
is a signed **assertion**. Anyone must trust that I computed 0.02198 correctly.

But emem's entire differentiator is the `deterministic_index` class: **`recomputable_from_source`**.
Derivations break that line — and they don't have to.

**The wish:** a sandbox that fetches `code_cid`, re-runs it against the parent tokens, and checks the
value reproduces. A derivation that reproduces **earns `deterministic_index`** — "I re-ran your code
on the cited inputs and got your number." Then:

> Our whole world model becomes independently recomputable from raw Sentinel-2 bytes by a stranger.
> Not "here is my splat, trust me" — "here is my splat, and the signed, re-runnable derivation chain
> from raw satellite pixels to this exact Gaussian."

World Labs and Niantic ship opaque splats. **Neither can offer re-derivability.** This is the feature
that makes "provenance-native world models" a real category and not a slogan — and you are one
sandbox away from it, because you already content-address the code and the inputs. Start with a
pure-function tier (no network, deterministic, wasm or a pinned Python) — same-DOY delta, zone
k-means, the verdict raster all qualify.

## GC-2 — derivations are invisible by place. That is the drift emem exists to stop.

I registered a farm verdict. Then I recalled that exact cell. **My derivation is not there** —
confirmed: `recall` returns the 5 primary facts, not my delta ("absent from recall / recall_polygon /
query_region — reachable only by token"). So the next agent standing at that farm **cannot find my
work and will recompute it** — probably with the phenology bug I just corrected around.

That is textbook referential drift: two agents, same place, re-deriving instead of co-referring —
the precise thing your front page says emem prevents. Derivations currently can't participate in it.

**Two parts:**
- **(a) Opt-in place-indexing for derivations.** Let a caller flag a derivation as
  place-discoverable so `recall`/`recall_polygon` at that cell surface it (clearly typed as a
  derivative, attester-attributed, not masquerading as a primary). Co-located agents then build on
  each other instead of past each other.
- **(b) Reverse lineage — the invalidation index.** "What is downstream of this parent `fact_cid`?"
  When Element84 reprocesses a scene, or a baseline shift lands (the BOA case from P1 — the one
  failure mode that silently corrupts everything), you can answer *which derived facts and which
  worlds are now stale* and propagate it. A memory that claims to stop drift must know what to
  invalidate when a source moves. The forward edge is free (parents are in the token); index the
  reverse edge and staleness propagates itself.

## GC-3 — be the substrate WE are collaborating on. It's no longer aspirational.

Last session emem's memory channel refused my note and we ended up collaborating through a markdown
file on a shared disk — the exact fallback the product exists to eliminate. **This session I minted a
real token on the shared substrate.** The gap between those two facts is the whole opportunity.

The flagship demo of "shared verifiable memory for agents" is sitting right here: two agents, same
building, building next-gen geospatial infra together. Make the handoff itself flow as tokens — I
hand you an `emem:bundle:` of findings + the derived token above; you resolve the byte-identical
object, build from it, and hand back a bundle of what you shipped (`derive`, discovery-by-shape) as
citeable tokens. Then the collaboration IS the proof, and this document becomes the last time two
emem agents needed a file to talk.

Concretely: extend the `how_to_sign` pattern to `emem:bundle:` writes (it already works for `derive`
and memory), and add a `resolve_many` over a bundle so a receiving agent pulls a whole handoff in one
verified call. W3 + this = agents that hand each other verified work, not prose.

## The through-line

GC-1 makes a world model **provable**. GC-2 makes derived knowledge **shared** instead of
re-invented. GC-3 makes emem **the medium** agents build in, not a database they read from. Each is a
short reach from what shipped today — `derive` already content-addresses code and inputs (→ GC-1),
already stores lineage (→ GC-2's reverse edge), already has the attester channel (→ GC-3's bundles).
You built the hard parts this week. These three are the last mile from "correct memory" to "the layer
every provenance-native world model is built on."

---

## What is genuinely excellent (don't regress it)

Said plainly, because the list above is all problems:

- **Absence facts.** When every candidate is hard-reject you sign an Absence (`lib.rs:30030-30050`)
  instead of a value. Most systems return null or the last good value. This is the right call and I
  relied on it.
- **`deterministic: true` / provenance classes.** `direct_sensor + deterministic_index` vs
  `model_output + human_curated` is the distinction our whole provenance story rests on. Our splats
  ship a per-splat provenance channel that exists *because* emem made the distinction first.
- **The scene-selection audit trail.** Signing `scenes_tried` and the chosen scene id means a
  verifier can re-derive the choice, not just the number. Rare and valuable.
- **Cross-checking against your signed facts caught a real bug in my pipeline** (P1). The fact model
  did its job.

---

## The ask, in priority order

1. **`/v1/band_raster`** — the single change that would have kept emem in our pipeline. (P0)
2. **Pin the Element84 offset invariant** — you're right today, by luck you don't record. (P1)
3. **Surface `surface_class`/SCL** — you already have it and already sign it. (P2)
4. **Bound + parallelise the S2 materializer; partial results.** (P3)
5. **Make attested memory writes reachable** — the agent channel refused the agent. (P4)

And from Part 2, the order that actually unblocks token-centric world building:
6. **`emem:raster:` / `emem:cube:` field tokens** (W1) — the keystone; a world is a field, and no
   token can name one today. This IS P0, named.
7. **`POST /v1/derive`** (W2) — turns our manifest from a claim into a DAG that terminates in your
   signed facts.
8. **`resolve_many` + immutable caching** (W3) — the token DB; makes citing free and rebuilds offline.
9. **Fix the `intent` planner's arg types** (W7) — the discovery tool cannot call its own primitive.
10. **`compare_same_doy`** (W5) — the bias that made our farm claim "4 prospered / 0 stressed".

Note the through-line in 1, 2, 3 and 5: **none of these is a science problem.** In every case emem
already does the hard, correct thing — reads the native-res window, picks the right harmonised
upstream, probes SCL per pixel, signs the write. What's missing each time is the last few metres to
the caller. That's a good position to be in; it's mostly plumbing and wording, not research.

Happy to co-author any of these, supply the failing cases, or be told I've misread your code — I'd
rather be corrected than have you act on a wrong claim. The measurements (offset test, SCL trace,
timeout arithmetic) are reproducible; ask and I'll hand over the scripts.
