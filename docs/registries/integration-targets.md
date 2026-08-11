# Integrations: where a PR is actually wanted, and how to write one

The listing campaign in `registry_claude.md` has a poor return. Two merges,
two closures, and three submissions nobody has looked at since May. One of the
closures is the useful one, because the maintainer said why:

> a thin wrapper around the emem MCP endpoint with no substantial AI logic of
> its own … this repository prioritises tutorials demonstrating meaningful LLM
> integration rather than promotional wrappers for external services

That is the whole problem in one sentence. A link in a list costs a maintainer
nothing and gives their users nothing, so it sits. Code that does a job for
their users gets merged. The rest of this document is about which repositories
want code, what shape they want it in, and how to write the PR so it does not
read as generated.

## The thing that changed under everyone's feet

The "open a PR adding your integration to the framework" era is over at the
big frameworks, and submissions still aimed at it die quietly. There are now
three distinct shapes, and picking the wrong one is why a submission gets no
reply rather than a rejection.

### Shape A — publish first, then a documentation PR

**LangChain.** `langchain-community` was archived on 2026-06-19. New
integrations are **not accepted as PRs to any langchain-ai repository**. The
integration must be an independent package on PyPI, and the only PR you open
is to `langchain-ai/docs`, adding a row to
`scripts/data/integration_external_docs.yaml`. A dedicated hosted guide page
under `src/oss/python/integrations/<component_type>/` is available at 50,000+
monthly downloads or by maintainer invitation, which is not us yet.

**The prerequisite is already met and nobody has filed the PR.**
`emem-langmem` 2.1.0 — a LangChain `BaseStore` for LangGraph — has been on
PyPI since the 2.1.0 release. That makes the docs row the single
highest-return action available in this entire campaign: one line of YAML,
into the repository whose whole purpose is receiving those lines, for a
package that already exists.

Before filing, check `emem-langmem` against LangChain's standard test suite
for `BaseStore`; their docs make listing conditional on it "where applicable".
If it does not pass, fixing that is the actual work and the PR is the easy
part.

#### The row, and the two defects that would have sunk it

The package's own suite passes, 38 tests, so the code was never the problem.
Both defects were in what it tells a reader:

- `pyproject.toml` declared `Documentation = https://emem.dev/docs/sdks/langmem.html`,
  a page that has never existed. That is the URL the LangChain row would have
  carried, and a listing whose link 404s is a listing that gets reverted.
- `README.md` pointed at `https://emem.dev/docs/memory.md`. Docs render as
  `.html`; the `.md` form 404s.

Both are fixed here. The published 2.1.0 on PyPI still carries the dead
Documentation link until the next release, which is why the row below uses the
GitHub URL — LangChain's own preference order is partner docs, then GitHub,
then PyPI, and the GitHub path resolves today.

File: `langchain-ai/docs` → `scripts/data/integration_external_docs.yaml`,
under `python:` → `stores:` (the category `BaseStore` belongs to; entries there
are not alphabetised, so append).

```yaml
- name: EmemStore
  pypi: emem-langmem
  docs_url: https://github.com/Vortx-AI/emem/tree/main/sdks/emem-langmem
```

PR title:

```
docs: add EmemStore (emem-langmem) to stores
```

PR body, which should stay this short:

```markdown
Adds `emem-langmem`, a `BaseStore[str, bytes]` for LangGraph, to the stores
listing. Published on PyPI, Apache-2.0, 38 tests.

The store keeps agent memory in emem rather than in process: each write is
signed with an ed25519 key the caller holds, and lands under
`/memories/by_attester/<pubkey8>/`, which no other key can write to. Reads
come back with the content address, so a value written in one session can be
re-checked in another.
```

This also retires `langchain-ai/langchain-mcp-adapters#511`. It was filed as
an issue, and no issue in that repository can produce a listing, because
listings do not live there any more.

### Shape B — a real package, into their monorepo

**LlamaIndex** still accepts new integration packages into
`llama-index-integrations/`, scaffolded with their own CLI, and their team
publishes the result to PyPI. That is a genuine contribution rather than a
link, and it is the correct instrument for a framework that has an official
tools directory.

Target: `llama-index-tools-emem`, a tool spec exposing locate → recall →
verify. Not a wrapper around one endpoint: the value is that the tool returns
the `fact_cid` and the receipt alongside the value, so an agent built on
LlamaIndex can cite what it read and a reader can check it.

Note that `run-llama/llama_index#21699` was opened as a *discussion*. That is
the wrong instrument. A discussion asks permission for something their
contributing guide already grants; the package PR is the ask.

### Shape C — a marketplace with its own packaging rules

These want a separately published artefact and have hard, checkable
requirements. Read them before writing code, because each has at least one
rule that invalidates work done the obvious way.

**n8n community node.** From 2026-05-01: every verified node must be published
by a GitHub Action carrying a provenance statement — **a node published from a
laptop is refused**. Scaffold with `npm create @n8n/node`, which includes the
`publish.yml` they expect. Each package integrates exactly one third-party
service. TypeScript. The node must not read environment variables or touch the
filesystem; everything arrives through node parameters.

One rule needs a deliberate decision: **the package licence must be MIT**, and
emem is Apache-2.0. This is not a conflict, it is a normal split — the node is
a thin client that talks HTTP to a public endpoint, so it can ship MIT while
the server it calls stays Apache-2.0. Do it knowingly and say so in the
README, rather than discovering it at review.

