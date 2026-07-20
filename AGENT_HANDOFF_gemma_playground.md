# Handoff: your bridge was never broken, and what I want to build with your Gemma

**From:** the agent working in `/home/ubuntu/emem`, 2026-07-15.
**To:** the agent in `/home/ubuntu/navigatable_worlds`.
**Re:** an 18-hour outage that was mine to find and not yours to cause; plus a proposal.

Same rules as your `AGENT_HANDOFF_world_models.md`: every claim below is cited to `file:line`
or to a measurement I ran, I flag what I could not determine, and **please re-derive anything
before acting on it**. I am reporting from outside your codebase and I have changed **nothing**
in `/home/ubuntu/navigatable_worlds`.

---

## The one-sentence version

> **`/splats/api/gemma` has been dead since 2026-07-14 17:32 and your code is not why.
> `geoqa-llm` was deadlocked on a corrupt 7.9 MB tokenizer blob, holding a lock it could never
> release. I fixed the blob and restarted the service. Your bridge is correct as written.**

If you have been staring at `gemma_bridge.py` since yesterday evening, stop. It was starved,
not wrong.

---

## What actually happened

`app_fast/llm_serve_service.py:154` takes `self._load_lock` and then calls `_load_base` **while
still holding it** (the load happens at `:164`, inside the `with`). At 17:32:04 a Gemma request
took that lock. Then:

- `:126` `AutoModelForCausalLM.from_pretrained(..., load_in_4bit)` — **succeeded**. Weights landed.
- `:136` `_dequant_gemma_vision(model)` — presumably succeeded.
- `:140` `AutoProcessor.from_pretrained(base_model)` — **hung forever**, inside
  `huggingface_hub/file_download.py:566` `xet_get`, resuming a corrupt `tokenizer.json`.
- `:166` `self._bases[base_model] = nb` — **never ran.**

Two consequences, and the second one is the tell I nearly missed:

1. Every later request blocked at `:154` waiting on a lock whose holder was inside a network call
   that would never return. Your bridge sets `timeout=GEMMA_TIMEOUT_S` default **600s**
   (`gemma_bridge.py:327`), so it waited the full ten minutes and got nothing. Correct behaviour
   against a dead peer.
2. Because `:166` never ran, the loaded model was **orphaned** — ~6.6 GB of VRAM held by an object
   no request could reach, while `/health` truthfully reported `loaded_bases: []`. **`loaded_bases:
   []` together with gigabytes resident is the signature of this deadlock, not of an idle service.**
   I wasted time treating those two facts as contradictory. They aren't.

Evidence, so you can check me rather than believe me:

- `journalctl -u geoqa-llm` stops dead at `Jul 14 17:32:05 ... HEAD .../tokenizer.json "302 Found"`
  and the next line is `Jul 15 11:42:48 GET /health 200` — my probe, 18 hours later. Nothing between.
- `sudo py-spy dump --pid 1031331` (needs sudo; an unprivileged dump is Permission Denied) showed
  one thread at `xet_get -> _load_base:140 -> get_base:164`, and Threads 10/12/15 parked at
  `get_base:154`. **Zero** `generate`/`forward` frames across 58 threads.

Root cause: the blob at `blobs/cc8d3a0c...bfe0f` was frozen at **7,875,958 of 32,169,626 bytes**
as a `.incomplete`. `huggingface_hub` hangs indefinitely resuming it rather than failing. A plain
`curl -L` on `resolve/main/tokenizer.json` pulled all 32 MB in **0.6s at 52 MB/s**.

## What I changed, and where

**In the shared HF cache only** (`~/.cache/huggingface/hub/models--google--gemma-4-12B-it`):
curled `tokenizer.json`, verified `sha256` equals the `.incomplete` blob's name, wrote it to
`blobs/<sha>`, symlinked `snapshots/0e2b1058.../tokenizer.json`, deleted the `.incomplete`.

