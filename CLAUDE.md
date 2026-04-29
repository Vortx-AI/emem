# Repo conventions for Claude

## Commit messages

- **Never** add `Co-Authored-By: Claude ...` (or any Claude/Anthropic trailer) to commit messages.
- No `Generated with Claude Code` footers, no `🤖` markers, no AI attribution of any kind in commits, PR titles, or PR bodies.
- Plain, human-authored-style commit messages only.

If you accidentally include such a trailer, the local `commit-msg` hook will reject the commit. Do not bypass with `--no-verify`.
