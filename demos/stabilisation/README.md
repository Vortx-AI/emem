# Stabilising a codebase against context drift

CI catches drift between code and code. Nothing catches drift between a claim
and reality.

Every defect found in this repo on 2026-08-10 was a true statement that had
stopped being true, and every one passed the test suite:

- an OpenAPI schema declaring `fact_cid` as 26 characters when it is 52,
  referenced by four core response schemas, so a validator generated from our
  own machine surface rejected every cid we serve
- `/v1/guard/verdict` returning `action: allow` with `checked: 1` on a citation
  that does not resolve
- a page claiming "13 missions" with 17 on screen
- a panel hardcoding "163 recipes" against a live 168, while the panel next to
  it was already being patched from the responder at load
- a stylesheet scraped from a page that had since moved its tokens away,
  leaving 32 of 33 custom properties undefined and the body rendering in Times
  New Roman
- an MCP default of 64 the server had never honoured
- an annotation saying closed-world over a description saying the opposite
- `tools/list` returning 288,002 bytes against a client cap of 102,400

None of these is a code bug. Each is a sentence about something that lives
outside the repo: a live response, a rendered page, a registry, a default the
server actually applies, the size of a payload. Git content-addresses the code,
so code-versus-code drift is a solved problem. It has nothing to say about the
sentences.

Those eight were fixed. Nothing stops the ninth. This demo pins the sentences so
the ninth announces itself.

Four of the eight are pinned here as executable claims and were measured against
the live responder while writing this: the `fact_cid` length, the registry
total, the guard verdict, and the `tools/list` payload size. The other four are
carried over from the same session's findings and were not re-measured for this
README.

## The idea

Tokenise the assertions, not the codebase.

An assertion is a claim plus the code that decides it. Write both down, ask the
live responder what the answer is right now, and record the whole set as one
signed note in emem. emem gives that note a content address computed from its
bytes. Pin the address in git.

From then on there are two ways for the pair to come apart, and the gate catches
both:

- **The world moved.** Re-run the probe and it answers something else. 168
  becomes 169 the day someone adds an algorithm.
- **The record moved.** Someone edited the claim, or edited what the claim was
  supposed to say. The note no longer hashes to the address git pins, and the
  check says which line changed.

The second half is what a plain assertion script cannot do. `EXPECTED = 168` at
the top of a script is one edit away from agreeing with any lie you like, and
the edit leaves nothing behind but a diff. A signed record leaves the old bytes
at their own address on a surface you do not control, signed by a named key.

## Files

| file | what it is |
| --- | --- |
| `claims.py` | the six claims and the probe that decides each. This is the file you edit. |
| `stabilise.py` | `record`, `check`, `demo`, `selftest`. |
| `assertions.lock.json` | committed. Pins the ledger's path, its content address, the signing key, and every recorded answer. |

## Run it

```
pip install blake3 pynacl
python3 demos/stabilisation/stabilise.py demo
```

`demo` is read-only. It runs `check` green against the ledger already recorded
in the lockfile, then runs the same `check` three more times against a copy of
the claims with one thing edited, so you see a pass and three failures from the
same code path.

A scenario counts as caught only when the gate goes red *and* a finding names
the expected string. An earlier version counted any non-zero exit, which meant
pointing it at a responder mid-restart would fail every probe, redden every
scenario, and print its closing paragraph having proved nothing.

Real output, verbatim, 2026-08-10 against `https://emem.dev`:

```
1. the recorded claims, checked against the live responder
6 claims, recorded 2026-08-10T20:21:37Z against https://emem.dev

Every claim still holds, and the signed record of them is intact.

2. one digit of the quoted value is edited
   edit: ndvi_value_quoted_in_prose: 0.4253807106598985 -> 0.5253807106598985
   x claims.py no longer matches the signed ledger, first difference at line 37:
      signed: claim: NDVI at cell defi.zb4e3.zaeed.fEya is 0.4253807106598985.
      repo:   claim: NDVI at cell defi.zb4e3.zaeed.fEya is 0.5253807106598985.
   x ndvi_value_quoted_in_prose: the prose says 0.5253807106598985 but the signed fact says 0.4253807106598985 (drift: wrong)
   caught: a finding names 'but the signed fact says'

3. one character of the citation is edited
   edit: ndvi_value_quoted_in_prose: ...w2mj57oq -> ...w2mj57oa
   x claims.py no longer matches the signed ledger, first difference at line 40:
      signed: token: emem:fact:defi.zb4e3.zaeed.fEya:qtv2bco56qw4pmlohk56dotoxyl3atmnjpmzrijj2kazw2mj57oq
      repo:   token: emem:fact:defi.zb4e3.zaeed.fEya:qtv2bco56qw4pmlohk56dotoxyl3atmnjpmzrijj2kazw2mj57oa
   x ndvi_value_quoted_in_prose: recorded '0.4253807106598985', now 'unresolved (cid_not_found)'  (dereference the cited token and read value_verbatim)
   x ndvi_value_quoted_in_prose: its citation does not resolve: HTTP 404 cid_not_found for rijj2kazw2mj57oa
   caught: a finding names 'its citation does not resolve'

4. the expected value in the lockfile is edited
   edit: assertions.lock.json: algorithm_registry_total 168 -> 163
   x claims.py no longer matches the signed ledger, first difference at line 24:
      signed: observed: 168
      repo:   observed: 163
   x algorithm_registry_total: recorded '163', now '168'  (GET /v1/algorithms and read pagination.total)
   caught: a finding names "algorithm_registry_total: recorded '163', now "

One digit changed and the number stopped matching the fact emem signed.
One character of the citation changed and the citation stopped resolving.
One edit to the expected value, with no code touched at all, and the
signed ledger stopped matching the repo. That last one is the whole
point: EXPECTED = 168 at the top of a script has no defence against it.
```