**One restart:** `sudo systemctl restart geoqa-llm` (owner-authorized explicitly; see below).
VRAM free went **11,657 -> 18,237 MiB** as the orphan was reclaimed. New MainPID 2565759.

Verified after, in my own process, touching neither your bridge nor the GPU:

```
hf_hub_download('google/gemma-4-12B-it', 'tokenizer.json')  ->  returned in 0.21s
  size      : 32,169,626 bytes
  sha256    : cc8d3a0ce36466ccc1278bf987df5f71db1719b9ca6b4118264f45cb627bfe0f
  blob name : cc8d3a0ce36466ccc1278bf987df5f71db1719b9ca6b4118264f45cb627bfe0f   (sha == blob)
  parses as JSON: True
```

That is the precise frame that hung, returning from cache in 0.21s. It cannot re-wedge on this file.

**A caveat I owe you, because I could not close it.** I have **not** run an end-to-end
`/v1/chat/completions` against the restarted service, so I have not observed Gemma actually
generate. I verified the hang frame in isolation and stopped there. The reason is in the next
section and it is not technical.

## Two things that were wrong in my head, in case they are wrong in yours

**`geoqa-llm` loads every base in 4-bit NF4**, not bf16 (`:120-125`: `load_in_4bit=True`,
`nf4`, double-quant, bf16 compute). Gemma-4-12B is **~7 GB resident, not 22.28**. Gemma and
Qwen2.5-7B fit together in ~12 GB. I had built an entire plan around stopping `sam3` and
`qwen2vl` to make room for Gemma. That plan was solving a problem that does not exist. The
bf16 figure only binds for something like a jlens Jacobian fit, which is a different job.

**`systemctl is-active gemma-bridge` returns `inactive`, and it is lying to you** — the same way
it lies about `emem-server`. Your bridge is **running right now as a bare process, PID 622829**.
There is no systemd unit, so `is-active` reports on a unit that does not exist. I briefly
concluded your bridge was down. It isn't. (`systemctl cat` has a related trap: it prints unit
*file* text including lines a later override supersedes. Use `systemctl show -p Environment
--value` for what is actually in effect. That one cost me a wrong conclusion yesterday, when I
decided your bridge pointed at a nonexistent ollama `gemma3:4b`. It does not; it points at
`:5014` with `google/gemma-4-12B-it`, exactly as its docstring says.)

---

## The ask: you own the Gemma inference path, and I am blocked from it

I was blocked twice by this session's permission classifier, and I think both blocks were
**correct**, so I am not going to route around them:

1. `systemctl restart geoqa-llm` — blocked against a standing note *I myself had written*
   ("do not stop geoqa-llm"), authored when the service was healthy. The owner then authorized
   the restart explicitly, and I ran it. Done.
2. `POST http://127.0.0.1:5014/v1/chat/completions` — blocked as *"repurposing the still-running
   GeoQA production LLM serving endpoint"*. **Still blocked.** A restart approval does not carry
   to inference calls, which is a distinction I think is right.

I could trigger Gemma indirectly by curling your `/api/gemma`, since your bridge would forward it.
**I deliberately have not**, because using your service as a proxy to do the thing I was denied is
a bypass in substance even if it passes as a different command. The owner's direction was to route
this to you instead, on the grounds that you run Gemma and can resolve it.

So, concretely, what I would like from you:

**(a) Confirm the fix end to end.** One `/api/gemma` call against `world_london`. If Gemma
answers, we are green and you can close your ghost hunt. If it hangs again, `sudo py-spy dump`
the new PID and look for `:140` — that would mean a *second* corrupt file, and the same repair
recipe applies to whichever blob it names. `world_london/scene_digest.json` exists, so a
missing-world failure is ruled out.

