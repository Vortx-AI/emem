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
  secret-shape    a shell variable named like a secret (TOKEN, KEY, SEED,
                  SECRET, AUTH, PASS, CRED), or a header dump (-D); the
                  review read an emem:trace token in $TOKEN as a
                  credential (review, 2026-10-20)
  inventory       a skill's text names files under scripts/ that the skill
                  does not ship; eleven skills told a reviewer their scripts
                  read local files when they had no scripts (the review held
                  field-signals as reading a credential, 2026-10-28)
  scanner-curl    a `curl ... https://` that the HOL plugin scanner cannot
                  exempt (its RISKY_SKILL_INSTRUCTION rule, high). Only a
                  shell fence whose every command is a literal GET, with
                  -fsSLI flags, one URL and no $ | ; & < > ( ) { }, passes.
                  A docs rewrite brought back 69 such curls in 19 skills on
                  2026-11-07, two days after they were removed; the scanner
                  failed on main before anything here noticed
  file-type       a file in the emem plugin that the directory refuses from
                  an unverified publisher: anything but Markdown, text,
                  JSON, YAML and images, a text file starting with #!, a
                  NUL byte, or extra files in .claude-plugin/ (the
                  directory held the plugin as LOCAL_CODE_FILE_TYPE,
                  2026-11-12; the verifiers moved to tools/verifiers/)