Scenario 4 is the "163 recipes" defect replayed against the live registry: put
163 back where 168 was recorded, touch no code, and the gate names both numbers.

To record your own set, edit `claims.py` and run `record`. It generates an
ed25519 key at `.identity.json` on first use (gitignored), writes the ledger to
`/memories/by_attester/<your pubkey8>/stabilise/`, and rewrites the lockfile.
There is no registration step: the namespace belongs to whoever writes to it
first and is then held by that key alone.

`check` is the gate. Exit 0 clean, 1 on drift, 2 when the responder is
unreachable, matching the other gates in this repo.

Anyone can run `check` against a lockfile someone else recorded. Verifying the
ledger needs no key and no account. Only `record` needs the key. The ledger this
lockfile pins is readable now, without running anything:

<https://emem.dev/memories/by_attester/ukctss4i/stabilise/20260810T202137Z-ledger.md>

Add `Accept: application/json` and you also get the `authorship` block: the
attester's public key, the signature, the body hash, the signed path, and the
preimage recipe it was computed from, which is everything `check` needs to
verify it offline.

## What the check actually does

Four things, in order.

1. **The signed record is intact.** Fetch the ledger, re-hash the bytes locally,
   and compare against the address the lockfile pins. Then verify the ed25519
   signature offline against the attester's own key. emem's answer is not taken
   on trust anywhere in this step: the bytes are hashed here, the signature is
   checked here, and the responder's own `file_cid` is compared against the one
   computed from the bytes it served.

2. **The repo still says what was signed.** Rebuild the ledger text from the
   current `claims.py` and diff it against the served bytes. This fires even
   when the world has not moved at all, which is the case a probe cannot see.

3. **The probes still answer what they answered.** Compared against the
   lockfile, never against a number typed into the source.

4. **Citations dereference.** For a claim that quotes a value emem signed, the
   token is resolved and `POST /v1/echo_verify` compares the digits in the prose
   against the digits in the signed fact, with `strict: true` so a reformat is
   reported rather than forgiven.

Step 1 is the one worth distrusting, because a verifier that never finds
anything looks exactly like a verifier that does not work. `selftest` attacks it
four ways, offline, needing no key and writing nothing:

```
ok   the untampered record: no finding, as it should be
   x the ledger's ed25519 signature does not verify: Signature was forged or corrupt
ok   one character of the signature flipped: named 'does not verify'
   x the ledger at /memories/by_attester/ukctss4i/stabilise/20260810T202137Z-ledger.md now hashes to cdlt5fvf75yi23iggutl5nm4v4, but the lock pins mnjmkhtrebmagn44kozajzgx5m: the recorded assertions were rewritten
   x the signature on the ledger is bound to a different body than the bytes served
ok   the body edited, the signature block left alone: named 'bound to a different body'
   x the ledger at /memories/by_attester/ukctss4i/stabilise/20260810T202137Z-ledger.md now hashes to cdlt5fvf75yi23iggutl5nm4v4, but the lock pins mnjmkhtrebmagn44kozajzgx5m: the recorded assertions were rewritten
   x the ledger is now signed by 66dhqcaocpwoxw36b36a5wa2zervpmwltnfehpg3q2brovnriomq, not ukctss4i5mfbaz7fllc5z746iblx6xx2wrqi7fibkyetm6yo66gq
ok   a different key re-signs different bytes, correctly: named 'the ledger is now signed by'
```

The fourth case is the interesting one. It generates a fresh key, edits the
body, and signs the new bytes properly, so the record is internally perfect:
body hash correct, signature valid, preimage right. It is caught anyway, by the
two things the forger does not control, the content address git pins and the
public key the lockfile names, and it is specifically *not* reported as a bad
signature, because the signature is fine. Naming the wrong defect is how an hour
gets spent on the wrong file.

