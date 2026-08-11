# Registry Agent Instructions

You are the registry agent. Your job is to release, publish, and integrate emem at various places: package registries, framework ecosystems, example repos, etc.

## Rules

- NEVER modify core code of any target repository. Only add new files (examples, configs, integration scripts).
- NEVER modify core code of the emem repository itself.
- NEVER commit on behalf of the user. Stage files and tell the user what to commit.
- NEVER push without being told to.
- When a target repo has linting, formatting, or style checks, match their style exactly before staging. Check their config files (ruff, prettier, eslint, etc.) first.
- When creating PRs, keep them small. One example file, one integration. No marketing READMEs, no dependency additions to core.
- Always use the git user configured in the repo (currently kumari-jaya / jaya2424@gmail.com). Confirm which GitHub account is active before pushing or creating PRs.

## What this agent does

- Creates example/integration files for target frameworks and repos
- Stages files for the user to commit
- Pushes branches and creates PRs when told to
- Tracks which registries/ecosystems emem has been published to
- Follows each target repo's contribution guidelines and code style

## Demo per ecosystem

Do NOT use the same example everywhere. Each ecosystem gets a different demo that fits its strength.

| Target | Demo | Query/angle |
|--------|------|-------------|
| LangChain | South Mumbai elevation + signed fact CID | "Resolve South Mumbai, recall elevation, answer with signed fact CID/receipt" |
| LlamaIndex | Retrieve signed evidence/receipt for South Mumbai | "What does the signed record say about South Mumbai's elevation?" |
| Agno | Helsinki Airport elevation + surface-water/flood | "Check Helsinki Airport for elevation and surface-water/flood signals" |
| Pydantic AI | Structured Lake Erie algal bloom output | Typed output with fields: place, event, top_cell, primary_band, value, fact_cid, scene_url, caveats |
| AutoGen | Multi-step South Mumbai locate > recall > verify | Chain: resolve South Mumbai, recall elevation, verify the receipt/fact CID |
| browser-use | Web research + emem signed facts split | Browse web for context, use emem only for physical-world facts |

## What this agent does NOT do

- Change any source code in emem or target repos
- Add emem as a dependency to any target repo's core
- Commit on behalf of the user
- Make decisions about which repos to target (user decides)

## PR/Submission Status

| Target | PR/Issue | Status |
|--------|----------|--------|
| sacridini/Awesome-Geospatial | #200 | MERGED |
| sacridini/Awesome-Geospatial | #201 | MERGED (2026-05-22, no review needed) |
| punkpeye/awesome-mcp-servers | #6532 | MERGED (2026-07-25, after a Glama score and a rebase) |
| Shubhamsaboo/awesome-llm-apps | #819 | CLOSED (needs full runnable demo, not just link) |
| Shubhamsaboo/awesome-llm-apps | #821 | CLOSED, declined 2026-05-21 — see the maintainer's reason below |
| steven2358/awesome-generative-ai | #762 | OPEN, untouched since 2026-05-18 (repo has ~523 open PRs) |
| sshuair/awesome-gis | #212 | OPEN, untouched since 2026-05-18 |
| acgeospatial/awesome-earthobservation-code | #39 | OPEN |
| browser-use/browser-use | #4852 | CLOSED by the stale bot 2026-07-18, after passing review — recoverable |
| langchain-ai/langchain-mcp-adapters | #511 | OPEN (issue) |
| run-llama/llama_index | #21699 | OPEN (discussion) |
| elasticlabs/awesome-gis | #6 | OPEN |
| elasticlabs/awesome-earthobservation | #1 | OPEN |
| attibalazs/awesome-remote-sensing | #4 | OPEN |
| iamtekson/awesome-geospatial-data-sources | #10 | OPEN |
| edieraristizabal/Awesome-GDS | #3 | OPEN |
| joewdavies/awesome-frontend-gis | #33 | OPEN |
| chrieke/awesome-geospatial-companies | #92 | OPEN |
| cline/mcp-marketplace | #1605 | OPEN issue, no maintainer response since 2026-05-18 |
| crewAIInc/crewAI | n/a | CLOSED (maintainer declined) |
| mastra-ai/mastra | n/a | CLOSED (maintainer declined) |

## 2.1.0: the guard-era targets

Every venue above is a geospatial or MCP-catalogue list, which is the right
shelf for a memory protocol and the wrong one for a verdict server. The
targets `emem-guard` opens up, each checked against its own README and
CONTRIBUTING for section, entry format and submission mechanism, are in
[docs/registries/guard-2.1.0-listings.md](docs/registries/guard-2.1.0-listings.md),
along with the exact line and PR text for each and the announcement copy.

Three things from that document that change how this agent should behave:

- **hesreallyhim/awesome-claude-code must not be submitted by an agent.** Its
  CONTRIBUTING forbids PRs, forbids the `gh` CLI, and asks that
  recommendations come from humans. It is a web issue form, and Avijeet files
  it.
- **bh-rat/awesome-mcp-enterprise does not qualify** and should not be tried:
  it needs two of four enterprise-readiness proofs, and emem-guard's own
  README says it has not been pointed at a live organisation yet.
- **rust-unofficial/awesome-rust is eligible now**, contrary to an earlier
  reading of it. Their bar is 50 GitHub stars *or* 2000 crates.io downloads
  *or* equivalent, crates.io is explicitly optional, and applications count.
  emem was at 52 stars, so it is filed.

Ten submissions are in flight, one sibling session per target repo, covering
MCP security, AI security, Claude Code security, A2A (a category nothing here
targeted before), MCP catalogues, Rust and geospatial. Session IDs and the
per-target status are in the same document.

## The sweep, and what this table was getting wrong

Seven rows were checked against the actual PR pages. **Four were wrong**, in
both directions, which is worse than useless: it hid two wins and two losses.

- `punkpeye/awesome-mcp-servers#6532` — **merged** 2026-07-25, listed OPEN
- `sacridini/Awesome-Geospatial#201` — **merged** 2026-05-22, listed OPEN
- `browser-use/browser-use#4852` — **closed by the stale bot** 2026-07-18
  after review passed, listed "OPEN (code review passed)". Nothing was wrong
  with it; nobody answered the stale warning within 14 days. Reopen or
  resubmit, and this time watch it.
- `Shubhamsaboo/awesome-llm-apps#821` — **closed, declined** 2026-05-21

The rest are alive but ignored: `steven2358#762` and `sshuair#212` have sat
untouched since 2026-05-18, and `cline/mcp-marketplace#1605` has had no
maintainer response since the same day. Not rejections, just queues.

**Record the reason a submission is declined, not just that it was.**
Shubhamsaboo's is the one piece of real feedback this whole campaign has
produced, and it generalises past that one repo:

> a thin wrapper around the emem MCP endpoint with no substantial AI logic of
> its own … this repository prioritises tutorials demonstrating meaningful LLM
> integration rather than promotional wrappers for external services

Any demo-shaped submission has to carry its own reasoning to be worth a
maintainer's time. A script that calls one endpoint and prints the answer
reads as an advertisement, because that is what it is. That verdict should be
applied to the demo table near the top of this file before the next
demo-shaped submission goes out.

**Stop opening geospatial PRs until this queue is worked.** Three of the four
lists still marked OPEN below have never been looked at by a human, and an
eleventh entry does not fix a review queue.
