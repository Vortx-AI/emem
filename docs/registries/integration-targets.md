# Integration targets: where projects want a PR, and how to write one

This page is a plan. It names the third-party projects that accept emem code,
the form each one wants, and how to write the PR.
We checked the states on this page on 7 November 2026, unless a line says otherwise.

## Why listings did not work

The listing campaign in [`registry_claude.md`](../../registry_claude.md) had a poor return.
Two submissions merged, two closed, and three got no reply after May 2026.
One closure gave a reason:

> a thin wrapper around the emem MCP endpoint with no substantial AI logic of
> its own … this repository prioritises tutorials demonstrating meaningful LLM
> integration rather than promotional wrappers for external services

A link in a list gives the maintainer's users nothing, so it waits.
Maintainers merge code that does a job for their users.
The rest of this page is about which projects want code, and in which form.

## Pick the form the project wants

Large frameworks no longer take integration PRs into their own code.
There are three forms now.
A submission in the wrong form gets no reply, not a rejection.

| Form | What the project wants | Projects |
|---|---|---|
| A | A package you publish, then one docs PR that lists it | LangChain |
| B | A package PR into their monorepo; they publish it | LlamaIndex |
| C | A package in their marketplace, with their own packaging rules | n8n, Dify |

### Form A: publish the package, then open a docs PR (LangChain)

GitHub shows the `langchain-ai/langchain-community` repository as archived since 19 June 2026.
LangChain does not take new integrations as PRs to its code repositories.
The integration is a separate package on PyPI.
The only PR goes to `langchain-ai/docs`. It adds a row to `scripts/data/integration_external_docs.yaml`.
The header of that file says a hosted guide page needs 50,000 monthly downloads or a feature by LangChain.
emem does not meet that bar.

The package exists: `emem-langmem` 2.4.4 is the newest version on PyPI.
It holds `EmemStore`, a LangChain `BaseStore[str, bytes]` from `langchain_core.stores`.
It is not a LangGraph store, so `StateGraph.compile(store=...)` cannot use it.
The PyPI summary of 2.4.4 still says "drop-in agent memory for LangGraph".
Change that description in a new release before you file the docs PR.
No PR for it is open or closed in `langchain-ai/docs` (GitHub search for `emem-langmem`, 0 results).

Do these steps before you file:

1. Fix the defects in [docs/sdks/langmem.md, Known issues](../sdks/langmem.md).
   `yield_keys()` with no prefix fails, and a flat tuple key raises `TypeError`.
2. Fix the install conflict. The package pins `langchain-core>=0.3,<1.0`.
   The newest `langgraph` (1.2.20) needs `langchain-core` 1.4.7 or later.
3. Run LangChain's standard `BaseStore` tests on the package.
   LangChain lists an integration on the condition that it passes them "where applicable".
4. Release the fixed package to PyPI.

The first pass found two link defects. A later release fixed both.
PyPI 2.4.4 gives `Documentation` as the GitHub folder `sdks/emem-langmem`, which returns 200.
The package README links to `https://emem.dev/docs/memory.html`, which returns 200.

The row goes in `scripts/data/integration_external_docs.yaml`, under `python:` then `stores:`.
Entries there are not in alphabetical order, so add the row at the end.
The file sets the link order: partner docs, then GitHub, then PyPI.
The partner page `https://emem.dev/docs/sdks/langmem.html` returns 200 and describes `EmemStore`, so use it.

```yaml
- name: EmemStore
  pypi: emem-langmem
  docs_url: https://emem.dev/docs/sdks/langmem.html
```

PR title:

```
docs: add EmemStore (emem-langmem) to stores
```

PR body. Keep it this short:

```markdown
Adds `emem-langmem`, a LangChain `BaseStore[str, bytes]`, to the stores
listing. Published on PyPI, Apache-2.0.

The store keeps agent memory in emem rather than in process: each write is
signed with an ed25519 key the caller holds, and lands under
`/memories/by_attester/<pubkey8>/`, which no other key can write to. Reads
come back with the content address, so a value written in one session can be
re-checked in another.
```

Before you send it, add the test result from step 3 to the body.
Do not write a test count that you did not run.

