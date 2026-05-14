# emem Privacy Policy

_Last updated: 2026-05-14_

emem is an open, content-addressed protocol that returns signed facts about
geographic cells. This document describes the data the **canonical responder**
operated by **Vortx AI Private Limited** (India) at `https://emem.dev` (and
mirrored at `https://vortx-ai-emem.hf.space`) collects, processes, and
retains. Self-hosted emem deployments are governed by their own operators
and are out of scope.

## Tl;dr

- **No accounts. No keys. No PII in the canonical channel.** L0 and L1 read endpoints are anonymous.
- We do not sell or share user data with third parties for advertising.
- We log every request server-side (path, GET query string, status, duration, user-agent, hashed IP) and we run **Google Analytics 4** on the HTML landing page only, under Consent Mode v2 with default-denied for all storage. See §"Google Analytics" below for what that means in practice.
- The responder logs request metadata (timestamp, hashed IP, user-agent, path, query string, status, duration) for operational health and abuse mitigation. Retention is enforced at 30 days via systemd journald (`MaxRetentionSec=30day` in `/etc/systemd/journald.conf.d/30day-retention.conf`). After 30 days the entries are vacuumed from the journal.
- POST request bodies are NOT logged. GET query strings ARE logged (paired with the hashed IP) so they appear in operational logs for the retention window.

## What we collect

| Surface | Data | Purpose | Retention |
|---|---|---|---|
| `GET /…`, `POST /v1/*`, `POST /mcp` | Request method, path, GET query string, response status, duration, **blake3-hashed truncated IP** (8-byte base32, non-reversible — see `agent_ip_hash` in the access log layer at `crates/emem-api-rest/src/lib.rs`), user-agent header, accept header, traceparent header | Server health, abuse mitigation, capacity planning | 30 days, enforced by `MaxRetentionSec=30day` on systemd journald |
| `POST /v1/attest`, `POST /v1/attest_cbor` | The signed attestation payload itself: ed25519 attester pubkey, fact CIDs, Merkle root, attestation timestamp | Persisted to the public, content-addressed corpus by design — that is the whole protocol | Indefinite (the corpus is a public ledger) |
| `POST /v1/recall*`, `POST /v1/intent`, `POST /v1/locate`, `POST /v1/ask`, `POST /v1/backfill` | Request body (cell, place name, free-text question, bands, time window). Bodies are used in-memory only to compute the response and are **not** logged; only the path appears in the access log. | Not persisted beyond the request | None |
| `GET /v1/locate?place=…`, `GET /v1/elevation?lat=…&lng=…`, etc. | The full query string is captured by the access log middleware. If you submit a sensitive place name as a GET query, it is in the operational log for the 30-day retention window, paired with the hashed IP. | Operational | 30 days |
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
- No data from other tools, files, or memory of your AI agent
- No location data beyond what you explicitly include in a request
- No payment information (the public responder is free for L0/L1)

## Google Analytics

The HTML landing page at `https://emem.dev/` (and only that page; not `/v1/*`, not `/mcp`, not `/openapi.json`, not the markdown surfaces) loads Google Analytics 4 with the operator's configured measurement ID under **Consent Mode v2** with the following default values, set before `gtag.js` is loaded. The measurement ID is configured via the `EMEM_GA_MEASUREMENT_ID` environment variable on the server; the live value for this responder is published machine-readably at `/.well-known/agent-card.json` under `provider.data_protection.third_party_analytics[0].measurement_id`. The repo holds only a placeholder so forks do not inherit this responder's GA stream by default.

```
ad_storage:              denied
ad_user_data:            denied
ad_personalization:      denied
analytics_storage:       denied
functionality_storage:   denied
personalization_storage: denied
security_storage:        granted
```

Until and unless an explicit consent banner flips these to `granted` (the canonical responder does not currently render a banner), GA4 emits only **cookieless aggregated pings**. Concretely, with the defaults above:

- **No `_ga` or `_ga_<container>` cookies are set.** No browser-side identifier is stored.
- **No raw IP is transmitted.** GA4 anonymises IP by default; we additionally pass `anonymize_ip: true` for defensive compatibility with auditing tools.
- **No advertising signals are processed** (`ad_storage`, `ad_user_data`, `ad_personalization` all denied).
- **No personalised reporting** in the GA4 console; only modeled aggregate visit counts.
- The pings are sent over `transport_type: beacon` (`navigator.sendBeacon`), which is non-blocking and queued by the browser.

This is the **GDPR-compliant default**. The aggregate visit counts let us see whether the site is being used by humans and by AI crawlers (broken out by user-agent in the Google Analytics console) without processing personal data. Inspect the actual gtag configuration at view-source on `https://emem.dev/`.

**Consent banner.** A consent banner is rendered for human visitors on first visit. AI crawlers do not run JS and do not see the banner. The banner offers two equally-prominent buttons:

