# claude-skills/

Installable [Anthropic Skills](https://code.claude.com/docs/en/skills)
for the [emem](https://emem.dev) Earth-memory protocol. Each skill is
a directory containing a `SKILL.md` (YAML frontmatter + body) and any
bundled scripts; Claude Code loads the metadata at startup and
auto-invokes a skill when the user's intent matches its `description`.

## Install

Drop the bundle into your project's `.claude/skills/` directory:

```sh
git clone https://github.com/Vortx-AI/emem.git
mkdir -p .claude/skills
cp -r emem/claude-skills/emem-* .claude/skills/
```

Or as a personal skill across all your Claude Code projects:

```sh
mkdir -p ~/.claude/skills
cp -r emem/claude-skills/emem-* ~/.claude/skills/
```

## Skills in this bundle

| Skill                       | When it auto-triggers                                                                |
|-----------------------------|--------------------------------------------------------------------------------------|
| `emem-locate-and-recall`    | "what's the [weather/elevation/NDVI/...] at [place name]?"                           |
| `emem-verify-receipt`       | "verify this emem receipt offline" / "is this fact authentic?"                       |
| `emem-find-similar`         | "find places similar to X" / "cities with the same urban signature as Y"             |
| `emem-recall-polygon`       | "what's the [band] inside this polygon/region/watershed?"                            |

## Constraints

- **Network access** — these skills call `https://emem.dev`. They work
  in **Claude Code** (full network access) and **Claude.ai**
  (variable). They do **not** work on the **Claude API runtime**,
  which has no network access — use the MCP server at
  `https://emem.dev/mcp` instead for that surface.
- **No auth required** — emem reads are public. The skills include
  no secrets and ship no credentials.
- **No state** — skills are stateless; each invocation issues fresh
  HTTP calls and returns content-addressed receipts.

## License

Apache-2.0, same as emem.
