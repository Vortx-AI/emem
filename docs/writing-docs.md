# How to write emem docs

This page gives the rules for the prose in this repository and the tool that checks them. The rules come from
CONTRIBUTING.md and from ASD-STE100, the Simplified Technical English standard. Many readers of these docs read
English as a second language, and many readers are machines. Short sentences and one word for one thing help both.

## The rules

1. Write one idea in each sentence.
2. Keep an instruction to 20 words or fewer. Keep a description to 25 words or fewer.
3. Use the active voice. Say who or what does the action: "The responder signs the fact."
4. Use simple words. Write "use", not `utilize`. Write "about", not `approximately`. Write "through", not `via`.
5. Use one term for one thing. When a page says "fact", it does not also say "record" for the same object.
   The [glossary](glossary.md) gives the term to use.
6. Do not use em or en dashes. Use a comma, a colon or a full stop.
7. Give each number its source: a registry, an endpoint or a measurement with its date.
8. Say what ships and what does not ship in the same paragraph. Do not describe roadmap work as a feature.
9. Do not use filler words such as `seamless`, `robust`, `powerful`, `leverage`, `delve` and `not just X`.
   They do not tell the reader a fact.
10. Write the full form of a verb: "do not", not `don't`.

Code, commands, URLs, file names, field names and tokens are exempt. Put them in backticks.

## Before and after

Before:

```text
emem lets agents seamlessly leverage verifiable memory, not just a cache.
```

After: "emem stores signed facts. An agent reads a fact and checks its signature without a call to emem."

Before:

```text
A token pays when the value must survive a summariser, be checked by someone who does not
trust you, or travel with others in one bundle.
```

After: "Use a token when a summariser can change the value. Use it when the reader does not trust you.
Use a bundle to send many facts in one line."

## Check a page

Two scripts check Markdown. Both need only Python 3.

`scripts/doc_lint.py` is the gate in CI. The `doc-lint` job runs it on each push and each pull request. It reads
`README.md`, `CONTRIBUTING.md` and `docs/**/*.md`. It finds these problems:

- em and en dashes,
- words from a short buzzword list, and words that describe an era of machine intelligence,
- relative links that do not resolve,
- text that gives `fact_cid` as the truncated `[..16]` digest.

Five older pages have a fixed dash limit in the script. Their dash count can only go down. When the script finds a
problem, the `doc-lint` job fails. On 7 November 2026, `main` has no branch protection, so a failed check does not
block a merge by itself. The reviewer must look at the checks.

`scripts/ste_lint.py` checks the STE rules above: sentence length, words from the replace list, filler words, passive
voice, "-ing" forms, phrasal verbs and contractions.

```bash
python3 scripts/doc_lint.py                              # the CI gate, over all docs
python3 scripts/ste_lint.py docs/quickstart.md           # STE rules, one page
python3 scripts/ste_lint.py --changed origin/main        # STE rules, only the lines you changed
python3 scripts/ste_lint_test.py                         # the tests of the STE lint
```

Each finding of `ste_lint.py` gives the file, the line, the rule and a suggestion. Errors are long sentences, dashes,
words from the replace list and filler words. Warnings are heuristics. A warning asks you to read the sentence again.
It does not prove a fault.

With no `--fail-on` option, `ste_lint.py` only reports. The older docs have many findings, so a full check would block
every build. For a pull request, check only your own lines:

```bash
python3 scripts/ste_lint.py --changed origin/main --fail-on error
```

The word lists are in `scripts/ste_lint_rules.json`. They are a selected subset. They are not the licensed
ASD-STE100 dictionary. When the team decides that a word is a defined technical term, add it to
`exempt.allow_words`. Add product and source names to `exempt.proper_names`.

## Pages the STE lint skips

The STE lint skips six records: the collaboration log, the changelog, the two whitepapers (`whitepaper-v1.md` and
`whitepaper-v2.md`), the agent notes and the test results in `tests/comprehensive/results/`. We keep their text as it is.
The list is `skip_paths` in `scripts/ste_lint_rules.json`.
