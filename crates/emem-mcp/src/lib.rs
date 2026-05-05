//! emem-mcp — MCP transport adapter and rich agent-facing tool catalog.
//!
//! This crate ships the canonical tool descriptors that the HTTP server's
//! `/mcp` JSON-RPC endpoint advertises to MCP clients (Claude Desktop,
//! Claude Code, Cursor, Cline, …). The same descriptors back the
//! OpenAPI manifest and the `/v1/agent_card` route — agents converge
//! on the same ground truth regardless of how they discover the
//! protocol.
//!
//! Every descriptor carries:
//!
//! - `name`           — wire-stable identifier (`emem_recall`, …).
//! - `title`          — human-readable title surfaced to the user via MCP.
//! - `description`    — one-sentence summary for the tool list.
//! - `when_to_use`    — natural-language trigger guidance for the LLM.
//! - `input_schema`   — JSON Schema (subset) of the request body.
//! - `example_args`   — paste-ready example arguments.
//! - `level`          — conformance level (L0/L1/L2).
//! - `category`       — Read / Write / Verify / Introspect / Plan.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Rich MCP tool descriptor. Backwards-compatible with the minimal MCP
/// `Tool` shape (name + description + inputSchema) but adds emem-specific
/// fields for richer agent guidance plus the four MCP behavioural
/// annotations the Anthropic Software Directory expects (`title`,
/// `readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Tool name (e.g. `"emem_recall"`).
    pub name: &'static str,
    /// Human-readable display name. Surfaced as the MCP `annotations.title`
    /// so hosts (Claude Desktop, Claude.ai connector picker, …) show a
    /// friendly label instead of the wire identifier.
    pub title: &'static str,
    /// One-sentence summary.
    pub description: &'static str,
    /// Natural-language trigger guidance the LLM uses to decide when to call.
    pub when_to_use: &'static str,
    /// JSON Schema of the request body.
    pub input_schema: &'static str,
    /// Paste-ready example arguments.
    pub example_args: &'static str,
    /// Required conformance level (L0 / L1 / L2).
    pub level: &'static str,
    /// Tool category for organisation.
    pub category: ToolCategory,
    /// MCP annotation: tool does not modify server-side state. `true` for
    /// every Read / Verify / Introspect / Plan primitive.
    pub read_only_hint: bool,
    /// MCP annotation: tool may make destructive changes. `true` only for
    /// L2 writes (`emem_attest`, `emem_challenge`).
    pub destructive_hint: bool,
    /// MCP annotation: repeated calls with the same args yield the same
    /// observable effect on the server side.
    pub idempotent_hint: bool,
    /// MCP annotation: tool interacts with an "open world" of external
    /// entities. `true` when the call may auto-fetch upstream imagery /
    /// OSM / weather; `false` for purely local introspection.
    pub open_world_hint: bool,
}

/// Tool category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    /// Read primitive over the cached/materialized fact store.
    Read,
    /// Write primitive (attest, challenge).
    Write,
    /// Verification primitive.
    Verify,
    /// Self-describing introspection — agents fetch protocol metadata.
    Introspect,
    /// Intent-routed planning primitive.
    Plan,
}

impl ToolCategory {
    /// MCP `annotations.readOnlyHint` value derived from category.
    /// Read, Introspect, Plan, and Verify never mutate server-side state.
    /// Write does (signs and persists facts).
    pub const fn read_only_hint(self) -> bool {
        matches!(
            self,
            Self::Read | Self::Introspect | Self::Plan | Self::Verify
        )
    }

    /// MCP `annotations.destructiveHint`. Only Write primitives may be
    /// considered destructive (they extend the signed ledger; the protocol
    /// itself is append-only, but downstream agents may treat new
    /// attestations as state changes that affect their reasoning).
    pub const fn destructive_hint(self) -> bool {
        matches!(self, Self::Write)
    }
}

const SCHEMA_RECALL: &str = r#"{"type":"object","required":["cell"],"properties":{
"cell":{"type":"string","description":"cell64 string, e.g. 'damO.zb000.xUti.zde78'"},
"band":{"type":"string","description":"optional single band key — convenience alias for bands:[band]. Use when you want exactly one band (e.g. 'geotessera.2020', 'modis.ndvi_mean') and would otherwise have to wrap it in an array. Both `band` and `bands` are accepted; if both are given they are merged."},
"bands":{"type":"array","items":{"type":"string"},"description":"optional band keys to filter, e.g. ['indices.ndvi','geotessera']"},
"tslot":{"type":"integer","description":"optional time slot (band-tempo-relative integer offset from emem epoch)"}
}}"#;

const SCHEMA_QUERY_REGION: &str = r#"{"type":"object","required":["geometry"],"properties":{
"geometry":{"type":"string","description":"cell64 string, or 'cells:c1,c2,c3'"},
"bands":{"type":"array","items":{"type":"string"}},
"agg":{"type":"string","enum":["mean","median","p90","vector_centroid"],"description":"optional per-band aggregation"}
}}"#;

const SCHEMA_COMPARE: &str = r#"{"type":"object","required":["a","b"],"properties":{
"a":{"type":"string","description":"cell64 of cell A"},
"b":{"type":"string","description":"cell64 of cell B"},
"family":{"type":"string","description":"optional band-key prefix (e.g. 'indices.')"}
}}"#;

