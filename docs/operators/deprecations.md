# Deprecations

This page lists the emem surfaces that the project deprecated or removed, and what to use instead.
It also states which identifiers stay stable.
All checks on this page ran against emem.dev (version 2.4.4) on 7 November 2026.

| Section | Use it to |
|---|---|
| [Read the live list](#read-the-live-list) | Detect API changes from an agent |
| [Deprecated surfaces](#deprecated-surfaces) | See what still works but is past its removal date |
| [Removed surfaces](#removed-surfaces) | Find the replacement for a surface that is gone |
| [What stays stable](#what-stays-stable) | Know which identifiers you can store for a long time |

## Read the live list

`GET /v1/deprecations` returns the deprecation list as JSON.
It needs no key.

```sh
curl -s https://emem.dev/v1/deprecations
```

The answer on 7 November 2026:

```json
{
  "deprecated": [],
  "policy": "Pre-1.0. A breaking change carries a deprecation entry here one minor release before removal. Entries name the path, the replacement, and the earliest version that removes the old surface.",
  "schema": "emem.deprecations.v1",
  "served_at": "2026-11-07T00:41:21Z",
  "spec": "https://emem.dev/spec.md"
}
```

The `deprecated` array is empty.
The surfaces in [Deprecated surfaces](#deprecated-surfaces) are not in it.
This page is the more complete list until the endpoint carries them.
The `policy` text still says "Pre-1.0", but the live version is 2.4.4.
The handler is `serve_deprecations` in `crates/emem-api-rest/src/lib.rs`.

An agent that keeps the tool catalogue between sessions can read this endpoint once per session.
The response has the header `Cache-Control: public, max-age=3600, stale-while-revalidate=86400`.

## Deprecated surfaces

These surfaces still work.
The first three passed the version set for their removal, and no release has removed them yet.
The old memory names stay until 3.0.

| Surface | Announced in | Removal set for | State on 7 November 2026 | Use instead |
|---|---|---|---|---|
| `GET /.well-known/ai-plugin.json` (OpenAI plugin manifest) | 0.0.6 | 0.0.8 | Served: HTTP 200, 2,883 bytes | `/.well-known/agent-card.json` and `/.well-known/mcp.json` |
| `EMEM_TOPIC_BACKEND=transformer` | 0.0.3 | 0.0.7 | Accepted, mapped to `model2vec` with no warning | `EMEM_TOPIC_BACKEND=model2vec` |
| `EMEM_TOPIC_BACKEND=fastembed` | 0.0.3 | 0.0.7 | Accepted, mapped to `ort` with no warning | `EMEM_TOPIC_BACKEND=ort` |
| The 8 MCP names `memory_view`, `memory_create`, `memory_str_replace`, `memory_insert`, `memory_delete`, `memory_rename`, `memory_supersede`, `memory_list_by_kind` | 2.1.0 | 3.0 | Dispatched, not listed in `tools/list` | The same name with the prefix `emem_`, for example `emem_memory_view` |

OpenAI no longer supports the plugin manifest format.
The two topic-backend values are operator settings, not wire surfaces.
The mapping is in `crates/emem-api-rest/src/topic_router.rs`.
Three of the old memory names delete data.
They had the same names as the verbs of Claude's own memory tool, so the project added the prefix.
The MCP dispatcher in `crates/emem-api-rest/src/lib.rs` still accepts each old name. Seven of the eight tool descriptions say "Version 3.0 removes it". The description of `emem_memory_supersede` does not say it.
On 7 November 2026, `tools/call` with `memory_view` gave a normal result.
The project replaced the `fastembed-rs` wrapper with direct `ort` calls, because the wrapper locked at session start on the production host.

## Removed surfaces

| Surface | Removed in | Use instead |
|---|---|---|
| `POST /v1/jepa_predict_v2` and MCP `emem_jepa_predict_v2` (learned dynamics head on the GPU sidecar) | 2.4.2 | No direct replacement. `POST /v1/jepa_predict` is the closed-form NDVI AR(2) model. |
| `POST /v1/triple_consensus` and MCP `emem_triple_consensus` (Clay, Prithvi and Tessera change ensemble) | 2.4.2 | No direct replacement. Use `POST /v1/state_diff` for one encoder, or `POST /v1/change_attribution` for the evidence ledger. |
| `fastembed-rs` ONNX Runtime wrapper | 0.0.4 | `ort` and `tokenizers` called directly, running BAAI/bge-base-en-v1.5 |
| Hand-written `GAZETTEER` constant (REST) | 0.0.6 | `emem_fetch::geonames::lookup` over the embedded GeoNames cities-1000 table |
| Hand-written `gazetteer_reverse_label` (primitives) | 0.0.6 | `emem_fetch::geonames::nearest_label(lat, lng, max_km)` |
| Fixed 0.15 consensus gate in the `clay_prithvi_tessera_triple_consensus@1` formula | 0.0.6 | `parameters.consensus_threshold` in the algorithm registry (default 0.15) |
| Fixed `max_cells: 256` default in `/v1/query_region` | 0.0.6 | A default from the bbox area: about one cell per 100 km², limited to 64 through 1,024 cells |
| Fixed 4x pre-scan size in `find_similar` mode `hamming_then_rerank` | 0.0.6 | An adaptive factor `ceil(1/recall)`, limited to 4 through 16; it stays at 4 until the recall estimate has data |

Releases 0.0.4 and 0.0.9 added these two routes. Release 2.4.2 removed them, and they were never on the deprecation list.
What a caller sees for the two removed routes:

- REST: `POST /v1/triple_consensus` and `POST /v1/jepa_predict_v2` return HTTP 404 with `"code": "not_found"`.
- MCP: `tools/call` with either old name returns JSON-RPC error `-32602`, "Unknown tool".
  For `emem_jepa_predict_v2`, the message also says "Did you mean emem_jepa_predict?".
- The registry entry `clay_prithvi_tessera_triple_consensus@1` still resolves at `GET /v1/algorithms/<key>`.
  Its `here.status` is `cannot_run`, and `here.why` names the route that the responder does not serve.

Receipts that the old routes signed still verify.
See [What stays stable](#what-stays-stable).

## What stays stable

| Identifier | Rule | How it holds on 7 November 2026 |
|---|---|---|
| Receipts | A receipt stays verifiable after the responder changes version. | `POST /v1/verify_receipt` verifies `preimage_version` 0, 1 and 2. New receipts use version 2. See [protocol.md](../protocol.md). |
| Fact CIDs | A fact CID does not change. A new registry version does not make an old fact invalid. | New registries get new CIDs. The old facts keep their CIDs. |
| Algorithm keys | The `@N` suffix is the version. An old version stays readable after a new one ships. | `flood_risk@1` and `flood_risk@2` both return HTTP 200 at `/v1/algorithms/<key>`. |
| REST routes under `/v1/` | A removal must first appear on the deprecation list. | Release 2.4.2 broke this rule: it removed `/v1/jepa_predict_v2` and `/v1/triple_consensus` without a deprecation entry. |
| MCP tool names | A rename must first appear on the deprecation list. | The 8 old `memory_*` names still dispatch; see [Deprecated surfaces](#deprecated-surfaces). A removed name gets "Unknown tool". |

Registries with a content-addressed manifest (bands, algorithms, functions, sources, schema) do not lose entries silently.
A change publishes a new manifest CID, and the old CIDs stay resolvable for offline checks.
`GET /v1/manifests` lists the current manifest digests.

## How to read the tables

- *Announced in* is the version that first published the deprecation.
- *Removal set for* is the earliest version that can remove the surface.
  A removal can come later than this version.
  It cannot come sooner than one minor release after the announcement.
- *Use instead* is the replacement.
  The replacement ships before the announcement, so you can move to it at once.

## Contact

For operations questions, write to `avijeet@vortx.ai`.
For migration questions, open a discussion at https://github.com/Vortx-AI/emem/discussions with the tag `deprecation`.
