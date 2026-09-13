# Reasoning splats: what `/v1/ask` should deliver to a model

*Status: research + design. Every number in section 1 is measured live against
emem.dev on 2026-09-13 (d2f13f1). Section 3 onward is proposal.*

## The deliverable, first

A model asks one question and receives, inside the same wire budget it already
pays:

- **The evidence as primitives it can draw.** One per reading: where (emem's own
  cell address and a coordinate), what (band, value, unit), when (age), what
  kind of claim it is (provenance class), which stage introduced it, and the
  fact it rests on. ~120 bytes each. Forty of them is 4.8 KB.
- **Addresses that resolve.** Each stage carries an `emem:state:` token that
  dereferences to the bytes it commits to, so a second ask skips what did not
  change and a third party can check what did.
- **Typed structure, not a JSON string.** `structuredContent` for the splat,
  `resource_link` for anything heavy, so a model reads fields instead of parsing
  a blob, and fetches the bulk only if it wants it.

That is the game-changer sentence, and none of it is rhetoric: the pieces are
measured below, and the budget for them already exists inside what we currently
spend on material no model asked for.

## 1. Where the budget goes today, measured

One question (`Trafalgar Square, London`, "how busy is it right now?"):

| channel | bytes | reasoning | states |
| --- | --- | --- | --- |
| REST | 99,321 | yes | 4 |
| MCP | 22,684 | slimmed to 679 B | 4 |
| A2A | 106,575 | yes | in text |

REST envelope, 99,908 bytes:

| field | bytes | share |
| --- | --- | --- |
| `reasoning` | 22,066 | 22.1% |
| `algorithms_for_question` | 20,561 | 20.6% |
| `live_perception` | 15,977 | 16.0% |
| `facts_summary` | 9,120 | 9.1% |
| `receipt` | 8,273 | 8.3% |
| `fact_cids` | 6,608 | 6.6% |
| `algorithm_outcomes_summary` | 4,083 | 4.1% |
| `freshness` | 2,463 | 2.5% |
| `topic_routing` | 2,403 | 2.4% |
| `band_observations_summary` | 1,573 | 1.6% |
| `answer` | 1,519 | 1.5% |

**The ratio this document exists for.** A reading costs 83 bytes:
`{"age_s": 1924904, "band": "weather.temperature_2m", "unit": "degC", "value":
16.2}`. Twelve of them are 1.6% of the envelope. The block describing how we
found them is 22%, and contains none of them. Reference material for recipes
that never ran is another 20.6%.

Three structural wastes, each measured:

1. Grounded fact cids are carried three times: envelope `fact_cids`, per-step
   `new_fact_cids`, per-state `derived_from`. At 33 cids of 52 characters, ~5 KB
   per copy.
2. Repeated explanatory prose inside `reasoning`: 1,404 bytes of the same two
   sentences, once per stage.
3. `algorithms_for_question`: 20,561 bytes of citations for algorithms that did
   not run on this question.

## 2. What the transport already offers and we do not use

Over MCP `emem_ask` returns exactly one `text` item containing JSON as a string.
No `structuredContent`. No `resource_link`. Both are in the protocol version we
serve. A model therefore parses a blob, and pays for every byte of it whether it
wanted that section or not.

## 3. `emem.reasoning_splat.v1`

One primitive per piece of evidence:

    { "cell": "defi.zb64a.cAzU.zfa27",
      "at": [51.5084, -0.1284],
      "band": "weather.temperature_2m",
      "value": 16.2, "unit": "degC",
      "age_s": 1924904,
      "class": "direct_sensor",
      "stage": "recalled",
      "fact": "7662nrfj..." }

Absence is a primitive too, because a picture of only what was found asserts a
coverage nobody measured:

    { "cell": "...", "band": "hansen.loss_year", "absent": true }

And the stage skeleton, unchanged from what ships today: `stage`, `at_ms`,
`grounded_total`, `state`.

A model can plot these, colour by class or age, show found against absent, and
cite any point by its fact. Two asks diff by address: same stage token, same
inputs, skip it.

## 4. Build order

1. **`GET /v1/state/<cid>`.** The addresses we already emit say
   `verifiable_today: false` on every response. The way to stop publishing that
   sentence is to make it untrue, not to delete it. Order, and it is not ours to
   shortcut: canonicalisation spec, published test vector, then the route. A
   fetchable record under an unspecified encoding turns "I cannot check this"
   into "I checked it and it failed".

2. **The splat projection**, beside `reasoning`, with a hard byte budget and a
   `truncated` computed from the pre-cap count. (Twice this week a count taken
   from an already-capped list could not witness its own capping.)

3. **`structuredContent` + `resource_link` over MCP.** Splat as typed output;
   `algorithms_for_question`, full `reasoning` and `live_perception.temporal_context`
   as links fetched on demand. This is the budget fix and the model-native fix in
   one move.

4. **Prose out of the per-response envelope.** The `_means`, `_why_not` and
   `does_not_cover` sentences are schema documentation charged to every answer.
   They belong in the schema a model fetches once, leaving the response carrying
   values, addresses and typed flags.

5. **Contribution, only when computable.** Which evidence moved the answer is
   the most valuable attribute a splat could carry, and we cannot derive it
   today. It stays absent until an algorithm reports per-input sensitivity. An
   invented weight would be the most damaging number in the payload, because it
   is the one a reader most wants to believe.

## 5. The line we hold

A splat is a projection of signed evidence, not a rendering of the world: the
points are readings at addresses. Absence ships as data beside presence, so a
consumer can see coverage rather than infer it. And a stage address is stable
for the same inputs and moves when the facts under it move, which is the
property that makes skipping safe.
