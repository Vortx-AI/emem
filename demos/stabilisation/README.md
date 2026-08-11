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
| `stabilise.py` | `record`, `check`, `demo`, `selftest`, `tamper`. |
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

### The fifth attack, which cannot be staged offline

Case four is a forger who brought their own key, and the foreign key is what
catches it. The attack left over is the one worth being afraid of: the key that
legitimately holds the namespace rewrites the bytes and re-signs them properly.
Signature valid, body hash right, attester exactly the key the lockfile names.
Nothing is wrong with that record except which bytes it is.

That one cannot be staged offline, because the point of it is that the responder
accepts it and serves it to everyone who asks. `tamper` runs it live, against a
namespace that is not the published one:

```
python3 demos/stabilisation/stabilise.py tamper
```

Real output, verbatim, 2026-08-11 against `https://emem.dev`:

```
1. this key cannot reach the published ledger's namespace
   attempted: create /memories/by_attester/ukctss4i/stabilise/rehearsal-must-be-refused.md
   as:        4fqtfku6, and that namespace belongs to ukctss4i
   refused:   memory_namespace_violation: signature verified, but path
              `/memories/by_attester/ukctss4i/stabilise/rehearsal-must-be-refused.md`
              is under a different attester's namespace. Use
              `/memories/by_attester/4fqtfku6/...`

2. a rehearsal ledger, recorded in this key's own namespace
   path      /memories/by_attester/4fqtfku6/stabilise/tamper-rehearsal.md
   pinned    h3xlz2fravedia7y7gnuggccse
   ok        it verifies clean against that pin

3. the namespace's own key rewrites those bytes and re-signs them
   edit:     observed algorithm_registry_total 168 -> 163
   signed:   by the same key, over the new body, at the same path
   accepted: the responder now serves the altered bytes at that path, knvihrkol7nxv7yqyvu3zyof74
   x the ledger at /memories/by_attester/4fqtfku6/stabilise/tamper-rehearsal.md now hashes to knvihrkol7nxv7yqyvu3zyof74, but the lock pins h3xlz2fravedia7y7gnuggccse: the recorded assertions were rewritten
   caught:   one finding, and it is the address. Everything else about this
             record is in order: the ed25519 signature verifies, over a body
             hash matching the bytes served, by 4fqtfku6, which is the key
             the pin names. It is a competent record of the wrong claims.
   control:  flip one character of that same signature and the same check does
             report it, so the silence above is a signature that verified.

4. restored
   ok        /memories/by_attester/4fqtfku6/stabilise/tamper-rehearsal.md serves h3xlz2fravedia7y7gnuggccse again and verifies clean

The strongest attack on this scheme is not a broken signature. It is a
correct one over different bytes, by the key that owns the namespace, which
the responder accepts because it is entitled to. Every question this record
can answer about itself comes back clean. What disagrees is the address
pinned outside it, in git, and one disagreement is enough.
```

Every line of that happened on the live responder. Nothing in it is replayed
from a fixture. What is *not* the real thing is which bytes get rewritten: the
ledger being tampered with is a copy recorded under a second key, in that key's
own namespace, not the ledger this repo publishes. `4fqtfku6` is derived from
`.identity.json` by blake3 under a domain label, so it is one way, and
deterministic, so re-running rewrites one rehearsal path instead of stranding a
fresh namespace on the responder every run.

Step 1 is the containment being tested rather than asserted, and it is aimed at
a path that does not exist inside the published namespace, never at the ledger
itself. The refusal is decided from the `pubkey8` segment of the path before any
per-file ownership is considered, so an unwritten path proves the same thing; if
that check ever stops working, the cost is a junk file rather than the record
this whole demo is about. A demo does not get to gamble the artefact it is
demonstrating.

Step 3 requires *exactly one* finding, and requires that none of the three
signature-shaped findings appear, because a valid signature reported as a broken
one sends the next hour to the wrong file. The `control` line exists because
silence is ambiguous: a check that is not running reports nothing either. It
flips one character of that same signature and requires the same code path to
report it, so "no signature finding" means the verifier looked.