const SCHEMA_COMPARE_BANDS: &str = r#"{"type":"object","required":["cell","a","b"],"properties":{
"cell":{"type":"string","description":"cell64 (`cell64` accepted as alias)"},
"a":{"type":"string","description":"band A key (e.g. 'copdem30m.elevation_mean')"},
"b":{"type":"string","description":"band B key (e.g. 'gmrt.topobathy_mean')"},
"tslot_a":{"type":"integer","minimum":0,"default":0,"description":"tslot for band A — default 0 (the static slot)"},
"tslot_b":{"type":"integer","minimum":0,"default":0,"description":"tslot for band B — default 0"},
"predicate":{"type":"object","description":"Optional consistency predicate. When set, the response carries a signed `verdict` (true|false|incomparable) over the comparison.","properties":{"kind":{"type":"string","enum":["abs_diff_le","abs_diff_lt","cosine_ge","cosine_gt","l2_distance_le"]},"threshold":{"type":"number"}},"required":["kind","threshold"]}
}}"#;

const SCHEMA_FIND_SIMILAR: &str = r#"{"type":"object","required":["key"],"properties":{
"key":{"type":"string","description":"cell64 (look up that cell's vector) or 'inline:[x,y,...]' literal vector"},
"k":{"type":"integer","minimum":1,"maximum":1000,"default":10},
"band":{"type":"string","default":"geotessera","description":"vector band to scan (default: 128-D Tessera foundation embedding). For mode=hamming/hamming_then_rerank you can pass either the cosine band (e.g. 'geotessera') or its binary sibling ('geotessera.bin128') — the responder picks the right one."},
"mode":{"type":"string","enum":["cosine","hamming","hamming_then_rerank"],"default":"cosine","description":"Scoring mode. cosine = fp32 over full vector (precise, ~256 B/cell scan). hamming = sign-bit popcount over the binary sibling band (~16 B/cell, ~1000× faster, ~65% recall@10). hamming_then_rerank = triage with Hamming on 4·k candidates then re-rank by cosine — matches cosine precision at ~16× less work."}
}}"#;

const SCHEMA_DIFF: &str = r#"{"type":"object","required":["cell","band","tslot_a","tslot_b"],"properties":{
"cell":{"type":"string"},
"band":{"type":"string"},
"tslot_a":{"type":"integer"},
"tslot_b":{"type":"integer"}
}}"#;

const SCHEMA_TRAJECTORY: &str = r#"{"type":"object","required":["cell","band","window"],"properties":{
"cell":{"type":"string"},
"band":{"type":"string"},
"window":{"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":2,"description":"[start_tslot, end_tslot] inclusive"}
}}"#;

const SCHEMA_VERIFY: &str = r#"{"type":"object","required":["claim","cell"],"properties":{
"cell":{"type":"string"},
"mode":{"type":"string","enum":["fast","resolve","zk"],"default":"fast"},
"claim":{"type":"object","required":["band","op","value"],"properties":{
  "band":{"type":"string"},
  "op":{"type":"string","enum":["eq","ne","lt","le","gt","ge","in","ni","exists","absent"]},
  "value":{},
  "tslot":{"type":"integer"},
  "window":{"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":2},
  "agg":{"type":"string","enum":["any","all","mean","min","max"]}
}}
}}"#;

const SCHEMA_INTENT: &str = r#"{"type":"object","required":["type"],"properties":{
"type":{"type":"string","enum":["where_is","what_is_here","is_like","did_change","find_like","confirm"]},
"description":{"type":"string"},
"cell":{"type":"string"},
"a":{"type":"string"},"b":{"type":"string"},
"band":{"type":"string"},
"window":{"type":"array","items":{"type":"integer"}},
"key":{"type":"string"},"k":{"type":"integer"},
"claim":{"type":"object"}
}}"#;

const SCHEMA_NONE: &str = r#"{"type":"object","properties":{}}"#;

// Anthropic's tool input_schema validator (consumed by Claude.ai
// connectors and the Anthropic API `tools` array) only accepts the
// JSON-Schema subset {type, properties, required} at the root. Top-level
// `anyOf`/`oneOf`/`allOf` and a top-level `description` cause
// `tools.<idx>.custom.input_schema` 400s when Claude Code adds the
// connector. The "exactly one location" requirement therefore lives in
// the per-property descriptions (LLMs respect description text) plus a
// soft-envelope safety net at the handler — the schema itself stays
// strictly within the Anthropic-accepted subset.
const SCHEMA_LOCATE: &str = r#"{"type":"object","properties":{
"place":{"type":"string","description":"Free-text place name (e.g. 'Mount Everest', 'Tokyo'). REQUIRED unless `lat`+`lng` is provided. Aliases also accepted: `q`, `query`, `name`."},
"q":{"type":"string","description":"Alias for `place` — accepted because OSM/Mapbox/Google Geocoding all use `q`. Provide either this or `place` (or `lat`+`lng`)."},
"lat":{"type":"number","description":"WGS-84 latitude in degrees, paired with `lng`. REQUIRED with `lng` unless `place`/`q` is provided."},
"lng":{"type":"number","description":"WGS-84 longitude in degrees, paired with `lat`. REQUIRED with `lat` unless `place`/`q` is provided."}
}}"#;

const SCHEMA_ASK: &str = r#"{"type":"object","required":["q"],"properties":{
"q":{"type":"string","description":"User's natural-language question about the place (e.g. \"is this neighbourhood flood-prone\")."},
"place":{"type":"string","description":"Free-text place name (e.g. \"Mount Fuji\", \"Ashok Nagar, Ranchi\"). REQUIRED unless `cell` or `lat`+`lng` is provided. Extract the noun phrase from the user's turn; the responder geocodes via OSM Nominatim."},
"cell":{"type":"string","description":"cell64 string (alternative to `place` — use when you have one from a prior emem_locate / emem_recall response). Provide this OR `place` OR `lat`+`lng`."},
"lat":{"type":"number","description":"WGS-84 latitude (paired with `lng`; alternative to `place` / `cell`)."},
"lng":{"type":"number","description":"WGS-84 longitude (paired with `lat`)."},
"include_image":{"type":"boolean","default":false,"description":"Bundle a Sentinel-2 RGB scene URL for the resolved cell. Adds ~1-2 s on first call."},
"verbose":{"type":"boolean","default":false,"description":"When true, return the full envelope: per-algorithm formula strings, temporal_recipe blocks, per-fact band_metadata duplicates, and the long _explanation prose. Default (since 2026-05-05) is false so the response fits MCP's 25 KB cap; the signed receipt + fact CIDs + algorithm keys + algorithms_cid are always retained. Pass true to get the full body when debugging."}
}}"#;