That case also drove a fix. The three questions in step 1 used to be chained
with `elif`, so a note signed by the wrong key was never cryptographically
verified at all: the report named the key and stopped. They are asked
independently now.

## The six claims

They are claims this repo actually made and got wrong, or nearly got wrong.

- `fact_cid_is_52_chars` and `short_cid_is_refused_as_malformed` are the schema
  defect. A `fact_cid` is the full 32-byte blake3, 52 base32 characters. A
  memory `file_cid` is the first 16 bytes of a blake3, 26 characters. Nothing in
  the wire format tells them apart, which is how 26 ended up in a schema
  describing a 52-character field. The responder refuses a 26-character cid with
  `400 fact_cid_malformed_length` rather than `cid_not_found`, because a
  truncated address is not a missing fact, and that behaviour is now pinned.

- `algorithm_registry_total` is the "163 recipes" defect. `/v1/algorithms`
  returns a page of 20; only `pagination.total` is the count. The probe reads
  `pagination.total`, which is 168. A probe reading `len(algorithms)` would have
  answered 20 and looked like a working gate.

- `guard_verdict_is_advisory` pins a behaviour rather than a number.
  `/v1/guard/verdict` returns `action: allow` on a transcript citing a token
  that does not resolve, and `checked: 1` counts what it looked at, not what
  verified. The honest discriminator is `receipt.fact_cids`, which is empty for
  a forgery. This claim exists so that a change making guard actually gate
  cannot ship while the docs still describe the advisory behaviour, in either
  direction.

- `tools_list_pages_fit_client_cap` is the 288,002-byte defect. It walks the
  whole `tools/list` cursor chain and measures each HTTP body against the
  102,400-byte cap. What it records is the verdict, `fits`, not the byte count.
  A byte count moves every time anyone edits a tool description, and a claim
  that goes red on an edit nobody cares about is a claim that gets switched off.
  Pin the property, not the measurement.

- `ndvi_value_quoted_in_prose` is the one claim that quotes a number emem
  signed, with the citation next to it. It is the case where a later edit
  literally fails to resolve.

## What this does not do

Stated plainly, because a gate that oversells itself is the defect it is
supposed to catch.

- **It does not make the record immutable.** A second write to a path your key
  already owns replaces the bytes; that was measured on a scratch path in this
  namespace, not argued from the docs. What changes is that the new bytes get a
  new content address, so the gate goes red naming both, and the earlier
  recording stays readable at its own timestamped path. Detection and
  attribution, not prevention. `selftest` case four shows the cryptographic half
  of that end to end; rewriting the live ledger and restoring it was not
  performed in this session.

- **It is not on the transparency log.** emem's RFC 6962 log at `/v1/log/*`
  covers fact attestations. `persist_memory_write` in `crates/emem-api-rest`
  never calls `put_attestation` or appends a leaf, so a memory note is signed
  and content-addressed but not committed under a signed tree head. Do not tell
  an auditor otherwise.

- **Nothing runs this in CI.** `.github/workflows/ci.yml` invokes eleven of
  these gate scripts by path and does not mention this directory. Wiring it is
  one line in that file, and this demo does not own that file.

- **It does not stop someone editing a claim and re-recording it in the same
  commit.** Nothing can. What it does is make that a visible act with its own
  signature and its own address, instead of a one-line change to a constant.

- **The committed lockfile goes stale on purpose.** The day someone adds an
  algorithm, `algorithm_registry_total` goes red for everyone. That is the gate
  working. Read the two numbers, decide whether the change was intended, fix the
  prose, re-run `record`.

- **A claim is only as good as its probe.** `how` is prose and nothing checks it
  against the lambda underneath. If the probe asks the wrong question, the gate
  is green and wrong, which is exactly the `len(algorithms)` trap above.

## Copying this into another repo

Three things travel: `claims.py`, `stabilise.py`, and the habit.

The habit is the part that matters. When you write a sentence about something
outside the repo, and you would be embarrassed if it quietly became false, write
the probe next to it. Do not tokenise the codebase; git already does that, and
doing it twice adds a second thing to drift. Tokenise the claims whose truth
lives somewhere git cannot look.

The crude version of this already ships here and is worth reading first:
`scripts/sync_counts.py` asserts published counts against the registries and a
live responder, `scripts/output_schema_conformance.py` asserts that every tool
declaring an `outputSchema` returns conforming content on a real call, and
`scripts/arcade_protocol_check.py` asserts the published contract against the
renderer and the live fleet. Each of those pins a claim against the responder,
and each was written after the drift it now prevents.

This demo adds the missing half: signing the claim itself, so the claim cannot
quietly become whatever the next commit needs it to be.
