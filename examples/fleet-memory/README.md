# Fleet memory: two vendors, one verifiable map

This example runs the robotics claim on the site. Robot A (vendor A)
finds a landmark, reads the terrain at it, and hands over three short
lines. Robot B (vendor B) finds the same landmark from its own words,
resolves the same signed bytes, and checks the signature itself. The
two robots share no backend and no credentials, and B does not trust A.

The script sends no key, so it makes no writes. It reads only, except
for one alias request that the responder refuses (see below).

## Run it

```bash
pip install requests
python3 fleet_memory.py
# optional, for the offline signature check:
pip install blake3 cbor2 cryptography
```

Output from a run against emem.dev on 7 November 2026, with the
optional packages installed:

```
== robot A (vendor A) ==
landmark identity : emem:entity:4itylz3kjtwy3lgr76ku52ffqa (already named, reused)
alias not linked  : HTTP 403, unsigned; digest to sign 173a66751b34738b...
observed          : copdem30m.elevation_mean = 4.966909408569336 (direct_sensor, deterministic=True)
observed          : surface_water.recurrence = 0.0 (model_output, deterministic=False)

-- handoff across the vendor boundary: 3 lines, 206 characters --

== robot B (vendor B) ==
converged on      : emem:entity:4itylz3kjtwy3lgr76ku52ffqa (from 'ramp seven at the Maasvlakte terminal')
resolved          : emem:fact:defi.zb64f.fOxa.zb6f2:4zi5cjhapkvu... -> copdem30m.elevation_mean (class=direct_sensor)
resolved          : emem:fact:defi.zb64f.fOxa.zb6f2:h2oeyiebg7td... -> surface_water.recurrence (class=model_output)
receipt valid     : True
offline verify    : VALID (recomputed locally, no trust in the responder)
```

## What the example shows

Resolve first, and mint only on a miss. Robot A asks
`/v1/entity/resolve` for its label before it does anything else. If
any robot already named the landmark, A reuses that identity. A second
mint for the same object makes a second identity, and that is the
referential drift the protocol exists to stop. A mint is a signed
write, so this script stops on a miss instead.

An alias joins two phrasings to one identity. Crews at vendor A say
"Maasvlakte ramp 7". Crews at vendor B say "ramp seven at the
Maasvlakte terminal". One alias link makes both phrasings resolve to
the same identity. The run on 13 July 2026 linked that alias, so
robot B still converges today.

An alias changes what every reader resolves, so it is a signed write
to the shared entity space at tier T3. The script sends the alias
without a key. The responder refuses it with HTTP 403 before it stores
anything, and the refusal names the 32-byte digest to sign. To make
the write, sign that digest with your own Ed25519 key and resend. A
note uses the same pattern with a different preimage; see
[Write your first signed note](../../docs/memory.md#write-your-first-signed-note).
`GET /v1/enlist` says how a key reaches T3.

The handoff is tokens, not payloads. Three lines and 206 characters
carry one identity and two observations. Each fact token resolves to
the same signed bytes, with the provenance class of the reading.

The two readings have different classes. `copdem30m.elevation_mean`
is `direct_sensor` and `deterministic=True`, so anyone can recompute
it from the raw source. `surface_water.recurrence` is `model_output`
and `deterministic=False`. The bands registry
(`crates/emem-core/data/bands-v0.json`) gives JRC surface water that
class, and the fact carries a caution to corroborate it.

Robot B can check the receipt without the network. With the optional
packages, the script rebuilds the receipt preimage with BLAKE3 and
checks the Ed25519 signature locally. It handles preimage v1 and v2,
including the v2 Merkle segment. Receipts served today use v2.

## What does not ship

- No ROS 2 client package exists. The example uses plain HTTP and runs
  anywhere Python runs, including on a robot.
- Trace-bound writes do not ship. The substrates manifest
  (`crates/emem-core/data/substrates-v0.json`) lists `robot.fleet.v1`
  with `status: candidate` and `admission: os_trace_required`. Under
  that profile, a fleet write needs the complete OS execution trace of
  the robot (`emem.os_trace.v1`, checked by the `emem-trace` crate).
  The required layers are syscall, scheduler, memory, sensor bus,
  energy, thermal and inference.
- The upgrade of this example to trace-bound writes is step 6 in
  [docs/plans/encoder-substrates.md](../../docs/plans/encoder-substrates.md).
  The write-path gate is in [docs/roadmap.md](../../docs/roadmap.md).
