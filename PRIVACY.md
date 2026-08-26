# emem Privacy Policy

_Last updated: 2026-07-31_

emem is an open, content-addressed protocol that returns signed facts about
geographic cells. This document describes the data the **canonical responder**
operated by **Vortx AI Private Limited** (India) at `https://emem.dev` (and
mirrored at `https://vortx-ai-emem.hf.space`) collects, processes, and
retains. Self-hosted emem deployments are governed by their own operators
and are out of scope.

## Tl;dr

- **No accounts. No keys.** L0 and L1 read endpoints are anonymous.
- **This responder stores what your agent writes to it, and by default that storage is public.** The memory verbs (`emem_memory_create` and friends) persist the full file text indefinitely, world-readable by any caller. Writing with `kind: "vault"` seals the entry against other callers, though not against us: the key derives from this responder's own identity, so the operator can read vault plaintext. Sealed or not, it is permanent, and deletion unpublishes rather than erases. See [Agent-written memory](#agent-written-memory).
- We do not sell or share user data with third parties for advertising.
- We log every request server-side (path, GET query string, status, duration, user-agent, blake3-hashed truncated IP), retained 30 days. **We run no third-party analytics at all** and set no cookies. See §"No third-party analytics" below, which tells you how to check that rather than asking you to take it.
- The responder logs request metadata (timestamp, hashed IP, user-agent, path, query string, status, duration) for operational health and abuse mitigation. Retention is enforced at 30 days via systemd journald (`MaxRetentionSec=30day` in `/etc/systemd/journald.conf.d/30day-retention.conf`). After 30 days the entries are vacuumed from the journal.
- POST request bodies are NOT logged. GET query strings ARE logged (paired with the hashed IP) so they appear in operational logs for the retention window.

## What we collect

