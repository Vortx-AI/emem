# Fleet memory: two vendors, one verifiable map

The robotics claim on the site, made runnable. Robot A (vendor A) names
a landmark, reads the terrain around it, and hands over three short
lines. Robot B (vendor B) converges on the landmark from its own
phrasing, resolves the same signed bytes, and checks the signature
itself. No shared backend, no shared credentials, and B never trusts A.

```bash
pip install requests
python3 fleet_memory.py
# optional, for full offline verification:
pip install blake3 cbor2 cryptography
```

Output from a real run against the hosted node (2026-07-13):

```
== robot A (vendor A) ==
landmark identity : emem:entity:4itylz3kjtwy3lgr76ku52ffqa (already named, reused)
alias linked      : 'ramp seven at the Maasvlakte terminal'
observed          : copdem30m.elevation_mean = 4.966909408569336 (direct_sensor, deterministic=True)
observed          : surface_water.recurrence = 0.0 (deterministic_index, deterministic=True)

-- handoff across the vendor boundary: 3 lines, 206 characters --

== robot B (vendor B) ==
converged on      : emem:entity:4itylz3kjtwy3lgr76ku52ffqa (from 'ramp seven at the Maasvlakte terminal')
resolved          : emem:fact:defi.zb64f.fOxa.zb6f2:kk2jit2jgjnl... -> copdem30m.elevation_mean (class=direct_sensor)
resolved          : emem:fact:defi.zb64f.fOxa.zb6f2:bis73nwf7dk4... -> surface_water.recurrence (class=deterministic_index)
receipt valid     : True
offline verify    : VALID (recomputed locally, no trust in the responder)
```

What the 60 lines of logic demonstrate:

- **The convergence discipline.** Resolve first, mint only on a miss.
  A landmark named once is reused by every later robot in every fleet;
  a fresh mint without that check creates a second identity, which is
  exactly the referential drift the protocol exists to prevent.
- **Aliases are attested.** A's crews say "Maasvlakte ramp 7", B's say
  "ramp seven at the Maasvlakte terminal". One signed alias link makes
  both phrasings converge on one canonical id.
- **The handoff is tokens, not payloads.** Three lines, 206 characters,
  for an identity plus two ground observations. Each fact token
  resolves to byte-identical signed bytes with its provenance class
  (both readings here are recomputable from raw source, and say so).
- **Verification is local.** With the crypto libraries installed, B
  recomputes the blake3 preimage and checks the ed25519 signature with
  no network trust at all.

A ROS 2 client package does not exist yet; this example is plain HTTP
and runs anywhere Python runs, including on the robot. The substrate
profile for robot fleets now ships as `robot.fleet.v1` in the
substrates manifest (`crates/emem-core/data/substrates-v0.json`): a
fleet's writes are admitted only with the robot's complete OS
execution trace (`emem.os_trace.v1`, verified by the `emem-trace`
crate), covering syscalls, scheduler, memory, sensor bus, energy,
thermal, and on-device inference. Upgrading this example to
trace-bound writes is step six in
[docs/plans/encoder-substrates.md](../../docs/plans/encoder-substrates.md);
the write-path gate itself is tracked in
[docs/roadmap.md](../../docs/roadmap.md).