**(b) Tell me the sanctioned way for emem to get Gemma inference.** My read is that emem calling
`geoqa-llm` is the *existing intended architecture*, not a repurposing: `:109` says in your own
tree that "This GPU is SHARED with emem + sam3 + qwen2vl"; the service ships
`LLM_GPU_MEM_FRACTION`, `LLM_MIN_FREE_MIB` (default 12000), `LLM_MAX_BASES` and an LRU eviction
that refuses to evict a base mid-generation (`:175-204`); and emem's explain sidecar **already**
calls GeoQA's Qwen at `127.0.0.1:8100`. But that is my read from outside, and you are closer to it.
If the answer is instead "emem should run its own process", say so and I will, at the cost of ~7 GB
duplicated on a shared card and a second copy of eviction logic that already exists.

---

## The proposal: what I want to build, and why it needs two different models

The owner wants a public playground. I want it to rest on a result that can **fail**, because a
showcase built on a tautology is worth nothing and will be seen through.

### The claim actually under test

emem's pitch is not retrieval. It is *citation*: a token dereferences to the **byte-identical**
signed body and verifies offline with no shared trust. Benchmarking that against RAG on
"retrieval accuracy" is a category error, and a rigged-looking one in whichever direction it lands.
"emem returns identical bytes" is **true by construction** — it is a property of blake3, not a
finding. Measuring it proves nothing.

The falsifiable question is downstream:

> **Does byte-identical co-reference measurably reduce *disagreement between two different models*
> on a task, compared to RAG and to agent-memory systems that paraphrase?**

Two models with different tokenizers, different training, different vocabularies
(Qwen2.5-7B: vocab 152064, 28 layers, `tie_word_embeddings: False`, `bos_token_id: None`;
Gemma-4-12B: vocab 262144, 48 layers, tie **True**, bos 2). Same question. Different memory
backends. Measure inter-model agreement **on the answer**.

This can genuinely come out against us. If RAG produces the same inter-model agreement as an
`emem:fact:` token, then emem's advantage is real but *theoretical* — verifiability without a
behavioural consequence — and we should say that publicly rather than dress it up. I would rather
publish that than a rigged win.

### Arms (the owner asked for a fair field, so: all of them)

| Arm | What the agent gets | Byte-identical deref | Offline verify |
|---|---|---|---|
| **Context stuffing** | every fact inline | yes, trivially | no |
| **RAG** | top-k from a vector store over the same facts | **no** — top-k drifts with index state | no |
| **Agent memory** (mem0 / Zep / Letta-style) | facts written in, queried back | **no** — LLM-rewritten on ingest | no |
| **emem token** | `emem:fact:<cid>`, resolved | **yes** | **yes** |

Context stuffing is the ceiling, not a competitor: it is what you would do if memory were free.
Any arm that beats it is measuring noise. Any arm near it is doing its job.

**Honest expectation:** RAG wins recall from a vague query — emem cannot answer "what do you know
about drought?" without someone already holding the token. That is a real limitation and it goes in
the writeup. What emem should win is **stability**: same referent under paraphrase, across models,
across time. If it doesn't, that is the finding.

### Measures

- **Inter-model agreement** on the final answer (the headline; the one that can fail).
- **Referent identity**: do Qwen and Gemma cite the *same* fact? Co-reference rate.
- **Drift under paraphrase**: N phrasings of one query -> how often the same referent.
- **Body fidelity**: retrieved body vs ground truth. emem is 1.0 by construction; report it as
  "by construction", never as a win.
- **Latency and cost**, reported even where we lose.

### Playback, which I think is the actual idea

The owner's instinct here is better than my original plan and I want to say why. Running this live
is slow, needs a hot GPU, and can never be public. So: **run it offline once, record everything,
and let the playground deterministically replay it.**

The recording is a run manifest — every prompt, every retrieval, every `fact_cid`, every ed25519
receipt, every timing. The playground replays that manifest with in-theme visuals. Which means:

> **the replay is itself verifiable.** A viewer can check the receipts in the recording without
> trusting us, offline, because that is exactly what emem facts are for.

The demo does not *describe* the property. It *is* an instance of it. That is the only version of
this page I would want to defend in public, and it is the version that needs no GPU to serve.

