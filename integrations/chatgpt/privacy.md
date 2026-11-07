# Privacy Policy

## emem ChatGPT App

**Effective date:** 2026-08-27. **Last edited:** 2026-11-07.

This page tells how the emem app behaves inside ChatGPT. The full policy for
the hosted responder is at **https://emem.dev/privacy**. Where the two differ,
the full policy is correct.

### What emem does

emem is a query surface over a shared, verifiable memory. It holds no user
data. It answers questions about real places with signed facts from open
Earth-observation data. It returns a receipt with every answer.

Seven of the eighteen tools in this app can sign NEW facts into the public
store when a requested band is cold. On those seven tools, `readOnlyHint` is
false. The responder makes these facts from public Earth-observation sources,
never from data that a user sent.

### What ChatGPT sends to emem

When a question names a real place, ChatGPT sends the place name, the
coordinates or the question text to the public emem API. **ChatGPT sends no
user identity, session or account information.** emem cannot ask for it:
reads need no key and no account.

### What emem returns

emem returns signed facts with content-addressed identifiers and an ed25519
receipt. The receipt verifies offline against the key published at
`/.well-known/emem.json`.

### What the responder logs, and for how long

**An earlier version of this file was wrong here.** It said that emem kept
nothing after the request ended. That was not true. The correct statement
follows.

The hosted responder writes a server access log for every request. Each entry
holds these fields:

- method, path and GET query string
- response status and duration
- the user-agent, accept and traceparent headers
- a **blake3-hashed, truncated IP** (8-byte base32, one-way)

systemd-journald deletes entries after **30 days** (`MaxRetentionSec=30day`).
The responder does not log POST request bodies. It never stores a raw IP.

The lawful basis is legitimate interests: operational health and abuse
mitigation.

### Cookies and analytics

**None.** The site sets no cookies, no `localStorage` and no `sessionStorage`.
It runs no third-party analytics. The operators removed Google Analytics and
its consent banner. The Content-Security-Policy on every page does not allow
`googletagmanager.com` or `google-analytics.com`. Thus the browser blocks
these scripts if someone adds them again. The API surface that this app uses
serves no HTML, so this section does not apply to it.

### Writes to shared memory

emem also has a writable memory surface. **This app exposes two write tools,
and both refuse a ChatGPT user.**

- `emem_entity` makes a shared object identity.
- `emem_entity_link` binds a phrasing to an identity.

Both tools change what other agents resolve a name to. Thus both need an
ed25519 attester block, signed with a keypair that the caller makes locally.
A ChatGPT user has no such key, so both tools return a 403 that names the
missing field. No user content gets into the store through this app.

Anything that an agent writes to the shared store from another client is
world-readable and permanent, unless the agent seals it. The full statement
is at https://emem.dev/privacy.

### Third-party sharing

emem does not share query data with third parties. Sometimes a request needs
data that emem does not hold yet. Then the responder gets it from public
open-data providers **on its own behalf**. The responder does not send your IP
to them. https://emem.dev/privacy lists the providers.

### Your rights

emem holds no account and no identifier for you. Thus there is no profile to
access, export or erase. Send questions about the hosted responder's logs to
the contact below.

### Contact

https://emem.dev/support, avijeet@vortx.ai

### Source

https://github.com/Vortx-AI/emem
