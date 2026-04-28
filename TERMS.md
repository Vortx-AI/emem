# emem Terms of Service

_Last updated: 2026-04-28_

These terms govern your use of the canonical emem responder operated by
Vortx-AI at `https://emem.dev` (and mirrored at
`https://vortx-ai-emem.hf.space`). The protocol itself is Apache-2.0 and
self-hosting is unrestricted; these terms cover only the public hosted
instance.

## 1. The service

emem is a read-mostly MCP / REST server that returns content-addressed,
ed25519-signed facts about geographic cells. L0 and L1 read endpoints are
free, anonymous, and rate-limited. L2 write endpoints (`/v1/attest`,
`/v1/challenge`) require a registered ed25519 attester key.

## 2. Acceptable use

You agree not to:

- Use emem to violate applicable law in your jurisdiction or ours.
- Use emem to facilitate the violation of any applicable AI provider's
  usage policy (e.g., Anthropic's Usage Policy when emem is used as a
  Claude connector).
- Submit attestations you know to be false, or attestations that violate the
  intellectual-property or privacy rights of third parties.
- Use emem to identify, locate, or surveil specific natural persons. emem
  serves geographic cells (~305 m today), not people.
- Attempt to circumvent rate limits, DoS the service, or extract data at a
  rate inconsistent with normal agent use.
- Re-publish bulk corpus exports under a more restrictive licence than the
  upstream providers permit (see `/v1/sources` for per-provider licences).

## 3. Rate limits and SLA

The hosted responder enforces a per-IP rate limit (60 req/min, 120 burst).
The service is offered **as-is, without uptime SLA**. For production
workloads, run your own instance — the canonical image is at
`ghcr.io/vortx-ai/emem`.

## 4. Attestations

When you submit an attestation:

- You warrant that you own the ed25519 attester key and are authorised to
  bind that identity to the submitted facts.
- The submitted facts become part of the public, content-addressed corpus
  and **cannot be retracted** (other attesters can submit a `Challenge`,
  which marks the fact as disputed but does not delete it — content
  addressing is by design).
- The responder may re-sign your attestation under its own identity for
  redistribution; this does not transfer authorship.

## 5. Third-party data

emem auto-materialises facts from public open-data providers (Copernicus,
JRC, Hansen, ESA, Overture, OSM, Open-Meteo, …). Each provider's licence
applies to the underlying values and is surfaced via `/v1/sources`. The
responder's signed receipt covers _the responder's promise that the fact was
fetched from the named source at the named time_, not the upstream
provider's terms.

## 6. No warranty

emem is provided **AS IS**, without warranty of any kind, express or
implied. The responder makes no guarantee that any fact is current,
complete, or fit for any particular purpose. Do not use emem as the sole
source of truth for life-safety, financial, or legally binding decisions.

## 7. Limitation of liability

To the maximum extent permitted by law, Vortx-AI shall not be liable for
any indirect, incidental, special, consequential, or punitive damages
arising out of your use of the hosted responder.

## 8. Indemnity

You agree to indemnify and hold Vortx-AI harmless from any claims arising
out of attestations you submit, your violation of these terms, or your
violation of any third-party right.

## 9. Open source

The protocol, schema, and reference implementation are licensed under
Apache-2.0. The licence file is `LICENSE` in
[github.com/Vortx-AI/emem](https://github.com/Vortx-AI/emem).

## 10. Changes

We may revise these terms as the protocol evolves. The canonical version
is `TERMS.md` in the repo; the live HTTPS rendering is at
`https://emem.dev/terms`. Material changes will be summarised in the
`CHANGELOG.md`.

## 11. Contact

- Issues, support, security: <https://github.com/Vortx-AI/emem/issues>
- Legal / commercial: **avijeet@vortx.ai**