- **Accept**: flips `analytics_storage` and `functionality_storage` to `granted` via `gtag('consent', 'update', ...)`. From that moment, GA4 sets the `_ga` cookie (2-year retention) and a `_ga_<container>` cookie (2-year retention), transmits the (default-anonymised) IP, and emits regular analytics events. The decision is recorded in a first-party cookie `emem_consent` (Path=/, Max-Age=180 days, SameSite=Lax, Secure) with value `accept`.
- **Reject**: leaves all storage purposes denied. GA cookies are not set, no IP is sent, only cookieless aggregated pings continue. The decision is recorded in the same first-party cookie `emem_consent` with value `reject`.

The Esc key dismisses with **Reject** (default-deny on accidental dismiss). The banner is not a cookie wall: the entire site remains fully usable without any decision (every endpoint and link works regardless of consent state).

**Why a cookie and not localStorage?** Earlier versions of this site stored the consent decision in `localStorage`. We switched to a first-party cookie on 2026-05-06 because EU-strict browser configurations (Firefox Strict tracking-protection mode, Brave Shields, the "delete site data on close" Safari / Edge defaults common in the EEA) were clearing `localStorage` between sessions. That made the banner re-prompt on every refresh — a bad UX and arguably a dark pattern. First-party cookies survive those configs reliably while remaining strictly necessary under ePrivacy Art. 5(3).

**Revoking or changing consent.** Click **Manage cookies** in the footer at any time. This deletes the `emem_consent` cookie (`Max-Age=0`), re-renders the banner, and lets you make a new decision. To clear all GA cookies in the same step, also clear cookies for `emem.dev` in your browser.

**Cookie / storage inventory.** The site sets no `localStorage`, no `sessionStorage`, no `IndexedDB` entries. The only cookie set before any consent decision is **none**. The only cookie set after any consent decision (accept or reject) is `emem_consent`, which is exempt from prior consent under ePrivacy Art. 5(3) because remembering a consent decision is strictly necessary to honour it.

**Verifying the claims.** Open Chrome DevTools → Application → Cookies on `https://emem.dev/`:

- Before any decision: Cookies tab MUST be empty for `emem.dev`. Local Storage and Session Storage tabs MUST be empty.
- After Reject: Cookies tab shows `emem_consent=reject` only. No `_ga*`. Local Storage stays empty.
- After Accept: Cookies tab shows `emem_consent=accept`, `_ga`, and `_ga_<container>`. Local Storage stays empty.

If you see different behaviour, this policy is wrong; please email `avijeet@vortx.ai`.

**Lawful basis (GDPR Art. 6).** Under default-denied Consent Mode v2 (the state before any banner click), no personal data is processed and Art. 6 does not gate the cookieless pings. After explicit Accept, lawful basis is **Art. 6(1)(a) consent**, freely given (the banner is dismissable with Reject), specific (analytics + functionality only; never advertising), informed (this section), and unambiguous (explicit click on a clearly-labelled button).

**Cross-border transfer.** The cookieless pings reach Google US infrastructure. Google's TADPF self-certification is the legal basis for the transfer. Standard Contractual Clauses (SCCs) apply as a fallback.

**Opt-out.** Install the [Google Analytics opt-out browser add-on](https://tools.google.com/dlpage/gaoptout) for absolute opt-out. With our default-denied config, this is rarely needed (no cookie is set in the first place).

**Why GA at all if it sets no cookies?** The aggregate visit counts let the operator see traffic shape (which agent populations hit the site, which countries, peak hours) without instrumenting a separate analytics stack. Server-side aggregates are also exposed at `/v1/agent_stats` for any caller; the GA console is the operator-facing companion.

## Geocoder cache

Free-text place queries submitted to `/v1/locate` (the `place` field) are
cached locally on the responder against the upstream Nominatim response.
Cache key is the normalized query string; cache TTL is 30 days; cache
contents are local to this responder and never shared upstream. If you
prefer your place queries not be cached, use the `lat` + `lng` form of
`/v1/locate` instead — coordinate lookups are not cached.

## Third parties

When a request triggers auto-materialization, the responder fetches data
from public open-data providers — these requests are made *by the emem
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

- **Access / portability** — request a copy of any operational log line
  that can be tied to an IP you control.
- **Erasure** — request deletion of any such log line ahead of the 30-day
  rotation. Note: signed attestations submitted to `/v1/attest` cannot be
  retracted (see TERMS.md §4); content addressing is by design.
- **Rectification** — request correction of any inaccurate record we hold
  about you.
- **Object / restrict** — ask us to stop processing operational metadata
  associated with your IP for anything beyond fulfilling the request.
- **Withdraw consent / opt out of "sale" or "sharing"** — emem does not
  sell or share personal data with third parties for advertising or
  cross-context behavioural purposes; there is nothing to opt out of.
- **Non-discrimination** — exercising any of the above will not change the
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