const SCHEMA_RECALL_POLYGON: &str = r#"{"type":"object","properties":{
"place":{"type":"string","description":"Free-text place name; resolved through the layered geocoder. REQUIRED unless `polygon_bbox` is provided."},
"polygon_bbox":{"type":"object","properties":{
  "min_lat":{"type":"number"},"max_lat":{"type":"number"},
  "min_lng":{"type":"number"},"max_lng":{"type":"number"}
}, "description":"Explicit polygon bbox; alternative to `place` when caller already has coordinates. REQUIRED unless `place` is provided."},
"bands":{"type":"array","items":{"type":"string"},"description":"Bands to recall at each fan-out cell."},
"tslot":{"type":"integer"},
"max_cells":{"type":"integer","minimum":1,"maximum":256,"default":64,"description":"Cap on cells sampled from the polygon."}
}}"#;

const SCHEMA_GRID_INFO: &str = r#"{"type":"object","properties":{}}"#;
const SCHEMA_COVERAGE_MATRIX: &str = r#"{"type":"object","properties":{}}"#;

const SCHEMA_FETCH: &str = r#"{"type":"object","required":["cid"],"properties":{
"cid":{"type":"string","description":"Content-address of any persisted fact (Primary or Absence). Returned by every recall, attest, materialize, and verify call as `fact_cid` / `fact_cids`."}
}}"#;

const SCHEMA_BACKFILL: &str = r#"{"type":"object","required":["cell","band"],"properties":{
"cell":{"type":"string","description":"cell64 or place name (auto-resolved)."},
"band":{"type":"string","description":"Band key. Must be a band whose materializer supports historical fetch — see `emem_coverage_matrix` field `history_available_from`/`history_available_to`."},
"start_unix":{"type":"integer","description":"Window start as Unix epoch seconds (UTC). Defaults to the band's `history_available_from`."},
"end_unix":{"type":"integer","description":"Window end as Unix epoch seconds (UTC). Defaults to now."},
"max_facts":{"type":"integer","minimum":1,"maximum":1024,"default":64,"description":"Cap on number of facts materialized in one call."}
}}"#;

const SCHEMA_HEAT_SOLVE: &str = r#"{"type":"object","required":["cell"],"properties":{
"cell":{"type":"string","description":"cell64 string. Forecast LST evolution at this cell."},
"hours_ahead":{"type":"number","default":6,"description":"Forecast horizon in hours; capped at 168 (one week)."},
"diffusivity_m2_per_s":{"type":"number","default":1.0e-6,"description":"Thermal diffusivity α (m²/s). Default urban surface (Oke 2017 §2.3); use ~5e-7 for vegetation, ~1.4e-7 for water."}
}}"#;

const SCHEMA_WAVE_SOLVE: &str = r#"{"type":"object","required":["coastal_cell","offshore_height_m","period_s"],"properties":{
"coastal_cell":{"type":"string","description":"cell64 of the coastal destination."},
"offshore_height_m":{"type":"number","minimum":0,"maximum":30,"description":"Offshore significant wave height H_s (m)."},
"period_s":{"type":"number","minimum":2,"maximum":30,"description":"Wave period (s); typical wind-wave + swell envelope is 6-18 s."},
"n_offshore_cells":{"type":"integer","minimum":1,"maximum":64,"default":8,"description":"Cells to sample seaward when building the bathymetric profile."}
}}"#;

const SCHEMA_JEPA_PREDICT: &str = r#"{"type":"object","required":["cell"],"properties":{
"cell":{"type":"string","description":"cell64 to forecast at."},
"band":{"type":"string","default":"indices.ndvi","description":"Band to forecast. v1 supports 'indices.ndvi' only."},
"lookback_months":{"type":"integer","minimum":1,"maximum":24,"default":6,"description":"How many past months of history to read."},
"forecast_horizon_months":{"type":"integer","minimum":1,"maximum":1,"default":1,"description":"Horizon in months ahead. v1 supports 1 only."}
}}"#;

