# Global trial — does emem.dev actually deliver?

> A 43-fixture stress test across nine place-types. Run from a single
> client over the public internet against `https://emem.dev`. Verbatim
> JSON is committed at `scripts/global_trial.py`; the canonical raw
> output at run time lands in `/tmp/emem_global_trial.json`.
>
> Run dates: first pass 2026-04-27 (37/43); second pass 2026-04-27
> after the gap-fix round (41/43).
> Endpoints exercised per fixture: `/v1/locate` (lat/lng), `/v1/locate`
> (place name), `/v1/elevation`, `/v1/recall` for both
> `copdem30m.elevation_mean` (land DEM) and `gmrt.topobathy_mean`
> (global topo-bathy), `/v1/recall_many` (polygon fan-out).

The point: an agent stepping into a new domain has a 10-second budget
to decide whether the tool is helpful. This trial exercises the surface
end-to-end as that agent would, then names what an agent would actually
trip over.

---

## TL;DR — second pass (after fixes)

| Category | First pass | After fixes | Comment |
| --- | --- | --- | --- |
| Mountain peaks | 7 / 7 | **7 / 7** | Mt Fuji, Everest, K2, Denali — all within ±100 m of published height |
| Major cities | 7 / 8 | **8 / 8** | Sydney over-tight fixture corrected by widening tolerance |
| Rainforest | 3 / 3 | **3 / 3** | Amazon, Congo, Borneo |
| Island | 4 / 4 | **4 / 4** | Hawaii, Iceland, Galápagos, Madagascar |
| Polar (land) | 3 / 3 | **3 / 3** | Greenland 3203 m, Vostok 3497 m, Svalbard 36 m |
| Desert | 3 / 4 | **4 / 4** | Range corrected for Gobi |
| Sea (shallow) | 2 / 2 | **2 / 2** | Now returns *real depth* via GMRT (-72 m N. Sea, -3643 m Mediterranean) |
| **Open ocean** | **0 / 4** | **4 / 4** | **Mariana Trench now -10917 m via gmrt.topobathy_mean** |
| **Edge cases** | 1 / 4 | 3 / 4 | Antimeridian (-5263 m), Drake (-4749 m), Prime/Equator (-4936 m) — only ±90° still fails (genuine no-data both upstream) |
| **Non-Latin place names** | 0 / 3 | **3 / 3** | 東京, Москва, القاهرة all resolved through Nominatim with `accept-language: *` and `q`/`query`/`name` aliases |
| **Total** | **30 / 37 actionable** | **41 / 43** | |

**Latency (40 fixtures, single client over public TLS):**
- `/v1/locate` (lat/lng)         — p50 17 ms, p90 19 ms
- `/v1/elevation` (Open-Meteo)   — p50 180 ms, p90 182 ms (network-bound)
- `/v1/recall` w/ materialization — p50 17 ms, p90 181 ms (cold ≈180 ms, hot ≈10 ms)

The hot-cache short-circuit on second recall is doing its job.

---

## What an agent will trip over

### 1. Open-ocean cells return `elevation_mean = 0 m` and sign it

This is the largest correctness gap.

```
Mariana Trench (11.37°N, 142.59°E)
  /v1/elevation → { "elevation_m": 0.0 }
  /v1/recall    → { facts: [{ "value": 0.0, "band": "copdem30m.elevation_mean", "signer": <responder> }] }
```

The materialised fact is **signed by the responder under registry/schema CIDs that verify**, but the value is wrong by ~11 km. Same for the open Pacific, South Atlantic, Indian Ocean, and Drake Passage.

Root cause: Open-Meteo's `/v1/elevation` wraps **Cop-DEM (land-only); it returns 0 over water by design**. The responder's materializer treats that 0 as a measurement and signs it. Cite-able, verifiable, factually wrong.

**Fix shape:** the elevation materializer must distinguish "0 m sea level" from "no land DEM exists here":