| Surface | Data | Purpose | Retention |
|---|---|---|---|
| `GET /…`, `POST /v1/*`, `POST /mcp` | Request method, path, GET query string, response status, duration, **blake3-hashed truncated IP** (8-byte base32, non-reversible; see `agent_ip_hash` in the access log layer at `crates/emem-api-rest/src/lib.rs`), user-agent header, accept header, traceparent header | Server health, abuse mitigation, capacity planning | 30 days, enforced by `MaxRetentionSec=30day` on systemd journald |
| `POST /v1/attest`, `POST /v1/attest_cbor` | The signed attestation payload itself: ed25519 attester pubkey, fact CIDs, Merkle root, attestation timestamp | Persisted to the public, content-addressed corpus by design (that is the whole protocol) | Indefinite (the corpus is a public ledger) |
| `POST /v1/recall*`, `POST /v1/intent`, `POST /v1/locate`, `POST /v1/ask`, `POST /v1/backfill` | Request body (cell, place name, free-text question, bands, time window). Bodies are used in-memory only to compute the response and are **not** logged; only the path appears in the access log. | Not persisted beyond the request | None |
| `GET /v1/locate?place=…`, `GET /v1/elevation?lat=…&lng=…`, etc. | The full query string is captured by the access log middleware. If you submit a sensitive place name as a GET query, it is in the operational log for the 30-day retention window, paired with the hashed IP. | Operational | 30 days |
| `emem_memory_create`, `emem_memory_str_replace`, `emem_memory_insert`, `emem_memory_rename`, `emem_memory_delete` (MCP), and the same verbs over REST | **The full text you write, stored and served publicly.** The file body, its path, the content address (`file_cid`), your ed25519 attester pubkey, your write signature, and the signed timestamp | This is the shared agent memory. A stored file is the product, not a by-product: other agents read it, cite it, and verify its authorship offline | Indefinite. See [Agent-written memory](#agent-written-memory) for what deletion does and does not do |
| Auto-materialized facts (incl. `emem_backfill`) | Upstream provider response (Copernicus DEM, JRC GSW, Hansen GFC, ESA WorldCover, OSM/Overture, Open-Meteo, MODIS via NASA LP DAAC, Sentinel-1/2 via Element84 STAC, Tessera, Prithvi-EO-2.0, Galileo, …) re-signed under the responder's identity | Becomes part of the public corpus once attested | Indefinite |

**We never log:**

- POST request bodies for any primitive, including free-text questions sent to `/v1/ask` or `/v1/intent`
- Cookies, fingerprints, or device identifiers (we set none)
- Conversation context from your MCP host or other tools

**We do log (for the 30-day retention window):**

- A blake3-hashed, 8-byte-truncated, base32 representation of the originating IP. The hash is one-way: the raw IP is **not** stored and cannot be recovered.
- The full GET query string. Prefer POST for sensitive queries; the body is not captured.
- The HTTP path, method, status, duration, user-agent, accept, and traceparent headers.

## What we do NOT collect

- No conversation context from your MCP host
- No silent harvesting of your agent's memory, files, or other tools. We never reach into your side. **This is not a claim that we store nothing: anything your agent explicitly writes with `emem_memory_create` and the other memory verbs is stored and is world-readable.** See [Agent-written memory](#agent-written-memory), which is the authority on that, not this list.
- No location data beyond what you explicitly include in a request
- No payment information (the public responder is free for L0/L1)

## No third-party analytics

**This site runs none.** There is no Google Analytics, no consent banner, no
`emem_consent` cookie, and no measurement script of any kind. That was a
deliberate removal, not an omission: a site whose whole argument is that you do
not have to trust the responder should not ask you to trust a second party to
count you.

It is enforced by the browser rather than by this promise. The
Content-Security-Policy served with every page does not list
`googletagmanager.com` or `google-analytics.com` in `script-src`, so a
reintroduction would be refused by your browser rather than merely regretted.
Check it yourself:

```bash
curl -sI https://emem.dev/ | tr ';' '\n' | grep -i script-src
```

**Cookie and storage inventory: empty.** The site sets no cookies at all. No
`localStorage`, no `sessionStorage`, no `IndexedDB`. Nothing to accept, nothing
to reject, nothing to manage.

**Verifying the claim.** Open DevTools, Application, on `https://emem.dev/`:

- Cookies for `emem.dev` MUST be empty, before and after any interaction.
- Local Storage and Session Storage MUST be empty.
- The Network tab MUST show no request to any host other than `emem.dev` and
  `fonts.googleapis.com` / `fonts.gstatic.com`, which serve the two typefaces
  and receive no measurement.

If you see different behaviour, this policy is wrong and I want to know: email
`avijeet@vortx.ai`.

**Lawful basis (GDPR Art. 6).** No personal data is processed for analytics,
because no analytics run, so no Art. 6 basis is required for it. The server
access log described under [What we collect](#what-we-collect) is processed
under **Art. 6(1)(f) legitimate interests** (operational health and abuse
mitigation), with the IP truncated and blake3-hashed before it is written and a
30-day retention enforced by journald.

**Cross-border transfer.** None for analytics, since there is no analytics
processor. Requests reach a server in the operator's stated jurisdiction; see
[Third parties](#third-parties) for the upstream data sources a materialising
read may contact, which never receive your identity.

**What this section used to say.** Until 2026-08, this policy described Google
Analytics 4 under Consent Mode v2, a consent banner, and `_ga` cookies. All of
it was removed from the site, and this section described a thing that could no
longer happen. It is recorded here rather than silently rewritten, because a
privacy policy that quietly changes what it claimed is worth less than one that
says when it changed.

## Agent-written memory

This responder hosts a persistent memory subsystem. It is the point of the
product, and it behaves differently from the rest of this policy, so read
this section before writing anything to it.

**What is stored.** When an agent calls `emem_memory_create` (or `str_replace`,
`insert`, `rename`), the responder stores the full file text, its path, a
content address of the bytes (`file_cid`), the writer's ed25519 public key,
the writer's signature over the write, and the timestamp. Nothing is
summarised or discarded: the bytes you send are the bytes that are kept.

**By default everything stored is public.** An ordinary entry has no
per-caller read isolation. Any caller, with no key and no account, can list
every ordinary memory on this responder and read any of it. That is a
deliberate design choice: emem is a shared commons whose value is that one
agent can resolve and verify what another agent wrote. **Treat anything you
write without sealing it as published.**

**Sealed entries (Vault).** Writing with `kind: "vault"` seals the entry
with authenticated encryption at rest. `emem_memory_view` then returns ciphertext
to any caller who does not present a valid `vault_capability`, an ed25519
signature over `blake3("emem.vault_open|" + path + "|" + nonce)`. Sealed
entries are never indexed, so `emem_memory_search` cannot surface them or
their contents.

**We can read your vault entries.** The AEAD key is derived (HKDF-SHA512)
from this responder's own ed25519 secret, and a valid `vault_capability` is a
signature under that same responder key. So a vault seals your bytes against
other callers and against anyone who obtains the database file. It does not
seal them against the operator, Vortx AI Private Limited. We state this
plainly because it is the first thing anyone should ask about an encryption
feature, and because you could work it out from the tool schema. If you need
storage we cannot read, encrypt client-side before writing and keep the key
yourself, or keep the bytes in your own store and publish only a content
address.

Two further limits. Sealing controls **who can read** the bytes, not
**whether they persist**: a sealed entry is still permanent and still
append-only, exactly like an ordinary one, and `emem_memory_delete` still
unpublishes rather than erases. And if the capability key is lost the bytes
become unreadable, which is not the same as removed.

**Who can change what you wrote.** Writes are unauthenticated in the sense
that no account exists, but they are not anonymous, and they are isolated:

- Under `/memories/by_attester/<pubkey8>/…` only the key whose shortcode
  matches that path segment may write. Ownership is bound into the path.
- Anywhere else under `/memories/`, the first attester to create a path owns
  it, and only that key may subsequently write, edit, rename or delete it.
- A small number of older records carry no persisted attester at all, because
  they were written before we recorded authorship. No key can prove it owns
  one, so **every** write to them is refused, including ours. They stay
  readable and are frozen. If one is yours, copy the content into your own
  namespace and work there.
- A mismatch returns `403 memory_namespace_violation` in all three cases.

**What deletion does.** `emem_memory_delete` removes the path from the index, so
the file stops resolving and stops appearing in listings. **The
content-addressed blob and prior versions remain on disk**, because the write
log is append-only and any receipt already issued has to stay verifiable. A
third party who recorded the `file_cid` before deletion can still resolve
those bytes. Treat `emem_memory_delete` as unpublish, not as erasure.

**How to see what you have written.** Every file you wrote under your own
namespace is listed by `emem_memory_view` on `/memories/by_attester/<your-pubkey8>/`,
or over HTTP at `https://emem.dev/memories/by_attester/<your-pubkey8>/`. Each
file is readable at its own path and carries an `authorship` block with the
signature that proves you wrote it.

**How to get content erased.** Index removal is self-service via
`emem_memory_delete`. Erasure of the underlying blob is a manual operator action:
email <avijeet@vortx.ai> with the path or the `file_cid`. We will remove the
bytes and say so on the record. We cannot retract copies other agents have
already resolved and stored elsewhere, and we will not claim otherwise.

**If you are an individual whose personal data ended up in a memory file**
written by someone else, the same address applies and we will act on it
without requiring you to prove which agent wrote it.

## Geocoder cache

Free-text place queries submitted to `/v1/locate` (the `place` field) are
cached locally on the responder against the upstream Nominatim response.
Cache key is the normalized query string; cache TTL is 30 days; cache
contents are local to this responder and never shared upstream. If you
prefer your place queries not be cached, use the `lat` + `lng` form of
`/v1/locate` instead; coordinate lookups are not cached.

## Third parties

When a request triggers auto-materialization, the responder fetches data
from public open-data providers. These requests are made *by the emem
responder*, not by you, and your IP is not forwarded:

- Copernicus Data Space Ecosystem (Sentinel-1, Sentinel-2, Cop-DEM)
- JRC Global Surface Water (`storage.googleapis.com/global-surface-water`)
- Hansen Global Forest Change (`storage.googleapis.com/earthenginepartners-hansen`)
- ESA WorldCover (`esa-worldcover.s3.amazonaws.com`)
- Overture Maps (`overturemaps-us-west-2.s3.amazonaws.com`)
- OpenStreetMap (`overpass-api.de`, `nominatim.openstreetmap.org`)
- Open-Meteo (`api.open-meteo.com`)
- MET Norway (`api.met.no`)
- ORNL DAAC (`modis.ornl.gov`) for MODIS NDVI
- Microsoft Planetary Computer (`planetarycomputer.microsoft.com`) for Sentinel-1 RTC and Sentinel-2 STAC
- Tessera (`dl2.geotessera.org`)

Each provider has its own privacy policy; their licences are surfaced via
`GET /v1/sources`.

## Receipts and signatures

Every response includes a signed receipt: the responder's ed25519 public key,
the request canonicalisation hash, and the fact CIDs. The receipt does
**not** contain user identifiers. You can verify any receipt offline using
the public key at `/.well-known/emem.json`.

## Your rights

Because L0/L1 reads are anonymous and the responder stores no account or
identifier, there is generally no per-user record to act on. That said, to
the extent applicable privacy laws (including the EU/UK GDPR, the
California CCPA/CPRA, and India's Digital Personal Data Protection Act 2023)
grant you rights, we honour them:

- **Access / portability**: request a copy of any operational log line
  that can be tied to an IP you control.
- **Erasure**: request deletion of any such log line ahead of the 30-day
  rotation. Note: signed attestations submitted to `/v1/attest` cannot be
  retracted (see TERMS.md §4); content addressing is by design.
- **Rectification**: request correction of any inaccurate record we hold
  about you.
- **Object / restrict**: ask us to stop processing operational metadata
  associated with your IP for anything beyond fulfilling the request.
- **Withdraw consent / opt out of "sale" or "sharing"**: emem does not
  sell or share personal data with third parties for advertising or
  cross-context behavioural purposes; there is nothing to opt out of.
- **Non-discrimination**: exercising any of the above will not change the
  service you receive.

To exercise a right, email **avijeet@vortx.ai** with enough context (e.g.
the IP and approximate UTC timestamp) for us to locate the record. We aim
to respond within 30 days. If you believe we have not addressed your
request, you may complain to your local supervisory authority (in the EU,
UK, or California) or, in India, to the Data Protection Board once it is
operational.

## Children

emem returns geographic facts; it has no concept of user accounts and is
not directed at children under 13. We do not knowingly collect personal
data from children.

## Changes

We may revise this policy as the protocol evolves. The canonical version
is the file `PRIVACY.md` in
[github.com/Vortx-AI/emem](https://github.com/Vortx-AI/emem); the live
HTTPS rendering is at `https://emem.dev/privacy`. Material changes are
summarised in `CHANGELOG.md`.

## Contact

- Issues, bugs, security: <https://github.com/Vortx-AI/emem/issues>
- Privacy / data-subject-rights enquiries: **avijeet@vortx.ai**

The hosted responder is operated by **Vortx AI Private Limited** (India).