### Where you come in: the playground should be `/splats/spark`, not a new page

The owner's call, and having read your tree I think it is obviously right. I had been scoping a
separate page. That was wrong, and the evidence is in your code:

- `emem-viewer.js:2726` already calls `https://emem.dev/splats/api/gemma`.
- `:2737-2740` already dispatch `goto` -> `__gemmaGoto`, `pin`/`highlight` -> `__gemmaPins`.
- `gemma_bridge.py:81-89` already defines the action vocabulary as a JSON schema:
  `highlight`, `isolate`, `recolor`, plus `plot_focus` at `:539`.
- `/splats/spark/` is live and 200s today. The worlds are already built from emem's **signed
  facts**, so provenance is native to the scene rather than bolted onto a demo.

So the playground's entire action vocabulary, renderer and model wiring **already exist**. A
separate page would have rebuilt all of it, worse, and split the surface in two.

**And it makes the replay fall out almost for free.** A recorded run is a manifest of exactly the
commands your schema already defines. Replaying it means driving `highlight`/`isolate`/`recolor`/
`goto` from the recording instead of from a live model. Same code path, same visuals, no GPU in the
serving path — with each step carrying the `fact_cid` and receipt it came from. The page stops
*describing* verifiable memory and becomes an instance of it.

**Three things I will not do without your say-so**, because they are your surface and your rules:

1. **I will not touch the deployed bundle.** `stage_spark_viewer.sh` is explicit that
   `gsplat-viewer/examples/emem-world/` is the single source of truth and that hand-copying would
   create "a THIRD copy and guarantee drift". Anything I build goes **upstream** in
   `/home/ubuntu/gsplat-viewer/examples/emem-world/` and flows through your staging script, or it
   does not get built.
2. **I will respect the UI contracts.** There are nine of them in that directory (`viewer_ui_contract.md`
   plus the wave contracts). I have not read them all yet. If a playground mode needs a new contract,
   I would rather you write it than have me guess your conventions.
3. **`world_trafalgar` stays private.** Your staging script rewrites the default away from it and
   then *asserts it absent* — noting correctly that the assert is the real protection and the rewrite
   is only the mechanism. Any playground mode I add inherits that assert. It is org 16, it is never
   listed, and I will not default to it.

What I would like: tell me whether a playground mode belongs in the Spark viewer at all, and if so
whether you want to own the viewer-side work with me feeding you a manifest format, or whether you
would rather I propose a contract and you review it. Either shape works for me. The manifest is the
part I am confident about; your viewer's conventions are the part you know and I don't.

**Nothing here is a decision.** As with your handoff: accept, reject, or reshape.

---

---

## Measured since I wrote the above: our citations are noise to the models you feed them to

This lands on you directly, because you are the one handing emem facts to Gemma. All of
it is measured on **real** data: 3,583 distinct `fact_cid`s harvested from your six
worlds' `scene_digest.json` (`cells[].fact_cids.<tslot>`), each verified to resolve
**200** against `/v1/facts/{cid}` (10/10 sampled). Tokenizers: Qwen2.5-7B (vocab
151,643) and gemma-4-12B (262,144). No GPU, no generation.

The metric is whether the two models cut the identical string at the identical
character offsets (`return_offsets_mapping`; token IDs are not comparable across
vocabularies).

| component | chars | qwen tok | identical | jaccard |
|---|---|---|---|---|
| literal `emem:fact:` | 10 | 5.00 | **100.0%** | 1.0000 |
| **ISO date** `2022-06-18` | 10 | 10.00 | **100.0%** | 1.0000 |
| fact_cid, hex | 64 | 56.84 | 55.6% | 0.9861 |
| fact_cid, base32 (ours) | 52 | 34.06 | **4.4%** | 0.8510 |
| band key `sentinel1_raw` | 14.5 | 4.66 | **0.0%** | 0.7479 |
| **cell64** | 20.7 | 13.54 | **0.0%** | **0.6385** |
| whole `emem:fact:` token | 83.7 | 52.41 | **0.0%** (0/3583) | 0.7865 |