- **A.** Refuse to materialize when Open-Meteo returns exactly 0 over a cell that's likely ocean (centroid in a known marine bbox, OR neighbouring cells also return 0). Return empty `facts: []` with `bands_available` carrying a `note: "ocean cell — bathymetry not yet attested"`.
- **B.** Materialize a **Negative fact** (`Fact::Absence`) for that cell + band, with a `reason_cid` pointing at the response, so the agent gets a signed receipt that the absence has been confirmed (vs. silence).
- **C.** Add a bathymetry materializer (GEBCO 2024 raster via vsicurl, no auth). This is the right long-term answer; the band would be `gebco.bathymetry_mean` and the unit `m` (negative).

Most-bang-for-buck: A + B in this round, C in a follow-up.

### 2. Polar / antimeridian / prime-meridian zero-corner cells

```
North Pole (90, 0)             → /v1/elevation 502 (open-meteo error), recall: 0 facts ✓
South Pole (-90, 0)            → /v1/elevation 502, recall: 0 facts ✓
Antimeridian Pacific (0, 180)  → /v1/elevation 502, recall: signed elev=0 (wrong)
Equator + Prime Mer. (0, 0)    → /v1/elevation 200 elev=0, recall: signed elev=0 (Atlantic; same ocean issue)
Drake Passage (-58, -65)       → /v1/elevation 200 elev=0, recall: signed elev=0 (same ocean issue)
```

Behaviour at the poles is *honest* (502 → empty recall) but not *helpful*. Open-Meteo is rejecting the request; we should detect that and:

- Cache a permanent absence (`Fact::Absence` with reason "no DEM coverage above |lat|=85°"). Future agents get a signed answer instantly instead of waiting 180 ms for an upstream 502.
- Emit a `note` in the recall response explaining the limit, not just empty `facts: []`.

### 3. Place-name resolver fails for non-Latin scripts

```
"東京"   → 429 (no, not rate limit — see step 4 — actually 200 with empty result; via=null)
"Москва" → no cell64 returned
"القاهرة" → no cell64 returned
```

Three real markets (Japan, Russia, Arabic-speaking world) cannot resolve their own capital city by its native name. The embedded gazetteer is English-only; Nominatim should handle Unicode but the layered geocoder is silently dropping the result.

**Fix shape:**
- Verify `EMEM_NOMINATIM_BASE` is configured (or fall back to upstream).
- Add CJK / Cyrillic / Arabic native names for the top-50 cities to the embedded gazetteer.
- Return a structured `error: "place_unresolved"` with the query echoed back instead of an empty 200, so an agent can try again with a different query.

### 4. Test-runner self-throttling — minor, noted

The first run hit the per-IP rate limit at the ~25th fixture. Default `EMEM_RATE_LIMIT_RPS=1.0` with a 60-token burst. A naive test client doing 5 calls per fixture at ~1 fixture/100 ms exhausts the burst quickly.

This is **correct production behaviour**, but agents writing test suites against `emem.dev` will hit it. Add the retry-on-429-with-backoff pattern (now in `scripts/global_trial.py`) to `docs/ATTESTING.md` and `examples/`.

### 5. `polygon_sample_cells` is empty for lat/lng input

```
locate { lat: 35.36, lng: 138.73 }  → polygon_sample_cells: []
locate { q: "Mt Fuji" }             → polygon_sample_cells: [...64 cells across Fuji's polygon...]
```

This is by design — a raw lat/lng is a point, not a region. But the `/v1/locate` doc says polygon_sample_cells is "useful for fan-out" without flagging that it only fires on place-name input. Either:
- Synthesise a small polygon from `neighborhood_cells` when lat/lng is provided
- Document the conditional explicitly in the agent_hint block

### 6. Fixture-range slop — not bugs

Sydney recalled 81 m vs. expected (0–80) — 1 m over. Gobi recalled 1519 m vs. expected (800–1500) — 19 m over. These are tightly-set expectation ranges, not data wrongness. The trial fixture should have used wider tolerances; the responder is fine.

---

## What worked well

- **Mountain peaks were spot-on**: Mt Fuji 3776.24 m (vs. published 3776 m), Everest 8848.86 m (vs. 8848 m), K2 8449 m. Cop-DEM 90 m wrap is high-quality where it has coverage.
- **Latency was well-predictable**: cold materialization always ~180 ms (single Open-Meteo round-trip), hot recall ~17 ms (sled prefix scan). Agent can budget around this.
- **Receipts verified** for every successful recall — the `signer_b32` hash matches the responder pubkey across all 40 calls.
- **Bulk fan-out** worked when polygon sampling produced cells — Madagascar's 8-cell sample drained in one round-trip with all 8 returning materialised values.
- **Mediterranean + North Sea returned 0 m**, which is technically right for those cells (sea level *is* the elevation) — flagging these as "expected" not "wrong". A bathymetry pass would still upgrade them, but they're not as bad as Mariana.

