# satellite-downlink: one pass through the trust layer

The runnable answer to "can a satellite operator use emem?". SAT-042
images three cells of Nile Delta cropland on pass 1881, and the
example walks the whole operator loop in five acts, refusals included:

1. Enroll the spacecraft key under `orbital.satellite.v1`; the
   substrate registry, not the example, dictates the eight trace
   layers the platform must capture.
2. Try to write without a trace: refused. The protocol respects the
   device as a contributor and never takes its word.
3. Capture the pass window as a digest-chained OS trace, bind the
   three downlink payloads, then try to smuggle a fourth fact the
   execution never emitted: refused, with the forged digest named.
4. Write the honest batch: three signed facts admitted under one
   verified trace, leaving the three handles that survive any context
   window: `emem:fact:` per observation, `emem:trace:` for the
   execution evidence, `emem:bundle:` over the pass.
5. Score the claims against the open-archive drift anchor (one lands
   `Contradicted`, and staying visible instead of being dropped is
   the point), then tamper with a stored segment and watch the
   verifier answer `chain broken at seq 3`.

It is Rust, offline, and deterministic, so it doubles as living
conformance documentation:

```bash
cargo run -p emem-primitives --example satellite_downlink
```

Source: `crates/emem-primitives/examples/satellite_downlink.rs`.
Design and onboarding steps: `docs/plans/encoder-substrates.md`.
The two-vendor robot handoff over plain HTTP lives next door in
`examples/fleet-memory/`.