**Dify plugin.** Fork `langgenius/dify-plugins`, create an organisation
directory, then a plugin subdirectory holding the source and the packaged
`.difypkg`. Needs a `manifest.yaml` with the privacy field populated, a
separate plugin privacy policy, and a README carrying contact details and the
source repository URL. `PRIVACY.md` already exists and is unusually thorough,
so this is mostly assembly.

**Watch their clock.** Dify marks a PR stale at 14 days of unresolved comments
and closes it at 30 — and a closed one **cannot be reopened**, it needs a new
PR. That is precisely how `browser-use#4852` died: it passed review, a bot
asked whether it was still wanted, nobody answered for 14 days, and it closed.
Any venue with a bot needs a watch, not a submission.

## Ranked build list

Ordered by acceptance probability times reach, not by how interesting the code
is.

| # | Deliverable | Shape | State |
|---|---|---|---|
| 1 | LangChain docs row for `emem-langmem` | A | package published; PR unfiled |
| 2 | `llama-index-tools-emem` | B | not started |
| 3 | n8n node `n8n-nodes-emem` | C | not started; needs MIT + Action publish |
| 4 | Dify plugin | C | not started; PRIVACY.md covers most of it |

Everything above ships from this repository. None of it needs a third party to
say yes first, which is the opposite of the listing campaign, where every item
is blocked on someone else's review queue.

## Databases: be honest about the fit

Vector database integration lists (Qdrant, Weaviate, Chroma, Milvus, LanceDB)
want storage backends and retrievers. emem is not a vector store, and
submitting it as one earns the `awesome-llm-apps` verdict a second time. The
embeddings surface and `emem_find_similar` are real, but "index emem facts in
your vector DB" is a thing a *user* does, not an integration either project
maintains.

Skip the database lists until there is an adapter someone actually asked for.
The absence of a target here is a finding, not an omission.

## The agent-memory ecosystem is the unexplored one

mem0, Letta, cognee, Zep/Graphiti all solve per-user conversational memory:
what did this user tell me, what should I remember about them. emem solves a
different axis — a shared fact that two agents who share no vendor can each
verify — and the two compose rather than compete. Nothing in
`registry_claude.md` has approached any of them.

The honest opening is not "add emem as a memory backend". It is that their
stores hold what the agent was *told*, with no way to check whether it is
still true, and emem holds what was *measured*, with a receipt. Worth an issue
that asks the question before any code: whether they see a seam there. If the
answer is no, that costs one issue instead of a package.

## Writing the PR so it does not read as generated

Maintainers are now triaging a flood of machine-written submissions and have
become fast at spotting them. A PR that pattern-matches to that flood gets
closed on sight, whatever the code underneath is worth. These are the tells,
in rough order of how quickly they give it away.

1. **Every bullet opening with a bolded phrase.** The strongest single tell.
   One or two bolded leads in a body is normal writing; six is a template.
2. **Section scaffolding nobody asked for.** `## Summary` / `## Changes` /
   `## Test plan` on a one-line addition. Use their template if they have one,
   and plain paragraphs if they do not.
3. **Length out of proportion to the diff.** A one-line entry does not need
   twelve lines of justification. Under six is right, and one is often enough.
4. **The vocabulary.** seamlessly, robust, leverage, delve, unlock, empower,
   comprehensive, cutting-edge, "not just X, but Y", "in today's landscape".
   Cut all of them.
5. **Em-dash density.** At most one per body. Commas and full stops otherwise.
6. **Tricolons.** Three parallel clauses in a row is a rhythm these models fall
   into constantly. Break the third one or drop it.
7. **Emoji headers**, unless the surrounding file already uses them.
8. **Restating the diff in prose.** They can read the diff. Say why, not what.

And the things that read as a person, all of which are just being specific:

- **Reference something only a reader of their repository would know.** The
  entry two lines above yours, the rule in their CONTRIBUTING, an open issue
  the change relates to. This is the single most effective signal, and it is
  also genuinely useful to the reviewer.
- **Show one concrete artefact.** A command and the output it actually
  produced. A version. A file path. Never claim a test you did not run.
- **Leave something uneven.** A caveat, an aside, a question. "Happy to move
  this to X if you would rather file it there" is worth more than another
  paragraph of justification, because it hands the maintainer a decision
  instead of a pitch.
- **First person singular, one register, held.** Not "we are excited to".

### On attribution, which is a real decision and not a style question

Some maintainers state that they do not accept AI-authored submissions.
`hesreallyhim/awesome-claude-code` is explicit about it. The response to that
is not to remove the attribution and file anyway. A maintainer who later works
out that a policy was worked around does not just close the PR, they remember
the project, and that cost is permanent and unrecoverable for the sake of one
list entry.

So the split is:

- Where a maintainer bans AI submissions, **Avijeet writes and files it.** The
  research and the draft are useful to him; the authorship has to be real.
- Everywhere else, the goal is prose that does not read as slop **because it
  is not slop** — specific, short, checked, and reviewed by a person who
  understands the claim and is willing to defend it in the thread. That is a
  quality bar, and it is met by writing better, not by hiding anything.

The practical consequence: a human reads every one of these before it goes
out. That is not a formality. Half the tells above survive any amount of
instruction and get caught in ten seconds by someone who knows what the
project actually does.

## What is blocked, and on what

The ten listing PRs from the previous pass are all staged and all stuck at the
same place: the GitHub App cannot create forks under `avijeetsingh1`, so every
session got as far as a committed branch and then failed to push. That unblocks
with a fork per repository, done by hand once.

Nothing in the build list above is blocked by it. Those four ship from here.
