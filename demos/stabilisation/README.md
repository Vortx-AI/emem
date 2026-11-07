# Stabilisation demo: sign the claims a repo makes about the outside world

This demo keeps a list of claims that the repo makes about things outside the
repo. Examples are a live response, a registry total and a payload size. Each
claim has a probe that asks the live responder for the answer now. `record`
writes the answers into one signed emem note and pins its content address in
git. `check` fails when a probe answers something else or when the note no
longer matches the repo.

Use it when the docs state a fact about a live system. CI then tells you
when that fact becomes false.

Try it. This command signs nothing and writes nothing:

```sh
uv run --no-project --with blake3 --with pynacl \
  python3 demos/stabilisation/stabilise.py selftest
```

If you use pip, run `pip install blake3 pynacl` first, then
`python3 demos/stabilisation/stabilise.py selftest`. We did not run the pip
form on 7 November 2026 because this host has no pip.

## Status on 7 November 2026

| command | result on 7 November 2026 | why |
| --- | --- | --- |
| `selftest` | pass, exit 0 | It names each of the four attacks correctly. |
| `check` | drift, exit 1 | The world moved, and `claims.py` has claims that nobody recorded yet. |
| `demo` | stops after step 1 | It runs the red scenarios only when step 1 is green. |
| `record` | not run | It is a signed write. Only the key holder can run it. |
| `tamper` | not run | It writes to emem.dev. |

`check` reports these findings today:

- `algorithm_registry_total`: recorded `168`, now `172`. The registry grew.
- `sdk_doc_links_resolve`: recorded `0 dead of 6`, now
  `1 dead of 20: https://emem.dev/.well-known/openai-apps-challenge`. That
  URL answers 404 until an operator sets `EMEM_APPS_CHALLENGE`.
  `integrations/chatgpt/submission.md` explains this. The URL count also
  grew from 6 to 20, because `claims.py` now scans `sdks/`, `python/` and
  `integrations/`.
- `core_profile_whole_on_page_one`, `unknown_recall_field_is_refused` and
  `cell_matches_reports_a_check_that_ran` are in `claims.py` but not in the
  lockfile. Someone added them after the last `record`.
- `claims.py` no longer matches the signed ledger, because of the three new
  claims.

The gate works as designed here. To make it green again, the key holder reads
each finding, fixes the claim text, and runs `record`.

## Files

| file | content |
| --- | --- |
| `claims.py` | The ten claims and the probe for each. You edit this file. |
| `stabilise.py` | The subcommands `record`, `check`, `demo`, `selftest` and `tamper`. |
| `assertions.lock.json` | Committed. It pins the ledger path, its content address, the signing key and every recorded answer. |
| `.gitignore` | Keeps `.identity.json`, your local signing key, out of git. |

The lockfile pins this ledger, recorded on 11 August 2026 at 07:22:18 UTC:

<https://emem.dev/memories/by_attester/ukctss4i/stabilise/20260811T072218Z-ledger.md>

You can read it without a key. Send `Accept: application/json` to also get
the `authorship` block. That block has the public key, the signature, the body
hash, the signed path and the preimage recipe. `check` needs nothing else to
verify it offline.

## Why sign the claims

On 10 August 2026 the maintainers found eight defects in this repo. Each was
a true statement that had become false. Each passed the test suite:

- An OpenAPI schema declared `fact_cid` as 26 characters. It is 52. Four core
  response schemas used it, so a validator built from that schema refused
  every cid the responder serves.
- `/v1/guard/verdict` returned `action: allow` with `checked: 1` for a
  citation that did not resolve.
- A page said "13 missions" and showed 17.
- A panel had "163 recipes" in its source. The live registry had 168.
- A stylesheet copied from a page that later moved its tokens left 32 of 33
  custom properties undefined.
- An MCP default was 64, and the server never applied it.
- An annotation said closed-world. The description under it said the opposite.
- `tools/list` returned 288,002 bytes. The client cap was 102,400.

None of these is a code bug. Each is a sentence about something outside the
repo. Git gives the code a content address, so drift between code and code is
easy to see. Git cannot see whether a sentence about a live system is still
true.

Four of the eight are claims in `claims.py`: the `fact_cid` length, the
registry total, the guard verdict and the `tools/list` size. The other four
come from the same session's notes. No probe checks them.

The method has two parts. Write each claim with a probe that decides it. Then
sign the set of answers as one emem note and pin its address in git. After
that, two different changes make `check` fail:

| change | how `check` sees it |
| --- | --- |
| The world moved. | A probe answers something else, for example 168 becomes 172. |
| The record moved. | Someone edited a claim or a recorded answer. The note no longer hashes to the address in git, and `check` names the first changed line. |

A plain script with `EXPECTED = 168` cannot see the second change. One edit
makes it agree with any value. A signed note keeps the old bytes at their own
address, signed by a named key, on a server you do not control.

## What `check` does

`check` does four steps, in this order. Exit 0 means clean, 1 means drift and
2 means the responder is unreachable. The other gates in this repo use the
same codes.

1. It checks the signed record. It fetches the ledger, hashes the bytes
   locally and compares the hash with the address in the lockfile. It
   verifies the ed25519 signature offline against the attester key. It trusts
   no answer from emem in this step.
2. It checks that the repo still says what was signed. It rebuilds the ledger
   text from the current `claims.py` and compares it with the served bytes.
   This step fails even when the world did not move.
3. It runs every probe again and compares each answer with the lockfile,
   never with a number in the source.
4. It checks citations. For a claim that quotes a signed value, it resolves
   the token and calls `POST /v1/echo_verify` with `strict: true`. It reports a
   changed format and does not accept it.

`check` reads `EMEM_RESPONDER` (default `https://emem.dev`) to find the
responder.

### `selftest`: four attacks on step 1, offline

A verifier that never finds anything looks the same as a verifier that does
not work. `selftest` attacks step 1 four ways. It reads the ledger once, then
works offline. It needs no key and writes nothing. Output on 7 November 2026:

```
ok   the untampered record: no finding, as it should be
   x the ledger's ed25519 signature does not verify: Signature was forged or corrupt
ok   one character of the signature flipped: named 'does not verify'
   x the ledger at /memories/by_attester/ukctss4i/stabilise/20260811T072218Z-ledger.md now hashes to koo6eb5ectlqyxhs3p5br2gsou, but the lock pins kw5fb7k6qr2mwikkjd6s353la4: the recorded assertions were rewritten
   x the signature on the ledger is bound to a different body than the bytes served
ok   the body edited, the signature block left alone: named 'bound to a different body'
   x the ledger at /memories/by_attester/ukctss4i/stabilise/20260811T072218Z-ledger.md now hashes to koo6eb5ectlqyxhs3p5br2gsou, but the lock pins kw5fb7k6qr2mwikkjd6s353la4: the recorded assertions were rewritten
   x the ledger is now signed by c7jvu2ywys3jgaa4awumdpcdjy455poypa6s4dowxtfk4zm5f6zq, not ukctss4i5mfbaz7fllc5z746iblx6xx2wrqi7fibkyetm6yo66gq
ok   a different key re-signs different bytes, correctly: named 'the ledger is now signed by'

A corrupt signature, a swapped body, and a competent re-signing by another 
key are each named for what they are, and the clean record is left alone.
```

The fourth key changes on each run, because `selftest` makes a new one.

The fourth attack makes a new key, edits the body and signs the new bytes
correctly. The body hash, the signature and the preimage are all correct. Two
things still catch it: the address in git and the public key in the lockfile.
`selftest` requires that this attack is not reported as a bad signature,
because the signature is valid. A wrong defect name sends the reader to the
wrong file.

This attack also found a bug. The three questions in step 1 were in an `elif`
chain, so `check` never verified a note signed by the wrong key. It named the
key and stopped. Now `check` asks the three questions independently.

### `demo`: one pass and three failures

`demo` is read-only. It runs `check` against the recorded ledger. If that is
green, it runs the same `check` three more times against a copy of the claims
with one edit each:

1. It changes one digit of the quoted NDVI value.
2. It changes one character of the NDVI citation.
3. It changes the recorded registry total in the lockfile from 168 to 163.

A scenario counts as caught only when `check` fails and a finding contains the
expected text. An earlier version counted any non-zero exit. A responder that
was restarting then made every scenario red and proved nothing.

On 7 November 2026 step 1 is not green (see the status table), so `demo`
stops after step 1 and says so. This output is from 10 August 2026, against
an earlier ledger with six claims:

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

Scenario 4 replays the "163 recipes" defect. No code changes, and the gate
names both numbers.

### `tamper`: the namespace owner rewrites its own note

The worst case is the key that owns the namespace. It rewrites the note and
signs the new bytes correctly. The signature is valid, the body hash is
correct and the attester is the key the lockfile names. You cannot stage this
offline, because the point is that the responder accepts the rewrite.