The old issue `langchain-ai/langchain-mcp-adapters#511` closed as `not_planned` on 10 June 2026.
An issue in that repository cannot give a listing, because LangChain keeps listings in `langchain-ai/docs`.

### Form B: send a package PR into the monorepo (LlamaIndex)

LlamaIndex takes new integration packages into `llama-index-integrations/`.
You make the package with their CLI, and their team publishes it to PyPI.

The target is `llama-index-tools-emem`, a tool spec for locate, recall and verify.
Its value is that each tool returns the `fact_cid` and the receipt with the value.
An agent on LlamaIndex can then cite what it read, and a reader can check it.
The package does not exist yet: `llama-index-tools-emem` is not on PyPI (404).
The repository has an example only, in [`examples/llamaindex/`](../../examples/llamaindex/).

`run-llama/llama_index` discussion #21699 is open since 18 May 2026, with 0 comments.
A discussion asks for permission that their contributing guide already gives.
The package PR is the request.

### Form C: publish to a marketplace with its own rules (n8n, Dify)

Each marketplace wants a separately published package and has rules you can check.
Read the rules before you write code.

#### n8n community node

The n8n verification guidelines set these rules:

| Rule | State of `integrations/n8n` (`n8n-nodes-emem` 0.1.0) |
|---|---|
| From 1 May 2026, publish through GitHub Actions with a provenance statement. n8n refuses a node published from a laptop. | Not ready. `.github/workflows/publish-n8n-node.yml` runs `npm publish --provenance` on a tag `n8n-*.*.*`. No such tag exists. `package.json` gives `repository.url` as `https://github.com/Vortx-AI/n8n-nodes-emem.git`, and that repository does not exist (GitHub API 404 on 7 November 2026). Provenance checks that this URL is the repository that built the package. Set it to `https://github.com/Vortx-AI/emem` with `directory: integrations/n8n` first. |
| Scaffold with `npm create @n8n/node`, which includes the expected `publish.yml` | Not checked. Compare the files in `integrations/n8n` with the scaffold output before you publish. |
| One package integrates exactly one third-party service | Met. The package has one node, `Emem`. |
| The node is in TypeScript | Met. `nodes/Emem/Emem.node.ts`. |
| The package licence is MIT | Not met. `package.json` says `Apache-2.0`. |
| No external dependencies | Met. The only entry is a peer dependency on `n8n-workflow`. |
| No access to environment variables or the file system | Met. The node source reads neither. |

The package is not on npm (the registry returns "Not found").

The licence needs a decision.
The node is a thin HTTP client for a public endpoint.
It can ship under MIT while the emem server stays Apache-2.0.
Make that choice on purpose, and say so in the node README.

#### Dify plugin

To submit, fork `langgenius/dify-plugins`. Add an organisation folder, then a plugin folder.
The plugin folder holds the source and the packaged `.difypkg`.
The plugin needs these parts:

- a `manifest.yaml` with a value in the privacy field
- a separate privacy policy for the plugin (start from [`PRIVACY.md`](../../PRIVACY.md))
- a README with contact details and the source repository URL

The plugin `vortx-ai/emem` is on the Dify marketplace at version 2.4.0, updated 13 September 2026.
The live responder is 2.4.4.
The plugin source is not in this repository, so the version gate cannot see it.
Copy the source into `integrations/dify/` and add it to the version bump surfaces.

Watch the PR after you file it.
The original note says Dify marks a PR stale after 14 days of unresolved comments.
It closes the PR after 30 days, and nobody can reopen a closed PR.
That is how `browser-use/browser-use#4852` ended: it closed on 18 July 2026.
It passed review. Then a bot asked if the author still wanted it, and nobody answered.
A venue with a bot needs someone to watch it.

## Build list in order

The order is the chance of acceptance multiplied by reach.

| # | Deliverable | Form | State on 7 November 2026 |
|---|---|---|---|
| 1 | LangChain docs row for `emem-langmem` | A | Package 2.4.4 on PyPI. Defects and install conflict open. PR not filed. |
| 2 | `llama-index-tools-emem` | B | Not started. Example only. |
| 3 | n8n node `n8n-nodes-emem` | C | Code and publish workflow in the repo. Licence is Apache-2.0, not MIT. `repository.url` names a repository that does not exist. Not on npm. |
| 4 | Dify plugin | C | Marketplace at 2.4.0, responder at 2.4.4. Source not in this repo. |