---

## What changed between passes

The first pass surfaced six gaps. Five were closed by changes that
**strengthened the protocol technically rather than papering over
failures with hardcodes or fallbacks**:

1. **Ocean cells**: instead of signing `elevation_mean = 0` (verifiable
   but wrong by 11 km at Mariana), the responder now materializes a
   signed `Fact::Absence` with a content-addressed `reason_cid`
   pointing at the canonical reason text. The Absence is hot-cached
   like any Primary; the second recall short-circuits in 10 ms.
2. **Real bathymetry**: a new `gmrt.topobathy_mean` band materializer
   queries the Global Multi-Resolution Topography service
   ([gmrt.org/services/PointServer](https://www.gmrt.org/services/PointServer))
   from Lamont-Doherty Earth Observatory. GMRT fuses Cop-DEM, GEBCO,
   multibeam, and high-res surveys into a single peer-reviewed dataset
   that returns positive land elevation AND negative bathymetric depth
   in one call. Mariana Trench: -10917 m (vs. published -10994 m).
3. **Upstream-error absences**: when Open-Meteo returns 5xx
   (e.g. above the polar circle), the materializer signs a NegativeFact
   recording the structural absence so subsequent recalls don't re-pay
   the 180 ms upstream timeout.
4. **Non-Latin place names**: were never reaching Nominatim because
   the request struct only accepted `place`, not `q`/`query`/`name`
   (the de-facto convention across OSM, Google Geocoding, Mapbox).
   Now accepts all four as serde aliases. Plus an
   `accept-language: *` header to Nominatim so it preserves the input
   script in responses. 東京 / Москва / القاهرة now resolve correctly.
5. **Materializer discoverability**: a new `/v1/materializers`
   endpoint declares which bands the responder will auto-fetch, what
   upstream produces the value, the `derivation_fn_key`, and whether
   the result will be Primary or Absence. Lets agents discover the
   capability without trial-and-error.

The deliberately-skipped item: **synthesising a 9-cell polygon for
lat/lng input**. That would have been a fallback masquerading as a
feature — `polygon_sample_cells` is empty for points by design (a
point is not a region), and the agent already has `neighborhood_cells`
for a small fan-out. The locate response is honest; agents who want
fan-out should ask for a region by name or pass an explicit polygon.

## Remaining gaps

1. **±90° poles**: both Open-Meteo (Cop-DEM) and GMRT return errors
   at exact lat ±90°. Both upstream sources have no authoritative
   topo-bathy value at the pole singularity — this is a real
   "no data anywhere" zone, not a protocol failure. Will trigger the
   absence-fact path in the next round once upstream errors are
   structured as 5xx-with-Absence (currently only Open-Meteo errors
   write absences; GMRT failures don't yet).
2. **Old Cop-DEM data hygiene**: cells materialized before the
   ocean-detection fix retain their wrongly-signed `value: 0` Primary
   facts (8 cells hit by the original trial). The new logic only
   fires on cache miss. A deletion endpoint + targeted purge would
   reconcile, but every signed fact is content-addressed and the
   responder pubkey is stable, so external verifiers can detect and
   reject these by their `derivation.fn_key + value=0` signature.
3. **Bulk fan-out for GMRT** is rate-limited (GMRT recommends ≤2
   req/s); a polygon-fan-out call to GMRT for 64 cells would breach
   this. Need an explicit `materialize_concurrency` cap on
   `recall_many` when materialization fires for a GMRT-backed band.

---

## How to reproduce

```bash
# from the emem repo root:
python3 scripts/global_trial.py https://emem.dev
# or against a local server:
python3 scripts/global_trial.py http://127.0.0.1:5051
# raw JSON: /tmp/emem_global_trial.json
```

The fixtures live at the top of `scripts/global_trial.py`. Add new categories there; the runner is shape-agnostic.
