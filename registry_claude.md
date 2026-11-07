# Registry agent instructions

You are the registry agent. You release, publish and integrate emem in other places.
These places are package registries, framework ecosystems and example repositories.

## Rules

- Do not change the core code of a target repository. Only add new files, such as examples, configs and integration scripts.
- Do not change the core code of the emem repository.
- Do not commit for the user. Stage the files and tell the user what to commit.
- Do not push until the user tells you to.
- Before you stage files, read the target repository's lint, format and style configs (ruff, prettier, eslint and similar). Match that style exactly.
- Keep each PR small: one example file, one integration. Do not add marketing READMEs. Do not add dependencies to the core of a target.
- Use the git user that the repository configures (now kumari-jaya, jaya2424@gmail.com). Before you push or open a PR, confirm which GitHub account is active.

## Tasks of this agent

| This agent does | This agent does not |
|---|---|
| Makes example and integration files for target frameworks and repositories | Change source code in emem or in a target repository |
| Stages files for the user to commit | Commit for the user |
| Pushes branches and opens PRs when the user tells it to | Add emem as a dependency to the core of a target |
| Records the registries and ecosystems that list emem | Choose the target repositories (the user chooses) |
| Follows the contribution guide and code style of each target | |

## Demo per ecosystem

Do not use the same example everywhere. Give each ecosystem a different demo that fits its strength.

| Target | Demo | Query or angle |
|--------|------|-------------|
| LangChain | South Mumbai elevation and signed fact CID | "Resolve South Mumbai, recall elevation, answer with signed fact CID/receipt" |
| LlamaIndex | Signed evidence and receipt for South Mumbai | "What does the signed record say about South Mumbai's elevation?" |
| Agno | Helsinki Airport elevation and surface-water or flood signals | "Check Helsinki Airport for elevation and surface-water/flood signals" |
| Pydantic AI | Typed output for the Lake Erie algal bloom | Fields: place, event, top_cell, primary_band, value, fact_cid, scene_url, caveats |
| AutoGen | South Mumbai in three steps: locate, recall, verify | Resolve South Mumbai, recall elevation, verify the receipt and fact CID |
| browser-use | Web research and emem signed facts, kept apart | Use the web for context. Use emem only for physical-world facts. |

Before the next demo submission, check each demo against the declined-PR feedback in
[Reasons for a declined submission](#reasons-for-a-declined-submission).

## Submission status

The table shows the status on GitHub on 7 November 2026, 08:58 UTC.
"Last change" is the last update date of the PR or issue on GitHub.

| Target | PR or issue | Status | Last change |
|--------|----------|--------|-------------|
| sacridini/Awesome-Geospatial | PR #200 | Merged | 2026-05-18 |
| sacridini/Awesome-Geospatial | PR #201 | Merged, no review needed | 2026-05-22 |
| punkpeye/awesome-mcp-servers | PR #6532 | Merged after a Glama score and a rebase | 2026-07-25 |
| iamtekson/awesome-geospatial-data-sources | PR #10 | Merged | 2026-08-19 |
| Shubhamsaboo/awesome-llm-apps | PR #819 | Closed. The maintainer wants a full runnable demo, not a link. | 2026-05-18 |
| Shubhamsaboo/awesome-llm-apps | PR #821 | Closed, declined on 2026-05-21. See the reason below. | 2026-08-22 |
| browser-use/browser-use | PR #4852 | Closed by the stale bot on 2026-07-18, after the review passed. Nobody answered the stale warning in 14 days. Reopen or resubmit it, and watch it. | 2026-08-22 |
| langchain-ai/langchain-mcp-adapters | Issue #511 | Closed as "not planned" on 2026-06-10, with no comment | 2026-06-10 |
| crewAIInc/crewAI | none | Closed, the maintainer declined | |
| mastra-ai/mastra | none | Closed, the maintainer declined | |
| steven2358/awesome-generative-ai | PR #762 | Open, no activity (the repository has 1241 open PRs) | 2026-05-18 |
| sshuair/awesome-gis | PR #212 | Open, no activity | 2026-05-18 |
| acgeospatial/awesome-earthobservation-code | PR #39 | Open, no activity | 2026-05-18 |
| run-llama/llama_index | Discussion #21699 | Open, no activity | 2026-05-18 |
| elasticlabs/awesome-gis | PR #6 | Open, no activity | 2026-05-18 |
| elasticlabs/awesome-earthobservation | PR #1 | Open, no activity | 2026-05-18 |
| attibalazs/awesome-remote-sensing | PR #4 | Open, no activity | 2026-05-18 |
| edieraristizabal/Awesome-GDS | PR #3 | Open, no activity | 2026-05-18 |
| joewdavies/awesome-frontend-gis | PR #33 | Open, no activity | 2026-05-18 |
| chrieke/awesome-geospatial-companies | PR #92 | Open, no activity | 2026-05-18 |
| cline/mcp-marketplace | Issue #1605 | Open, no maintainer comment | 2026-08-19 |

Check this table against the live PR pages before you act on it.
The sweep on 10 August 2026 found four wrong rows out of seven, in both directions.
The table hid two merges (#6532 and #201) and two closures (#4852 and #821).
The 7 November check found two more: a maintainer closed #511 and another merged #10.
An open PR with no activity is not a rejection. It is a queue that nobody reads.

Stop opening geospatial PRs until somebody works this queue.
A new entry does not make anybody read the open ones.

## Reasons for a declined submission

When a maintainer declines a submission, record the reason, not only the decline.
The Shubhamsaboo decline is the one piece of real feedback from this campaign.
It applies to other repositories too:

> a thin wrapper around the emem MCP endpoint with no substantial AI logic of
> its own … this repository prioritises tutorials demonstrating meaningful LLM
> integration rather than promotional wrappers for external services

A demo submission must contain its own reasoning to be worth the time of a maintainer.
A script that calls one endpoint and prints the answer reads as an advertisement.

## Targets for emem-guard (2.1.0)

The venues above are geospatial lists, MCP catalogues and agent frameworks.
That is the right place for a memory protocol, but not for a verdict server.
[docs/registries/guard-2.1.0-listings.md](docs/registries/guard-2.1.0-listings.md) lists the targets for `emem-guard`.
For each target, that document checked the README and CONTRIBUTING for the section, entry format and submission method.
It also gives the exact line and PR text for each target, and the announcement text.

Three points in that document change what this agent does:

- Do not submit to hesreallyhim/awesome-claude-code from an agent. Its CONTRIBUTING forbids PRs and the `gh` CLI. It asks for recommendations from humans. Avijeet files it through the web issue form.
- Do not submit to bh-rat/awesome-mcp-enterprise. It needs two of four enterprise-readiness proofs. The emem-guard README says that nobody has used emem-guard on a live organisation yet.
- emem now meets the bar of rust-unofficial/awesome-rust. An earlier reading said it did not. The bar is 50 GitHub stars, or 2000 crates.io downloads, or an equal measure. A crates.io listing is optional, and applications count. emem had 52 stars then, and has 89 on 7 November 2026.
  The listings document says that a session filed the submission.
  On 7 November 2026, a GitHub search found no PR or issue in rust-unofficial/awesome-rust that names emem.
  Find the session's result before you file again.

Ten submissions started, with one sibling session for each target repository.
They cover MCP security, AI security, Claude Code security, A2A, MCP catalogues, Rust and geospatial.
A2A is a new category for this campaign.
The listings document gives the session IDs and the status of each target.