`tamper` runs this attack live. It never touches the published ledger. It
uses a second key that it derives from `.identity.json` with BLAKE3 and a
domain label. The derivation is one-way and repeatable, so each run reuses one
rehearsal path in that key's own namespace.

```sh
python3 demos/stabilisation/stabilise.py tamper
```

We did not run `tamper` on 7 November 2026, because it writes to emem.dev.
This output is from 11 August 2026:

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

Each step happened on the live responder. Nothing comes from a fixture.

Step 1 tests the namespace rule on a path that does not exist in the
published namespace. The responder decides the refusal from the `pubkey8`
segment of the path, before it looks at file ownership. So an unwritten path
proves the same rule. If the rule fails, the cost is one junk file, not the
published ledger.

Step 3 requires exactly one finding, and none of the three signature
findings. The `control` line flips one character of the same signature and
requires a finding. This shows that the verifier ran and was silent for a
valid signature.

`tamper` runs step 1 of `check` only. The full `check` returns two findings on
the same rehearsal: the address, and the rebuild from `claims.py`. The
rehearsal says `observed: 163`, and the repo computes 168. An attacker can
remove the second finding by editing `claims.py` in the same commit. They
cannot remove the first without changing the address in git, and that change
is a diff with their name on it.

The maintainers tested these requirements. They patched `verify_record` and
`fetch_note` from outside in four ways. The patches blinded the address check,
named the wrong defect, made the responder refuse the rewrite and disabled the
signature check. Each patch made the run fail at the correct step with exit 1.

`tamper` tries four writes per run. The responder must refuse step 1. The three
that succeed are at one path, under a key derived from yours. `tamper` and
`record` are the only subcommands that write.

## The ten claims

The table shows each recorded answer and the live answer on 7 November 2026.

| claim | recorded on 11 August 2026 | live on 7 November 2026 |
| --- | --- | --- |
| `sdk_doc_links_resolve` | `0 dead of 6` | `1 dead of 20` |
| `fact_cid_is_52_chars` | `52` | `52` |
| `short_cid_is_refused_as_malformed` | `400 fact_cid_malformed_length` | same |
| `algorithm_registry_total` | `168` | `172` |
| `guard_verdict_is_advisory` | `allow fact_cids=0` | same |
| `tools_list_pages_fit_client_cap` | `fits` | same |
| `ndvi_value_quoted_in_prose` | `0.4253807106598985` | same |
| `core_profile_whole_on_page_one` | not recorded | `whole core profile in one page, chain ends` |
| `unknown_recall_field_is_refused` | not recorded | `400 invalid_argument, and the correct spelling still filters` |
| `cell_matches_reports_a_check_that_ran` | not recorded | `canonical=True bare_cid=False degraded=True` |

What each claim protects:

- `sdk_doc_links_resolve`: every `https://emem.dev/` URL under `sdks/`,
  `python/` and `integrations/` resolves. Only 404 and 410 count as dead. A
  POST-only route answers 405 to a HEAD request and is not dead. The 2.1.0
  release of emem-langmem shipped a Documentation link that never resolved.
- `fact_cid_is_52_chars` and `short_cid_is_refused_as_malformed`: the schema
  defect. A `fact_cid` is the full 32-byte BLAKE3, 52 base32 characters. A
  memory `file_cid` is the first 16 bytes, 26 characters. The wire format
  does not mark the difference. The responder refuses a 26-character cid
  with `400 fact_cid_malformed_length`, not `cid_not_found`.
- `algorithm_registry_total`: the "163 recipes" defect. `/v1/algorithms`
  returns a page of 20, so the probe reads `pagination.total`. A probe that
  reads `len(algorithms)` gets 20 and looks like a working gate.
- `guard_verdict_is_advisory`: a behaviour, not a number.
  `/v1/guard/verdict` answers `action: allow` for a citation that does not
  resolve. `checked: 1` counts what it looked at, not what verified. The
  useful signal is `receipt.fact_cids`, which is empty for a forgery. If
  guard starts to block, this claim fails until the docs change too.
- `tools_list_pages_fit_client_cap`: the 288,002-byte defect. The probe walks
  the `tools/list` cursor chain on `/mcp/full` and compares each HTTP body
  with the 102,400-byte cap. It records `fits`, not a byte count, because the
  size changes with every tool description edit.