/// Normative tool inventory, with rich agent-facing metadata.
pub const TOOLS: &[ToolDescriptor] = &[
    // ── Geocoder (must be first — every other primitive needs cell64) ──
    ToolDescriptor {
        name: "emem_locate",
        title: "Resolve place to cell64 + band inventory",
        description: "Resolves a place mention (free-text name, address, or lat/lng) to the protocol's cell64 identifier, and returns the topic-grouped inventory of bands and algorithms available at that location.",
        when_to_use: "Use whenever the input refers to a real-world location and the next step needs the cell64 identifier or wants to know which bands are available before recalling. The response carries `data_at_this_cell` with three sub-fields: `live_bands_by_topic` (every band recallable here, grouped by topic such as flood_water_event_window, vegetation_condition, built_up_human_geography), `algorithms_for_topic` (composition recipes that fuse those bands into named scores), and `declared_but_no_materializer_at_this_responder` (cube slots reserved without a live connector). For the single-shot path that runs the full chain server-side and returns one packaged answer, use `emem_ask` instead.",
        input_schema: SCHEMA_LOCATE,
        example_args: r#"{"place":"Mount Everest"}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },
    ToolDescriptor {
        name: "emem_ask",
        title: "Ask a free-text question about a place",
        description: "Single-shot free-text answer about a real-world location, backed by signed satellite/elevation/water/built-up receipts. Forwards a place mention plus a question; runs the locate → recall → algorithm chain server-side; returns one packaged envelope.",
        when_to_use: "Use when the question concerns a specific real-world place and a packaged, citation-bearing answer is preferable to manual primitive composition. Forward the user's question verbatim as `q` plus the location as `place` (free text), `cell` (cell64), or `lat`+`lng`. The server resolves the location, classifies the question to a topic, recalls every relevant band (auto-materializing Sentinel-2 / Sentinel-1 / Cop-DEM / JRC GSW / Overture / weather on miss), surfaces the algorithm recipes that compose those bands into named scores, and returns a single envelope with `topic_routing`, `facts`, `algorithms_for_question`, an optional Sentinel-2 RGB scene URL, and a `caveats` block (grid resolution, revisit cadence). All facts are signed by the responder; the receipt's `fact_cids` are content-addressed and citable. Set `include_image: true` to bundle the latest cloud-free Sentinel-2 thumbnail. Out-of-scope questions return `topic_routing.matched_topic: null` plus the full inventory so the caller can route elsewhere.",
        input_schema: SCHEMA_ASK,
        example_args: r#"{"q":"is this neighbourhood flood-prone for a flat purchase","place":"Ashok Nagar, Ranchi"}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },
    // ── Read primitives ──────────────────────────────────────────────
    ToolDescriptor {
        name: "emem_recall",
        title: "Recall facts at a cell (auto-materializes on miss)",
        description: "Recall facts about a cell — auto-materializes on miss for any band with a registered materializer.",
        when_to_use: "Call after `emem_locate` (or with a known cell64). Returns every Primary fact stored at that (cell, band, tslot). IMPORTANT: if the cell has no fact yet for a requested band AND that band has `has_materializer=true` (per `emem_coverage_matrix` / `emem_materializers`), the responder fetches the upstream value, signs it under its identity, persists it, and returns it in the same response (~180 ms first call, ~10 ms cached thereafter). So for any wired band you can recall ANY cell on Earth without seeding — just pass `bands: [<band>]`. The response carries `materialize_notes` listing what was just fetched. Empty result with no notes means the band has no materializer at this responder.",
        input_schema: SCHEMA_RECALL,
        example_args: r#"{"cell":"damO.zb000.xUti.zde78","bands":["weather.temperature_2m","copdem30m.elevation_mean"]}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },
    ToolDescriptor {
        name: "emem_recall_polygon",
        title: "Recall facts across a place's polygon",
        description: "Recall facts across every cell inside a place's polygon (single signed envelope). Closes the place-name-drift gap for wide features (parks, lakes, regions).",
        when_to_use: "Call when the user names a wide feature (national park, river basin, country, large urban area) where one cell is too small. Pass `place` and the geocoder will fan out across the polygon — or pass `polygon_bbox` directly if you have coordinates. Returns `merged_facts`, `by_cell`, and a `polygon_bbox.source` indicator (`nominatim_boundingbox` = real polygon, `centre_cell_bbox` = fallback to one cell because the geocoder had no polygon).",
        input_schema: SCHEMA_RECALL_POLYGON,
        example_args: r#"{"place":"Yellowstone National Park","bands":["copdem30m.elevation_mean"],"max_cells":8}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },
    ToolDescriptor {
        name: "emem_query_region",
        title: "Aggregate facts over a region",
        description: "Query facts over a region (single cell or list of cells), optionally aggregated per band.",
        when_to_use: "Call when the user asks 'how does region X look', 'what's the average NDVI here', or wants a region-level summary. Use `agg=mean|median|p90|vector_centroid` to fold per-band values.",
        input_schema: SCHEMA_QUERY_REGION,
        example_args: r#"{"geometry":"cells:damO.zb000.xUti.zde78,damO.zb000.xUto.sisA","agg":"mean"}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },
    ToolDescriptor {
        name: "emem_compare",
        title: "Compare two cells (cosine + scalar deltas)",
        description: "Compare two cells: cosine similarity over shared vector bands + per-band scalar deltas.",
        when_to_use: "Call when the user asks 'how similar is X to Y', 'compare these two places', or wants a difference vector. Returns a single cosine score and per-band deltas.",
        input_schema: SCHEMA_COMPARE,
        example_args: r#"{"a":"damO.zb000.xUti.zde78","b":"damO.zb000.xUto.sisA"}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_compare_bands",
        title: "Compare two bands at one cell",
        description: "Compare two bands at the same cell. Scalar pair → metric=delta, value=b-a. Vector pair (equal dim) → metric=cosine + per-dim delta. Returns a signed receipt naming both source fact CIDs.",
        when_to_use: "Call when the user wants cross-source consistency at one place ('does Cop-DEM agree with GMRT here?'), cross-vintage drift ('how did the embedding change between 2017 and 2024 at this cell?'), or any band-vs-band comparison within a single cell. `cell` + `a` + `b` are required; `tslot_a`/`tslot_b` default to 0 (the static slot used by Cop-DEM, GMRT, ESA WorldCover, etc.).",
        input_schema: SCHEMA_COMPARE_BANDS,
        example_args: r#"{"cell":"damO.zb000.wapu.yAxe","a":"copdem30m.elevation_mean","b":"gmrt.topobathy_mean"}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_find_similar",
        title: "k-NN over the corpus by embedding",
        description: "k-NN over the corpus by cell embedding or inline vector.",
        when_to_use: "Call when the user asks 'find places like X', 'where else looks like this', or hands an embedding to find neighbours. `key` is either a cell64 or `inline:[x,y,...]`. Default band is `geotessera` (128-D Tessera foundation embedding); pass `band: \"geotessera.multi_year\"` for the 1024-D 8-vintage fusion.",
        input_schema: SCHEMA_FIND_SIMILAR,
        example_args: r#"{"key":"damO.zb000.xUti.zde78","k":10}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_trajectory",
        title: "Time series for one (cell, band)",
        description: "Time series for one (cell, band) over an inclusive [start, end] tslot window. Returns only what's already attested — does NOT trigger materialization. For historical backfill use `emem_backfill`.",
        when_to_use: "Call when the user asks 'how did X change over time' for a band that already has multiple historical tslots seeded. IMPORTANT differences from `emem_recall`: (1) trajectory does NOT auto-materialize past tslots — it returns only facts that have already been attested at this responder, so for fast-tempo bands like `indices.ndwi` you'll typically see ONE point at the latest tslot until an attester seeds history. (2) tslots are non-negative `u64`; there's no negative-offset 'last 2 years' shorthand. For LONG-TERM history questions ('flooded in last 2 years', 'forest loss since 2020') prefer either (a) a static-tempo summary band that one fact answers — `surface_water.recurrence` covers 1984-2021 in a single signed value, no trajectory needed — or (b) `emem_backfill` to materialize and sign the missing tslots in one call.",
        input_schema: SCHEMA_TRAJECTORY,
        example_args: r#"{"cell":"damO.zb000.xUti.zde78","band":"indices.ndvi","window":[0,12]}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_diff",
        title: "Signed delta between two tslots",
        description: "Compute a DerivativeFact (delta) between a band's values at two tslots.",
        when_to_use: "Call when the user asks 'what changed between t1 and t2', 'give me the delta'. Returns a signed DerivativeFact + receipt — the delta itself is content-addressed and citable.",
        input_schema: SCHEMA_DIFF,
        example_args: r#"{"cell":"damO.zb000.xUti.zde78","band":"indices.ndvi","tslot_a":0,"tslot_b":12}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_fetch",
        title: "Resolve a fact by content-address (CID)",
        description: "Fetch a fact by its content-address (CID). Returns the full signed Primary or Absence fact — the same body served by REST `/v1/facts/{cid}`. Closes the citation loop: any fact_cid surfaced by recall, materialize, attest, or verify can be re-resolved by another agent without REST.",
        when_to_use: "Call whenever you have a `fact_cid` (e.g. from `emem_recall`'s response, an `emem_attest` receipt, an `emem_materializers` outcome, or a citation in another agent's reply) and need the full fact body — its value, unit, sources, signer, signed_at, and derivation. Particularly useful for verifying that a citation a downstream agent gave you actually resolves on this responder. The response is byte-identical across responders for the same CID — the CID itself is the validator.",
        input_schema: SCHEMA_FETCH,
        example_args: r#"{"cid":"qbq2dy7adyuvozs7s3gqg5jnpkcwq2duegltjyhbxsivuqbpjofq"}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_backfill",
        title: "Materialize historical facts in a window",
        description: "Materialize and sign every per-tslot fact for one (cell, band) inside a [start_unix, end_unix] window. Returns a signed list of (tslot, fact_cid, status) for each step. Slow but possible — one upstream fetch per tslot, capped by `max_facts`.",
        when_to_use: "Call when the user wants HISTORY for a fast/medium-tempo band and `emem_trajectory` returned only the latest point. The responder iterates the tslot range derived from the band's tempo, calls the per-tslot historical materializer, signs each result, and persists. After completion `emem_trajectory` over the same window returns the full series. Bands without a historical materializer (e.g. `weather.*` from met.no's nowcast) return `status: \"present_only\"` for past tslots — check `emem_coverage_matrix.history_available_from`/`history_available_to` to see how far back each band can be backfilled. Prefer this over staking an attestation when the upstream is publicly fetchable.",
        input_schema: SCHEMA_BACKFILL,
        example_args: r#"{"cell":"damO.zb000.xUti.zde78","band":"modis.ndvi_mean","start_unix":1640995200,"end_unix":1735689600,"max_facts":24}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },

    // ── Physics primitives — explicit-FD PDE solvers + JEPA-pattern predictor ──
    ToolDescriptor {
        name: "emem_heat_solve",
        title: "2-D heat-equation forecast (urban LST evolution)",
        description: "Forward-step 2-D explicit finite-difference solver for the heat equation ∂u/∂t = α∇²u over a 3×3 cell stencil centred on `cell`. Reads `modis.lst_day_8day` (Land Surface Temperature) at the centre and 8 cell64 neighbours, integrates N hours ahead under a CFL-stable timestep, returns a signed forecast. Real PDE rollout — not a decay-scoring heuristic.",
        when_to_use: "Use when the user wants a short-horizon LST forecast (urban heat island, surface-temperature evolution, heatwave onset modelling) at a specific cell. Default α=1e-6 m²/s matches urban surface diffusivity (Oke 2017); pass a smaller α for water bodies or higher for vegetated surfaces. The solver caps at one-week horizons because the 8-day MODIS composite stops being a representative initial condition past that. Each call materialises 9 MODIS facts (one per neighbour) on miss — first call ~5 s cold, ~30 ms warm. Receipt cites all 9 input fact CIDs.",
        input_schema: SCHEMA_HEAT_SOLVE,
        example_args: r#"{"cell":"damO.zb000.xUti.zde78","hours_ahead":6}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },
    ToolDescriptor {
        name: "emem_wave_solve",
        title: "1-D shallow-water swell propagation to coast",
        description: "Forward-step 1-D explicit finite-difference solver for the shallow-water wave equation ∂²u/∂t² = c²∂²u/∂x² with c² = g·h, where depth h comes from `gmrt.topobathy_mean` along the seaward gradient. Models how an offshore swell of height H_s and period T propagates toward `coastal_cell`. Returns arrival height + time + depth + phase-speed profiles, all under a CFL-stable timestep.",
        when_to_use: "Use when the user wants to predict swell arrival at a coast (storm-surge planning, shoreline-impact assessment, surf forecasting). The solver walks `n_offshore_cells` cells seaward from `coastal_cell` along the bathymetric gradient (default 8 cells = 80 m of profile at the active 10 m grid), samples GMRT depth at each, and integrates the wave equation forward until the wavefront reaches the coast plus one period. Receipt cites every depth fact CID along the profile. Returns 422 with a clear message if `coastal_cell` is land-locked.",
        input_schema: SCHEMA_WAVE_SOLVE,
        example_args: r#"{"coastal_cell":"damO.zb000.xUti.zde78","offshore_height_m":2.0,"period_s":8.0}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },
    ToolDescriptor {
        name: "emem_jepa_predict",
        title: "Constrained JEPA-pattern next-month NDVI predictor",
        description: "Predict next-month NDVI at a cell using a constrained JEPA-pattern AR(2) seasonal predictor. Reads up to 24 past months of `indices.ndvi`, fits a closed-form predictor `y_{t+1} = α·(lag-12 NDVI or recent mean) + β·(last + slope) + γ·recent_mean`, returns the prediction clamped to NDVI's physical range. Coefficients (α=0.6, β=0.3, γ=0.1) are NOT learned — they're fixed from the agricultural-NDVI literature. v2 (future) will train an actual encoder + predictor on the geotessera embedding pool.",
        when_to_use: "Use when the user wants a one-month-ahead NDVI forecast at a specific cell (crop-stress monitoring, growing-season tracking, vegetation-anomaly anticipation). Lookback defaults to 6 months; if fewer monthly tslots are attested at this cell, the predictor uses what's there and surfaces the count in `lookback_months_used`. Returns 422 if no NDVI history exists at the cell — chain to `emem_backfill` first to seed history. Receipt cites every input NDVI fact CID.",
        input_schema: SCHEMA_JEPA_PREDICT,
        example_args: r#"{"cell":"damO.zb000.xUti.zde78","lookback_months":6}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },

    // ── Verify / write ───────────────────────────────────────────────
    ToolDescriptor {
        name: "emem_verify",
        title: "Verify a structured claim against a cell",
        description: "Verify a structured claim against a cell's facts. Returns verdict + evidence CIDs + signed receipt.",
        when_to_use: "Call when the user asks a yes/no question about a cell ('is the NDVI > 0.7 here', 'has this been deforested'), or when downstream code wants citable evidence for a logical predicate.",
        input_schema: SCHEMA_VERIFY,
        example_args: r#"{"cell":"damO.zb000.xUti.zde78","claim":{"band":"indices.ndvi","op":"gt","value":0.5,"tslot":0}}"#,
        level: "L1", category: ToolCategory::Verify,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    // L2 write surfaces (`emem_attest`, `emem_challenge`) are intentionally
    // NOT exposed as MCP tools because they require an ed25519 attester key
    // an LLM-driven host cannot generate (the signing happens client-side).
    // Advertising them caused every Claude.ai connector-onboarding tile
    // click to error with "unknown tool" because no dispatch arm could
    // accept a CBOR-encoded Attestation envelope through MCP's JSON-only
    // tool-call path. Authorized writers continue to use the REST/CBOR
    // routes directly: POST /v1/attest, POST /v1/attest_cbor, POST
    // /v1/challenge — all documented in /openapi.json + /agents.md.

    // ── Introspection ────────────────────────────────────────────────
    ToolDescriptor {
        name: "emem_bands",
        title: "Active band ontology",
        description: "Active band ontology (offsets, dims, tempo, privacy).",
        when_to_use: "Call once at session start to learn the band registry — every other primitive's `band` argument MUST come from this list.",
        input_schema: SCHEMA_NONE,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_functions",
        title: "Active function registry",
        description: "Active function registry (derivation recipes).",
        when_to_use: "Call when you need to know which derivative ops are available for `emem_diff` or how a band is computed from upstream sources.",
        input_schema: SCHEMA_NONE,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_sources",
        title: "Active source-connector registry",
        description: "Active source-connector registry (URL templates, providers, licenses).",
        when_to_use: "Call when you need to inspect which upstream EO providers are wired (Copernicus DEM, JRC GSW, ESA WorldCover, etc.) — useful for license attribution in agent answers.",
        input_schema: SCHEMA_NONE,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_schema",
        title: "Active CDDL/JSON schema bundle",
        description: "Active CDDL/JSON schema bundle by CID.",
        when_to_use: "Rarely needed at chat time. Useful for offline verification of receipts / attestations against the exact schema version a responder used.",
        input_schema: SCHEMA_NONE,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_errors",
        title: "Stable error code catalog",
        description: "Stable error code catalog.",
        when_to_use: "Call to enumerate the wire-stable error codes — useful when the LLM wants to programmatically branch on responses.",
        input_schema: SCHEMA_NONE,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_manifests",
        title: "Active manifest CIDs",
        description: "Active manifest CIDs (bands / functions / sources / schema).",
        when_to_use: "Call to learn which exact registry versions a responder is serving. Cite these CIDs alongside any answer where reproducibility matters.",
        input_schema: SCHEMA_NONE,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_grid_info",
        title: "Active grid encoding",
        description: "Active grid encoding: cell64 ground resolution, lat/lng axis sizes, DGGS lineage.",
        when_to_use: "Call once at session start (or when the user asks about cell resolution / 'how big is a cell'). Returns the actual ground resolution today (~9.54 m × 9.55 m square at the equator (lat 21 bits × lng 22 bits, matching Sentinel-1/Sentinel-2 native pixel pitch). The cell64 bit layout reserves a resolution-tag field for future hierarchical refinement targeting H3-equivalent res-13 (~3.4 m) cells in v0.1.) and the spec target. Useful before you reason about whether one cell is enough or whether you need `emem_recall_polygon`.",
        input_schema: SCHEMA_GRID_INFO,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_coverage_matrix",
        title: "Per-band live status & history bounds",
        description: "Per-band live status — what data is alive AND auto-materializable, with history bounds, tempo cadence, and the responder pubkey that signs the band.",
        when_to_use: "Call BEFORE `emem_recall` when you don't know which bands answer at this responder. For each band returns `has_materializer` (true → an empty recall will auto-fetch+sign, no seeding needed), `facts_count` (how many cells already cached), `last_attested_unix_s` (freshness), `tempo_seconds` (slot duration), `history_available_from` / `history_available_to` (oldest/newest Unix epoch the materializer can fetch — use these to bound an `emem_backfill` request), and `responder_pubkey_b32` (the ed25519 key whose signature attests this band — use to detect federation / multi-responder setups). Bands with `has_materializer=false AND facts_count=0` are cube placeholders without a wired connector — don't bother recalling them.",
        input_schema: SCHEMA_COVERAGE_MATRIX,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_materializers",
        title: "Auto-fetch registry (per-band materializers)",
        description: "Auto-fetch registry: which bands the responder will materialize on a recall miss, the upstream provider, license, value shape, and history bounds.",
        when_to_use: "Call once at session start (alongside `emem_bands` and `emem_coverage_matrix`) to learn which bands answer for ANY cell on Earth without seeding. Each entry declares `upstream_scheme`, `upstream_endpoint`, `derivation_fn_key`, `value_kind` (primary | absence | primary_or_absence), `coverage` (where the upstream has data), `unit`, `tempo`, `confidence`, and `history_available_from` / `history_available_to` (when the upstream supports historical fetch via `emem_backfill`). Use this when the user asks 'do you have flood data here', 'what providers feed this', or you need license attribution. The response also carries an `agent_hint` block explaining the trust model (responder signs, not upstream) and the absence-fact contract.",
        input_schema: SCHEMA_NONE,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_data_availability",
        title: "Per-band temporal coverage catalog",
        description: "Temporal catalog: for every materializable band the upstream-of-record window the data genuinely covers, the temporal `kind` (static | annual_snapshot | annual_stack | time_series | now_only | per_release), tempo seconds, upstream wire path, and whether `emem_backfill` is meaningful.",
        when_to_use: "Call before `emem_backfill` or any historical recall to check whether a band has a meaningful past at the requested time. Each entry includes `history_available_from_unix` / `history_available_to_unix` (and ISO strings) plus `backfill_supported`. Use this to avoid trial-and-error 422s on now-only bands (`weather.*`) and to enumerate the per-year `geotessera.YYYY` vintages the responder ships. The catalog is driven by the same registry the recall path consults — so what it lists is exactly what materializes.",
        input_schema: SCHEMA_NONE,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_algorithms",
        title: "Composition recipes (algorithms)",
        description: "Content-addressed dictionary of composition recipes — formulas that fuse attested band facts (and embeddings) into derived scores, classifications, and similarity metrics.",
        when_to_use: "Call when the user's question is COMPOSITE (flood risk, urban density, water consensus, change-since-2020) rather than a single band readout. Each entry has `kind` (solo | combined | embedding), the input `bands` (assemble one `emem_recall` body from them), the `formula` in plain math, the `output` shape, and a `citation`. The agent applies the formula in-process and quotes the algorithm key + `algorithms_cid` (from `emem_manifests`) alongside the input fact_cids — that gives the receipt enough context for any other operator to replay the same composition deterministically. Embedding entries (cosine, novelty, change, neighborhood-consistency) operate on `geotessera`; for the most common k-NN pattern the protocol-native `emem_find_similar` is faster than fetching vectors and computing locally.",
        input_schema: SCHEMA_NONE,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_coverage_map",
        title: "Coverage map (SVG image)",
        description: "Live SVG render of the responder's corpus density, returned as a proper MCP EmbeddedResource content block (image/svg+xml) — multimodal MCP agents can render it natively.",
        when_to_use: "Call when the user asks 'where do you have data?', 'show me the coverage', or wants a visual brief of the responder's corpus footprint. Returns a 1440×720 Plate-Carrée SVG (1° × 1° bins, log-scale colour, continent envelopes for orientation) plus a structuredContent summary (cell_count, total_facts, responder pubkey, REST URL). Multi-content-block reply: an EmbeddedResource (mimeType `image/svg+xml`, with text + uri) followed by a one-line text summary so text-only clients still see the cell / fact counts. For the bare image bytes, fetch `/v1/coverage_map.svg` over plain REST.",
        input_schema: SCHEMA_NONE,
        example_args: r#"{}"#,
        level: "L0", category: ToolCategory::Introspect,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },
    ToolDescriptor {
        name: "emem_cell_scene_rgb",
        title: "Sentinel-2 true-colour thumbnail (PNG)",
        description: "True-colour Sentinel-2 L2A RGB thumbnail centred on a cell. PNG returned as a native MCP ImageContent block (mimeType image/png). Pure-Rust pipeline: STAC search + HTTP-Range COG reads + 2-98 percentile stretch + PNG encode.",
        when_to_use: "Call when the user wants a VISUAL of a place — 'show me what this looks like', 'before/after the flood', 'is there a forest here', 'is this developed'. Returns a 256×256 px RGB image (~2.56 km × ~2.56 km at S2's 10 m native resolution), centred on the cell. Pass `cell` as a cell64 string OR a place name (auto-resolved). `max_cloud` filters scenes by `eo:cloud_cover` (default 20 %); raise it (60–80 %) for cloud-prone tropics if you keep getting 'no scene' errors. `datetime` is an RFC 3339 interval like `\"2024-01-01T00:00:00Z/2024-12-31T00:00:00Z\"` for a temporal slice (defaults to last 90 days). `structuredContent` carries the STAC item id, capture time, cloud_cover, EPSG, and per-channel reflectance percentile stretch values used — quote those alongside the image so the receipt is reproducible.",
        input_schema: r#"{"type":"object","properties":{"cell":{"type":"string","description":"cell64 or place name"},"max_cloud":{"type":"number","default":20,"description":"max eo:cloud_cover percent"},"datetime":{"type":"string","description":"RFC 3339 interval; defaults to last 90 days"}},"required":["cell"]}"#,
        example_args: r#"{"cell":"damO.zb000.waro.zcb89","max_cloud":20}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },
    ToolDescriptor {
        name: "emem_cell_geojson",
        title: "Cell polygon as GeoJSON",
        description: "Cell polygon as a native MCP EmbeddedResource (mimeType application/geo+json). Properties carry centre lat/lng, bbox, approx size in metres, and the 8-cell neighbourhood — drop straight into Mapbox / Leaflet / Deck.gl / QGIS without a GIS pipeline.",
        when_to_use: "Call when the agent (or a downstream renderer) needs the cell as geographic geometry — for map overlays, polygon-clipping ops, or feeding a styling pipeline. Pass `cell` as cell64 or place name. The result is a GeoJSON Feature with Polygon geometry; for a FeatureCollection that includes every recalled fact's value as a property, fetch /v1/cells/{cell64}/recall_geojson?bands=... over plain REST instead.",
        input_schema: r#"{"type":"object","properties":{"cell":{"type":"string","description":"cell64 or place name"}},"required":["cell"]}"#,
        example_args: r#"{"cell":"damO.zb000.waro.zcb89"}"#,
        level: "L0", category: ToolCategory::Read,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: false,
    },

    // ── Intent-routed planner ────────────────────────────────────────
    ToolDescriptor {
        name: "emem_intent",
        title: "Intent-routed planner",
        description: "Submit a typed Intent; receive a plan or executed result.",
        when_to_use: "Call when the user asks something like 'where is X' or 'is A like B' and you don't want to pick a primitive yourself — the planner maps Intent variants to the right tool call.",
        input_schema: SCHEMA_INTENT,
        example_args: r#"{"type":"what_is_here","cell":"damO.zb000.xUti.zde78"}"#,
        level: "L0", category: ToolCategory::Plan,
    read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true,
    },
];

/// Look up a tool descriptor by name.
pub fn lookup(name: &str) -> Option<&'static ToolDescriptor> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Tools at or below a given level (`"L0"` returns L0 only; `"L2"` returns all).
pub fn tools_at_level(level: &str) -> Vec<&'static ToolDescriptor> {
    let max = match level {
        "L0" => 0,
        "L1" => 1,
        "L2" => 2,
        _ => 0,
    };
    TOOLS
        .iter()
        .filter(|t| {
            let n = match t.level {
                "L0" => 0,
                "L1" => 1,
                "L2" => 2,
                _ => 99,
            };
            n <= max
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn introspection_tools_present() {
        for t in &[
            "emem_bands",
            "emem_functions",
            "emem_sources",
            "emem_algorithms",
            "emem_schema",
            "emem_errors",
            "emem_manifests",
        ] {
            assert!(lookup(t).is_some(), "missing introspection tool: {t}");
        }
    }

    #[test]
    fn level_filter_works() {
        let l0 = tools_at_level("L0");
        let l2 = tools_at_level("L2");
        assert!(l0.len() < l2.len());
    }

    #[test]
    fn every_tool_has_when_to_use() {
        for t in TOOLS {
            assert!(!t.when_to_use.is_empty(), "missing when_to_use: {}", t.name);
            assert!(
                !t.input_schema.is_empty(),
                "missing input_schema: {}",
                t.name
            );
            assert!(
                !t.example_args.is_empty(),
                "missing example_args: {}",
                t.name
            );
            assert!(!t.title.is_empty(), "missing title: {}", t.name);
            // Title length cap keeps MCP UI surfaces clean and stays well
            // under any reasonable client truncation.
            assert!(
                t.title.len() <= 80,
                "title too long ({} chars): {}",
                t.title.len(),
                t.name
            );
        }
    }

    #[test]
    fn newly_added_tools_present() {
        assert!(
            lookup("emem_fetch").is_some(),
            "emem_fetch must be registered"
        );
        assert!(
            lookup("emem_backfill").is_some(),
            "emem_backfill must be registered"
        );
    }

    #[test]
    fn tool_names_match_anthropic_regex() {
        // Anthropic's hosted MCP frontend rejects names that don't match
        // ^[a-zA-Z0-9_-]{1,64}$. Enforce here so we never regress.
        for t in TOOLS {
            assert!(
                t.name.len() <= 64
                    && t.name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
                "tool name '{}' violates Anthropic naming regex",
                t.name,
            );
        }
    }

    #[test]
    fn category_annotation_hints_are_consistent() {
        // Read/Introspect/Plan/Verify must be read-only; only Write may be
        // destructive. This invariant is what we expose to MCP clients via
        // annotations.{readOnlyHint,destructiveHint}.
        for t in TOOLS {
            match t.category {
                ToolCategory::Read
                | ToolCategory::Introspect
                | ToolCategory::Plan
                | ToolCategory::Verify => {
                    assert!(t.category.read_only_hint(), "{} must be read-only", t.name);
                    assert!(
                        !t.category.destructive_hint(),
                        "{} must not be destructive",
                        t.name
                    );
                }
                ToolCategory::Write => {
                    assert!(
                        !t.category.read_only_hint(),
                        "{} must not be read-only",
                        t.name
                    );
                    assert!(
                        t.category.destructive_hint(),
                        "{} must be destructive",
                        t.name
                    );
                }
            }
        }
    }
}
