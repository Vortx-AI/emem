# Stabilising a codebase against context drift

CI catches drift between code and code. Nothing catches drift between a claim
and reality.

Every defect found in this repo on 2026-08-10 was a true statement that had
stopped being true, and every one passed the test suite:

- an OpenAPI schema declaring `fact_cid` as 26 characters when it is 52,
  referenced by four core response schemas
- `/v1/guard/verdict` returning `action: allow` with `checked: 1` on a citation
  that does not resolve
- a page claiming "13 missions" with 17 on screen
- an architecture note hardcoding "163 recipes" against a live 168
- a stylesheet scraped from a page that had since moved its tokens away,
  leaving 32 of 33 custom properties undefined
- an MCP default of 64 the server had never honoured
- an annotation saying closed-world over a description saying the opposite

None of these is a code bug. Each is a sentence about something that lives
outside the repo: a live response, a rendered page, a registry, a default the
server actually applies. Git content-addresses the code, so code-versus-code
drift is a solved problem. It has nothing to say about the sentences.

Those seven were fixed. Nothing stops the eighth. This demo pins the sentences
so the eighth announces itself.

Three of the seven are pinned here as executable claims and were measured
against the live responder while writing this: the `fact_cid` length, the
registry total, and the guard verdict. The other four are reported from the
same session's findings and were not re-measured for this README.

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
- **The record moved.** Someone edited the claim. The note no longer hashes to
  the address git pins, and the check says which line changed.

The second half is what a plain assertion script cannot do. `EXPECTED = 168` at
the top of a script is one edit away from agreeing with any lie you like, and
the edit leaves nothing behind but a diff. A signed record leaves the old bytes
at their own address on a surface you do not control, signed by a named key.

## Files

| file | what it is |
| --- | --- |
| `claims.py` | the five claims and the probe that decides each. This is the file you edit. |
| `stabilise.py` | `record`, `check`, `demo`. |
| `assertions.lock.json` | committed. Pins the ledger's path, its content address, the signing key, and every recorded answer. |

## Run it

```
pip install blake3 pynacl
python3 demos/stabilisation/stabilise.py demo
```

`demo` is read-only. It runs `check` green against the ledger already recorded
in the lockfile, then runs the same `check` twice more against a copy of the
claims with one character edited, so you see a pass and two failures from the
same code path.

Real output, verbatim, 2026-08-10 against `https://emem.dev`:

```
========================================================================
1. the recorded claims, checked against the live responder
========================================================================
5 claims, recorded 2026-08-10T18:40:08Z against https://emem.dev

Every claim still holds, and the signed record of them is intact.

========================================================================
2. the same check, after one digit of the quoted value is edited
========================================================================
editing ndvi_value_quoted_in_prose: 0.4253807106598985 -> 0.5253807106598985
5 claims, recorded 2026-08-10T18:40:08Z against https://emem.dev

DRIFT:
  x claims.py no longer matches the signed ledger, first difference at line 32:
      signed: claim: NDVI at cell defi.zb4e3.zaeed.fEya is 0.4253807106598985.
      repo:   claim: NDVI at cell defi.zb4e3.zaeed.fEya is 0.5253807106598985.
  x ndvi_value_quoted_in_prose: the prose says 0.5253807106598985 but the signed fact says 0.4253807106598985 (drift: wrong)

Either the claim was true and stopped being true, or someone changed
what it says. Fix the claim, then re-run `record` to sign the new one.

========================================================================
3. the same check, after one character of the citation is edited
========================================================================
editing ndvi_value_quoted_in_prose: last character of the token, ...w2mj57oq -> ...w2mj57oa
5 claims, recorded 2026-08-10T18:40:08Z against https://emem.dev

DRIFT:
  x claims.py no longer matches the signed ledger, first difference at line 35:
      signed: token: emem:fact:defi.zb4e3.zaeed.fEya:qtv2bco56qw4pmlohk56dotoxyl3atmnjpmzrijj2kazw2mj57oq
      repo:   token: emem:fact:defi.zb4e3.zaeed.fEya:qtv2bco56qw4pmlohk56dotoxyl3atmnjpmzrijj2kazw2mj57oa
  x ndvi_value_quoted_in_prose: recorded '0.4253807106598985', now 'unresolved (cid_not_found)'  (dereference the cited token and read value_verbatim)
  x ndvi_value_quoted_in_prose: its citation does not resolve: HTTP 404 cid_not_found for rijj2kazw2mj57oa

Either the claim was true and stopped being true, or someone changed
what it says. Fix the claim, then re-run `record` to sign the new one.

One digit changed and the number stopped matching the fact emem signed.
One character of the citation changed and the citation stopped resolving.
Both also broke the content address of the signed ledger, which is the
check that works even when the world has not moved at all.
```