Exit 1 on any finding, with file and line.
"""
import glob
import json
import os
import re
import shlex
import sys
import urllib.parse

PLUGINS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "plugins")
ROOT = os.path.join(PLUGINS, "emem")
ENV_OK = {"CLAUDE_SKILL_DIR", "CLAUDE_PLUGIN_ROOT", "CLAUDE_PLUGIN_DATA", "HOME", "PWD", "PATH"}
ALLOWED_HOSTS = {"emem.dev"}
LOCAL_READ = re.compile(r"(~/\.|\$HOME\b|\$\{HOME\}|Path\.home\(|expanduser\()")
FETCH = re.compile(r"\b(?:curl|wget)\b[^\n]*?https?://([a-zA-Z0-9.-]+)")
SECRET_VAR = re.compile(r"(?:^|[\s;(])([A-Z_]*(?:TOKEN|KEY|SEED|SECRET|AUTH|PASS|CRED)[A-Z_]*)=")
HEADER_DUMP = re.compile(r"\bcurl\b[^\n]*(?:\s-D\s|--dump-header|header_json|\s-[iI]\b|--include|--head\b)")
PIPE_EXEC = re.compile(r"\|\s*(?:sudo\s+)?(?:sh|bash|zsh|python3?|node|perl|ruby)\b(?!\S)")
findings = []

# The scanner's rule, ported from hol-guard (skill_security.py, skill_curl_context.py,
# skill_command_urls.py, read 2026-11-07): continuations are joined except on comment
# lines; a `curl` with a URL on its line is risky unless it sits in a shell fence where
# every command line is a literal read-only GET.
CURL_FLAGS = {"--silent", "--show-error", "--fail", "--location", "--head", "--compressed", "--no-progress-meter"}
CMD_URL = re.compile(r"https?://[^\s`\"']+", re.I)


def _joined(text):
    out = []
    for line in text.splitlines(keepends=True):
        if not line.lstrip(" \t").startswith("#") and line.endswith("\\\n"):
            out.append(line[:-2] + " ")
        else:
            out.append(line)
    return "".join(out)


def _read_only_curl(cmd):
    if any(c in cmd for c in "$`|;&<>(){}"):
        return False
    try:
        toks = shlex.split(cmd)
    except ValueError:
        return False
    if not toks or toks[0] != "curl":
        return False
    saw, it = False, iter(toks[1:])
    for t in it:
        if t in CURL_FLAGS or re.fullmatch(r"-[fsSLI]+", t):
            continue
        if t in {"-X", "--request"}:
            if next(it, None) not in {"GET", "HEAD"}:
                return False
            continue
        u = urllib.parse.urlsplit(t)
        if saw or u.scheme not in ("http", "https") or not u.hostname or "@" in u.netloc \
                or any(c in u.path + u.query for c in "[]{}"):
            return False
        if u.query:
            k, _, v = u.query.partition("=")
            ok = (k in {"page", "limit", "per_page", "offset"} and re.fullmatch(r"[0-9]{1,4}", v)) \
                or (k == "format" and v in {"json", "xml", "text", "csv", "yaml"})
            if not ok:
                return False
        saw = True
    return saw


def _exempt_spans(text):
    spans, fence, start, off, safe, saw = [], "", 0, 0, False, False
    for line in text.splitlines(keepends=True):
        body = line.rstrip("\r\n").lstrip(" \t")
        w = len(body) - len(body.lstrip(body[:1])) if body[:1] in ("`", "~") else 0
        if not fence:
            if w >= 3:
                fence, safe, start, saw = body[:w], body[w:].strip() in {"", "bash", "sh", "shell"}, off + len(line), False
        elif w >= len(fence) and body[0] == fence[0] and not body[w:].strip():
            if safe and saw:
                spans.append((start, off))
            fence = ""
        elif line.lstrip().startswith("#"):
            safe = safe and not line.rstrip("\r\n").endswith("\\")
        elif line.strip():
            safe, saw = safe and _read_only_curl(line.strip()), True
        off += len(line)
    return spans


def scanner_curls(text):
    """Line numbers of the curls the scanner would flag."""
    text = _joined(text)
    spans = _exempt_spans(text)
    out = []
    for m in re.finditer(r"curl\s+", text, re.I):
        eol = text.find("\n", m.end())
        u = CMD_URL.search(text, m.end())
        if u and u.start() < (len(text) if eol == -1 else eol) and \
                not any(a <= m.start() and u.end() <= b for a, b in spans):
            out.append(text.count("\n", 0, m.start()) + 1)
    return out


def rel(p):
    return os.path.relpath(p, os.path.join(ROOT, "..", ".."))


GUARD_SKILL = os.path.join(PLUGINS, "..", "crates", "emem-guard", "SKILL.md")
if os.path.exists(GUARD_SKILL):
    for n in scanner_curls(open(GUARD_SKILL, encoding="utf-8").read()):
        findings.append((rel(GUARD_SKILL), n, "scanner-curl", "the plugin scanner flags this curl; describe the REST call or use a literal GET"))

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
            for v in SECRET_VAR.findall(line):
                findings.append((rel(f), n, "secret-shape", f"${v} reads like a secret; name it for what it holds"))
            if HEADER_DUMP.search(line):
                findings.append((rel(f), n, "secret-shape", "reads response headers, which is where credentials live; take what you need from a JSON body"))
            if PIPE_EXEC.search(line):
                findings.append((rel(f), n, "pipe-exec", line.strip()[:100]))
    for n in scanner_curls(text):
        findings.append((rel(f), n, "scanner-curl", "the plugin scanner flags this curl; describe the REST call or use a literal GET"))
    for n, line in enumerate(text.splitlines(), 1):
        if LOCAL_READ.search(line):
            findings.append((rel(f), n, "local-read", line.strip()[:100]))
    if "`scripts/`" in text and not os.path.isdir(os.path.join(os.path.dirname(f), "scripts")):
        n = text[: text.index("`scripts/`")].count("\n") + 1
        findings.append((rel(f), n, "inventory", "names files under scripts/ that this skill does not ship"))

manifest = os.path.join(ROOT, ".claude-plugin", "plugin.json")
if "userConfig" in json.load(open(manifest, encoding="utf-8")):
    findings.append((rel(manifest), 1, "credential", "userConfig belongs in the emem-signer plugin"))
mcp = os.path.join(ROOT, ".mcp.json")
for name, srv in json.load(open(mcp, encoding="utf-8")).get("mcpServers", {}).items():
    if srv.get("type", "stdio") == "stdio" or "env" in srv:
        findings.append((rel(mcp), 1, "credential", f"{name} is a local server; it belongs in the emem-signer plugin"))
    if "headers" in srv:
        findings.append((rel(mcp), 1, "credential", f"{name} sends headers; emem.dev takes no credential"))

# The directory takes no code from a publisher it has not verified
# (LOCAL_CODE_FILE_TYPE, 2026-11-12): only these types, an image that is the
# image its name says, no text file that starts with #!, every text file UTF-8
# with no NUL byte, and in .claude-plugin/ only plugin.json. The offline
# verifiers live in tools/verifiers/, outside the plugin.
TEXT_EXT = {".md", ".txt", ".json", ".yaml", ".yml"}
IMAGE_MAGIC = {".png": b"\x89PNG\r\n\x1a\n", ".jpg": b"\xff\xd8\xff", ".jpeg": b"\xff\xd8\xff",
               ".gif": b"GIF8", ".webp": b"RIFF", ".ico": b"\x00\x00\x01\x00"}
for dirpath, _dirs, names in os.walk(ROOT):
    for name in names:
        f = os.path.join(dirpath, name)
        inside = os.path.relpath(f, ROOT)
        ext = os.path.splitext(name)[1].lower()
        data = open(f, "rb").read()
        if any(c in inside for c in ':\\*?"<>|'):
            findings.append((rel(f), 1, "file-type", "a character in the path that not every computer keeps"))
        if inside.startswith(".claude-plugin" + os.sep) and name != "plugin.json":
            findings.append((rel(f), 1, "file-type", "only plugin.json belongs in .claude-plugin/"))
        if ext in IMAGE_MAGIC:
            if not data.startswith(IMAGE_MAGIC[ext]):
                findings.append((rel(f), 1, "file-type", f"not the {ext} image its name says"))
            continue
        if ext not in TEXT_EXT and name not in ("LICENSE", ".gitignore"):
            findings.append((rel(f), 1, "file-type", "the directory takes only Markdown, text, JSON, YAML and images from this publisher"))
            continue
        if data.startswith(b"#!") or b"\0" in data:
            findings.append((rel(f), 1, "file-type", "starts with #! or holds a NUL byte"))
        try:
            data.decode("utf-8")
        except UnicodeDecodeError:
            findings.append((rel(f), 1, "file-type", "not UTF-8"))

for f in sorted(glob.glob(os.path.join(PLUGINS, "*", "**", "*.py"), recursive=True)):
    for n, line in enumerate(open(f, encoding="utf-8"), 1):
        if LOCAL_READ.search(line):
            findings.append((rel(f), n, "local-read", line.strip()[:100]))

for f, n, rule, what in findings:
    print(f"{f}:{n}: {rule}: {what}")
if findings:
    sys.exit(1)
print("plugin skills: no undefined shell variables, sibling scripts, local reads, foreign hosts, piped execution, secret-shaped names, credentials, unshipped files, scanner-flagged curls or refused file types in the emem plugin")
