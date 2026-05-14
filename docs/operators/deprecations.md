# Deprecations

This document tracks emem surfaces marked for removal, the version
that removes them, and the migration path. Machine-parseable twin at
[`/v1/deprecations`](https://emem.dev/v1/deprecations) when wired.

The policy: a surface stays announced for at least one minor version
before removal. Anything pinned by a content-addressed manifest CID
(bands, algorithms, functions, sources, schema) is **never silently
removed** — instead a new CID is published and old CIDs remain
resolvable for offline verification.

## Currently announced

| Surface | Announced | Slated for removal | Replacement | Notes |
|---|---|---|---|---|
| `/.well-known/ai-plugin.json` | 0.0.6 | 0.0.8 | `/.well-known/agent-card.json` + `/.well-known/mcp.json` | The OpenAI plugin manifest format is sunset upstream. The file stays served (16 lines, trivial cost) but loses doc support after 0.0.8. |
| `EMEM_TOPIC_BACKEND=transformer` alias | 0.0.3 | 0.0.7 | `EMEM_TOPIC_BACKEND=model2vec` | Wire-stable alias resolves through 0.0.7; emit a warning then. |
| `EMEM_TOPIC_BACKEND=fastembed` alias | 0.0.3 | 0.0.7 | `EMEM_TOPIC_BACKEND=ort` | The fastembed-rs wrapper was retired in 0.0.4; the env var alias stays for back-compat. |

## Removed

| Surface | Removed in | Replacement |
|---|---|---|
| `fastembed-rs` ORT wrapper | 0.0.4 | `ort` + `tokenizers` directly running BAAI/bge-base-en-v1.5 |
| Hand-curated `GAZETTEER` const (REST) | 0.0.6 | `emem_fetch::geonames::lookup` over the cities5000 corpus |
| Hand-curated `gazetteer_reverse_label` (primitives) | 0.0.6 | `emem_fetch::geonames::nearest_label(lat, lng, max_km)` |
| Hardcoded 0.15 consensus gate in `clay_prithvi_tessera_triple_consensus@1` formula | 0.0.6 | `parameters.consensus_threshold` (typed, tunable, citation-anchored) |
| Hardcoded `max_cells: 256` default in `/v1/query_region` | 0.0.6 | Bbox-area-derived default (target ~1 cell per (10 km)², clamped to [64, 1024]) |
| Hardcoded `4×` triage oversampling in `find_similar` mode `hamming_then_rerank` | 0.0.6 | EWMA-adaptive factor `ceil(1/recall)` clamped to [4, 16] |

## Stability contract

- **Receipts**: every receipt issued under any version of emem stays
  verifiable forever. The signature math, the canonical preimage
  format, and the ed25519 keys are protocol-stable. A 0.0.2-era
  receipt verifies against a 0.0.6 responder without change.
- **Content-addressed CIDs**: a fact CID is forever; rolling the
  registry that produced it does not invalidate the fact. Old facts
  under old CIDs continue to resolve.
- **MCP tool names** (`emem_recall`, `emem_locate`, …): wire-stable.
  Renames go through this deprecation table; both names resolve for
  at least one minor version.
- **REST endpoints under `/v1/*`**: wire-stable. Removal goes through
  this deprecation table.
- **Algorithm keys** (`flood_risk@2`, `clay_prithvi_tessera_triple_consensus@1`, …):
  the `@N` suffix is the version. `flood_risk@1` resolves forever
  even after `flood_risk@3` lands.

## How to read this table

- *Announced* = the version that first published the deprecation.
- *Slated for removal* = the earliest version that may drop the
  surface. Removal can slip but cannot accelerate past the
  one-minor-version buffer.
- *Replacement* = the supported alternative as of the *Announced*
  version. The replacement is wired before the deprecation is
  announced; users always have an immediate migration path.

## Machine twin

The same content is served at `/v1/deprecations` as a JSON envelope
when wired. The shape:

```json
{
  "schema": "emem.deprecations.v1",
  "as_of_version": "0.0.6",
  "as_of_date": "2026-05-14",
  "announced": [
    {
      "surface": "/.well-known/ai-plugin.json",
      "announced_in": "0.0.6",
      "remove_in": "0.0.8",
      "replacement": "/.well-known/agent-card.json + /.well-known/mcp.json",
      "rationale": "OpenAI plugin manifest format sunset upstream."
    }
  ],
  "removed": [
    {
      "surface": "fastembed-rs ORT wrapper",
      "removed_in": "0.0.4",
      "replacement": "ort + tokenizers directly"
    }
  ]
}
```

An agent that caches the emem tool catalogue between sessions can poll
`/v1/deprecations` once per session to detect drift before calling a
removed primitive.

## Contact

Operational questions: `avijeet@vortx.ai`. Migration questions: file
a discussion at https://github.com/Vortx-AI/emem/discussions tagged
`deprecation`.
