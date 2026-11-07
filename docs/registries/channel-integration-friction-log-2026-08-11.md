# Integration friction log, 2026-08-11

This page is a record. An agent wrote it on 11 August 2026 for the channel at
`/memories/by_attester/`. It speaks to agents and to the developers they report
to. Sections 1 to 6 keep the August findings. The table below gives the state
on 7 November 2026.

## State on 7 November 2026

| Item from August | State on 7 November 2026 | How it was checked |
|---|---|---|
| §1: a trimmed receipt reports only `signature_valid: false` | Fixed. Without `merkle_proof`, the answer has `reason: "receipt_reshaped_after_signing"` and a `failure_detail` that names the absent field. | `POST /v1/verify_receipt` on a Bengaluru recall, whole and without `merkle_proof` |
| §1: recall size | The single-band Bengaluru recall is 6,301 bytes. Its receipt is 1,716 bytes. | `POST /v1/recall` |
| §2: `/docs/sdks/langmem.html` gives 404 | Fixed. The page gives 200. `/docs/memory.html` and `/docs/memory.md` also give 200. | `HEAD` requests |
| §2: the PyPI `Documentation` link | `emem-langmem` 2.4.4 is on PyPI. Its `Documentation` link goes to the GitHub directory `sdks/emem-langmem`. | `https://pypi.org/pypi/emem-langmem/json` |
| §3: `/v1/memory_token/resolve` has `value` only under `fact` | Fixed. `value` and `unit` are also at the top level, next to `fact`. | `POST /v1/memory_token/resolve` |
| §4: `langchain-community` archived | Still archived. | `gh api repos/langchain-ai/langchain-community` |
| §6 and filing 2: `llama-index-tools-emem` | The package is in `sdks/`. It is not on PyPI (404). No `run-llama/llama_index` PR names emem. | PyPI JSON API, `gh search prs` |
| Filing 1: `langchain-ai/docs` row | Not filed. No PR names emem, and `integration_external_docs.yaml` has no emem row. | `gh search prs`, `gh api` |
| Filing 3: reopen `browser-use/browser-use#4852` | Still closed (closed 18 July 2026, not merged). | `gh pr view` |
| §5: `punkpeye/awesome-mcp-servers#6532`, `sacridini/Awesome-Geospatial#201` | Merged on 25 July 2026 and 22 May 2026. | `gh pr view` |
| §5: `Shubhamsaboo/awesome-llm-apps#821` | Closed on 21 May 2026, not merged. | `gh pr view` |
| Filing 4: `efij/awesome-claude-code-security` | Issue #27 for emem-guard is open. It is an issue, not a PR. | `gh search issues` |
| Filing 4: `tamish560/awesome-mcp-security` | No issue or PR names emem. | `gh search issues` |

A note on 29 September 2026 had already recorded the three responder fixes.
The framework rules in §4 (LlamaIndex, n8n, Dify) were not checked again.

## 1. A trimmed receipt reported forgery

Read this finding first if you build an emem integration. A wrapper that
summarises a recall answer for a model context can cut the receipt down. It
can keep only responder, signature, `fact_cids` and `served_at`.
Do not do this.

From receipt preimage v2, the signature covers the inclusion proof. In August,
a receipt with a missing field gave no error at `POST /v1/verify_receipt`. It
returned:

```
signature_valid: false
```

An agent reads this answer as "the data was changed". Thus a trimming wrapper
made an agent accuse its own responder of forgery, on honest data, with no
warning.

The reproduction is below. On 7 November 2026 the same two requests, sent from
a Python script, gave `true` and then `false`.

```bash
R=$(curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
     -d '{"place":"Bengaluru","bands":["copdem30m.elevation_mean"]}')

# whole receipt
echo "$R" | jq '{receipt}' | curl -s -X POST https://emem.dev/v1/verify_receipt \
  -H 'content-type: application/json' --data-binary @- | jq .signature_valid
# true

# one field removed
echo "$R" | jq '{receipt: (.receipt | del(.merkle_proof))}' \
  | curl -s -X POST https://emem.dev/v1/verify_receipt \
      -H 'content-type: application/json' --data-binary @- | jq .signature_valid
# false
```

The behaviour is correct. It is the 2.0.0 fix. Under v1 the signature did not
cover the proof. Thus a proof deleted in transit left a receipt that reported
itself valid. The problem was that the caller could not tell this failure from
real tampering.

The August ask was a distinguishable answer: `signature_valid: false` with a
reason that names the missing field, or a separate `receipt_incomplete` code.
The responder now gives the reason (see the table). The advice to integration
authors stays the same. Pass the receipt through byte for byte, and trim the
facts. In August a single-band recall was 5,279 bytes. The receipt was 1,702
bytes, and band prose was most of the rest. Trim the prose. A caller can choose
to leave out prose, but not the receipt.

## 2. `emem-langmem` 2.1.0 pointed at a page that did not exist

The published package declared:

```
Documentation = https://emem.dev/docs/sdks/langmem.html
```

In August that page gave 404, and it had never existed. The README also
pointed at `https://emem.dev/docs/memory.md`. Docs rendered as `.html`, so
that link also gave 404. The package suite passed all 38 tests. Neither link
is code, so no test checked them.

The repository fixed both links. The PyPI version kept the dead link until the
next release.

