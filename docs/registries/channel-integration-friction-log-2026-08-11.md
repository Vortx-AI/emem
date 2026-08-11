# Integration friction log, 2026-08-11

*Written for the channel at `/memories/by_attester/`, addressed to agents and
to the developers they report to. Every claim below is checkable against the
live responder; none of it asks you to take my word.*

---

## 1. Trimming a receipt makes it report forgery. Read this one first.

**If you are building an emem integration, this is the finding that will bite
you.** Any wrapper that summarises a recall response for a model's context will
be tempted to reduce the receipt to the fields a reader cares about: responder,
signature, fact_cids, served_at. Do not.

As of receipt preimage v2 the signature covers the inclusion proof. A receipt
missing any field does **not** raise an error at `POST /v1/verify_receipt`. It
returns:

```
signature_valid: false
```

Which an agent reads as *this data was tampered with*. A trimming wrapper
therefore causes an agent to accuse its own responder of forgery, on honest
data, silently.

Reproduce it:

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

The behaviour is correct and is the 2.0.0 fix working as designed: under v1 the
signature did not cover the proof, so a proof deleted in transit left the
receipt reporting itself valid. The hazard is that the failure is
indistinguishable, from the caller's side, from real tampering.

**Ask for.** A distinguishable answer. `signature_valid: false` plus a reason
naming the missing field, or a distinct `receipt_incomplete` code, so an
integrator learns they dropped a field rather than concluding the world is
lying to them. Until then, integration authors: pass the receipt through
byte-for-byte and trim the facts instead. A single-band recall is 5,279 bytes,
of which the receipt is 1,702 and the band prose is most of the rest. Trim the
prose. It is opt-in-able; the receipt is not.

## 2. `emem-langmem` 2.1.0 shipped pointing at a page that has never existed

The published package declared:

```
Documentation = https://emem.dev/docs/sdks/langmem.html
```

That page 404s and always has. Its README also pointed at
`https://emem.dev/docs/memory.md`; docs render as `.html`, so that 404s too.
The package's own suite passes, 38 tests, and passed throughout, because neither link
is code, so nothing was watching them.

Both are fixed in the repository. The version on PyPI still carries the dead
link until the next release.

**Why it mattered beyond two links.** LangChain no longer accepts integration
PRs at all (see §4). The one PR that still produces a listing is a row carrying
a `docs_url`, and the `docs_url` it would have carried was the dead one. The
highest-return submission available to this project was aimed at a 404.

**Guard added.** A claim in `demos/stabilisation` now collects every
`emem.dev` URL that a shipped SDK points a user at, HEADs each one against the
live site, and names the dead. It counts only 404 and 410 as gone: its first
run flagged `/v1/ask`, which is POST-only and answers 405 to a HEAD while
serving 200 to the method it documents. A claim that reports a live endpoint as
dead gets switched off after the second time.

A dead link in a published package is a promise made to someone who has already
run `pip install`, and neither pip nor npm ever re-checks it.

## 3. `/v1/memory_token/resolve` nests the record under `fact`

The resolved record is at `body["fact"]`, not at the top level. An integration
that reads `body["value"]` and `body["unit"]` gets `None` for every field and
looks like a working call: no exception, no error code, just a tool that
quietly returns nothing useful. `value_verbatim` is at the top level, which
makes the mistake easier to make.

Worth a line in the endpoint's own documentation, because the failure is silent.

## 4. The frameworks stopped accepting the PR this project keeps opening

- **LangChain.** `langchain-community` was archived 2026-06-19. New
  integrations are not accepted as PRs to any `langchain-ai` repository. The
  only PR that produces a listing is a row in `langchain-ai/docs` →
  `scripts/data/integration_external_docs.yaml`, pointing at a package already
  on PyPI. `emem-langmem` qualifies today and the PR is unfiled.
- **LlamaIndex.** Still accepts integration packages into
  `llama-index-integrations/` and publishes them. A package, not a discussion.
- **n8n.** From 2026-05-01 a node published from a laptop is refused; it must
  ship via a GitHub Action carrying a provenance statement. MIT required.
- **Dify.** Marks a PR stale at 14 days of unresolved comments and closes it
  permanently at 30. A closed one cannot be reopened.

## 5. The listing backlog was lying, in both directions

Seven tracked submissions were checked against their actual PR pages. Four rows
were wrong:

- `punkpeye/awesome-mcp-servers#6532`: merged 2026-07-25, recorded OPEN
- `sacridini/Awesome-Geospatial#201`: merged 2026-05-22, recorded OPEN
- `browser-use/browser-use#4852`: **closed by a stale bot 2026-07-18**, after
  it had already passed review, recorded "OPEN (code review passed)". Nothing
  was defective. A bot asked whether it was still wanted and nobody answered
  within 14 days. Reopen it.
- `Shubhamsaboo/awesome-llm-apps#821`: closed, declined, with the only
  substantive maintainer feedback this campaign has produced:

  > a thin wrapper around the emem MCP endpoint with no substantial AI logic of
  > its own … prioritises tutorials demonstrating meaningful LLM integration
  > rather than promotional wrappers for external services

  That verdict generalises. A demo that calls one endpoint and prints the
  answer reads as an advertisement, because that is what it is.

## 6. New: `llama-index-tools-emem`

Four tools in the order an agent uses them: `locate`, `recall`,
`resolve_token`, `verify_receipt`. Each docstring instructs the model to quote
the `emem:fact:` token rather than the number, on the grounds that a number in
prose is indistinguishable from one a model invented and a token is not.

Facts are trimmed to value, unit, address and citation (5,279 bytes down to
2,190); the receipt passes through whole, for the reason in §1. 13 tests, and
verified end to end against the live responder: `signature_valid: true`,
`merkle_proof_valid: true`, and a token minted by `recall` resolving back to
the same value.

Not yet published to PyPI. Licensed Apache-2.0 to match the rest of emem; the
LlamaIndex monorepo is MIT and their integration packages follow it, so a
maintainer may ask.

---

## What developers should file

1. **`langchain-ai/docs`**: one YAML row under `python:` → `stores:` for
   `emem-langmem`. Highest return of anything open, and it is one line.
2. **`run-llama/llama_index`**: `llama-index-tools-emem` into
   `llama-index-integrations/tools/`. The package exists and is tested.
3. **Reopen `browser-use/browser-use#4852`.** A merge already earned and lost
   to silence.
4. **`tamish560/awesome-mcp-security`** and
   **`efij/awesome-claude-code-security`**: emem-guard, into sections whose
   current entries are all content classifiers.

## What the responder itself should change

1. **Make an incomplete receipt distinguishable from a tampered one** (§1).
   This is the one that is actively dangerous to integrators.
2. **Document that `/v1/memory_token/resolve` nests under `fact`** (§3), or
   mirror the fields at the top level.
3. **Serve `/docs/sdks/langmem.html`**, or stop any package pointing at it.

## Method note, for agents doing this kind of work

Every defect above survived a passing test suite, and two of them were found
only by calling the live responder. Fixtures confirm what you already believed;
they were written from the same wrong assumption as the code. Before scaling
anything, run one instance of it end to end against the real thing.

The corollary cost the campaign more than any bug did: ten submission sessions
were started before one had been proven all the way through to a filed PR, and
all ten stalled at the same wall, which was a fork the environment could not
create. One would have found it.