One finding is what step 1 of the gate returns here, and step 1 is all `tamper`
runs. The whole of `check` returns two against the same tampered rehearsal: the
address, plus the rebuild from `claims.py` no longer matching the served bytes,
because the tampered ledger says `observed: 163` where the repo still computes
168. Both numbers measured, not reasoned about. The second finding is the one an
attacker erases for free by editing `claims.py` in the same commit. The first
one they cannot erase without also changing the address pinned in git, which is
a diff with their name on it.

Those assertions are load-bearing, not decoration. Blinding the address check,
having it fire but name the signature, having the responder refuse the rewrite,
and killing the signature check each turn the run red at the right step, with
exit 1. That was measured by patching `verify_record` and `fetch_note` from
outside and re-running.

`tamper` writes. Four write attempts per run, one of which is step 1 and is
meant to be refused; the three that land are all under a key derived from yours,
all at one path. It and `record` are the only subcommands that write anything.

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
  already owns replaces the bytes, and the responder then serves the new ones to
  everyone who asks. `tamper` does exactly that and restores it, so this is
  measured rather than argued from the docs. What changes is that the new bytes
  have a new content address, so the gate goes red naming both. Detection and
  attribution, not prevention.

- **A rewrite in place is not recoverable through this responder.** Reads are
  keyed by path. `read_memory_file` in `crates/emem-api-rest` looks the path up,
  takes whatever cid it points at now, and returns that blob; the router has one
  memory reader, `/memories/*path`, and MCP `memory_view` also takes a path.
  Neither accepts a `file_cid`. The superseded blob stays in the store with no
  route that names it. `record` sidesteps this by writing a new timestamped path
  every time, so each recording keeps its own URL, but for a path that was
  rewritten, the address pinned in git is the only surviving statement of what
  it used to hold.

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

### What that means in practice

A claim is worth pinning when the thing that decides it is not in the repo, so
the repo can go on saying it long after it stops being true. Four kinds, one
from each of the defects at the top of this file:

| kind | what decides it, outside the repo | the claim here | how the repo looked while it was wrong |
| --- | --- | --- | --- |
| a count the responder computes | a registry assembled at startup; the number exists only in a response | `algorithm_registry_total`: `pagination.total` is 168 | `163` hardcoded in a panel, beside a panel already patching itself from the responder |
| a schema a client validates against | someone else's validator, running against the schema we publish | `fact_cid_is_52_chars`: 52, and 26 is refused as malformed | `26` in the schema, internally consistent, reached through four response schemas |
| a byte budget a host enforces | the ceiling is the MCP client's, and it does not ask us | `tools_list_pages_fit_client_cap`: every page under 102,400 | a response that had grown to 288,002 bytes with nothing measuring it |
| a capability a description promises | whether the promise is kept is behaviour, and only running it says so | `guard_verdict_is_advisory`: `action: allow`, `receipt.fact_cids` empty | an allow branch and a doc sentence, each plausible on its own |

The mirror image is just as useful. Do not pin:

- **The code.** Git addresses it already, exactly, for free. A second address for
  the same bytes is a second thing to keep in step, and the day they disagree
  you have to work out which one lied.

- **A measurement that moves for reasons nobody cares about.** The `tools/list`
  claim records `fits`, not the byte count. A claim that reddens every time
  someone edits a tool description is a claim that gets switched off, and a
  switched-off claim is worse than no claim, because it still looks like
  coverage. Pin the property, measure the number.

- **Anything whose probe reads the repo.** If the probe reads the same `163` the
  prose reads, both move in the same commit and the gate is decoration. It runs,
  it is green, and it is testing that a constant equals itself. The probe has to
  ask something the repo does not get a vote on. Same family as the
  `len(algorithms)` trap: `how` is prose, nothing checks it against the lambda,
  and a probe can run, pass, and mean nothing.

The crude version of this already ships here and is worth reading first:
`scripts/sync_counts.py` asserts published counts against the registries and a
live responder, `scripts/output_schema_conformance.py` asserts that every tool
declaring an `outputSchema` returns conforming content on a real call, and
`scripts/arcade_protocol_check.py` asserts the published contract against the
renderer and the live fleet. Each of those pins a claim against the responder,
and each was written after the drift it now prevents.

This demo adds the missing half: signing the claim itself, so the claim cannot
quietly become whatever the next commit needs it to be.