This mattered for a second reason. LangChain no longer accepts integration PRs
(see §4). The only PR that still gives a listing is a row with a `docs_url`.
That `docs_url` would have been the dead link. Thus the best submission open
to this project pointed at a 404.

The fix added a guard. A claim in `demos/stabilisation` collects each
`emem.dev` URL that a shipped SDK gives to a user. It sends a `HEAD` request to
each URL on the live site and names the dead ones. It counts only 404 and 410
as dead. Its first run flagged `/v1/ask`. That route is POST only: it gives 405
to `HEAD` and 200 to `POST`. The team turns off a claim after it reports a live
endpoint as dead a second time.

A dead link in a published package is a promise to a user who already ran
`pip install`. Neither pip nor npm checks the link again.

## 3. `/v1/memory_token/resolve` put the record under `fact`

In August the resolved record was at `body["fact"]`, not at the top level. An
integration that read `body["value"]` and `body["unit"]` got `None` for each
field. The call looked like it worked. There was no exception and no error
code. The tool returned nothing useful. `value_verbatim` was at the top level,
so the mistake was easy to make.

The failure gave no signal, so the August ask was one line in the endpoint
documentation.

## 4. The frameworks stopped accepting the PR this project opened

- LangChain archived `langchain-community` on 2026-06-19. No `langchain-ai`
  repository accepts new integrations as PRs. The only PR that gives a listing
  is a row in `langchain-ai/docs`, in
  `scripts/data/integration_external_docs.yaml`. The row points at a package
  on PyPI. `emem-langmem` qualified, and nobody had filed the PR.
- LlamaIndex still accepts integration packages into
  `llama-index-integrations/` and publishes them. It wants a package, not a
  discussion.
- n8n refuses a node published from a laptop from 2026-05-01. A GitHub Action
  must ship the node with a provenance statement. n8n requires MIT.
- Dify marks a PR stale after 14 days of unresolved comments. It closes the PR
  after 30 days. Nobody can reopen a closed PR.

## 5. The listing backlog was wrong in both directions

The author checked seven tracked submissions against their PR pages. Four rows
were wrong:

- `punkpeye/awesome-mcp-servers#6532`: merged 2026-07-25, recorded OPEN.
- `sacridini/Awesome-Geospatial#201`: merged 2026-05-22, recorded OPEN.
- `browser-use/browser-use#4852`: a stale bot closed it on 2026-07-18, after
  it passed review. The backlog recorded "OPEN (code review passed)". The PR
  had no defect. A bot asked if the PR was still wanted, and nobody answered
  in 14 days. The August ask was to reopen it.
- `Shubhamsaboo/awesome-llm-apps#821`: closed, declined. It gave the only
  maintainer feedback of substance in the campaign. The quote is verbatim:

  ```text
  a thin wrapper around the emem MCP endpoint with no substantial AI logic of
  its own … prioritises tutorials demonstrating meaningful LLM integration
  rather than promotional wrappers for external services
  ```

  That verdict applies more widely. A demo that calls one endpoint and prints
  the answer reads as an advertisement, because it is one.

## 6. New in August: `llama-index-tools-emem`

The package has four tools, in the order an agent uses them: `locate`,
`recall`, `resolve_token`, `verify_receipt`. Each docstring tells the model to
quote the `emem:fact:` token, not the number. The reason: a reader cannot tell
a number in prose from a number that a model invented, but a token can be
checked.

The package trims facts to value, unit, address and citation (5,279 bytes to
2,190). The receipt passes through whole, for the reason in §1. The package had
13 tests. An end-to-end run against the live responder gave
`signature_valid: true` and `merkle_proof_valid: true`. A token that `recall`
made resolved back to the same value.

In August it was not on PyPI. Its license is Apache-2.0, the same as the rest
of emem. The LlamaIndex monorepo and its integration packages use MIT, so a
maintainer may ask about this.

---

## What the August log asked developers to file

1. `langchain-ai/docs`: one YAML row under `python:` and then `stores:` for
   `emem-langmem`. This had the highest return of all open items, and it is
   one line.
2. `run-llama/llama_index`: `llama-index-tools-emem` into
   `llama-index-integrations/tools/`. The package exists and has tests.
3. Reopen `browser-use/browser-use#4852`. The PR earned a merge and lost it
   because nobody answered.
4. `tamish560/awesome-mcp-security` and `efij/awesome-claude-code-security`:
   emem-guard, in sections where all current entries are content classifiers.

The table at the top gives the state of each filing on 7 November 2026.

## What the August log asked the responder to change

1. Make an incomplete receipt distinguishable from a tampered one (§1). This
   was the one that put integrators at risk.
2. Document that `/v1/memory_token/resolve` puts the record under `fact` (§3),
   or copy the fields to the top level.
3. Serve `/docs/sdks/langmem.html`, or stop each package from pointing at it.

All three shipped by 29 September 2026. The table at the top shows each one
live on 7 November 2026.

## Method note for agents that do this kind of work

Each defect above got through a passing test suite. Only calls to the live
responder found two of them. Fixtures confirm what you already believe,
because the same wrong assumption shaped them and the code. Before you scale
any task, run one instance of it end to end against the real system.

One cost was larger than any bug. The campaign started ten submission sessions
before one session reached a filed PR. All ten stopped at the same wall: the
environment could not create a fork. One session alone would have found that
wall.
