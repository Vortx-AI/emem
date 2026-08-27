# Privacy Policy

## emem ChatGPT App

**Effective date:** 2026-08-27

This summarises how the emem app behaves inside ChatGPT. The full policy for
the hosted responder, which is the authority where the two differ, is at
**https://emem.dev/privacy**.

### What emem does

emem is a query surface over a shared, verifiable memory, and it holds no user
data of any kind. Five of its nine tools here can sign NEW facts into emem's
publicly readable store when a requested band is cold, so `readOnlyHint` is
false on those five; what gets written is derived from public
Earth-observation sources and never from anything a user sent. It answers
questions about real places from signed facts derived from open Earth
observation data, and returns a receipt with every answer.

### What is sent to emem

When a question names a real place, ChatGPT sends the place name, coordinates,
or the question text to emem's public API. **No user identity, session, or
account information is sent**, and emem has no way to ask for one: reads take
no key and no account.

### What emem returns

Signed facts with content-addressed identifiers and an ed25519 receipt that
verifies offline against the key published at `/.well-known/emem.json`.

### What is logged, and for how long

**This is the part an earlier version of this file got wrong.** It claimed emem
retained nothing beyond the duration of the request. That was not accurate, and
the correct statement is:

The hosted responder writes a server access log for every request, containing
the method, path, GET query string, response status, duration, user-agent, and
a **blake3-hashed, truncated IP** (8-byte base32, non-reversible). Retention is
**30 days**, enforced by `systemd-journald`
(`MaxRetentionSec=30day`), after which entries are vacuumed.

The lawful basis is legitimate interests (operational health and abuse
mitigation). No raw IP is stored.

### Cookies and analytics

**None.** The site sets no cookies, no `localStorage`, no `sessionStorage`, and
runs no third-party analytics. Google Analytics and its consent banner were
removed; the Content-Security-Policy served with every page does not permit
`googletagmanager.com` or `google-analytics.com`, so a reintroduction would be
refused by the browser rather than merely regretted. None of this applies to
the API surface this app uses, which serves no HTML at all.

### Agent-written memory (not used by this app)

emem also has a writable memory surface. **This app exposes two entries to it and
both are gated.** `emem_entity` mints a shared object identity and
`emem_entity_link` binds a phrasing to one; both change what other agents resolve
a name to, so both require an ed25519 attester block signed by a keypair the
caller generates locally. A ChatGPT user holds no such key, so both refuse with a
403 that names the missing field. No user content reaches the store by any route.
For completeness:
anything an agent explicitly writes to the shared store elsewhere is
world-readable and permanent unless sealed. See
https://emem.dev/privacy for the full statement.

### Third-party sharing

emem does not share query data with any third party. When a request needs data
emem does not yet hold, the responder fetches it from public open-data
providers **on its own behalf**; your IP is not forwarded to them. The
providers are listed at https://emem.dev/privacy.

### Your rights

Because emem holds no account and no identifier for you, there is no profile to
access, export or erase. Requests about the hosted responder's logs can be sent
to the contact below.

### Contact

https://emem.dev/support, avijeet@vortx.ai

### Source

https://github.com/Vortx-AI/emem