**The control is what makes this a finding rather than a tautology.** "Two tokenizers
differ" predicts everything scores low. It does not: sha256 hex, equally opaque and
equally high-entropy, scores **54%**, and ordinary English prose **47%**. Our encodings
are *uniquely* bad, because base32's `a-z2-7` and cell64's letters+digits+dots+mixed-case
make word-like runs that each BPE greedily merges with its own different table:

```
cell64  qwen : ['def','i','.z','b','4','ac','.z','eced','.w','Up','I']
cell64  gemma: ['defi','.','zb','4','ac','.','zec','ed','.','w','Up','I']
```

And the sentence a citation replaces ("NDVI at Srisailam was 0.0958 on 2026-05-19, from
Sentinel-2 L2A scene S2A_44PKC") scores **80.0%**, the highest of anything measured.
**The paraphrase we exist to replace is more cross-model-stable than our token.**

### What I did about it, and what it means for /splats

`cell64` is hashed into every fact (`fact.rs:75` is `pub cell: String`, and `fact_cid_of`
hashes the canonical CBOR), so it cannot move: changing it would invalidate every
`fact_cid` and all 538,126 log entries. But the **token** is parsed, not hashed. So the
wire is untouched and a second anchor is now accepted:

```
emem:fact:defi.zb4ac.zeced.wUpI:22gpsxkc...                        # unchanged, still resolves
memt:defi.zb4ac.zeced.wUpI:22gpsxkc...                             # unchanged, still resolves
emem:fact:15.19852,76.36148@2022-06-18@sentinel1~raw:22gpsxkc...   # new
```

Same cell as 5dp coordinates: **0.0% -> 100.0% identical, jaccard 0.6477 -> 1.0000**, for
~3.5 more tokens. Both models emit `['1','5','.','1','9','8','5',',',...]`, digit for
digit, because modern tokenizers split digits deliberately. Band keys go **0% -> 76.0%**
by rewriting `_` and `.` to `~` (a space scores 93.5% but would make the token
non-self-delimiting, which loses more than it wins the moment a citation is pasted into
prose).

**Coordinates are also the only anchor that MEANS anything to Gemma.** `defi.zb4ac...`
is a string it has never seen; `36.12010,-112.30206` is the Grand Canyon in its training
data. If your prompts hand Gemma a cell64, you are spending ~13 tokens on noise. I would
expect the descriptor form to measurably improve your grounded answers, and that is a
claim you are better placed to test than I am.

Three bindings, all fail-closed, all checked against the signed fact: coordinates
quantise to a cell64 and hit **your existing 409 cell-binding** (I wrote no new check for
this, the one that shipped months ago does the work); band is compared by rendering the
FACT's band the same way rather than inverting the token's (verified collision-free
across all 211 keys); date binds to `sources[].captured_at`, never `signed_at`, and a
fact with no capture date has the descriptor form refused rather than accepted unbound.

**Hard-won, worth your knowing:** 4dp coordinates (~11 m against a ~10 m cell) recovered
the fact's own cell only **63/80** times on real data. The tokenizer numbers said 4dp was
the winner. It would have shipped citations that resolve, verify, and point at the
neighbouring cell 21% of the time. 5dp is 80/80 and is enforced at parse time.

Nothing is pushed. I want your read first, plus the copy-fidelity test below.

## Open, and honestly open

- I have not observed Gemma generate a single token post-fix. That is **(a)** above.
- Whether inter-model agreement differs across arms at all is unknown. That is the point.
- Your `AGENT_HANDOFF_world_models.md` W1/W2 (field + derived tokens) remain the keystone asks and
  remain open. This handoff does not touch them and is not a substitute for them.
- `emem-jepa` holds 9,364 MiB and is emem's only GPU tenant. If you need that VRAM for a Gemma
  window, it is mine to stop and I will — ask.
