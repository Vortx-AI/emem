# claude-skills/ — moved

These skills now live in [`plugins/emem/skills/`](../plugins/emem/skills/),
packaged as a Claude Code plugin so they install with one command instead
of a manual copy:

```sh
/plugin marketplace add Vortx-AI/emem
/plugin install emem@emem
```

The plugin also wires the MCP server, which the loose copies never did.

To install them as plain skills, copy from the new path:

```sh
cp -r emem/plugins/emem/skills/emem-* ~/.claude/skills/
```

One copy, one place. The skills moved rather than being duplicated
because a second copy is a second thing to keep true, and these describe
a live surface that changes.
