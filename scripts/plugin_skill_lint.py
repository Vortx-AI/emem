#!/usr/bin/env python3
"""Hold the Claude Code plugin to what its directory review checks.

Each rule is a finding a review made, or a mistake made while fixing one:

  undefined-var   a shell block uses $VAR before any block in the skill
                  sets it (a rewrite left $PK8 unset, 2026-10-18)
  sibling-script  a skill runs a script from another skill's directory;
                  a skill must ship what it runs (review, 2026-10-17)
  local-read      a skill or bundled script reads the user's home or config
                  (~/, $HOME, Path.home, expanduser); secrets come from a
                  sensitive userConfig option (review, 2026-10-17)
  foreign-host    a command fetches from a host other than emem.dev, which
                  every skill's network notice says is the only one
  pipe-exec       something is piped into a shell or an interpreter
  credential      the emem plugin declares a userConfig option or a local
                  (stdio) MCP server; the review holds a plugin whose
                  credential could reach the parts that send data, so the
                  key lives only in the separate emem-signer plugin
                  (review, 2026-10-20)

Exit 1 on any finding, with file and line.
"""
import glob
import json
import os
import re
import sys

PLUGINS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "plugins")
ROOT = os.path.join(PLUGINS, "emem")
ENV_OK = {"CLAUDE_SKILL_DIR", "CLAUDE_PLUGIN_ROOT", "CLAUDE_PLUGIN_DATA", "HOME", "PWD", "PATH"}
ALLOWED_HOSTS = {"emem.dev"}
LOCAL_READ = re.compile(r"(~/\.|\$HOME\b|\$\{HOME\}|Path\.home\(|expanduser\()")
FETCH = re.compile(r"\b(?:curl|wget)\b[^\n]*?https?://([a-zA-Z0-9.-]+)")
PIPE_EXEC = re.compile(r"\|\s*(?:sudo\s+)?(?:sh|bash|zsh|python3?|node|perl|ruby)\b(?!\S)")
findings = []


def rel(p):
    return os.path.relpath(p, os.path.join(ROOT, "..", ".."))


for f in sorted(glob.glob(os.path.join(ROOT, "skills", "*", "SKILL.md"))):
    text = open(f, encoding="utf-8").read()
    defined = set()
    for m in re.finditer(r"```(?:sh|bash|shell)\n(.*?)```", text, re.S):
        start = text.count("\n", 0, m.start()) + 2
        for i, line in enumerate(m.group(1).splitlines()):
            n = start + i
            for u in re.findall(r"\$\{?([A-Z][A-Z0-9_]*)", line):
                if u not in defined and u not in ENV_OK:
                    findings.append((rel(f), n, "undefined-var", f"${u} is used before it is set"))
            for d in re.findall(r"(?:^|[\s;(])([A-Z][A-Z0-9_]*)=", line):
                defined.add(d)
            if re.search(r"\.\./[a-z0-9-]+/scripts/", line):
                findings.append((rel(f), n, "sibling-script", line.strip()[:100]))
            for host in FETCH.findall(line):
                if host not in ALLOWED_HOSTS:
                    findings.append((rel(f), n, "foreign-host", host))
            if PIPE_EXEC.search(line):
                findings.append((rel(f), n, "pipe-exec", line.strip()[:100]))
    for n, line in enumerate(text.splitlines(), 1):
        if LOCAL_READ.search(line):
            findings.append((rel(f), n, "local-read", line.strip()[:100]))

manifest = os.path.join(ROOT, ".claude-plugin", "plugin.json")
if "userConfig" in json.load(open(manifest, encoding="utf-8")):
    findings.append((rel(manifest), 1, "credential", "userConfig belongs in the emem-signer plugin"))
mcp = os.path.join(ROOT, ".mcp.json")
for name, srv in json.load(open(mcp, encoding="utf-8")).get("mcpServers", {}).items():
    if srv.get("type", "stdio") == "stdio" or "env" in srv:
        findings.append((rel(mcp), 1, "credential", f"{name} is a local server; it belongs in the emem-signer plugin"))

for f in sorted(glob.glob(os.path.join(PLUGINS, "*", "**", "*.py"), recursive=True)):
    for n, line in enumerate(open(f, encoding="utf-8"), 1):
        if LOCAL_READ.search(line):
            findings.append((rel(f), n, "local-read", line.strip()[:100]))

for f, n, rule, what in findings:
    print(f"{f}:{n}: {rule}: {what}")
if findings:
    sys.exit(1)
print("plugin skills: no undefined shell variables, sibling scripts, local reads, foreign hosts, piped execution or credentials in the emem plugin")
