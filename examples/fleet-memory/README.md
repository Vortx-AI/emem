# Fleet memory: two vendors, one verifiable map

Two robots from two vendors refer to one landmark and share two signed readings
about it. Robot A finds the landmark identity, reads the terrain, and hands over
three short tokens. Robot B finds the same identity from its own phrasing,
resolves the same signed bytes, and checks the signature itself. The robots
share no backend and no credentials, and B does not have to trust A.

## What you need

- Python 3 and network access to `https://emem.dev`.
- `requests`. For the offline signature check, also `blake3`, `cbor2` and
  `cryptography`.
- No key and no account. The script sends no writes. Every call is a read.
  On a recall miss, the responder fetches and signs the band itself.

## What you get

- One entity token for the landmark, reused by both robots.
- Two `emem:fact:` tokens that resolve to signed readings: elevation and
  surface-water recurrence.
- A receipt that the responder checks, and that your own machine checks offline.

## Run it

Run these commands from the root of the repository. If your system Python
refuses `pip install`, run them inside a virtual environment.

```bash
pip install requests blake3 cbor2 cryptography
python3 examples/fleet-memory/fleet_memory.py
```

Output from a run against emem.dev on 7 November 2026:

```
== robot A (vendor A) ==
landmark identity : emem:entity:4itylz3kjtwy3lgr76ku52ffqa (already named, reused)
alias linked      : 'ramp seven at the Maasvlakte terminal' (already linked)
observed          : copdem30m.elevation_mean = 4.966909408569336 (direct_sensor, deterministic=True)
observed          : surface_water.recurrence = 0.0 (model_output, deterministic=False)

-- handoff across the vendor boundary: 3 lines, 206 characters --

== robot B (vendor B) ==
converged on      : emem:entity:4itylz3kjtwy3lgr76ku52ffqa (from 'ramp seven at the Maasvlakte terminal')
resolved          : emem:fact:defi.zb64f.fOxa.zb6f2:4zi5cjhapkvu... -> copdem30m.elevation_mean (class=direct_sensor)
resolved          : emem:fact:defi.zb64f.fOxa.zb6f2:h2oeyiebg7td... -> surface_water.recurrence (class=model_output)
receipt valid     : True (server-side check)
offline verify    : VALID (recomputed locally, no trust in the responder)

Done. The map entry outlives both robots, both sessions, and both vendors; any third fleet can repeat robot B's half verbatim.
```

The words `already named` and `already linked` mean that `resolve` found
records from an earlier signed run. The script did not write them.

Without `blake3`, `cbor2` and `cryptography`, the offline line says `skipped`.
It says `INVALID` only when the signature does not match the preimage.
Any other error, for example a malformed receipt, prints `ERROR`.
The fact ids change when the responder signs a newer reading.

## What each step shows

| step | call | what it shows |
|---|---|---|
| A finds the landmark | `POST /v1/entity/resolve` with `text` | A reuses an identity that already exists. It does not make a second one. |
| A checks the alias | `POST /v1/entity/resolve` with B's phrasing | Two phrasings resolve to one `entity_cid`. |
| A reads the ground | `POST /v1/locate`, then `POST /v1/recall` | Each fact carries a `memory_token` and a provenance class. |
| B resolves the tokens | `POST /v1/memory_token/resolve` | B gets the same signed bytes that A read. |
| B checks the receipt | `POST /v1/verify_receipt`, then a local check | The local check rebuilds the BLAKE3 preimage and checks the Ed25519 signature. It covers the v1 segments and the v2 `MERKLE` segment that a recall receipt uses. |

The server-side check is the responder checking itself. The offline check is
the one that does not trust the responder.

On 7 November 2026, `resolve` returned `contested: true` with two candidates
for both phrasings: `4itylz3k...` and `qd54sx3d...`. The script takes the
first candidate. A real fleet should read `corroboration` and
`independent_attesters` before it picks one.
On 7 November 2026 these fields did not break the tie. Both candidates had
`corroboration: none_attributed` and `independent_attesters: 0`. Both had
the same cell and bounding box, `[3.9407, 51.9110, 4.1131, 51.9995]`. That
box is the whole Maasvlakte area, about 12 km by 10 km. The geocoder did not
find the ramp itself. Only the first, `4itylz3k...`, had an `external_ids`
entry (`osm: relation/1411107`), so that is one way to choose.

## What the script does not do

- It does not mint an entity or link an alias. Both need a signature and tier
  T3, because they write to the shared entity space (`POST /v1/entity` and
  `POST /v1/entity/alias`). An earlier signed run made the landmark and the
  alias. If `resolve` finds no landmark, the script stops and says that
  minting needs a signed write.
- It does not write trace-bound observations. The registry lists the
  `robot.fleet.v1` substrate profile with status `candidate` and admission
  `os_trace_required` (`GET /v1/substrates`, 7 November 2026). The responder
  has a device write path, `POST /v1/attest_traced`, for enrolled device keys.
  The enrolment route, `POST /v1/enroll_attested`, ships. On 7 November 2026,
  `GET /v1/device_platforms` lists one admitted anchor, `operator.vortx.v1`,
  on the `generic.linux-host` platform. That platform serves the execution,
  database, codebase and model classes. It does not serve the robot class.
  So no robot can enrol on emem.dev today, and this example does not use
  this path. Step 6 in
  [docs/plans/encoder-substrates.md](../../docs/plans/encoder-substrates.md)
  describes the upgrade.
- There is no ROS 2 client package. The example is plain HTTP, so it runs
  anywhere Python runs.
