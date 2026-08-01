# Changelog

emem follows the [Keep a Changelog](https://keepachangelog.com/) format.
CIDs are content-addressed; minor version bumps may roll bands /
algorithms / sources manifests, but old facts under old CIDs continue
to verify. 

## [Unreleased]

## [1.4.0] - 2026-07-31

Minor rather than patch, deliberately. This release adds REST surface
(`/v1/enroll_attested`, `/v1/enroll_verify`, `/v1/trace_resolve`,
`/v1/trace_encodings`, `/v1/device_platforms`, `GET /memories/*path`) and
changes a write rule: a caller that was writing to an open-namespace path
another key created is now refused. Nothing in the 1.0.0 stability promise
moves, because the wire format, receipt preimage and address space are
untouched and every fact signed under 1.3.x still verifies.

### Added
- Stage 2 of the device substrate: enrollment can carry platform-attestation
  evidence and the gate verifies it (`POST /v1/enroll_attested`,
  `POST /v1/enroll_verify`, `emem:attestation:` tokens). Every shipped
  platform anchor is `provisional`, so attested enrollment is refused by
  name today and `operator_asserted` remains the only admissible assurance.
- Stage 3: the trace-encodings registry (`emem-trace-encodings`, the
  11th manifest, 8 capture toolchains with their own integrity classes) at
  `GET /v1/trace_encodings`, plus the device-platform whitelist
  (`emem-device-platforms`, 16 platforms in 6 families under RATS) at
  `GET /v1/device_platforms`; the write gate refuses a segment naming an
  unregistered encoding, a layer the encoding cannot capture, or an
  encoding the enrolled platform does not emit. `POST /v1/trace_resolve`
  turns `emem:trace:` / `emem:attestation:` tokens back into records.
- Stage 4: streaming. `prev_trace_cid` chains per-window traces per
  (device key, boot id); a dropped, duplicated, or reordered window is
  refused at ingest and a reboot legitimately starts a fresh chain. The
  `orin_stream` example streams four real committed Sentinel-2 frames
  through the full path (`EMEM_FRAMES_DIR` swaps in your own captures).
- The `a2a` block of `/.well-known/mcp.json` now points at `/v1/ask`
  (signed, fact-cited answers with no language model in the loop) and
  `/v1/inbox` (the read-side mailbox), so a question on the channel does
  not need to wait for a peer.

### Security
- **Open-namespace writes are isolated.** Ownership was bound only under
  `/memories/by_attester/<pubkey8>/`. Everywhere else under `/memories/`
  any valid signature was accepted as an advisory binding, so a second key
  could overwrite, edit, rename or delete a file a first key had written.
  The first attester to create a path now owns it and later mutations must
  present the same key (403 `memory_namespace_violation`). **This is a
  behaviour change:** a caller that was writing to a path another key
  created will now be refused.
- **Authorisation is checked before content.** The ownership gate sat after
  the body checks in `str_replace` and `insert`, so a stranger probing
  another caller's file received `old_str not found` rather than a refusal.
  The gate never ran and the response disclosed whether the string was
  present.
- **Records with no recorded author are frozen.** A few open-namespace
  files predate authorship persistence and carry no attester, so ownership
  cannot be established either way. Where the write policy requires
  attestation (the release default) every mutation of them is refused,
  including the operator's. They stay readable.

### Fixed
- Recalling `protected.is_protected_area` or `overture.places_count`
  (the advertised spellings) now returns the stored fact instead of
  re-materializing into an empty response on every read: requested
  spellings canonicalize to the band name facts persist under.
- A trailing clause no longer becomes part of a place name. "how green is
  the area around Nashik right now, and what does the number mean?"
  resolved to an artwork in Georgia, because the prepositional-anchor
  window ran past the place and the geocoder matched the whole clause. The
  window stops at a clause boundary; `LOCATE_RESOLVER_VERSION` 2 -> 3
  invalidates rows cached under the old spans.
- MCP truncation degrades instead of dropping. An over-budget array was
  replaced wholesale with a stub, so `memory_view` on an attester with 135
  notes returned three entries and a caller could not tell that from "this
  agent wrote three notes". It now keeps as many leading elements as fit,
  reports `_kept` / `_len` / `_next_offset`, and leaves the field an array.
- `memory_view` accepts `offset` on directory listings, which is what the
  advertised `_next_offset` cursor needs to mean anything; the response
  echoes `offset` and `total`.
- Directory listings are newest-dated first within kind, so a truncated
  listing keeps the entries a conversation is actually about.
- The truncation escape hatch no longer names the wrong verb. It hardcoded
  `POST` for every tool, so an agent told to re-fetch a GET-only endpoint
  got a 405.
- `emem_reason` has a shape (`prose`) and a group, both carrying the
  `model_output` warning, and the reasoning loop's tool menu no longer
  loses `emem_ask` and `emem_recall` when their `readOnlyHint` is correct.

### Added
- `GET /memories/*path` serves a memory's body at its own canonical path:
  `text/markdown` by default, the full signed envelope on
  `Accept: application/json`. The path printed in every note header and
  citation was previously not fetchable over HTTP at all.

### Changed
- Data handling is disclosed where it is asked about. `PRIVACY.md` gains a
  collection-table row and an "Agent-written memory" section (what
  persists, that reads are public, that deletion unpublishes rather than
  erases, how to enumerate your own files, how to request operator
  erasure); `TERMS.md` gains 4a; the agent card and `/.well-known/mcp.json`
  carry the machine-readable form. `no_pii_in_canonical_channel: true` is
  replaced by `no_pii_emitted_by_responder` plus
  `pii_possible_in_agent_written_memory`, because agents write arbitrary
  text about people into a shared store.
- The vault's scope is stated rather than implied: its AEAD key derives
  from the responder's own ed25519 secret, so the operator can read vault
  plaintext. A vault seals bytes against other callers and against anyone
  who obtains the database file, not against the operator.
- All seven `memory_*` tool descriptions state the signing and namespace
  requirements, and `emem_memory_search` states that it reads every
  caller's files and excludes vault entries.

## [1.3.0] - 2026-07-26

**Satellites join the multi-agent system on the ground.** Until now a
satellite was where the memory's data came from; from this release it
can be a member of the system like any other agent: enrolled by key,
believed only with evidence, cross-checked by peers. The evidence rule
is the release's one idea, applied uniformly to every machine that
observes the world (telescope, microscope, CCTV, phone, drone, robot,
industrial machine, and an operator's own constellation): emem
respects the device as a contributor and refuses its output alone. A
device writes only what is bound inside its complete, unaltered OS
execution trace, and the founding open-archive substrate, which needs
no trace because anyone can recompute it, becomes the drift anchor
every device claim is scored against. A satellite operator can run the
entire loop today, self-hosted: enroll a spacecraft key, bind a
downlink payload into the pass's signed trace, write through the gate,
and keep the `emem:fact:`, `emem:trace:`, and `emem:bundle:` handles
(`cargo run -p emem-primitives --example satellite_downlink`). One
new crate (`emem-trace`, the 17th), one new manifest
(`emem-substrates`, the ninth), two new read surfaces
(`/v1/substrates`, `/v1/trace_verify`), and the repo's first committed
conformance vectors.

### Added
- The encoder trust layer, as code: any machine that observes the world
  (telescope, microscope, CCTV, phone, drone, robot, industrial machine)
  is respected as a contributor and refused on its word alone. Its output
  is admitted only when bound inside its complete, unaltered OS execution
  trace. Three pure pieces ship: the substrate profile registry
  (`emem-substrates`, the ninth content-addressed manifest, ten profiles
  with per-class admission rules, required trace layers, and measurement
  grain down to microns), the `emem.os_trace.v1` record and its
  domain-separated signing preimage (`os_trace_preimage_v1` in
  `emem-attest`), and the new `emem-trace` crate holding the verification
  engine (sixteen named reject reasons, verdict only on an empty list)
  plus the drift-anchor scoring rule that checks device claims against
  the recomputable Earth substrate. Design and wiring steps in
  `docs/plans/encoder-substrates.md`; ingest gating, `/v1/substrates`,
  and `/v1/trace_verify` are the open wiring work.
- The storage side of that gate, one commit later: `trace_gate::TraceGate`
  in `emem-storage` (a device-enrollment tree locking an attester key to
  an `os_trace_required` profile, a trace store keyed by `trace_cid`, and
  a fact-to-trace audit edge) and
  `MaterializingStorage::put_attestation_gated`, which refuses an
  enrolled key's write unless the trace verifies and binds every primary
  fact's payload digest, while never-enrolled keys keep the ungated path
  byte-for-byte. Enrolled keys write traced primary observations only:
  derivative facts, absences, and edges are refused as an untraced side
  door until a traced-derivation rule exists.
- The operator on-ramp, third commit in the series: the
  `orbital.satellite.v1` profile (a manufacturer's own constellation is a
  device substrate with the trace rule, distinct from the recomputable
  public archive), `GET /v1/substrates` and `POST /v1/trace_verify` with
  matching `emem_substrates` / `emem_trace_verify` MCP tools (104 tools,
  124 documented `/v1/*` paths), and a runnable end-to-end example,
  `cargo run -p emem-primitives --example satellite_downlink`: enroll a
  spacecraft key, bind a downlink payload into its OS trace, write
  through the gate, and keep the `emem:fact:`, `emem:trace:`, and
  `emem:bundle:` handles. Plus `emem:trace:` tokens (compose, parse, resolve from
  the store) and the repo's first committed conformance vectors,
  `spec/test_vectors/os_trace/` (admit, chain broken, output unbound,
  archive refused), deterministic and replayed in CI.

## [1.2.1] - 2026-07-21

A release-plumbing patch. 1.2.0 published unevenly — npm `@vortxai/emem`
and PyPI `ememdev` went out, but the MCP registry stayed on 1.1.0 and
`emem-langmem` never left the runner — so the platforms disagreed about
what 1.2.0 was. No protocol or API surface changes; this exists to make
one version land the same everywhere.

### Fixed
- `server.json` was left at 1.1.0 while the workspace, both Python SDKs
  and the TypeScript SDK had all moved to 1.2.0, so `mcp-publish`
  re-published the already-listed 1.1.0 and failed with a duplicate-version
  400. The version is now bumped in lockstep with `Cargo.toml`'s
  `workspace.package.version`, as `mcp-publish.yml` documents it must be.
- `publish-npm` and `mcp-publish` now treat an already-published version as
  an idempotent no-op instead of a hard failure, matching the
  `skip-existing: true` the PyPI publishers already carry. Re-running a
  release tag whose version is already out is a logged skip, not a red X;
  any other error still surfaces with its original exit code.

## [1.2.0] - 2026-07-21

The first version whose central claims were measured by someone other than
its authors, and changed as a result. An independent agent benchmarked emem's
dereference loop, found that the last mile between a signed fact and a model's
answer was unverified, and published the failure rate. Most of what follows is
the repair.

It also carries a signed outside review (`e6jfsgck6ifuwkjxgffxqgnrmy`). A
compliance agent that consumes emem facts to build a regulated product agreed
in advance to publish its review either way, verified both receipts and
reproduced the precision claim on a live fact, and endorsed the study as an
honest SAMPLE on two conditions we now keep beside the headline: it measures
value fidelity rather than verdict accuracy, and the retrieval result is scoped
to dense similarity on a homogeneous corpus. In their framing, an outside
review is not an outside re-run: no stranger has reproduced the numbers on
another host, so SAMPLE stands.

Additive and backward compatible with one stated exception. Every receipt
signed under 1.0.0 and 1.1.0 verifies unchanged, legacy `memt:` / `memb:` /
`meme:` tokens still resolve, and every new request field defaults to the
previous behaviour when absent. The exception is that `mean` and `sum` over
more than two parents are now compared against a published 4-ULP window rather
than for bit equality, which makes derivations verify that previously did not;
the measured gap rides on every response so a caller who requires bit-identity
can still demand it.

### Added
- **The last mile is now verifiable.** `POST /v1/memory_token/resolve` accepts
  a bare cid and an embedded cid recovered from surrounding prose, answering
  `degraded: true` rather than failing, because a model that drops
  `emem:fact:<descriptor>:` and keeps the tail is citing a real fact badly, not
  citing nothing. Every resolve carries `value_verbatim`, an exact decimal
  string to copy rather than a float to retype. `POST /v1/echo_verify` grades a
  value a model is about to publish against the signed fact and reports the
  drift, with a `strict` mode. A cid of the wrong length is reported as a
  damaged citation (`fact_cid_malformed_length`, `recoverable: false`) instead
  of a missing fact.
- **GC-1 tier-1 recomputation.** `POST /v1/derive` re-runs a pure op against a
  pinned `code_cid` and awards `deterministic_index` provenance when the result
  reproduces, so a derived number can carry the same class of evidence as a
  measured one.
- **The field-token family completes.** `emem:cube:` names a field over time
  (`band_cube@1`), `raster_bundle@1` binds N field tokens into one signed
  manifest, and DEM and encoder embeddings become field tokens in their own
  right. Anchors are emitted per cell so a compliance claim is spot-checkable
  by clicking one.
- **A2A collaboration is on the front door.** `/.well-known/mcp.json` carries a
  machine-readable `a2a` block with the invitation, the ten-rule standard, and
  the path for telling us where it hurts.
- **Offline authorship.** Memory writes persist the caller's signature, so a
  third party verifies who wrote a memory without asking us (T1).
- **`GET /live`**, a liveness probe that touches no storage, for callers who
  need to know the process is up without paying for a scan.
- Three agent skills (field tokens, signing and attestation, A2A
  collaboration), and refreshed OpenAI GPT and Gemini integrations.
- A per-attester write rate limit as a backstop, with the security posture
  published machine-readably rather than described in prose.
- [`examples/benchmark-arm/`](examples/benchmark-arm/): the dereference arm
  emem would defend, plus `differential_scorer.py`, an independent re-scorer
  written against the third-party study's published bytes. It disagrees with
  one of that study's headline figures and says so.

- **Reductions verify to a stated bound, and the bound is measured.** `mean`
  and `sum` over more than two parents are compared against a published 4-ULP
  window; everything with nothing to accumulate stays exact. The window was
  shipped, argued away on the principle that a verifier accepting "close
  enough" is not a verifier, and restored when the agent who made that argument
  measured at scale and reported against themselves. `sum` lands 0, 1, 2, 2 and
  0 ULP from an independent accumulation at N of 5, 16, 32, 64 and 128, so the
  failure is not monotonic and strict equality made verification unpredictable
  from the caller's side. Every recomputation now returns the `rule` that ran,
  the `ulp_tolerance`, and the measured `ulp_gap`, on success and on failure, so
  a caller needing bit-identity requires a gap of zero and can see it.
- **A live site test.** `scripts/site_test.py` checks the running site over
  HTTP: routes, internal links, anchors, images, per-page gzip budgets, counts
  asserted against the live API, authoring residue, and every inline script
  hashed against the CSP header sent with it. It runs in about two minutes and
  exits non-zero, so CI can gate on it.

### Changed
- `/humans` is retired and redirects to `/worlds`. It was not carrying its
  weight and it was diluting the pages that were.
- `emem-membench` is now `emem-scorecard`.
- `/docs/api` and the whitepaper were rewritten to match what the code does,
  including sections on GC-1 and A2A.
- `/verify` performs all three legs (bytes, authorship, inclusion against
  consistency) and accepts the token forms agents actually cite.

### Fixed
- **The docs were broken on every page and it did not look like it.** All six
  of mdbook's inline bootstrap scripts were CSP-blocked across all 26 `/docs/`
  pages, so search, theme persistence and the sidebar toggle were dead on the
  surface every page's nav labels "Docs". The pages still rendered. The hash
  pass walked the `include_str!` set and the docs book arrives via
  `include_dir!`; the guard test read this repo's source and was structurally
  blind to it. The new guard asserts the OUTPUT, hashing every inline script we
  would serve against the CSP we would send, and asserts a floor on how much it
  covered.
- **`/v1/derive` told callers the wrong thing about its own comparison.** The
  JSON response hardcoded `rule: canonical_float_equality` and the word
  "bit-for-bit" even when the reduction window had run with a measured gap of 2,
  while the CBOR receipt carried the truth. A test now pins the two surfaces to
  the same rule, because two surfaces disagreeing is the bug rather than one
  field being wrong.
- **The channel was 923 KB and grew with every note.** Message bodies were 80%
  of it. Only an excerpt ships now and the rest arrives on click from `/mcp
  memory_view`, rendered as the exact signed bytes: 60 KB gzipped, down from
  249 KB, while carrying 32 more notes. It also had no navigation out at all,
  the only page on the site like that, and 47 of its 203 permalinks pointed at
  nothing because any 26-character base32 word was linkified whether or not it
  named a note on the page.
- **A note over the 24 KB MCP wire budget was unreadable by its documented
  recovery path.** `memory_view` omits `content` and the truncation envelope
  suggests `fetch` (which returns null) and cursor/page arguments (which
  paginate cells). `view_range` is what works and is the one option the
  envelope does not name. The co-authored paper is over the line, so the paper
  published through emem was the note that crashed the generator for the page
  describing it.
- **Twenty commercial dead ends.** The use-case shelf on `/solutions` sent its
  highest-intent links into bare SVG files with no navigation, prose or way
  back. Six pages had no `h1`, including `/verify`. `/a2a` and `/demos` were
  mutually invisible under six different navigation variants, leaving three of
  six demos with one inbound link each. `/worlds`, which `/humans` redirects
  to, was 85 words wrapped around a WebGL viewer that never said what a point
  was.
- **Counts that had drifted on twelve surfaces**, including `agents.md` (served
  to agents) and `mcp-directory.md` (which feeds the registry listings): 96
  tools, 14 core, 82 extended, 114 and 121 REST paths, 32 diagrams. The server
  serves 102 tools, 15 core, 87 extended, 122 paths, and 37 diagrams, two of
  which were on disk but not baked and returned 404. `sync_counts.py` guards
  these with a blacklist of known-stale strings, so drift to a NEW wrong number
  passed it silently and it reported no drift throughout.
- **The TypeScript SDK would have published with no code in it.** No
  `.npmignore` meant npm fell back to the root `.gitignore`, which ignores
  `dist/`, while `package.json` points `main` at `dist/index.js`. This is the
  same class of bug that previously shipped empty Python wheels. Not yet
  verified with `npm pack --dry-run`; that remains a release gate.
- JRC forest bands were materialized but undiscoverable.
- `spot_check` anchors are band-aware and clamped (the DEM reproduction).
- Embedding chip selection uses per-pixel SCL rather than scene-level cloud
  cover alone.
- `recall_polygon` paginates and slims its projection so a large plot fits the
  MCP wire.
- The watchdog attaches gdb under `sudo`, so a wedged process finally produces
  a backtrace instead of a silent restart.
- Agent handoff notes moved out of the repository root into
  `.well-known/agent-notes/`.

### Known operational issue: restarts under load, cause partly identified

**Read this before depending on emem for anything with a deadline.**

"Stable" here means the interface contract holds: receipts verify across
versions, tokens keep resolving, changes are additive. It does not mean the
service never restarts, and it would be dishonest to let the version number
imply that.

`var/wedge/` holds **235 snapshots since 2026-07-03**, roughly 1 to 7 a day. A
watchdog restarts the unit within about 30 seconds, so this presents as
intermittent slowness rather than an outage, and the systemd restart counter
stays at 0 because the watchdog restarts the *service*, not a crashed process.

**A previous version of this section described these as a tokio runtime wedge in
which connections sat unaccepted in the listen queue. That description was
wrong, and the error was ours.** It came from reading the socket counts in the
snapshots as emem's own. They are not: the watchdog captured them with `ss -tan`,
which has no process filter, so `LISTEN 24` is the number of listening sockets on
the entire host, not connections queued against emem. Compared against a healthy
host, every snapshot's socket profile is ordinary: LISTEN identical, ESTAB and
TIME-WAIT within normal range, memory normal, and every thread sleeping rather
than blocked on disk.

What the record actually supports is narrower: **`/health` did not answer within
8 seconds, twice in a row, and the process was then killed.**

The likely cause of at least some of those restarts is now identified, and it is
the watchdog itself. It probed `/health`, which reports corpus statistics and
pays for a `storage.scan_index` pass to do it. Measured on the serving host:
80 ms idle, **905 ms under twenty concurrent recalls**, and worse under a
materialize storm. `/live` touches no storage and costs about 0.5 ms under the
same load. Both share the tokio runtime, so a genuine stall stops both, which
makes `/live` strictly the better liveness signal. The watchdog now probes
`/live` and keeps `/health` as a diagnostic: when `/live` answers and `/health`
does not, that is logged and nothing is restarted.

**How many of the 235 were real stalls is unknown**, and will stay unknown for
the earlier ones: backtrace capture was broken for all of them, so no stack was
ever recorded. That capture is now verified working. If genuine stalls remain
after this change, the next one will finally produce evidence instead of a
guess.

### Known gaps, stated rather than omitted
- The third-party benchmark is marked SAMPLE. It has no independent
  replication, no pre-specified power, no archival DOI, and covers two open
  7-12B models on one inference host. Independent re-scoring is closed as of
  this release; the rest need someone outside this collaboration.
- The npm package has never been published. The first publish needs a
  bootstrap token because no trusted publisher is registered.

## [1.1.0] - 2026-07-16

A stable, additive upgrade: every receipt signed under 1.0.0 verifies
unchanged, the new preimage segment is append-only (a receipt without a
field binding hashes byte-identically to before), and every new request
field defaults to yesterday's behaviour when absent.

### Added
- **Field tokens** (docs/plans/field-tokens.md, complete): `POST
  /v1/band_raster` returns a native-resolution Sentinel-2 window as a
  content-addressed canonical grid artifact whose receipt attests a
  DERIVATION, never a byte pipe; the signed derivation record persists
  and pins the scene, recipe, georeferencing, and best-effort per-cell
  anchors. `GET /v1/artifacts/{cid}` serves the bytes immutable and
  evictable-by-design; `POST /v1/raster/resolve` dereferences
  `emem:raster:` tokens with every claim bound to the signed record
  (mismatch is a typed 409). New receipt machinery: the FIELD preimage
  segment (`receipt_tag::FIELD`), `field_binding_v1`, `Receipt.field`,
  `sign_receipt_field`, the verifier-spec row, and `field_bound` on
  `/v1/verify_receipt`. MCP tools `emem_band_raster` and
  `emem_raster_resolve`.
- **Change attribution** (`change_attribution@1`): `POST
  /v1/change_attribution` and `emem_change_attribution` return the
  per-term evidence LEDGER for why a readout moved (`split` null by
  design, with the in-band note saying why), and each run persists as a
  derivative fact with its own `emem:fact:` token.
- **Partial results** (docs/plans/partial-results.md, complete):
  `recall_polygon` and `recall_many` accept `budget_ms` and answer
  first-class partial 200s with a typed `pending[]`, `converged`, and
  monotone identical-request retry; expiry detaches fetches so they
  persist. `emem_backfill` grew the preparer form (a `cells` list under
  the same contract), and the densification warmer loops the preparer
  on a schedule, off by default (`EMEM_WARM_INTERVAL_SECS` +
  `$EMEM_DATA/warm_priority.json`).
- **Batch dereference**: `POST /v1/memory_token/resolve_many` resolves
  up to 256 fact tokens in one call, partial by design with per-item
  receipts and one batch receipt; token dereferences now carry
  `Cache-Control: immutable` on full success.
- **The canonical grid codec** (`emem-codec::grid`), the evictable
  artifact store (`emem-storage::artifacts`), a doc-lint CI gate
  (`scripts/doc_lint.py`), the canonical merged whitepaper
  (`docs/whitepaper.md`), a ten-minute tutorial, and the agent-handoff
  example (a session checkpoint a successor identity resumes and
  verifies).
- **Python SDK signing surface** (`ememdev[signing]` + the `ememdev`
  CLI: whoami / sign / write) and the counts moved to 94 MCP tools,
  162 algorithms, 114 documented paths.

### Changed
- The Python module renamed `emem` to `ememdev` to match the install
  name; the PyPI name `emem` belongs to an unrelated project and a
  shared top-level module would collide on disk.
- Positioning: referential drift is one concept with two directions
  (the token pins the paraphrase side; the attribution ledger reports
  the world side), stated across the docs, the MCP preamble, and the
  served strings. The memory layer's scope and the hosted-privacy map
  are stated plainly: no client-controlled private memory exists on the
  hosted node until owner-scoped reads ship.

### Fixed
- DMSP-OLS nightlights decoding accepts the uncompressed TIFFs NOAA
  actually ships (was 97% of materialize failures in one window).
- The Lahaul-class geocode mis-rank (boundary now outranks village) and
  the geocode cache is resolver-versioned so fixes land immediately.
- The wheel-import gates in ci and publish workflows follow the module
  rename; the npm pack guard fails with evidence instead of a
  TypeError; two test-only preimage call sites missed by the FIELD
  sweep.


## [1.0.0] - 2026-07-09

First stable release. The wire format is settled: canonical CBOR, the ed25519
receipt preimage, and the cell64 address space will not break under a 1.x, and
every fact signed under a previous CID continues to verify. All 16 workspace
crates, the MCP server descriptor (`server.json`), and the agent, plugin, and
Gemini-extension manifests move to 1.0.0 together; the running responder reports
it through `env!("CARGO_PKG_VERSION")` at `/v1/agent_card` and in the MCP
`serverInfo`.

### 3D worlds: dense navigable demo at /splats, provenance v2, damped camera

- New `/splats` surface: a hosted, view-only demo of dense navigable worlds
  served from `EMEM_SPLATS_DIR`. Where `/worlds` draws one gaussian per signed
  cell, `/splats` pushes the same signed facts into a photoreal fly-through of
  roughly 3.4 million oriented surfels: geometry from a measured USGS 3DEP
  bare-earth DTM, colour from a Sentinel-2 L2A scene, and a signed Sentinel-2
  time series (eight frames, 2018 to 2025, each carrying its own `fact_cid`)
  behind a scrubber with a change-since-baseline mode. Density is added by
  bicubic interpolation plus a diffusion super-resolution pass whose
  low-frequency signal is locked to the measured Sentinel-2, and every splat is
  tagged `measured`, `interpolated`, or `synthesized`; the manifest is
  ed25519-signed over the whole pipeline and the invented layers peel back off
  to leave only the measured trust root. The route serves the static bundle with
  ETag + short caching and reverse-traversal guards, and proxies a single
  `POST /splats/api/gemma` to a loopback Gemma bridge so the Ask-Gemma panel
  works without exposing that service publicly. Three scenes today: the Grand
  Canyon, and the Tungabhadra and Srisailam reservoirs.
- `emem.splat_provenance.v2` densification in the exporter and the live
  `/worlds` viewer. `make_splats.py --densify F` subdivides each grid quad and
  labels every splat `measured` (its own `fact_cid`) or `derived` (up to four
  source cells, their `fact_cids`, and bilinear weights that sum to 1), so a
  derived continuous value re-derives exactly as `sum_i weight_i * source_i` and
  every source stays signature-checkable. Categorical bands (a loss year, a
  class code) inherit from the nearest signed cell instead of averaging, and a
  node on an original cell stays that exact signed cell, so densifying never
  invents a value or drops a measured fact. `--check-derived` re-verifies a whole
  sidecar offline; the `splat-math.js` and Python paths are pinned to 1e-6 by a
  golden fixture, and the viewer's pick panel resolves any derived splat to its
  signed sources.
- The `/worlds` camera is now fully damped and gimbal-free. Orbit, zoom, and pan
  ease toward damped targets so there is no overshoot or leak; a meridian-tangent
  camera up vector gives a full 360-degree vertical tumble with no gimbal lock;
  and pan derives its screen-plane basis from the live view geometry rather than
  a frame-stale matrix. `resetView` and `focus` set both the live and the target
  state so the transition stays smooth.

### 3D worlds: real gaussian splatting, signed splat export

- The `examples/3d-worlds/` renderer is now standard 3D Gaussian Splatting
  instead of additive point sprites: per-cell anisotropic covariance
  `Sigma = R S^2 R^T`, EWA screen-space projection `Sigma' = J W Sigma W^T J^T`
  (Zwicker 2001; Kerbl 2023) in an instanced-quad shader, back-to-front
  premultiplied-alpha compositing with a CPU depth sort. Every gaussian
  parameter is a measurement: footprint from the sampled grid pitch, tilt
  from slope/aspect finite-differenced over the neighbouring cells' signed
  elevation facts (the `/v1/terrain` neighbourhood computation, run
  client-side over facts already in the scene), thickness from the detrended
  neighbour residual RMS or a band's published standard error, opacity from
  the fact's attested confidence. Missing shape facts degrade to isotropic;
  old `EMEM_WORLD` configs without a `shape` block still render.
- Two new templates: `semantic-world.html` (Cairo/Giza coloured by the
  128-D GeoTessera foundation embedding, in-browser PCA to RGB — desert,
  Nile farmland, and urban fabric separate with no land-cover labels) and
  `carbon-world.html` (Rondônia: height = ESA CCI above-ground biomass,
  thickness = its standard-error band, colour = Hansen loss year).
- `make_splats.py` (stdlib only) exports any preset as portable signed
  splats: a standard 3DGS `.ply` (loads in SuperSplat / gsplat /
  antimatter15 viewers), the 32-byte `.splat` binary, a
  `emem.splat_provenance.v1` sidecar binding artifact sha256 to per-splat
  fact CIDs and the verbatim signed receipts, and a `scene.json` for
  offline rendering. `--verify` round-trips every receipt through
  `/v1/verify_receipt` (1,025/1,025 valid on the Grand Canyon export).
- `capture.mjs`: the previously prose-only GIF pipeline, committed — raw-CDP
  headless Chromium (no dependencies), deterministic `__renderFrame`
  stepping, a node-fetch relay for sandboxes whose egress resets Chromium's
  TLS, ffmpeg two-pass palette assembly.
- Correctness is pinned twice: `test/golden-scene.json` (hand-derived
  sigmas/quaternions asserted to 1e-6 by both the JS and the Python math)
  and `test/render-checks.mjs` (a rendered gaussian's radial profile fits
  `exp(-r^2/2s^2)` with R^2 > 0.99, isotropic covariance projects to a
  circle, compositing order flips across a half orbit). Cold-region
  loading splits timed-out `recall_many` batches down to 8 cells and skips
  what still cannot materialize instead of dying.
- README: all four worlds embedded as fresh orbit GIFs rendered from the
  live responder through this pipeline; the two first-generation
  splat-column DEM GIFs stay for comparison. The raw and processed data
  behind every world is committed under `examples/3d-worlds/scenes/`
  (scene JSON, .ply, .splat, provenance receipts), so the worlds render
  offline and the PLYs open in external 3DGS viewers without touching a
  responder.
- The responder now serves the worlds itself, pre-baked. `GET /worlds`
  renders any preset from artifacts on disk (`EMEM_WORLDS_DIR`, default
  `var/worlds`) with the same vendored renderer as the templates, and
  hash-checks the fetched scene against its provenance sha256 in the
  browser before drawing the first splat; `GET /v1/worlds` lists every
  baked world (counts, hashes, sizes) and `GET /v1/worlds/:preset/:file`
  serves `world.ply` / `world.splat` / `world.scene.json` /
  `world.provenance.json` / `meta.json` with ETag + an hour of caching.
  `scripts/bake_worlds.sh` builds every preset against the local
  responder in gentle mode (small batches, `--sleep` spacing), verifies
  every receipt, and swaps the finished directory in atomically;
  `scripts/stage_worlds.py` stages the committed
  `examples/3d-worlds/scenes/` artifacts instead, so a fresh clone gets a
  working `/worlds` without a single upstream fetch;
  `ops/systemd/emem-worlds-bake.timer` re-bakes weekly. Browsers never
  trigger the minutes-long materialize-and-sign build — building and
  serving are now different machines' problems by construction.
- `make_splats.py` provenance now also binds `scene.json` by sha256 (it
  is what a browser actually renders), and `--sleep` spaces recall
  batches for cold bakes against a shared responder.
- `.github/workflows/worlds-gifs.yml` (manual dispatch) re-captures the
  README orbit GIFs from the baked scenes served at `/v1/worlds`, with
  zero materialization load on the responder.
- The `/worlds` viewer is now interactive and self-explaining. It
  orbit/zoom/pan drags; a per-world legend states what height, thickness,
  and colour mean (so the carbon world reads as a deforestation frontier,
  not an abstract blob); **clicking a splat** opens the cell's measured
  band values, a `/verify/<fact_cid>` link, and a copy-`memt:`-token
  button; you can recolour by any other signed band in the scene, adjust
  vertical exaggeration and splat size live, and drape real Esri World
  Imagery (fetched for the scene's own bbox) under the signed geometry as
  clearly-labelled reference. Every panel minimises (and defaults to
  minimised on a narrow screen) so the render can fill the viewport. The
  interaction is exposed on
  `window.__ememWorld` / `EMEM_WORLD.onReady(api)` (`pick`, `recolor`,
  `rebuild`, `setBasemap`, `setSplatScale`, `focus`); the deterministic
  `EMEM_CAPTURE` path and the golden/pixel checks are unchanged, so the
  README GIFs stay byte-for-byte reproducible.

### Ops

- The recurring tokio-runtime stall is fixed at the root. sled is a
  blocking store, and the recall and materialize paths ran its reads and
  writes directly on the async workers; under a cold recall storm or a
  materialize burst, enough workers blocked inside sled that the runtime
  stopped serving `/health` and the watchdog SIGKILLed a live-but-wedged
  process (five incidents, 2026-05-31 to -07-03). All hot sled I/O now
  runs on the blocking pool behind a bounded semaphore
  (`EMEM_SLED_BLOCKING_CONCURRENCY`, default 2x cores clamped to [4,64]):
  `Cache::{lookup_many,get_many,put_many,tier_of}`, the `scan_cell`,
  as-of, and `iter_index` index scans, and `put_attestation`'s
  best-effort proof / multi-attester / scope index writes. Reproduced
  with four concurrent cold bakes (~3,900 cells materialized and signed,
  batch 64, no spacing): `/health` held at 200 across 92 samples over two
  and a half minutes with no restart, where the same load previously
  wedged the server; receipts still verify 65/65.
- `emem-watchdog.sh` now snapshots a wedged responder before restarting
  it (per-thread state/wchan, socket-state summary, gdb backtraces when
  available) into `var/wedge/`, and release binaries keep symbols + line
  tables (`strip = "none"`, `debug = "line-tables-only"`), so the
  recurring tokio-runtime stall finally leaves usable evidence behind.

### Docs

- README repositioned around shared memory for long-horizon and multi-agent
  systems: measured token economics for `memt` handoffs (79 chars, 46-49 BPE
  tokens, standing in for a ~1,600-token signed response), a positioning
  table against vector DBs / RAG / tile APIs, a "More than one writer" section
  documenting the live `/v1/attest` multi-writer path, memory kinds with TTLs
  (episodic / semantic / procedural / resource), a signing-key persistence
  warning for self-hosters, and a staged roadmap (transparency log, absence
  proofs, attester spec, quorum reads). Restored content lost in recent README
  churn: hunt / memory_search / physics solvers / eudr_dds / `as_of` time
  travel, the EO point-sample and EGM2008 datum notes, Fields of The World
  figures, the 27-topic count, and the gallery link. Citation now matches the
  Zenodo record title. Fixed the well-known jq path in the verify walkthrough
  (`.responder.pubkey_b32`).
- Three new in-theme diagrams from a new generator (`scripts/gen_shared_memory.py`):
  `34-shared-memory` (one memory, many agents), `35-two-agents-one-memory`
  (the memt handoff, with measured sizes), and
  `36-memory-outlives-the-context-window` (compaction survival), SVG plus
  1600px PNG twins. New README section on surviving context compaction,
  surfacing the refinement pass and the sleep-time consolidation agent.
- 3D worlds from live memory: two browser templates under `examples/3d-worlds/`
  (`single-band-world.html`, `multi-band-world.html`, vendored three.js, no
  build step) render any bbox as a gaussian splat world, one splat column per
  signed fact, fetched via `query_region` + `recall_many`. README embeds two
  orbit GIFs rendered from the live responder (Grand Canyon, 736 elevation
  facts; Interlaken, 2,202 facts across elevation + NDVI + water) under
  `docs/media/`.
- `docs/ARCHITECTURE_NOTES.md`: file-level map of canonical serialization, CID
  derivation, receipt signing, keys, storage trees, the append-only
  AttestationLog, tslot logic, and attester write auth.
- Removed the committed `:memory:/geocoder.sled` runtime artifact (already
  gitignored). Corrected stale route counts in `sdks/emem-ts/README.md`.

## [0.1.0] — 2026-06-14

_Per-geography embeddings, a narrative layer, and an agent-surface truth pass.
The minor bump signals the new geography-scale read path; old facts under old
CIDs continue to verify._

### Geography-scale embeddings + region algorithms

- **`POST /v1/tessera_field`** renders a dense Tessera 128-D embedding field for
  a bounding box as a colour raster, read from the geotessera COG window(s) and
  mosaicked across 0.1° tiles — one region read, never cell-by-cell. Tessera is
  the only foundation encoder published as precomputed COG tiles, so this stays
  cheap over a whole region; the other three stay per-cell GPU inference.
- **`POST /v1/region_archetype_map`** clusters that field into k land-cover
  archetypes with deterministic k-means (greedy farthest-point seeding, Lloyd
  iterations over L2-normalised vectors) and returns a categorical map plus a
  legend `[{archetype_id, rgb, hex, pixel_fraction, pixels, fingerprint_rgb}]`.
  Same bbox+year+k returns byte-identical output; honest sparse (transparent
  where no tile covers a pixel, `available:false` when coverage is too thin).
- Both render endpoints share one `read_tessera_field_grid` helper. The homepage
  map gains an "embedding field" and a "land archetypes" overlay, debounced on
  camera-settle and gated to zoom ≥ 10 and ≤ 0.5°.

### Narration

- **`POST /v1/explain`** (REST only) forwards a signed `/v1/ask` answer to a
  local Gemma 4 12B sidecar for a plain-language narrative. It returns
  `signed:false` and points back to the canonical receipt + `fact_cids`: a
  narrator over the signed facts, never the authority.

### Agent-surface + docs truth pass

- `reference` surfaces ~17 endpoints that shipped but were undocumented —
  thematic wrappers (`terrain`, `spi`, `burn_severity`, `deforestation_alert`,
  `sar_forest_disturbance`, `rice_ch4`), the embedding/geography analytics
  (`embedding_centroid`, `region_similarity`, `embedding_diversity`,
  `neighborhood_consistency`, `triple_consensus`, `state_multi`), and the
  memory-graph reads (`edges/recall`, `memory/search`, `memory_contradictions`).
- Whitepaper correctness: Tessera is no longer described as GPU-pinned — it
  streams from precomputed GeoTIFF tiles on CPU, not the 20 GB inference budget.
- Homepage leads with the "latent, not an image" cards directly under the map,
  and the "who it serves" card captions no longer overlap.
- Refreshed every stale count: 81 MCP tools, 93 `/v1` paths, 96 OpenAPI paths,
  43 cube slots, 124 band names, 160 algorithms, 16 crates.

### EUDR comprehensive data + visuals + optional NRT (2026-06-01)

- **Scientific date correctness in the EUDR visual-evidence block.** The annual
  NDVI / S1 timeline labelled every year only with its July-1 *request anchor* —
  not the *real Sentinel acquisition date*, which the signed fact already stores
  in `Source.captured_at`. And the scene PNG used a full-calendar-year window, so
  the image and the NDVI number for a year could be different scenes. Now every
  per-year block carries `ndvi_observed_date_range` / `s1_observed_date_range`
  (the true overpass dates, read off the signed facts), the scene image is
  **co-registered** to the NDVI date (search window ±`EMEM_VISUAL_SCENE_WINDOW_DAYS`,
  default 20 d), and `scene_metadata[]` ties each image to its window. The block
  schema is now `emem.visual_evidence.v2` with an explicit `date_correctness_note`.
- **New `forest_context` per-plot enrichment** (under the same opt-in
  `request_visual_evidence` flag, runs concurrently within the existing budget):
  the **current** ESA WorldCover 2021 land-cover distribution across the plot
  (with the Cropland+Built-up fraction — the Article 2(4) "predominantly under
  agricultural use" signal) plus the Hansen forest-gain fraction. Every value is
  a signed Primary fact with cited `fact_cids`. Informational corroboration only
  — it does NOT change the legal verdict (still the validated JRC GFC2020 +
  Hansen + JRC TMF consensus). Both bands are window-capable static COGs, so the
  added cost is O(1) per plot (one coalesced range read per band).
- **Optional NASA OPERA DIST-ALERT (near-real-time disturbance) scaffold.** New
  `opera_dist.veg_dist_status` / `opera_dist.veg_dist_date` bands + materializer
  for the only genuine NRT (2–4 day) disturbance COG product. It is **not
  requester-pays** — it is Earthdata-Login gated. The operator provisions a free
  60-day EDL token **server-side** via `EMEM_EARTHDATA_TOKEN` or
  `EMEM_EARTHDATA_TOKEN_FILE` (read at runtime; **never committed to the public
  repo**). When unset, the bands sign an honest Absence (reason `not_enabled`) —
  identical to today's behaviour, zero regression. `VEG-DIST-DATE` is decoded
  from its native days-since-2020-12-31 encoding to a **real calendar date**. The
  live STAC/COG fetch is staged behind a structured `NotImplemented` so the
  credential path + optionality land now without fabricating a pixel. The
  responder logs whether OPERA is enabled at startup (never the token value).
  (Registry: bands 42→43, materializer-wired 122→124, cube stays 1792-D.)

### Sentinel-1 SAR + cold-path (2026-06-01)

- **New `POST /v1/sar_forest_disturbance` (+ `emem_sar_forest_disturbance` MCP
  tool, `sar_forest_disturbance@1` algorithm).** Cloud- and night-independent
  Sentinel-1 C-band confirmation of forest clearing — the signal RADD was meant
  to provide, served from the **anonymous** Microsoft Planetary Computer
  `sentinel-1-rtc` collection (no requester-pays, no API key). Intact forest
  scatters VV strongly + stably; clearing collapses the canopy volume term, so
  VV backscatter drops ~3–5 dB. Samples VV at a baseline-year July-1 anchor and
  the latest scene, reports `vv_drop_db` and a `disturbed` flag at the
  Reiche-2018 3 dB threshold (`EMEM_SAR_DISTURBANCE_DROP_DB`). Both VV reads are
  signed Primary facts (cited `fact_cids`); honest `inconclusive` when either
  vintage is unavailable. It is an **additive scout signal, not a standalone
  legal verdict** — a VV drop can be transient (soil moisture, harvest, flood
  recession), so the response tells the agent to confirm against the optical
  Hansen/JRC-TMF consensus (`/v1/eudr_dds`, `/v1/deforestation_alert`) before
  crediting a decision.
- **Galileo cold-floor trim.** `materialize_galileo_base` awaited the S2 chip,
  then joined S1+DEM+TC. DEM is scene-independent, so it now starts concurrently
  with the S2 fetch (off the critical path). Galileo is the slowest foundation
  encoder and bounds the parallel `state_multi` fan-out, so this shaves its cold
  floor.

### EUDR audit fixes (2026-06-01, after a 4-agent regulatory + code + doc deep-dive)

- **Fixed a deforestation-verdict correctness bug (could false-pass).** The
  per-cell "forest at cut-off" check used `hansen_ly > 20`, but
  `forest_change.lossyear` is a *calendar year* (2001–2024, materializer adds
  2000), so the guard was always true — a no-op. The default batched-polygon
  path omitted the before-cut-off loss check entirely. Result: a cell with
  high `treecover2000` that had been **cleared before the 2020 cut-off** could
  be classified `pass` instead of `not_in_scope`. Both paths now share one
  verdict evaluator (`eudr_verdict_for`) that correctly excludes land cleared
  at/before the cut-off year. Added a regression test.
- **`cut_off_date` now actually works.** It was accepted and echoed but the
  verdict hardcoded the 2021 boundary and ignored it. The operator's cut-off
  year is threaded into the verdict, so a what-if / time-series audit ("was
  this plot deforestation-free as of date X") returns the correct answer. The
  response surfaces `cut_off_year_applied` + a `cut_off_note`.
- **Input validation on the signed DDS.** `quantity_kg` must be > 0 and
  `commodity_hs` must be a numeric Combined-Nomenclature code with at least the
  4-digit HS heading (the level EUDR Annex I scopes at; HS-6+ recommended).
  Previously a DDS could be signed with 0 kg or a non-numeric "COCOA" code that
  silently mis-bucketed the Annex II classification.
- **Regulatory currency note.** The response now carries a
  `regulation_status_note`: per Regulations (EU) 2024/3234 and 2025/2650 the
  application dates were deferred to 30 Dec 2026 (large operators) / 30 Jun
  2027 (micro-small); the 2020-12-31 cut-off is unchanged.
- **Doc truth-pass.** Corrected the agent-card `forest_loss` descriptor (stale
  Hansen **v1.11 / 2001–2023** → **v1.12 / 2001–2024**, old `hansen.*` band
  aliases → canonical `forest_change.*`). Updated the `eudr_dds@1` registry
  formula and the response `methodology_note` to state plainly that the WRI-Sims
  driver-attribution and RADD SAR steps are **deferred (signed Absence today)**
  and the live verdict is the JRC GFC2020 + Hansen + JRC TMF consensus only —
  rather than narrating them as if they fire. (The EUDR endpoint's own
  Hansen/TMF version strings were already accurate.)

### Cold-materializer latency (2026-06-01, after a 6-agent cold-path + GPU deep-dive)

- **Parallelized the `/v1/state_multi` encoder fan-out.** It looped over the
  four foundation encoders (geotessera, clay_v1, prithvi_eo2, galileo)
  **serially**, so a cold cell paid the SUM of four cold materializations
  (~16 s, galileo dominating with its extra S1+DEM modalities). They now
  materialize concurrently via `join_all` — wall time collapses toward the
  slowest single encoder, and because the three Sentinel-2 chip encoders read
  the *same* scene's COG tiles, the existing per-slot-`OnceCell`
  `cog::TILE_CACHE`/`PROFILE_CACHE` single-flight coalesces their overlapping
  reads into one upstream fetch instead of three sequential ones. Output is
  byte-identical (results folded back in encoder order); the
  `EMEM_MATERIALIZE_CONCURRENCY` semaphore still bounds upstream parallelism.
- **Parallelized `/v1/triple_consensus`.** The clay_v1 + prithvi_eo2
  `two_vintages` materializations (up to 4 cold GPU embeds) ran serially; they
  now run via `tokio::join!`. Same byte-identical output.
- **Confirmed: no Requester-Pays or auth-gated bucket on any hot path.** All
  17 live materializers use anonymous public endpoints (AWS Open Data, Google
  Cloud Storage, CEDA, JRC, Zenodo, Overpass, …). RADD — removed earlier for
  its ~30 s Requester-Pays S3 timeout — is fully out of EUDR/hunt/
  deforestation_alert, and its lone materializer fast-fails to an honest
  signed Absence **sub-millisecond with zero network call** (pure tile math →
  immediate `Err`), so it can never wedge a request. (Background: the only
  near-real-time disturbance product with direct COG range-reads is NASA OPERA
  DIST-ALERT, which needs a free Earthdata-Login token — not wired, pending an
  explicit decision to store NASA credentials. Hansen lossyear, already wired
  and anonymous, remains the fast public historical-loss proxy.)

### MCP/API test-report fixes (2026-06-01, after a deep external MCP audit)

- **MCP silent truncation fixed at the wrap layer.** Every `tools/call` result
  was emitted twice on the wire (a `content` text block **plus** a
  `structuredContent` mirror of the same JSON), ~2× the inner size — so even a
  13 KB answer breached the host's ~25 KB cap and the agent received JSON cut
  mid-token. `mcp_wrap_call_tool_result` is now budget-aware
  (`EMEM_MCP_RESPONSE_BUDGET_BYTES`, default 24 KB): it keeps the load-bearing
  `content` text always, drops the doubling `structuredContent` mirror when the
  two-copy envelope would breach budget, and when even one copy is over budget
  it generically slims the inner JSON (keeping the small identifying/auth fields)
  and attaches an honest `_emem_truncation` marker naming every omitted field +
  a REST pointer. No payload is ever silently lost.
- **`/v1/state_multi` slim default.** The four foundation vectors (~13 KB) are
  omitted by default — the response carries each encoder's `dim`/`l2_norm`/
  `fact_cid`/`memory_token`/`tslot` plus a `vectors_omitted` flag. Pass
  `vectors:true` (or `include:["vectors"]`) to inline the raw floats.
- **Geocoder cascade hardening.** `nominatim_bbox_for` and `sample_cells_in_bbox`
  now validate + normalise upstream bboxes (reject non-finite, swap inverted
  corners, drop wider-than-sphere relations). `sample_cells_in_bbox` and
  `sample_cells_in_polygon` are never-empty for a finite bbox (centroid-cell
  fallback) — fixes the "0 sample cells at a neighbourhood" defect.
- **NDVI false-Absence fixed at the root.** The Sentinel-2 materializer now
  picks scenes SCL-first across multiple candidates (`EMEM_S2_MAX_SCENES`,
  default 4) instead of false-Absencing when only the *latest* scene's pixel is
  cloudy — a clear scene a few days back is preferred. The signed fact records
  how many scenes were probed; a genuine Absence (every candidate cloudy) names
  that. `jepa_predict` now distinguishes a signed cloud-Absence from "no data"
  in its error so an agent backfills the right way. No fabricated values.
- **Warm-path result caches** for `/v1/state` and `/v1/memory_contradictions`,
  keyed on the request + a global corpus write-generation counter so they
  invalidate the instant a new fact lands (TTLs `EMEM_STATE_CACHE_TTL_MS` /
  `EMEM_CONTRADICTIONS_CACHE_TTL_MS`). Repeat calls drop from ~950 ms / ~6.3 s
  warm to a HashMap lookup, without ever serving stale data.
- **Cop-DEM resolution doc truth-pass.** The responder fetches Copernicus DEM
  **GLO-30 (30 m)** from the AWS Open Data COG mirror, but several strings still
  advertised the retired Open-Meteo "90 m" path. Fleet metadata, materializer
  wire-path, the input-resolution table, the elevation interpretation strings,
  and the band-source comments now all say 30 m. Permanent-water cells
  (WorldCover class 80, e.g. the Dead Sea) carry a clearer caveat pointing to
  `bathymetry_m`.
- **RADD provenance kept honest + current.** No public unauthenticated COG
  exists for RADD / GLAD / GFW-integrated alerts (verified 2026-06-01: S3 is
  Requester-Pays 403, the HTTPS geotiff path 404s, the Data API needs a key), so
  the connector continues to sign an honest Absence rather than fabricate a "no
  alert" Primary. The version tag is bumped to the verified-current `v20260524`,
  the source scheme + derivation fn_key now *derive* from that tag (so signed
  provenance can't drift from the disclosed vintage), and the disclosure names
  the one fetchable NRT alternative (NASA OPERA DIST-ALERT, Earthdata-Login
  gated).

## [0.0.9] — 2026-05-30

_Finishes the remaining v0.0.8 items and lands the v0.0.9 "memory that connects
& evolves" feature set. Every change is additive — v0.0.6/v0.0.7/v0.0.8 receipts
and attestations still verify byte-identically (regression-tested)._

### Audit + hardening round (after a 5-lens recon sweep; all gates green, 684 tests)

- **Edges reverse lookup** (`obj→subj`): `/v1/edges/recall` + `emem_edges_recall`
  gain `obj` + `direction:"in"`; the forward/reverse filters share one
  bi-temporal helper so they can't drift. Ambiguous requests error (no silent
  empty).
- **Contested marker surfaced in recall**: the refinement loop's
  `emem.fact_contested` flag now rides recall responses as advisory metadata —
  kept *outside* the signed fact CID / receipt preimage (byte-identical when
  absent).
- **`emem-scorecard` live mode**: `--live --dataset <jsonl>` loads a corpus into a
  responder and computes the four axes + topline from real responder output
  (committed 16-item sample; full-dataset instructions in `docs/benchmarks.md`).
  Honestly labels the lexical-fallback read path when no embedder is loaded.
- **`emem-sleep-agent` crate** (opt-in `EMEM_SLEEP_AGENT=1`): the LLM rewrite/merge
  loop layered on the deterministic refinement — picks contradicted/high-churn
  memory, asks a configurable LLM to reconcile, writes a superseding (non-
  destructive) memory. Real LLM path; `--dry-run` works offline and refuses to
  fabricate when no key is configured.
- **Fixes**: removed the dead `fail_below_de_minimis` from the eudr tool prose;
  corrected `/v1/attest` OpenAPI (`batch_root` byte-array, required `kind`);
  malformed `cell` IDs no longer fuzzy-resolve to a confident wrong place
  (cell64-shaped-but-invalid → typed error; weak geocoder matches report
  `is_high_confidence:false`); MCP async-task TTL/result-writeback hardened +
  a `mcp_tasks_lock()` helper; **MCP task id now folds a monotonic counter** so
  concurrent same-tool spawns can't collide (was a real ~1-in-4 flake); `/v1/soil`
  per-cell cap (`EMEM_SOIL_MAX_CELLS`, default 64) with honest `coverage_capped`.
- **Surface counts** reconciled to the real **70 tools (10 core / 60 extended)**
  and 65 read-only across README/docs/web/json discovery files.
- **Test coverage** added for the real `recall_edges_tree` bi-temporal boundary
  (`valid_to == as_of` is inclusive), MCP task capacity/eviction, scope's
  exact-four-tuple miss, the refinement severity floor + pair cap, and edge
  self-loops; replaced the no-op `refinement_disabled_by_default`.

### v0.0.9 — connectivity layer (the headline)

**Temporal knowledge-graph edges** (commit `d369948`):
- New sibling type `EdgeFact { subj, pred, obj, valid_from, valid_to?,
  confidence, signer, signed_at, .. }` in `crates/emem-fact/src/edge.rs`,
  content-addressed like a fact. NOT a `Fact` variant — the frozen `Fact` CBOR
  is untouched.
- Sled trees `emem.edges` / `emem.edge_spo` (big-endian `valid_from` so range
  scans ascend) / `emem.edge_ops`; `add_edges` / `recall_edges` / `has_edge` on
  the Storage trait (default no-op so all impls compile). Bi-temporal
  supersession: a newer `valid_from` shadows the older without deleting it.
- Edges ride the existing `Attestation` envelope (`edges: Vec<EdgeFact>`,
  serde-default); `verify_attestation` folds edge leaves into the merkle root
  only when non-empty → empty-edges attestations are byte-identical.
- New optional receipt preimage segment `edges_blake3_hex` (after `as_of`,
  before `manifest`); `sign_receipt_with_edges([])` short-circuits to the
  existing path → byte-identical for every legacy call site.
- `POST /v1/edges` + `POST /v1/edges/recall`; MCP tool `emem_edges_recall`;
  `include:["edges"]` attaches a fact's edges to `/v1/recall`.

**Contradiction-fed refinement loop** (commit `ab7d2ec`):
- Opt-in (`EMEM_REFINEMENT_ENABLED`, default off) scheduler pass that consumes
  the existing signed contradiction signal and records one `disagrees_with`
  edge per high-severity disagreeing pair (`valid_from` = contested tslot,
  `confidence` = severity), plus a non-destructive `emem.fact_contested` marker
  on the lower-confidence fact. The fact body is never mutated.
- Idempotent by construction: dedupe is on the logical key
  `(subj, pred, obj, valid_from)` (not the CID, since `signed_at` varies), so a
  re-run emits zero new edges. Edges are batched into one responder-self-signed
  attestation — no new key material.

**MCP async tasks + protocol 2025-11-25** (commit `84d943a`):
- Verified `2025-11-25` is a real published MCP revision; added it as the
  negotiation default (older revisions still supported).
- Async task handles for long-running tools (`emem_eudr_dds`, `emem_hunt`):
  `tools/call` with a `task` param returns `{taskId, status}`; `tasks/get` /
  `tasks/result` / `tasks/list` / `tasks/cancel` poll it. Bounded registry
  (256 slots, TTL eviction). Sync path unchanged when no `task` param.
- `tasks` capability advertised only at `2025-11-25`. `sampling` /
  `elicitation` deliberately NOT advertised (no implemented path — honesty).

**`emem-scorecard` scorecard** (commit `6e2561d`):
- New crate/binary scoring a responder on four MemoryAgentBench-style axes
  (retrieval, test-time learning, long-range, conflict resolution) + a
  LongMemEval-S-style topline. `--self-test` runs offline against a built-in
  fixture; live mode embeds the responder's signed receipt. Scores are
  computed, never fabricated.

### v0.0.8 — completion

**Scope filtering end-to-end** (commit `c150cc7`): the existing `Scope`
four-tuple now filters reads. New `emem.scope_index` tree populated on scoped
writes; `scan_cell_in_scope` (default falls back to `scan_cell`); optional
`scope` threaded through `recall` + the other read primitives. A fact written
under `{user_id:"u1"}` is invisible to a `u2` recall.

**Vault memory kind** (commit `c150cc7`): `MemoryKind::Vault`, AEAD-sealed
(ChaCha20-Poly1305, key via HKDF-SHA512 off the responder ed25519). Reads return
ciphertext unless the caller signs a `vault_open` capability over
`blake3("emem.vault_open|"||path||"|"||nonce)`. Excluded from `memory_search`
and contradiction detection.

### Fixed (P2 correctness) (commit `d3b829b`)

- `aqi_class@1` PM2.5 24-hour breakpoints updated to the 2024-05-06 EPA final
  rule (Good→Moderate 12.0→9.0; Unhealthy upper 150.4→125.4; Very Unhealthy
  250.4→225.4); citation updated. Registry CID legitimately changes.
- Removed the dead `eudr_cell_verdict` code 5 `fail_below_de_minimis` (no path
  emitted it; strict EUDR has no de-minimis — the 0.5 ha MMU floor → `below_mmu`
  is the real behavior).

### Content & narrative

- Docs: one "connects & evolves" thesis; de-jargoned intros; reconciled stale
  counts (46 source schemes, 16 data + 13 utility connectors) + declared-but-
  unwired honesty note; new `why-agents` / `only-emem` / `connect-and-evolve`
  pages.
- Website: peer-comparison block (vs Mem0 / Letta / Zep / Anthropic memory
  tool), connect-&-evolve diagram, try-it lifted to the hero, edges surfaced.
- Agent surfaces: `agent_card` "first 5 minutes" + cite-this-fact blocks;
  de-jargoned `emem_state` / `state_multi` / `temporal_route` /
  `memory_contradictions`; `examples/connect-and-evolve.md` runnable walkthrough.

### Runtime algorithm endpoints

Five algorithms that the registry previously carried as `documentation_only`
(their formula needs a multi-year series or a two-scene pair that the scalar
evaluation-AST cannot express) now have runnable surfaces. Each signs its
result and returns an honest `inconclusive` verdict — no fabricated number —
when its inputs aren't materializable. The registry entries stay
`documentation_only` with a `runtime_path` pointer at the new endpoint.

- `POST /v1/deforestation_alert` — `carbon.deforestation_alert_proxy`: the full
  NDVI-drop + Tessera embedding-change composite, each half degrading
  independently.
- `POST /v1/triple_consensus` — `clay_prithvi_tessera` change-ensemble; degrades
  to a signed `inconclusive` without the GPU sidecar or two distinct vintages.
- `POST /v1/spi` — McKee-1993 Standardized Precipitation Index drought metric.
- `POST /v1/burn_severity` — Key & Benson dNBR burn severity.
- `POST /v1/rice_ch4` — IPCC-2019 Tier-2 rice-cultivation CH4 (Eq 5.1).

Added to the `/openapi.json` spec builder, the `/v1/agent_card` `surfaces` map,
and `docs/agents.md`.

## [0.0.8] — 2026-05-28


**Change 1a — Scope foundation** (commit `ab1fa85`):
- New `crates/emem-fact/src/scope.rs`: `Scope { user_id,
  agent_id, run_id, org_id }` with canonical CBOR + blake3
  digest + 8 unit tests.
- `Receipt` gains `Option<Scope> scope`; serde skip-if-none
  keeps v0.0.6/v0.0.7 receipts byte-identical.
- New `Server::sign_receipt_with_scope` extends the preimage
  with `scope_blake3_hex` between `served_at` and `primitive`
  only when the scope is non-empty. Short-circuits to the
  legacy `sign_receipt` on empty scope so the byte stream
  matches v0.0.7 exactly.
- `/v1/verify_receipt` branches on receipt scope presence and
  reports `scope_bound: bool` in the response.
- Backward-compat verified end-to-end on prod.

**Change 3 (partial) — A2A v1.2 Agent Card** (commit `3d9d58c`):
- `/.well-known/agent-card.json` `protocolVersion` bumped `0.2` -> `1.2.0`.
- description rewritten with memory-substrate framing.
- `supportsAuthenticatedExtendedCard: false` added per A2A v1.x.
- The `POST /a2a/tasks` task adapter is deferred to a follow-up
  commit. A2A clients keep working today through the existing
  `additionalInterfaces` entry pointing at `/mcp`.

**Change 4 — `emem-langmem` Python BaseStore** (commit `3d9d58c`):
- New package `sdks/emem-langmem/`. LangChain `BaseStore[str, bytes]`
  over emem MCP. Maps `mget`/`mset`/`mdelete`/`yield_keys` onto
  `memory_view`/`memory_create`/`memory_delete`. Sync + async.
- 7/7 mocked-transport tests green.
- PyPI publish gated on the v0.0.8 tag.

**Change 2 (partial) — `Core` memory kind**:
- `MemoryKind` enum extended with `Core` (MIRIX six-type taxonomy
  parity). `from_wire` / `as_str` / `default_ttl_days` round-trip
  the new variant; default TTL infinite.
- New `listing_priority()` orders kinds Core -> Procedural ->
  Semantic -> Episodic -> Resource. `memory_view` directory
  listings now sort Core entries first so an agent boot-strap
  reads the persona block before crawling the tree.
- New env var `EMEM_MEMORY_TTL_CORE_DAYS` for retention override.
- `Vault` (AEAD-sealed compartment) deferred to its own commit
  because the ChaCha20-Poly1305 + sled tree + ed25519 cap-binding
  work is substantial; no half-implementation lands.

**Carried forward to v0.0.9**:
- Change 1 fan-out — scope wired through the other 8 read
  primitives (find_similar, trajectory, query_region,
  recall_polygon, state, state_multi, memory_bundle, memory_search).
  /v1/recall proves the pattern; the other 8 are mechanical.
- Change 2b — Vault memory kind (AEAD + cap-binding). Substantial
  ChaCha20-Poly1305 + HKDF-from-attester-ed25519 + sled tree + cap
  preimage work; deserves its own commit cycle rather than
  risking a rushed crypto implementation in the v0.0.8 cut.


## [0.0.7] — 2026-05-28

### Added
- **Official MCP Registry listing.** `server.json` at the repo root
  follows the `2025-12-11` schema; a new GitHub Actions workflow
  (`.github/workflows/mcp-publish.yml`) auto-publishes the manifest on
  every `v*` tag via GitHub OIDC. Manual one-off available via
  `scripts/mcp-publish.sh`. Registry name: `io.github.Vortx-AI/emem`;
  the GitHub aggregator at `github.com/mcp` ingests on a delay.

### Fixed
- `server.json` `documentationUrl` pointed at a renamed `docs/SPEC.md`;
  repointed to `https://emem.dev/docs/whitepaper.html`.
- `/v1/locate` now mirrors the resolved `cell64` as `cell` at the top
  level of the response. The request-side alias was already there;
  this closes the response-side gap so naive agents doing
  `recall({cell: locate_resp.cell})` work first try.
- `docs/errors.md` regenerated from the live `/v1/errors` payload
  (28 codes, schema `emem.errors.v1`). Previously documented codes
  the responder never emits are gone; the catalog is now wire truth.
- Discovery surfaces (`web/agent.json`, `web/ai-plugin.json`,
  `examples/gemini-extension.json`, all docs + diagram SVGs) refreshed
  to the canonical counts: 69 MCP tools (10 core / 59 extended),
  18 static MCP resources + 8 URI templates, 42 user-callable bands,
  46 source schemes, 159 algorithms, 80 paths under `/v1/*`,
  83 total OpenAPI paths.

### Notes
- Wire change is purely additive: the new `cell` field is a mirror of
  `cell64`, never replaces it. v0.0.6 and v0.0.7 receipts continue
  to verify under v0.0.7 code.


### Added (2026-05-16)

- **Memory substrate, five new endpoints.** `POST /v1/state` returns a signed
  dense per-place embedding (`view=encoder` default 128-D Tessera; `view=cube`
  full 1792-D voxel). `POST /v1/state_multi` fans across `geotessera` +
  `clay_v1` + `prithvi_eo2` with a typed `missing[]` for unwired encoders.
  `POST /v1/state_diff` returns the per-element residual, its L2 norm, and the
  cosine between two vintages at one cell, with both source `fact_cid`s. `POST
  /v1/memory_token` composes a `memt:<cell64>:<fact_cid>` citation handle;
  `POST /v1/memory_token/resolve` dereferences it in one round-trip back to the
  signed fact body.
- **Liveness streaming.** `GET /v1/stream` ships Server-Sent Events: a signed
  `corpus.state` tick every `interval` seconds (default 15, clamped to
  [5, 300]). Each tick carries a deterministic preimage and ed25519 signature
  so subscribers verify without re-fetching. 30 s keep-alive comment keeps
  proxies from dropping the connection. Per-cell subscribe filters remain
  flagged in §20.
- **Operator attestation upgrade.** `/.well-known/emem.json` now binds
  `binary_blake3` (BLAKE3 of the running executable, read once from
  `/proc/self/exe`) + `git_commit` (compile-time from `cargo:rustc-env`) +
  `build_timestamp` under the responder's ed25519 key, so a verifier can
  confirm the live binary corresponds to the published source tree without
  trusting the operator. `tee_quote` stays `null` on this responder; full
  Intel SGX / AMD SEV-SNP attestation populates it when deployed under a TEE.
- **Agent benchmark.** `GET /v1/benchmark` returns 5 hand-verified eval items
  (elevation recall, NDVI, find_similar neighbours). `POST /v1/benchmark/grade`
  scores a submitted answers map per item with exact-match for `fact_cid` and
  cell-plus-score for `find_similar` items.
- **Corpus liveness snapshot.** `GET /v1/corpus_state_stats` returns the same
  payload the SSE tick carries (signed), as a one-shot poll for agents that do
  not want to hold an SSE connection.
- **7 new MCP tools.** `emem_state`, `emem_state_multi`, `emem_state_diff`,
  `emem_memory_token`, `emem_memory_token_resolve`, `emem_corpus_state_stats`,
  `emem_benchmark` join the `tools/list` catalog and are dispatchable via
  `tools/call`. Total MCP tools: 58 (was 51).
- **Seven framework MCP examples.** Geospatial-agent reference implementations
  under `examples/` for LangChain (LangGraph), LlamaIndex, AutoGen, CrewAI,
  Pydantic AI, Agno, and Mastra (TypeScript). Each auto-discovers the live
  emem MCP tool catalog and answers a Helsinki Airport flood-risk question
  with cited receipts. Override the MCP URL via `EMEM_MCP_URL` to point at a
  local responder.

### Changed

- `/v1/agent_card` `primary_tools` now leads with `emem_state` +
  `emem_memory_token` so agents that read the card first wire the substrate
  before recall. `surfaces` map adds explicit pointers for state*/memt*/
  stream/benchmark/corpus_state_stats.
- `/v1/discover` `primitives` block adds the five substrate endpoints;
  `fanout` adds `stream`, `benchmark`, `corpus_state_stats`, and
  `operator_attestation`. `/.well-known/agent-card.json`
  `capabilities.streaming` flipped to `true` since `/v1/stream` ships.
- `/.well-known/emem.json` adds `stream_url`, `corpus_state_stats_url`,
  `benchmark_url`, `state_url`, `memory_token_url`,
  `memory_token_resolve_url` pointers alongside the existing `tools_url` /
  `openapi_url` / `mcp_url`.
- `/v1/openapi.action.json` (Custom GPT subset) extended to include the 7
  substrate + liveness operationIds.
- Homepage `/` cuts three sections (Encoders, Workloads, Limits) and adds two
  (Memory substrate at §05; Live stream + Operator attestation at §09 with
  live JS that fetches `/v1/stream` and `/.well-known/emem.json` on load).
  Mobile breakpoints added at 720 / 560 / 480 px. Wide tables wrapped in
  `.scroll-x`. Hero CTAs collapsed to four clear labels (Read whitepaper /
  OpenAPI / GitHub / Copy MCP config — with `execCommand` fallback).

### Fixed

- `tools/call` dispatch arms added for the 5 substrate tools — earlier they
  were listed in `tools/list` but rejected with `unknown tool` on call.
- `examples/langchain/`, `examples/llamaindex/`, `examples/agno/` switched
  from hardcoded `https://emem.dev/mcp` to `os.getenv("EMEM_MCP_URL", ...)`
  so local-responder testing works without patching source.

## [0.0.6] — 2026-05-14

Triple-encoder consensus pattern, foundation-embedding `/v1/ask`
fan-out, /verify in-browser receipt verifier, MCP/REST parity sweep,
docs rewritten against ground truth.

### Added

- **Six triple-consensus algorithms.** `deforestation_triple@1` (Hansen
  GFC mask uplift), `wetland_change_triple@1` (JRC GSW recurrence
  delta), `urban_expansion_triple@1` (Overture buildings delta + s2.B11
  SWIR corroboration), `disaster_anomaly_triple@1` (spatial, 2-σ
  neighbour z-score), `climate_archetype_triple@1` (12-class
  Köppen-Geiger classifier with type-locality centroid seed),
  `coastal_erosion_triple@1` (bathymetry-clamped to active-coastline
  cells). algorithms-v0.json grew 149 → 155 entries.
- **AlgorithmSpec `parameters` + `learned_from` + `prerequisites`
  fields.** Algorithms ship typed tunable thresholds with citation
  provenance for every tuned number. Accessors: `Algorithm::param_f64`,
  `param_str`, `param`.
- **`/v1/ask` foundation-embedding fan-out.** Keyword intent
  classifier (Similarity / Change) wires a concurrent fan-out across
  Clay v1.5 + Prithvi-EO-2.0 + Tessera; response carries
  `foundation_embeddings` envelope with per-encoder neighbours and
  cross-encoder consensus voting (`all_three` / `two_of_three` /
  `one_or_none`). Budget read from
  `clay_prithvi_tessera_triple_consensus@1.parameters.ask_timeout_ms`.
- **13 new MCP tools.** Domain shortcuts (one-shot locate → recall →
  aggregate): `emem_at`, `emem_ndvi`, `emem_air`, `emem_lst`,
  `emem_soil`, `emem_water`, `emem_forest`, `emem_weather`. Utility +
  bulk: `emem_recall_many`, `emem_elevation`, `emem_fleet`,
  `emem_temporal_route`, `emem_verify_receipt`. TOOLS catalog 36 → 49.
- **Köppen-Geiger archetype seed** at
  `crates/emem-core/data/climate_archetype_centroids_v1.json` — 12
  type localities (Beck et al. 2018, Scientific Data 5:180214) that
  back `climate_archetype_triple@1`.
- **find_similar Hamming inline-derive.** `load_cell_bin128`
  inline-derives bin128 from any cached geotessera vintage via
  TurboQuant sign-bit packing (seed
  `emem.binary_embedding.turboquant.v1`) when the binary sibling band
  is absent. ~1 ms vs ~30 ms materializer round-trip.
- **EWMA-adaptive triage** for `find_similar` mode
  `hamming_then_rerank`. Lock-free AtomicU64 stores observed
  Hamming↔cosine recall@k; the oversampling factor becomes
  `ceil(1/recall)` clamped to [4, 16]. ~50-call warm-up; before
  warming, behaves as the historical 4× floor.
- **query_region default `max_cells` is bbox-area-derived** —
  target ~1 cell per (10 km)², clamped to [64, 1024]. Honours
  explicit `max_cells` from the caller. Small parks no longer pay
  the full 256-cell cost; large regions get a denser sample.

### Changed

- **Gazetteer unified through `emem_fetch::geonames`** (68 581
  cities5000 corpus). Deleted two duplicate hand-curated GAZETTEER
  tables (30 + ~120 entries) in `find_similar.rs` and lib.rs; forward
  and reverse lookups now route through `emem_fetch::geonames::lookup`
  and `nearest_label(max_km=25)`. Index warmed on a worker thread at
  router boot.
- **`enrich_find_similar_response()` extracted** into a shared helper.
  REST handler and the MCP `emem_find_similar` arm both call it, so
  similarity_method / band_used / deep_recall_url / scene_png_url /
  place_label_cached land byte-identically across surfaces.
- **Topic-router 0.35 threshold** now declares
  `_threshold_learned_from` provenance in `topics-v0.json`. Code-side
  fallback is a named `DEFAULT_TOPIC_THRESHOLD` const.
- **`flood_risk@2` 5-m DEM agreement threshold** moves to the
  `parameters` block with `learned_from` citing the ESA Cop-DEM PSD
  §5.4 CE90 vertical accuracy as the physical anchor.

### Fixed

- **Topic-router methane/SWIR aliases.** Analytics topic gets 16 new
  aliases (methane plume, SWIR anomaly, 2190 nm, fugitive emissions,
  …) and `s2.B11`/`s2.B12` bands. Keyword pre-pass surfaces
  `analytics` and `methane_plume_swir_anomaly@1` on direct methane
  questions.
- **find_similar auto-materialize on miss.** REST + MCP handler
  triggers `try_materialize_bands` for the requested vector band when
  find_similar returns `CidNotFound`, then retries — one call instead
  of two.
- **Locate admin-boundary fallback.** Karnal-district-style queries
  now reroute through Overture `divisions/division_area` when
  Nominatim returns a POI courthouse. Cached POI-scale (<0.01°)
  bboxes trigger forced Overture override. Surfaced as
  `via: "overture_admin_fallback"`,
  `polygon_source: "overture_division_area"`.
- **JEPA v2 short-circuit on untrained.** `jepa_v2::is_trained()`
  metadata-only OnceLock; when false, `physics::jepa_predict_v2`
  returns `lag_window.last()` and skips ONNX + sidecar entirely.
  Receipt carries `via: "short_circuit_untrained"` and the original
  `untrained_baseline` honesty warning. Saves ~4.6 s of CUDA warmup
  on the residual-zero identity function.
- **GPU sidecar VRAM budget** walked up to 20 GB so Clay v1.5,
  Prithvi-EO-2.0, Galileo, and JEPA v2 co-reside without
  per-process cap trips.
- **Clay teacher pre-stage.** Pre-stage
  `timm/vit_large_patch14_reg4_dinov2.lvd142m` (~1.1 GB) at boot so
  `HF_HUB_OFFLINE=1` holds. Clay v1.5 ckpt hyperparams save
  `teacher="vit_large_patch14_reg4_dinov2.lvd142m"`, which differs
  from the `samvit_base_patch16.sa1b` class default in
  `claymodel/module.py`.

### Surfaces

- **`/verify` and `/verify/<fact_cid>`.** In-browser ed25519 receipt
  verifier. Reconstructs the canonical preimage
  `(request_id | served_at | primitive | cells | fact_cids)` and runs
  the signature math with `@noble/curves@1.6.0` +
  `@noble/hashes@1.5.0` from esm.sh. Falls back to
  `POST /v1/verify_receipt` if CDN imports time out. Handles every
  wire shape (`signature: byte[]` or `sig_b32`; `responder: byte[]`
  or `responder_pubkey_b32`).
- **`/humans` rebuild.** Try-it drawer (T key) for 20 primitives,
  manifest grid replacing the Poincaré disk, ontology SVG replacing
  the force-directed cell cloud, glossary chip strip, human/raw
  toggle on every right-pane card, first-paint preload on the
  densest attested cell.
- **Docs rewritten against ground truth.** 49 MCP tools, 155
  algorithms, 71 OpenAPI-documented paths (68 under `/v1/*`), 35 band
  cube slots + 118 materializer-wired band names, 43 source schemes,
  12 data connectors, 26 declared topics. Every count-bearing claim
  across README, AGENTS.md, docs/, web/agent.json,
  web/ai-plugin.json, web/humans.json, web/humans-llms.txt, and the
  /humans HTML text content now matches the live server.

## [0.0.5] — 2026-05-11

### Added

- **Fields of The World agricultural-boundary supplement.** Per-field
  polygons from the FTW global product (~3.17 B fields, 10 m, 241
  countries, CC-BY-4.0) via PMTiles range reads on `source.coop`.
  Surfaced as the standalone `/v1/field_boundaries` primitive and as
  the `include: ["ftw_fields"]` supplement on `/v1/recall_polygon`.
- **GeoNames cities5000 + Overture `divisions/division_area`.**
  Locator cascade upgraded so the OSM rate limit is no longer the
  bottleneck on every `/v1/locate` call; the gazetteer answers
  ~68 581 populated places in-process, and admin boundaries resolve
  through Overture polygons.

## [0.0.4] — 2026-05-05

Public interactive surface at `https://emem.dev/humans` — the page is its
own API console. Every visible cell carries `data-emem-cell`, `band`,
`fact-cid`, `tslot` attributes; every interactive control carries
`data-emem-action`. A scraping LLM extracts everything from the rendered
DOM. A live console pane prints every `/v1/*` call the page makes with
copy-as-curl / copy-as-python / copy-as-MCP pivots and a replay button.

#### Added
- `web/humans.html` (~3.2 K LoC, single self-contained file) replaces
  the v1 dashboard. Constellation field, Verlet force-graph, Poincaré
  registry view, Sigstore-Rekor-style attestation log, lasso →
  `/v1/recall_polygon`, embedding-PCA reprojection over 128-D Tessera
  vectors fetched via `/v1/recall_many`, command palette, hash chips
  with click-to-copy, focus mode, collapsible rails, mobile bottom-sheet,
  touch-lasso path, URL state encoding (`?cell=…&proj=embed&mode=log&layout=…`)
  so a tweeted link reproduces the exact view.
- Sibling routes wired in `crates/emem-api-rest/src/lib.rs`:
  `/humans.json` (JSON twin, `schema=emem.humans.v1`),
  `/humans/llms.txt` (page-scoped llms.txt convention),
  `/humans-og.svg` (1200×630 OpenGraph card).
- Pinned offline-verify libs from `esm.sh`: `@noble/curves@1.6.0/ed25519`
  + `@noble/hashes@1.5.0/blake3`. Preimage builder mirrors
  `crates/emem-storage/src/server.rs:132-148` byte-for-byte; verifies
  receipts in the browser without re-contacting the responder.

#### Fixed
- CSP header was blocking `https://esm.sh`, so the page silently fell
  back to server-side `/v1/verify_receipt` while labelling itself "CDN
  libs unavailable for offline path" — read as a verify failure.
  `crates/emem-api-rest/src/lib.rs` CSP now lists esm.sh in `script-src`
  and `connect-src`. Offline verify actually runs offline.
- `find_similar` 404 on cold cells (no `geotessera` attested) now
  auto-materialises via `/v1/recall` and retries instead of swallowing
  the responder's hint into a bare "HTTP 404".
- `installChips` was reading `textContent` after `setMan` had ellipsised
  it, so manifest-CID chips and the rail pubkey chip copied
  `"abc...123…"` instead of the full base32. Now prefers `el.title`.
- Family filter no longer blanks the canvas — cells derive a real
  dominant family from `/v1/coverage_matrix` instead of all defaulting
  to `'foundation'`.
- Lasso auto-exits after polygon submission; touch-lasso path added so
  the chip works on mobile (was unreachable — single-finger drag always
  routed through pan).
- rAF twinkle loop honours `prefers-reduced-motion`.
- Console `aria-live=off` (was announcing every API call to screen
  readers) + `CONSOLE_MAX_ROWS=250` cap so DOM doesn't grow unbounded.
- `--fg-mute` lifted from `#5A5C66` (2.96:1, fails WCAG AA) to
  `#7A7D87` (4.5:1).
- Five top-edge absolute clusters consolidated into a single bottom
  dock (`modes | projection | zoom | focus`); top of the canvas is
  now empty so hover tooltips and the centred hero have room.

#### Doc-only
- README's "Foundation embeddings" line corrected: ships 8 annual
  Tessera vintages 2017–2024 plus `bin128` and `multi_year`, not
  "vintage 2024" only.
- Deferred-section claim "upstream is 2024-only" rewritten — the
  upstream has all 8 vintages; the JEPA-v2 training blocker is
  candidate-pool selection (most cells need backfill before they
  carry the multi-year stack), not upstream availability.

### Sweep — 2026-05-08

Fresh memory rebuild from code, full P0+P1+P2 fix sweep, then docs
redo. The summary: every honesty gap surfaced by the parallel audit
is closed in code or removed from the surface; nothing is left as a
stub or "lands in v0.1".

#### Added
- `verify mode=Resolve` actually resolves on miss. Previously
  degraded silently to Fast. Now calls
  `storage.materialize_many(&[CanonicalKey])` with the targeted
  tslot (or single-point window) and re-scans. Open-ended windows
  with no targetable tslot fall back to Fast — documented in the
  doc comment, not silently swallowed. `MaterializeMiss` (no
  upstream connector) bubbles to the caller.
  (`crates/emem-primitives/src/verify.rs:92-111`.)
- `find_similar.filter` honours structured `Claim` predicates with
  per-cell verdict memoisation, applied in both cosine and binary
  scoring paths. Cells with no fact for the filter band are dropped
  (undecidable, not "false") so an agent asking "find places like X
  where NDVI > 0.5" does not get silent inclusion of cells with no
  NDVI history.
- `Receipt.merkle_proof` populated end-to-end:
  - `emem_attest::merkle_root_and_paths(leaves) -> (root,
    Vec<path>)` returns root + per-leaf bottom-up sibling paths in
    one pass.
  - `emem_attest::verify_merkle_path(leaf, idx, path, root)` rebuilds
    the root from a single proof.
  - `MaterializingStorage::put_attestation` persists per-fact
    `MerkleProof` records to a sled tree `emem.fact_proofs`, keyed
    by FactCid string. Leaves are sorted by their 32-byte leaf
    hash; `MerkleProof.leaf_index` is the sorted-order position.
  - `Server::sign_receipt` populates `Receipt.merkle_proof` from
    the first cited fact's stored proof.
- `query_region` accepts `bbox:lon_min,lat_min,lon_max,lat_max`
  geometry. Synthesis caps at `MAX_BBOX_CELLS = 4096` (~6.4 km ×
  6.4 km at the equator) and `MAX_REGION_FACTS = 65_536`. Beyond
  either cap the responder stops scanning and aggregates over what
  it has; `receipt.fact_cids` reflects exactly what contributed.
  GeoJSON polyfill returns a structured error.
- JEPA-v2 trained-checkpoint loader at
  `python/jepa_v2_sidecar/server.py:_Registry.load_dynamics`:
  `torch.load(weights_only=True)` → `load_state_dict(strict=True)`
  (architecture-name drift fails at load, not at prediction) →
  optional `blake2b_hex(state_dict_bytes) == declared_hash`. The
  pre-existing `RuntimeError("loader not shipped yet")` guard is
  gone; its concern (silent garbage outputs) is preserved by the
  strict load + hash check.
- Terraclimate failover: `crates/emem-fetch/src/terraclimate.rs`
  defines `NCSS_BASES = [UI primary, NCAR RDA secondary]`.
  `fetch_terraclimate_normal` tries each in order; the receipt's
  `Source.url` records which mirror answered.
- Documented the user-vs-system-mode `cap_net_bind_service` story in
  `ops/systemd/emem-server.service.example` and `docs/operators/operating.md`.
  An earlier attempt to add `AmbientCapabilities=CAP_NET_BIND_SERVICE`
  for the user unit failed in production: the kernel does not honour
  that directive for user-mode systemd (no UID transition for the
  user manager to prime), so the unit crash-looped with
  `status=218/CAPABILITIES`. The directive only works for system-mode
  units. User-mode deployments stay on `setcap cap_net_bind_service=+ep`
  re-applied by `scripts/redeploy.sh` after every release build.
- 4 query_region bbox lock-in tests (round-trip, oversized cap,
  malformed, inverted).
- 2 Merkle path lock-in tests (single-leaf empty path,
  odd-cardinality self-pair).
- 14 fresh docs files: `README.md`, `CONTRIBUTING.md`,
  `CHANGELOG.md` (this), `docs/{agents, protocol, architecture,
  registries, data-sources, inference, developing, operating,
  whitepaper}.md`.
- 12 fresh memory files capturing code-verified ground truth
  (`project_codec`, `project_registries`, `project_trust_layer`,
  `project_fetch_inventory`, `project_primitives`,
  `project_api_surface`, `project_cli_binaries`, `project_intent`,
  `project_inference`, `project_external_surface`,
  `project_integration_gaps`, `feedback_parallel_audits`).
- `chirps.daily.v2` connector wired end-to-end. New module
  `crates/emem-fetch/src/chirps.rs` (~310 LoC, 7 unit tests + 1 live
  test). Materializer `materialize_chirps_daily_precip` in
  `crates/emem-api-rest/src/lib.rs` signs Primary on real readings,
  Absence with structured `reason_text` on out-of-bounds (±50° lat),
  before-record (pre-1981), no-data (-9999.0 sentinel). New band
  `chirps.precip_daily_mm` at offset 1672 (1 dim); `reserved` shifted
  to 1673 with dims=119 (Σ=1792 preserved). Function `chirps.precip@1`
  registered. Live verification: Mumbai cell 2023-07-26 returns
  76.2 mm/day, 2023-07-27 returns 304.8 mm/day — heavy-monsoon ground
  truth, signed with populated `receipt.merkle_proof`.
- `/humans` interactive map at `https://emem.dev/humans`. Knowledge
  constellation of the corpus: every attested cell64 is a star
  positioned by Hilbert-ordered (lat,lng) projection (not Mercator),
  brightness scaled by fact density, colour by dominant band family.
  Click a star → right-pane shows facts + signed receipt + verify
  button; verify runs Ed25519 + BLAKE3 in-browser (`@noble/ed25519`
  + `@noble/hashes/blake3` via ESM CDN, falls back to
  `/v1/verify_receipt` if the imports fail). A `find_similar` graph
  view reveals the embedding topology. Console pane prints every
  `/v1/*` call so an LLM watching the page learns the agent API by
  observation. Single self-contained `web/humans.html`, 1101 LoC,
  served via `include_str!` on the new `/humans` route.

#### Changed
- HuggingFace Space `Dockerfile` pinned to
  `ghcr.io/vortx-ai/emem:0.0` (was `:latest`). A `:latest`
  deletion or upstream regression no longer dark-blacks the Space.
  SHA pin recommended in the comment for the next bump.
- `query_region` total-fact cap (`MAX_REGION_FACTS = 65_536`)
  added to defend against pathological dense-corpus + 4096-cell
  bbox combinations.
- `crates/emem-core/src/sources.rs` validator now accepts
  `providers: []` for a declared scheme. Replaces the older "no
  providers" hard-error that forced fake URLs into the manifest.
  An empty `providers[]` means the scheme name is recognised but
  no anonymous open-data path exists today — operators register
  their own key-bearing providers locally.
- `sources-v0.json:openet.30m.daily` providers list cleared with
  a `_note` documenting the blocker (the public S3 mirror returns
  `NoSuchBucket`; OpenET REST API and the GEE asset are both
  key-gated). Replaces the broken URL that previously made
  `/v1/sources` advertise a path that 404'd.

#### Removed
- `Mode::Zk` variant from `verify` — Rust enum
  (`emem-primitives/src/verify.rs`), MCP tool schema
  (`emem-mcp/src/lib.rs`), OpenAPI VerifyReq schema
  (`emem-api-rest/src/lib.rs`). The variant was advertised but had
  zero implementation; `mode=zk` returned 500 on every call. v0.1+
  may revisit.
- `Attestation.stake` field from `crates/emem-fact/src/attest.rs`
  and 9 call sites. Was reserved-for-v2.5; v2.5 will add a
  properly-named field if and when economics is designed.
- `find_similar.filter` Internal-error guard. Replaced with the
  actual evaluator above.
- 14 stale docs (`docs/{AGENTS, ATTESTING, CLIENTS, CONTRIBUTORS,
  DEPLOY, GO_LIVE, MATERIALIZERS, MILESTONE_v0.0.4, MULTIMODAL,
  PUBLISHING, SPACES, SPEC, TEMPORAL, WHITEPAPER}.md`) — replaced
  by the lowercase set above.

#### Fixed
- `verify mode=Resolve` no longer silently behaves as Fast.
- Production deploys no longer require manual `setcap` after every
  release rebuild.
- Receipts now carry Merkle inclusion proofs (was always `None`).

### Audit (parallel, 8 subsystems) — 2026-05-08

Eight Explore agents audited core+codec, fact / claim / attest /
storage, fetch / connectors, primitives, REST+MCP (live),
CLI+intent, GPU sidecar + JEPA / Prithvi / Galileo, and
SDKs+web+deploy. Findings:

- 73 REST endpoints + 34 MCP tools live and schema-aligned, tested
  on `127.0.0.1:5051`.
- 244+ workspace tests pass.
- Sources audit correction: original "11 unwired schemes" claim was
  wrong. Six are wired inline in `emem-api-rest/src/lib.rs`
  materialiser functions (gmrt, ornl_modis, nasa_power, open_meteo
  4-variant, soilgrids.v2, viirs.fire.nrt). Five are genuinely
  unwired: openet.30m.daily, dynamic_world.v1, tropomi.s5p.ch4,
  tropomi.s5p.no2, viirs.dnb.monthly.

## [0.0.4] — 2026-05-05

Polygon-aware boring endpoints, real physics primitives (heat /
wave PDE solvers + AR(2) NDVI predictor), agent-first homepage,
production SPEC.md, GDPR / UK-GDPR / DPDP-2023 / CCPA-CPRA
compliance surface.

### Added
- **Three real physics primitives.**
  - `POST /v1/heat_solve` — explicit FTCS 2D for `∂u/∂t = α∇²u`
    over a 9-cell stencil at the cell64 10 m pitch. Reads
    `modis.lst_day_8day` at the centre and 8 neighbours, integrates
    forward under `α·Δt/Δx² ≤ 0.20`, returns Kelvin forecast +
    initial condition + chosen `(n_steps, dt_seconds)`. Default
    α=1e-6 m²/s (Oke 2017 §2.3 table 2.4); horizon ≤168 h.
  - `POST /v1/wave_solve` — explicit CTCS 1D shallow-water for
    `∂²u/∂t² = c²∂²u/∂x²` along the seaward bathymetric gradient
    from `gmrt.topobathy_mean`, `c² = g·h`, `c` floored at 0.01 m.
    Sinusoidal forcing at the offshore boundary; hard wall at the
    coast; CFL safety 0.5. Land-locked rejection: offshore
    boundary ≥5 m AND ≥50 % of profile >1 m, else 422 + suggestion.
  - `POST /v1/jepa_predict` — closed-form AR(2) seasonal NDVI
    (`α=0.6, β=0.3, γ=0.1`, lookback ≤24 months). Surfaces
    `lag_12_used` so an agent can audit which terms drove the
    prediction. NOT a learned MLP.
- All three wired through MCP and OpenAPI; receipts verify offline
  via `POST /v1/verify_receipt`. Pure math (`heat_step_2d`,
  `wave_step_1d`, `jepa_predict_ar2_seasonal`) unit-tested without
  storage.
- `POST /v1/jepa_predict_v2` — pulls 3 latest Tessera vintages,
  routes to GPU sidecar, returns 128-D prediction. Receipt carries
  `untrained_baseline` warning until Tessera publishes
  multi-vintage history.
- **Polygon-aware boring endpoints** — `POST /v1/{ndvi, elevation,
  air, lst, soil, water, forest, weather, at}` resolve a place to
  an OSM polygon, fan out to up to 64 sample cells in parallel
  (`tokio::task::JoinSet`), return mean / median / min / max / std
  per band (mode + class distribution for categorical bands;
  centroid for vector embeddings). Knob: `n_cells` (default 16, max
  64, `1` forces point mode at the centroid).
- Visual + structured deliverables on polygon responses:
  `polygon.geojson` outline FeatureCollection, `polygon.scene_thumbs[]`,
  `polygon.scene_overlay_url` pointer, top-level `value_per_cell[]`
  + per-cell `geojson` FeatureCollection.
- `GET /v1/places/scene_overlay.svg?place=&band=&n_cells=&...` —
  server-rendered viridis SVG of the resolved polygon, cells
  coloured across the actual recalled min/max.
- `GET /v1/cells/:cell64/scene.rgb` — raw octet-stream RGB bytes
  with `x-emem-scene-{format,width,height,channels,...}` headers.
- `POST /v1/fetch` — REST mirror of MCP `emem_fetch`. Accepts
  either `{cid}` (lookup) or `{cell, band, [tslot]}` (materialise +
  persist).
- `POST /v1/elevation` cross-band coherent — recalls Cop-DEM (land),
  GMRT (ocean topobathy), ESA WorldCover (LC veto); reports
  `validity ∈ {land, ocean, coastline, unknown}`. Open ocean
  surfaces `elevation_m: null` + signed `bathymetry_m`, eliminating
  the `0.0` ambiguity.
- **Embedded band metadata** in `/v1/recall`, `/v1/cells/:cell`,
  `/v1/recall_polygon`, `/v1/ask`, boring endpoints. Every fact
  carries sibling `band_metadata` (description, units, value_range,
  interpretation, pitfalls, references) + `value_decoded` for
  categorical bands (ESA WorldCover LCCS, JRC Surface Water
  transition class, S2 SCL). Materialiser scalars
  (`copdem30m.elevation_mean`, `surface_water.*`, `s2.scl`) inherit
  metadata from their cube band and surface
  `inherited_from_cube_band`.
- `signer_pubkey_b32` + `responder_pubkey_b32` sibling fields on
  receipts. Raw 32-byte arrays remain intact for byte-for-byte
  verification; the base32-nopad string is for paste-into-`/v1/verify`
  ergonomics.
- `aqi_class@1` algorithm (chained `Where` ops on `cams.pm25` →
  EPA AQI 1-6), `weather_summary@1` (combined; sky / precip / temp /
  wind one-liner; Met Office / WMO METAR / Beaufort 8 thresholds).
  Total algorithms: 102 → 105.
- `air_quality` band entry — carved 7 dims off the front of
  `_reserved_512` (offset 192, shrunk 512 → 505) for CAMS scalars
  (`cams.pm25`, `cams.pm10`, `cams.no2`, `cams.o3`, `cams.so2`,
  `cams.co`, `cams.aod_550`). Bands count: 33 → 34;
  `total_dims` stays 1792.
- `/v1/ask` enrichments: `band_observations[]` inventory
  fall-through, `imagery_hint` block for imagery topics,
  `out_of_scope` caveat suppression when facts already exist,
  per-fact `band_metadata` + `value_decoded`.
- Agent surface: `/v1/openapi.action.json` (curated 28-op subset
  for OpenAI Custom GPT Action's 30-op cap), agent-first homepage
  rewrite, `/llms.txt` rewrite, `/agents.md` §5 anatomy of a
  numeric response, MCP resource templates
  (`emem://{band, algorithm, fact, cell}/...`).
- **GDPR / UK-GDPR / DPDP-2023 / CCPA-CPRA compliance surface.**
  SPEC.md §13 expanded to six subsections: per-band privacy class,
  no-PII-in-canonical-channel, Art. 6 lawful basis, data-subject-rights
  table, no-cookies disclosure, IP-handling
  (`agent_ip_hash = base32_nopad_lower(blake3(client_ip)[..8])`).
  `/v1/discover.fanout` adds `privacy`, `terms`, `spec`.
  `/.well-known/agent-card.json.provider` adds privacy / terms /
  support URLs and a `data_protection` extension.
- Privacy enforcement: `ops/systemd/journald-30day-retention.conf`
  with `MaxRetentionSec=30day`. POST canary verified absent in
  logs; GET canary present (paired with hashed IP, 30-day window).
- Production SPEC.md v0.0.4 (was v0.0.4-draft). §22 references
  split Normative + Informative; 43 citation keys defined for
  every upstream and every RFC-grade reference.
- Privacy + consent fixes (2026-05-06):
  - GA4 measurement ID moved out of public repo. `web/index.html`
    holds `__EMEM_GA_ID__`; the responder substitutes
    `EMEM_GA_MEASUREMENT_ID` at startup or strips the GA block
    entirely.
  - Consent storage moved from `localStorage` to a first-party
    cookie `emem_consent` (Path=/, Max-Age=180 days, SameSite=Lax,
    Secure). EU strict-mode browsers were clearing localStorage
    between sessions.

### Changed
- Workspace bumped to `0.0.4`.
- `/v1/discover` shrunk 130 KB → 1,026 B (134×). One-KB system-prompt
  fit: responder pubkey, 4 manifest CIDs, one-line algebra
  `Cell × Band × Tslot → Fact ; cid=blake3(cbor)/b32-32 ;
  sig=ed25519`, primitive→URL map, fanout pointers.
- `/llms.txt` 20 KB → 5 KB; `/agents.md` 27 KB → 16 KB;
  `index.html` 59 KB → 3.5 KB. Two paragraphs and working playground
  links a tool-less agent can follow.
- Hybrid topic routing — keyword exact-match boost runs ahead of
  the transformer pass even in transformer mode. Closes the
  `model2vec/potion-base-8M` 0.35-threshold gap on common Qwen-style
  prompts where a place noun dominated the embedding pool.
- Aggressive alias enrichment for `vegetation_condition`,
  `optical_raw_reflectance`, `radar_all_weather_sar`,
  `public_health` — the framings agents most commonly use that
  previously scored below threshold.
- Inventory-based algorithm dispatch tightened: requires
  `topics_matched > 0` AND every input the AST reads is in
  `want_bands`. Stops `flood_risk@2` and `aqi_class@1` from firing
  on "show me NDVI for Bengaluru".
- Cross-band coherent `/v1/elevation` (above) is the default now;
  point and polygon both route through `post_elevation_coherent`.
- Algorithm temporal-window materialisation parallelised via
  `tokio::task::JoinSet` — the previous serial 60 s timeout is
  gone for `/v1/temporal_route`.

### Removed
- 14 stale `docs/*.md` files (above) — replaced by lowercase
  `docs/{agents, protocol, architecture, registries, data-sources,
  inference, developing, operating, whitepaper}.md`.

### Fixed
- Polygon fan-out for embedded-gazetteer + cached places.
  `locate_inner` now enriches missing polygon bboxes via a single
  Nominatim `/search?q=…&limit=1` lookup at three sites — embedded
  hit, cache hit with no stored bbox, Photon hit with no extent —
  and re-caches the result so subsequent calls short-circuit.
- Polygon visual deliverables on `POST /v1/elevation` —
  `polygon.scene_thumbs[]`, `polygon.scene_overlay_url`,
  `polygon.geojson` outline now match what NDVI / LST / soil
  responses already shipped.
- Dockerfile: added `g++` to the build stage so `model2vec-rs`
  compiles cleanly in CI.
- Honest caveat suppression: `/v1/ask` `out_of_scope` only emits
  when `topics_matched`, `band_observations`, AND `facts.facts` are
  all empty.

## [0.0.3] — 2026-05-01

Closed gaps surfaced by the Katihar (Bihar) man-made-lake test
report — placeholders, hardcodes, silent fallbacks. Tightened every
protocol surface an external agent touches: geocoder cascade,
temporal vocabulary, algorithm registry, multimodal scene path,
brand identity.

### Added
- **Topic registry + transformer router.**
  `crates/emem-core/data/topics-v0.json` — 25 hand-authored topics,
  each `{key, description, aliases[], bands[], algorithms[]}`. The
  `topic_router` module embeds descriptions + aliases with
  model2vec-rs (`minishlab/potion-base-8M`, ~32 MB, sub-ms
  inference, pure-Rust) and routes free-text questions by cosine
  ≥0.35. Falls back to alias keyword matching when the model fails
  to load. Replaces ~639 lines of static `TOPIC_BANDS` /
  `TOPIC_ALGORITHMS` / `TOPIC_KEYWORDS` tables.
- **Formula-AST evaluator + composite dispatcher.** `Expr` enum
  in `emem-core::algorithms` (15 variants: Band, Const, Add, Sub,
  Mul, Div, Linear, WeightedBlend, Clamp, Where, Abs, Sigmoid,
  Relu, Max, Min). `Expr::evaluate(samples) -> Option<f64>`.
  Algorithms gain optional `evaluation: Expr` field; `flood_risk@2`
  is the proof-of-concept (round-trips canonical-CBOR JSON,
  produces 0.4836 byte-stably).
- **Temporal composition.** `Algorithm.temporal_recipe { windows[],
  label, note }`; `/v1/ask` and `/v1/intent` carry an additive
  `temporal_composition[]`. `flood_risk@2` adds GMRT topo +
  `dem_agreement` weighting term.
- **Sentinel-2 / Sentinel-1 fallback ladders.**
  `s2_search_with_fallback` (40 % cloud / 30 d → 60 % / 60 d → 80 % /
  90 d), `s1_search_with_fallback` (15 d → 30 d → 60 d).
- **Adaptive polygon density.** `RecallPolygonReq.cells_per_sqkm` +
  `drill_on_water` parameters; max-cells cap raised 256 → 1024.
- **Photon (komoot.io) geocoder** as the primary live fallback.
  Cascade: embedded → cache → Photon → Nominatim. Configurable via
  `EMEM_PHOTON_BASE`. `/v1/locate.via` reports the resolved path.
- **Overture release auto-discovery** via S3 ListObjectsV2 + XML
  parse; 24 h cached `ReleaseCache`; `EMEM_OVERTURE_RELEASE`
  override.
- Brand identity refresh — new logo + favicon variants; PNGs
  referenced from `index.html`, `agent.json`, `ai-plugin.json`,
  `gemini-extension.json`.

### Changed
- Tslot anchor: u64 anchored at **Unix epoch** (was emem-2026
  epoch in 0.0.2 — pre-2026 observations collapsed to `Tslot(0)`,
  which broke per-tslot historical backfill).
- `algorithms_for_topic[flood_*]` points at `flood_risk@2`;
  `flood_risk@1` retained so existing receipts still resolve.
- `/v1/recall_polygon.max_cells` cap raised 256 → 1024.

### Removed
- Static topic-routing tables (~639 lines): `TOPIC_BANDS`,
  `TOPIC_ALGORITHMS`, `TOPIC_KEYWORDS` `LazyLock` blocks in
  `emem-api-rest/src/lib.rs`. Same data now in `topics-v0.json`,
  consumed via `TopicRouter`.
- Overpass geocoder fallback. The public Overpass instance returns
  503 under load; Photon serves the same OSM corpus in ~100 ms via
  Elasticsearch.

### Fixed
- `/v1/locate` for rural OSM places (Katihar / Laliyahi) — embedded
  → cache → Photon → Nominatim resolves reliably; Overpass timed
  out.
- `/v1/cells/{cell}/scene.png` no longer black for tiles with
  scattered reflectance (percentile helper now returns
  `Option<f64>` and filters non-finite up front).

### Migration
- No breaking changes. `temporal_composition` and `temporal_recipe`
  are additive sibling fields.
- `flood_risk@1` still in the registry; receipts citing v1
  continue to verify.
- If you hardcoded `via == "overpass"`, change it to
  `via == "photon"`.

### Carryover from post-0.0.2 development
- Native HTTPS via in-process rustls + Let's Encrypt (TLS-ALPN-01).
- Persistent Ed25519 responder identity at
  `<EMEM_DATA>/identity.secret.b32` (mode 0600).
- `/v1/locate` (lat/lng or place name → cell64), `/v1/cells/{cell}/info`,
  `/v1/discover` (one-call agent bootstrap), `/v1/contributors[*]`
  (CoIL leaderboard), `/metrics` (Prometheus).
- Production middleware: 16 MiB body cap, 30 s timeout, per-IP
  token bucket (60/min, 120 burst), HSTS / CSP / X-Content-Type-Options /
  X-Frame-Options / Referrer-Policy / Permissions-Policy, optional
  HTTPS redirect via `EMEM_REDIRECT_HTTPS=1`, graceful shutdown on
  SIGTERM.
- `emem-livedemo` and `emem-realdemo` CLI binaries with full
  request + response + receipt traceability written to
  `var/demos/`.
- Cell64 codec gained `cell_from_latlng` / `latlng_from_cell64`
  pair in `emem-codec::geo` with documented bit layout.

## [0.0.2] — 2026-04-26

Initial open-source release. The protocol surface, primitives, MCP
server, and reference responder are all functional. See
`README.md` for the workspace layout and `docs/operators/operating.md` for
production deployment.

End.