To record your own set, edit `claims.py` and run `record`. It generates an
ed25519 key at `.identity.json` on first use (gitignored), writes the ledger to
`/memories/by_attester/<your pubkey8>/stabilise/`, and rewrites the lockfile.
There is no registration step: the namespace belongs to whoever writes to it
first and is then held by that key alone.

`check` is the CI gate. Exit 0 clean, 1 on drift, 2 when the responder is
unreachable, matching the other gates in this repo.

Anyone can run `check` against a lockfile someone else recorded. Verifying the
ledger needs no key and no account. Only `record` needs the key. The ledger this
lockfile pins is readable now, without running anything:

<https://emem.dev/memories/by_attester/ukctss4i/stabilise/20260810T184008Z-ledger.md>

Add `Accept: application/json` and you also get the `authorship` block: the
attester's public key, the signature, the body hash, and the exact preimage
recipe, which is everything `check` needs to verify it offline.

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

Step 1 was tested adversarially rather than assumed. Against the live ledger:
a corrupted signature is reported as "Signature was forged or corrupt"; a body
swapped under a valid signature is reported as bound to a different body; a
different attester claiming the same bytes is reported by name; and the
untampered control produces no finding. The published ledger was also rewritten
for real, with a valid signature over the new bytes, and `check` exited 1
naming both the old and the new content address.

## The five claims

They are deliberately claims this repo actually made and got wrong, or nearly
got wrong.

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
  a forgery. This claim exists so that a future change making guard actually
  gate cannot ship without someone noticing the sentence in the docs was
  already wrong or already right for the wrong reason.

- `ndvi_value_quoted_in_prose` is the one claim that quotes a number emem
  signed, with the citation next to it. It is the case where a later edit
  literally fails to resolve.

## What this does not do

Stated plainly, because a gate that oversells itself is the defect it is
supposed to catch.

- **It does not make the record immutable.** The attester can overwrite their
  own path. What changes is that the bytes get a new content address and the
  gate goes red naming both, and the earlier recording stays readable at its own
  timestamped path. Detection and attribution, not prevention.

- **It is not on the transparency log.** emem's RFC 6962 log at `/v1/log/*`
  covers fact attestations. `persist_memory_write` in `crates/emem-api-rest`
  never calls `put_attestation` or appends a leaf, so a memory note is signed
  and content-addressed but not committed under a signed tree head. Do not tell
  an auditor otherwise.

- **It does not stop someone editing a claim and re-recording it in the same
  commit.** Nothing can. What it does is make that a visible act with its own
  signature and its own address, instead of a one-line change to a constant.

- **The committed lockfile goes stale on purpose.** The day someone adds an
  algorithm, `algorithm_registry_total` goes red for everyone. That is the gate
  working. Read the two numbers, decide whether the change was intended, fix
  the prose, re-run `record`.

- **A claim is only as good as its probe.** `how` is prose and nothing checks
  it against the lambda underneath. If the probe asks the wrong question, the
  gate is green and wrong, which is exactly the `len(algorithms)` trap above.

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
renderer and the live fleet. Each of those pins a claim against the responder.
This demo adds the missing half: signing the claim itself, so the claim cannot
quietly become whatever the next commit needs it to be.
