# claude-skills/ has moved

The emem skills now live in [`plugins/emem/skills/`](../plugins/emem/skills/).
They ship as a Claude Code plugin. One command installs the skills and
the emem MCP server together:

```sh
/plugin marketplace add Vortx-AI/emem
/plugin install emem@emem
```

The old loose copies did not install the MCP server.

To install the skills as plain skills, copy them from a clone of the
repository. Run this from the repository root:

```sh
cp -r plugins/emem/skills/emem-* ~/.claude/skills/
```

The skills exist in one place only. Each skill describes a live service
that changes, and a second copy would be a second thing to keep true.