We build all four, here or in our own packages. LlamaIndex has not started.
Each still needs one review in the target project.
Every item in the listing campaign waits in another project's review queue.

## Skip vector database lists

Vector database integration lists (Qdrant, Weaviate, Chroma, Milvus, LanceDB) want storage backends and retrievers.
emem is not a vector store.
A submission as one gets the same answer as the closure quoted at the top.
The embeddings surface and `emem_find_similar` exist.
But a user indexes emem facts in a vector database. None of these projects maintains that as an integration.

Do not submit to these lists until a user asks for an adapter.
An empty list here is a finding.

## Ask the agent-memory projects first

mem0, Letta, cognee and Zep/Graphiti keep per-user conversational memory.
They store what a user said to the agent.
emem keeps a shared fact that two agents with no common vendor can each verify.
The two can work together.
Nothing in `registry_claude.md` has contacted any of them.

Do not open with "add emem as a memory backend".
Their stores hold what a user said, with no way to check if it is still true.
emem holds a measurement, with a receipt.
Open one issue that asks if they see a place for that.
If they say no, the cost is one issue, not one package.

## Write the PR so it does not read as generated

Maintainers now get many machine-written submissions, and they find them fast.
They close a PR that looks like one on sight.
These signs give it away, the fastest first:

1. Every bullet starts with a bold phrase. This is the strongest single sign.
   One or two bold leads are normal. Six is a template.
2. Section headings that nobody asked for, such as `## Summary`, `## Changes` and `## Test plan` on a one-line change.
   Use their template if they have one. If not, write plain paragraphs.
3. A body that is long for the size of the diff.
   A one-line entry needs fewer than six lines of text. One line is often enough.
4. The vocabulary. This repository keeps the word list as code: `BUZZWORDS` in `scripts/doc_lint.py`.
   CI runs it in the `prose convention holds` job.
   Run it on your PR body before you send it, and remove those words.
5. Dashes. `doc_lint.py` allows zero em or en dashes. Use commas, colons and full stops.
6. Three parallel clauses in a row. Break the third one or remove it.
7. Emoji in headings, unless the file already uses them.
8. A prose copy of the diff. The reviewer can read the diff. Say why, not what.

These things read as a person wrote them. Each one is a way to be specific:

- Refer to something that only a reader of their repository knows.
  For example, the entry above yours, a rule in their CONTRIBUTING file, or a related open issue.
  This is the most effective sign, and it also helps the reviewer.
- Show one real artefact: a command and its actual output, a version, a file path.
  Do not claim a test that you did not run.
- Leave one uneven part: a caveat, an aside or a question.
  For example: "Happy to move this to X if you would rather file it there."
  That gives the maintainer a decision instead of a pitch.
- Write in the first person singular, in one register, from start to end.
  Avoid stock phrases of excitement.

### Decide on attribution before you file

Some maintainers do not accept AI-written submissions.
`hesreallyhim/awesome-claude-code` says so explicitly.
Do not remove the attribution and file anyway.
A maintainer who learns that someone bypassed a policy closes the PR and remembers the project.
That cost is permanent, and one list entry is not worth it.

Use this split:

- Where a maintainer bans AI submissions, Avijeet writes and files the PR.
  The research and the draft help him, but the author must be real.
- Everywhere else, the text must be specific, short and checked.
  A person who understands the claim reviews it and defends it in the thread.
  That is a quality bar. Better writing meets it. A hidden author does not.

A person reads every PR before it goes out.
Half the signs above survive any instruction, and a person who knows the project sees them in seconds.

## What stops the listing PRs

This section comes from the original note. We did not check it again on 7 November 2026.
Ten listing PRs from the earlier pass stopped at the same point.
The GitHub App cannot create forks under `avijeetsingh1`.
Each session made a committed branch and then failed to push it.
To unblock them, make one fork per repository by hand.

The build list above does not depend on these forks.