- `ndvi_value_quoted_in_prose`: the one claim that quotes a signed number and
  carries its citation. A changed digit or a changed token both fail.
- `core_profile_whole_on_page_one`: a `tools/list` on `/mcp` with no cursor
  returns the full core profile and no `nextCursor`. Some hosts read page one
  only. They once got 12 tools of a 16-tool core profile.
- `unknown_recall_field_is_refused`: `determinstic: true`, with one letter
  missing, once returned every fact with status 200. The probe requires
  a refusal for the typo and real filtering for `deterministic: true`.
- `cell_matches_reports_a_check_that_ran`: `cell_matches` was `true` on every
  200. A bare cid names no cell, so the probe requires `false` for it and
  `true` for a full token.

## Record your own set

1. Edit `claims.py`.
2. Run `python3 demos/stabilisation/stabilise.py record`.

On first use, `record` makes an ed25519 key in `.identity.json` (set
`EMEM_IDENTITY` to use another path). It writes the ledger to
`/memories/by_attester/<your pubkey8>/stabilise/<timestamp>-ledger.md` and
rewrites the lockfile. There is no registration step. The first key that
writes to a namespace holds it.

Anyone can run `check` against a lockfile that someone else recorded. That
needs no key and no account.

## Limits

- The record can change. A second write to a path that your key owns
  replaces the bytes, and the responder serves the new ones. `tamper` does
  this and then restores the note. The new bytes have a new content address,
  so `check` fails and names both. This gives detection and attribution, not
  prevention.
- Reads by path return the current bytes only. To read older bytes, call the
  MCP tool `memory_view` with a `file_cid` instead of a path (checked on
  emem.dev on 7 November 2026). That works only while the responder still
  holds the old blob. `record` writes a new timestamped path each time, so
  each recording keeps its own URL.
- The pinned ledger is not in the transparency log. Its JSON has `"log":
  null`, because `record` wrote it on 11 August 2026. Current code
  (`persist_memory_write` in `crates/emem-api-rest/src/lib.rs`) appends each
  new memory note to the RFC 6962 log at `/v1/log/*` when the node has one.
  A new `record` would get a log entry.
- No CI job runs this demo. `.github/workflows/ci.yml` calls 40 scripts by
  path, and none is in this directory. To add it, add one line to that file.
- It does not stop someone from editing a claim and running `record` again in
  the same commit. It makes that change visible, with its own signature and
  its own address.
- The committed lockfile becomes stale on purpose. When someone adds an
  algorithm, `algorithm_registry_total` fails for everyone. Read the two
  numbers, decide if the change was intended, fix the text and run `record`.
- A claim is only as good as its probe. Nothing checks the `how` text against
  the probe code. A probe that asks the wrong question gives a green gate that
  is wrong, like the `len(algorithms)` trap.

## Use the method in another repo

Copy `claims.py` and `stabilise.py`. Then write a probe next to each sentence
about something outside the repo that you need to stay true.

Pin a claim when the repo does not decide it:

| kind of claim | what decides it | claim here | how the repo looked while it was wrong |
| --- | --- | --- | --- |
| A count the responder computes | A registry built at startup | `algorithm_registry_total` | `163` in a panel, next to a panel that read the live value |
| A schema that a client validates | Another party's validator | `fact_cid_is_52_chars` | `26` in the schema, used by four response schemas |
| A byte budget that a host enforces | The MCP client's cap | `tools_list_pages_fit_client_cap` | A 288,002-byte response that nothing measured |
| A promised behaviour | Only a real call shows it | `guard_verdict_is_advisory` | An allow branch and a doc sentence, each plausible alone |

Do not pin these:

- The code. Git already gives it a content address. A second address for the
  same bytes is one more thing that can drift.
- A measurement that changes for reasons nobody cares about. A claim that
  fails on every tool description edit gets switched off. A switched-off
  claim still looks like coverage. Pin the property, as the `tools/list`
  claim does with `fits`.
- A claim whose probe reads the repo. If the probe reads the same `163` as
  the text, both change in the same commit, and the gate tests that a
  constant equals itself.

Simpler gates of this kind already ship in this repo, without signatures:

- `scripts/sync_counts.py` checks published counts against the registries
  and a live responder.
- `scripts/output_schema_conformance.py` checks that each tool with an
  `outputSchema` returns conforming content on a real call.
- `scripts/arcade_protocol_check.py` checks the published contract against
  the renderer and the live fleet.

This demo adds the signature. A claim then cannot change without a new
address in git.
