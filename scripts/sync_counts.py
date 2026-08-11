#!/usr/bin/env python3
"""Single source of truth for emem's public-facing counts.

The problem this solves: the same counts (MCP tools, REST paths, algorithms,
bands, sources, topics, ...) were hand-copied into ~10 files — README, the
homepage, the docs, and the machine-readable catalogs — and drifted apart, none
matching the responder actually running. This script makes the registries the
authority and the static mirrors follow.

How it works:
  * The computable counts are derived OFFLINE and deterministically from the repo
    registries (crates/emem-core/data/*.json) and the MCP descriptor table
    (crates/emem-mcp/src/lib.rs). These are asserted against CANON so CANON can
    never silently rot.
  * The two runtime-only counts (materializer-wired band names; documented REST
    paths) are cross-checked against a live responder's /v1/agent_card and
    /openapi.json when one is reachable (EMEM_RESPONDER, default https://emem.dev).
    Offline, they fall back to CANON with a printed notice.

Usage:
  scripts/sync_counts.py            # print canonical counts + offline cross-check
  scripts/sync_counts.py --check    # CI guard: non-zero exit if any surface drifts
  scripts/sync_counts.py --write    # rewrite the count fields in the machine twin
                                     # (web/agent.json) in place

This is where the numbers come from. Do not hand-edit counts in the JSON twins;
re-run --write. Prose surfaces (README, docs, homepage) are guarded by --check.
"""
from __future__ import annotations

import json
import os
import re
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DATA = REPO / "crates" / "emem-core" / "data"
MCP_SRC = REPO / "crates" / "emem-mcp" / "src" / "lib.rs"
REST_SRC = REPO / "crates" / "emem-api-rest" / "src" / "lib.rs"
CARGO = REPO / "Cargo.toml"

# ---------------------------------------------------------------------------
# Canonical counts. Every value here is verified below: the offline-computable
# ones are asserted against the registries; the runtime ones are checked against
# a live /v1/agent_card when reachable. CANON is the value the static twins are
# written to and the value --check enforces across every surface.
# ---------------------------------------------------------------------------
CANON = {
    # 105 = every ToolDescriptor in emem-mcp, emem_tools and the labelled
    # reasoning tier (emem_reason) included: the
    # catalog tool is core because /mcp advertises the loop rather than
    # the full surface, so one visible tool must describe the rest.
    "mcp_tools": 107,
    "mcp_core": 16,
    "mcp_extended": 91,
    "algorithms": 168,
    "rest_paths_v1": 155,            # documented /v1/* paths in OpenAPI
    "rest_paths_openapi_total": 161,  # all paths in OpenAPI
    "cube_slots": 43,
    "materializer_wired": 129,
    "source_schemes": 46,
    "topics": 27,
    "foundation_encoders": 4,
    "mcp_resources": 19,       # resources/list entries (emem-mcp 7 + emem-api-rest 12)
    "mcp_uri_templates": 8,    # resource template entries (emem-mcp 3 + emem-api-rest 5)
    "crates": 18,
    "version": "2.1.0",
}


def _load(name: str):
    return json.loads((DATA / name).read_text())


def compute_offline() -> dict:
    """Derive the deterministically-computable counts straight from the repo."""
    algorithms = _load("algorithms-v0.json")["algorithms"]
    sources = _load("sources-v0.json")["sources"]
    topics = _load("topics-v0.json")["topics"]
    bands = _load("bands-v0.json")["bands"]

    mcp = MCP_SRC.read_text()
    # Exclude the `struct ToolDescriptor {` definition; count only literals.
    tools = len(re.findall(r"(?<!struct )ToolDescriptor\s*\{", mcp))
    core = len(re.findall(r'tier:\s*"core"', mcp))
    # Count resource/template literals (each entry has one descriptor literal).
    res_block = re.search(r"RESOURCES[^=]*=\s*&\[(.*?)\n\];", mcp, re.S)
    tmpl_block = re.search(r"RESOURCE_TEMPLATES[^=]*=\s*&\[(.*?)\n\];", mcp, re.S)
    resources = len(re.findall(r"\buri:\s*", res_block.group(1))) if res_block else 0
    templates = len(re.findall(r"\buri_template:\s*", tmpl_block.group(1))) if tmpl_block else 0
    # The REST layer augments the catalog that `resources/list` actually serves:
    # mcp_full_resources() = the emem-mcp RESOURCES above plus the docs entries in
    # mcp_static_resources(), and mcp_full_resource_templates() = the emem-mcp
    # RESOURCE_TEMPLATES plus the templated URIs in mcp_resource_templates(). Count
    # both so the total matches the live responder, not just the emem-mcp crate.
    rest = REST_SRC.read_text()
    rest_res = re.search(r"fn mcp_static_resources\(\)[^{]*\{(.*?)\n\}", rest, re.S)
    rest_tmpl = re.search(r"fn mcp_resource_templates\(\)[^{]*\{(.*?)\n\}", rest, re.S)
    resources += len(re.findall(r"(?m)^\s+\(\s*$", rest_res.group(1))) if rest_res else 0
    templates += len(re.findall(r'"uriTemplate"\s*:', rest_tmpl.group(1))) if rest_tmpl else 0

    cargo = CARGO.read_text()
    members_block = re.search(r"members\s*=\s*\[(.*?)\]", cargo, re.S)
    crates = len(re.findall(r'"crates/[^"]+"', members_block.group(1))) if members_block else 0

    version = re.search(r'(?m)^version\s*=\s*"([^"]+)"', cargo)
    return {
        "algorithms": len(algorithms),
        "source_schemes": len(sources),
        "topics": len(topics),
        "cube_slots": len(bands),
        "mcp_tools": tools,
        "mcp_core": core,
        "mcp_extended": tools - core,  # derived; a stray tier tag makes counting unreliable
        "mcp_resources": resources,
        "mcp_uri_templates": templates,
        "crates": crates,
        "version": version.group(1) if version else "?",
    }


def fetch_live(responder: str) -> dict | None:
    """Pull the runtime-authoritative counts from a reachable responder."""
    try:
        with urllib.request.urlopen(f"{responder}/v1/agent_card", timeout=15) as r:
            card = json.load(r)
        with urllib.request.urlopen(f"{responder}/openapi.json", timeout=15) as r:
            paths = json.load(r).get("paths", {})
    except Exception as e:  # offline / unreachable — fall back to CANON
        print(f"  (live cross-check skipped: {responder} unreachable: {e})")
        return None
    bt = card.get("band_taxonomy", {})

    def c(node):
        return (node or {}).get("count")

    v1 = [p for p in paths if p.startswith("/v1/")]
    return {
        "mcp_tools": c(bt.get("tools")),
        "algorithms": c(bt.get("algorithms")),
        "cube_slots": c(bt.get("cube_slots")),
        "materializer_wired": c(bt.get("materializer_wired")),
        "rest_paths_v1": len(v1),
        "rest_paths_openapi_total": len(paths),
        "version": card.get("version"),
    }


def verify_security_limits(responder: str) -> list[str]:
    """SECURITY.md states runtime limits. The responder enforces them and now
    declares them at /v1/agent_card under `runtime`. Nothing compared the two,
    and they diverged badly: the doc claimed a 60 req/min rate limit against an
    actual 600, and a `Retry-After: 60` against a header that sends 1. A limit
    stated in prose and nowhere else is a limit nobody can check, and a
    *security* doc is the worst place to be wrong by 10x.
    """
    sec = REPO / "SECURITY.md"
    if not sec.exists():
        return ["SECURITY.md is missing (it is include_str!'d into the binary)"]
    try:
        with urllib.request.urlopen(f"{responder}/v1/agent_card", timeout=15) as r:
            rt = json.load(r).get("runtime", {})
    except Exception as e:
        print(f"  (security-limit cross-check skipped: {responder} unreachable: {e})")
        return []
    if not rt.get("rate_limit_per_min"):
        return []  # responder predates the self-declared limits

    body = sec.read_text()
    hits = []
    checks = [
        (f'{int(rt["rate_limit_per_min"])} req/min', "per-IP rate limit"),
        (f'{int(rt["rate_limit_burst"])} burst', "rate-limit burst"),
        (f'`Retry-After: {rt["rate_limit_retry_after_secs"]}`', "Retry-After"),
        (f'{rt["gateway_timeout_secs"]} s', "request timeout"),
        (f'{rt["body_limit_mb"]} MiB', "body cap"),
    ]
    for needle, what in checks:
        if needle not in body:
            hits.append(
                f"SECURITY.md: the live responder declares {what} as "
                f"{needle!r} but SECURITY.md does not say so"
            )
    return hits


def verify_band_classes() -> list[str]:
    """The whitepaper's §7 inventory states a per-class band census in prose.
    Nothing compared it to the band manifest, and it drifted the moment a band
    was reclassified: the doc said 22 `model_output` / 8 `deterministic_index`
    while the manifest said 23 / 7. A provenance census is exactly the kind of
    claim a reader checks first, and §16.4 criticises this species of rot by
    name, so it should not be maintained by hand.
    """
    bands = REPO / "crates/emem-core/data/bands-v0.json"
    paper = REPO / "docs/whitepaper-v2.md"
    if not bands.exists() or not paper.exists():
        return [f"missing {bands.name if not bands.exists() else paper.name}"]

    counts: dict[str, int] = {}
    for band in json.loads(bands.read_text())["bands"]:
        cls = band.get("provenance_class", "unclassified")
        counts[cls] = counts.get(cls, 0) + 1

    # The census wraps across source lines, so collapse whitespace, then scope
    # to the inventory sentence itself. Searching the whole document would let a
    # stray numeral elsewhere satisfy the check.
    prose = re.sub(r"\s+", " ", paper.read_text())
    total = sum(counts.values())
    sentence = re.search(
        rf"The {total} bands of the active ontology distribute as .*?\.", prose
    )
    if not sentence:
        return [
            f"whitepaper §7 has no '{total} bands of the active ontology' census "
            f"(bands-v0.json has {total} bands)"
        ]

    drift = []
    census = sentence.group(0)
    for cls, n in sorted(counts.items()):
        # Allow prose between the count and the class ("2 reserved slots left
        # `unclassified`"), but keep it inside the one sentence.
        if not re.search(rf"\b{n}\b[^.]{{0,40}}`{re.escape(cls)}`", census):
            drift.append(
                f"whitepaper §7 census does not say {n} `{cls}`; "
                f"bands-v0.json has {n}"
            )
    return drift


# Surfaces a DIRECTORY or a cold agent reads. server.json is what the MCP
# registry serves; ai-plugin.json is the ChatGPT manifest; llms-install.md and
# skills.md are what an agent is pointed at to install and to learn the loop.
# A stale tool count in any of them is a directory advertising a surface that
# no longer exists.
TOOL_COUNT_SURFACES = [
    "server.json",
    "web/ai-plugin.json",
    "web/skills.md",
    "llms-install.md",
    "README.md",
    "web/llms.txt",
    "docs/agents.md",
]

# Every way this repo has ever phrased a tool count. Each pattern captures the
# number and names which CANON key it must equal.
_TOOL_CLAIMS = [
    (r"(\d+)[- ]tool core", "mcp_core"),
    (r"(\d+) core tools", "mcp_core"),
    (r"\((\d+) core,", "mcp_core"),
    (r"(\d+) MCP tools", "mcp_tools"),
    (r"(\d+) tools in total", "mcp_tools"),
    (r"all (\d+)\b", "mcp_tools"),
    (r"any of the (\d+) by name", "mcp_tools"),
    (r"(\d+) tools is callable", "mcp_tools"),
    (r"(\d+) extended\)", "mcp_extended"),
    (r"(\d+) extended,", "mcp_extended"),
]


# Documents that record what a count WAS, on purpose. A changelog that says
# "the counts moved to 94 tools" is correct forever, and rewriting it would
# destroy the only record of when it moved.
#
# These are SUBSTRINGS of a file NAME, not paths, so an entry can exempt a file
# in a directory nobody was thinking about, and an entry can name a real file
# that no scan ever reaches. Both happened. Every entry was re-checked on
# 2026-08-10 by asking two questions: does it match a file any scan here
# actually reads, and how many claims does it suppress today.
#
#   upstream-readme.md   REMOVED. It is a real file, vendored from the public
#                        MCP directory, and its counts really are other
#                        people's. It lives at the repo root, which none of
#                        the three scans below reach: docs/*.md, docs/**/*.md
#                        and web/*.html. The entry exempted nothing.
#   AGENT_HANDOFF        REMOVED. Same: the handoff notes live at the repo root
#                        and under .well-known/agent-notes/, neither scanned.
#   roadmap.md           REMOVED. The stated reason is "records what a count
#                        WAS", and a roadmap does not. Measured before removing:
#                        docs/roadmap.md fires zero claims today, so this is the
#                        gate widening to cover a document that should never
#                        have been outside it. A roadmap saying "107 tools
#                        today, 150 planned" is a live claim about the
#                        responder and gets checked like any other.
#   audit-repro          KEPT, and it suppresses 0 claims today. A dated audit
#                        reproduction is a record of what a run observed on
#                        2026-08-02; it will fire the moment a count moves, and
#                        correcting it would destroy the reproduction.
#   CHANGELOG.md         KEPT: every heading is a past release.
#   collaboration-log.md KEPT, suppresses 9. It reproduces other agents' SIGNED
#                        notes verbatim; editing them breaks the signatures.
#   whitepaper-v1.md     KEPT, suppresses 5, and .html KEPT, suppresses 7. The
#                        DOI-cited version, archived exactly as it shipped.
#   channel.html         KEPT, suppresses 1. Generated, and it embeds signed
#                        note bodies verbatim for the same reason as the log.
#   whitepaper-v2.html   KEPT, suppresses 4. Generated from
#                        docs/whitepaper-v2.md, which IS checked. A rendered
#                        table cell has no leading pipe, so the HTML cannot tell
#                        the quoted-v1 column from a live claim; the source can,
#                        and the source is the one to fix.
COUNT_HISTORY = (
    "CHANGELOG.md", "collaboration-log.md",
    "whitepaper-v1.md", "whitepaper-v1.html", "channel.html",
    "audit-repro",
    "whitepaper-v2.html",
)

# Prose that asserts a canonical quantity as a fact about this responder. Each
# pattern names one CANON key and one way the docs phrase it.
PROSE_CLAIMS = (
    ("mcp_tools",    r"(\d{2,4})\s+MCP tools\b"),
    # The REST surface, in every phrasing the repo actually used. None of these
    # existed until 2026-08-10 and all four were wrong at once: web/llms.txt
    # told agents "133 documented /v1 paths" and docs/agents.md "114 paths
    # under /v1/* (133 documented; 138 total in /openapi.json)" against a
    # responder serving 155 and 161. The tool and algorithm numbers in the same
    # sentences were current, which is the tell: whoever updated them updated
    # only what a pattern here was watching.
    ("rest_paths_v1", r"(\d{2,4})\s+(?:documented\s+)?/v1\s+paths"),
    ("rest_paths_v1", r"(\d{2,4})\s+paths\s+under\s*/v1"),
    ("rest_paths_v1", r"(\d{2,4})\s+documented\s+REST\s+paths\s+under\s*/v1"),
    # "144 documented, 138 under /v1/*": the first number is the whole OpenAPI
    # document, the second is the /v1 subset, so they bind to different keys.
    ("rest_paths_openapi_total", r"(\d{2,4})\s+documented,\s*\d{2,4}\s+under\s*/v1"),
    ("rest_paths_v1", r"\d{2,4}\s+documented,\s*(\d{2,4})\s+under\s*/v1"),
    ("rest_paths_v1", r"\((\d{2,4})\s+documented;"),
    ("rest_paths_openapi_total", r"(\d{2,4})\s+total\s+in\s*/openapi\.json"),
    # The MCP resource catalog. resources/list serves 19; agents.md and the
    # structured object in web/agent.json both still said 18.
    ("mcp_resources", r"(\d{1,3})\s+static\s+MCP\s+resources"),
    ("mcp_resources", r"(\d{1,3})\s+static\s*\+\s*\d{1,3}\s+URI templates"),
    ("mcp_uri_templates", r"\d{1,3}\s+static\s*\+\s*(\d{1,3})\s+URI templates"),
    # "168 algorithms" was watched; "162 composition recipes", the phrasing
    # docs/registries.md and the registry packs use, was not.
    ("algorithms",   r"(\d{2,4})\s+(?:named\s+)?composition (?:recipes|algorithms)"),
    ("materializer_wired", r"(\d{2,4})\s+materializer-wired"),
    # Both the parenthesised form, "107 tools (16 core, 91 extended)", and the
    # colon form, "107 tools: 16 core, 91 extended". ARCHITECTURE_NOTES.md used
    # the colon and drifted to 104/15/89 unnoticed, because requiring "(" meant
    # this pattern was still remembering a phrasing, which is the exact failure
    # the docstring below claims to have fixed.
    ("mcp_tools",    r"\b(\d{2,4})\s+tools\s*[:(]\s*(\d{1,3})\s*core"),
    ("mcp_core",     r"the\s+(\d{1,3})\s+tools of the core loop"),
    ("mcp_core",     r"(\d{1,3})[- ]tool core loop"),
    # The tier sentence, in the four shapes the docs that explain /mcp vs
    # /mcp/full actually use. None of these was watched, and all four were
    # wrong at once on 2026-08-10: docs/intro.md said "the 15 core tools" and
    # "all 104 (15 core, 89 extended)", docs/mcp-directory.md said "the 15 core
    # tools", "the full 105" and "all 102" twice, docs/integrations.md said "the
    # 15 core tools" and "all 102", and docs/ARCHITECTURE_NOTES.md said "all
    # 102", against a responder answering 107 and 16. Every one of those files
    # is scanned by this function and always was; the phrase list simply had no
    # pattern for the way they phrase it, which is the same invisible exemption
    # the colon form and the dotted version were.
    ("mcp_core",     r"(\d{1,3})\s+core tools\b"),
    ("mcp_tools",    r"(\d{2,4})\s*\(\d{1,3}\s+core,\s*\d{1,3}\s+extended\)"),
    ("mcp_core",     r"\d{2,4}\s*\((\d{1,3})\s+core,\s*\d{1,3}\s+extended\)"),
    ("mcp_extended", r"\d{2,4}\s*\(\d{1,3}\s+core,\s*(\d{1,3})\s+extended\)"),
    # "advertises all 104", "advertise all 102", "dispatches all 104 by name",
    # "every one of the 105 stays callable". A bare `all (\d+)` is not safe
    # here: "the full 32" is the BLAKE3 digest length in the same documents, so
    # each pattern carries the verb that makes it a tool count.
    ("mcp_tools",    r"advertises?\s+all\s+(\d{2,4})\b"),
    ("mcp_tools",    r"dispatch(?:es)?\s+all\s+(\d{2,4})\b"),
    ("mcp_tools",    r"every one of the\s+(\d{2,4})\b"),
    ("mcp_tools",    r"for all\s+(\d{2,4})\b"),
    ("algorithms",   r"(\d{2,4})\s+algorithms\b"),
    ("topics",       r"(\d{1,3})\s+topics\b"),
    ("rest_paths_openapi_total", r"\((\d{2,4})\s+total(?:\s+in OpenAPI)?\)"),
)


# A doc STATING the current version, as opposed to referring to an old one.
# Deliberately narrow: "the preimage last changed in 2.0.0" and "every release
# from v2.0.0 on" are history and must not match, so each pattern requires the
# word Version/version to introduce the number rather than merely sit near it.
VERSION_CLAIMS = (
    r"\|\s*Version\s*\|\s*(\d+\.\d+\.\d+)\s*\|",      # a spec-table row
    r"\*\*Version:\*\*\s*(\d+\.\d+\.\d+)",            # a doc header field
    r"(?:emem is at version|(?:^|[.!?]\s)Version)\s+(\d+\.\d+\.\d+)",
)


def verify_package_versions() -> list[str]:
    """Every packaging surface has to carry CANON's version.

    2.1.0 shipped to PyPI, Smithery and the MCP Registry and failed on npm,
    because `sdks/emem-ts/src/version.ts` is a hand-maintained twin of
    `package.json` and only one of them got bumped. The npm workflow caught it,
    but at publish time, after three other ecosystems had already released. The
    file's own docstring says "bump both together"; a docstring is not a gate.

    The Python SDKs are absent on purpose: they read __version__ from installed
    distribution metadata, so they cannot drift from pyproject.toml.
    """
    want = CANON["version"]
    checks = [
        ("sdks/emem-ts/package.json", r'"version"\s*:\s*"([^"]+)"'),
        ("sdks/emem-ts/src/version.ts", r'VERSION\s*=\s*"([^"]+)"'),
        ("sdks/emem-py/pyproject.toml", r'(?m)^version\s*=\s*"([^"]+)"'),
        ("sdks/emem-langmem/pyproject.toml", r'(?m)^version\s*=\s*"([^"]+)"'),
        ("server.json", r'"version"\s*:\s*"([^"]+)"'),
        ("web/agent.json", r'"version"\s*:\s*"([^"]+)"'),
    ]
    hits = []
    for rel, pat in checks:
        path = REPO / rel
        if not path.exists():
            hits.append(f"{rel}: missing (it is a packaging surface)")
            continue
        m = re.search(pat, path.read_text(encoding="utf-8", errors="replace"))
        if not m:
            hits.append(f"{rel}: no version field matched {pat!r}")
        elif m.group(1) != want:
            hits.append(f"{rel}: packages version {m.group(1)}, CANON says {want}")
    return hits


def verify_prose_version() -> list[str]:
    """The version stated in prose has to be the version the responder serves.

    `--check` already compared CANON to /v1/agent_card, and every count to the
    docs, but nothing compared the VERSION to the docs. So `docs/agents.md`
    carried `| Version | 2.0.0 |` through a release and the gate stayed green:
    the count patterns parse with int() and skipped a dotted string entirely.
    Same hole as the tool-count pattern, one type over.
    """
    want = CANON["version"]
    hits: list[str] = []
    targets = sorted(REPO.glob("docs/**/*.md")) + sorted(REPO.glob("web/*.html")) + [
        REPO / "README.md", REPO / "AGENTS.md", REPO / "CHANGELOG.md",
    ]
    for path in targets:
        if not path.exists() or any(h in path.name for h in COUNT_HISTORY):
            continue
        if path.name == "CHANGELOG.md":
            continue  # every heading in it is a record of a past release
        body = re.sub(r"<script.*?</script>|<style.*?</style>", "",
                      path.read_text(encoding="utf-8", errors="replace"), flags=re.S)
        flat = re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", body))
        for pat in VERSION_CLAIMS:
            for m in re.finditer(pat, flat):
                if m.group(1) != want:
                    ctx = flat[max(0, m.start() - 30):m.end() + 15].strip()
                    hits.append(
                        f"{path.relative_to(REPO)}: states version {m.group(1)}, "
                        f"the responder serves {want} ({ctx[:70]!r})")
    return hits


def verify_prose_counts() -> list[str]:
    """A count stated as prose has to match the responder it describes.

    sync_counts already rewrites the machine surfaces, and verify_tool_counts
    asserts the tool number on seven of them. Neither looked at the documents
    that carry the most detailed claims. /whitepaper, the flagship document,
    served "102 MCP tools (15 core, 87 extended)" and "127 total in OpenAPI"
    against a responder answering 107, 16, 91 and 160, and this script reported
    no drift the whole time, because the surface list it checks and the phrase
    list it looks for were both written by hand and neither included it.

    So this asserts the number rather than remembering a phrasing. Documents
    that record history on purpose are excluded by name above.
    """
    hits: list[str] = []
    # web/llms.txt is here because it is the most agent-facing prose in the
    # repo and it was guarded by nothing but a burn-down list of phrases that
    # had already gone stale. It served "133 documented /v1 paths" against a
    # responder answering 155, in the same sentence as a correct tool count.
    named = [
        REPO / "README.md", REPO / "AGENTS.md", REPO / "ARCHITECTURE_NOTES.md",
        REPO / "docs" / "ARCHITECTURE_NOTES.md", REPO / "web" / "llms.txt",
    ]
    targets = sorted(REPO.glob("docs/*.md")) + sorted(REPO.glob("web/*.html")) + named
    # The RENDER of those same docs. `docs/book/` is mdbook output, it is
    # gitignored, and `crates/emem-api-rest` bakes it with include_dir!, so
    # https://emem.dev/docs serves whatever happened to be on the builder's disk
    # when the binary was compiled. Nothing rebuilt it and nothing checked it,
    # so /docs/agents.html served "163 algorithms" and "118 documented, 114
    # under /v1/*" for weeks while docs/agents.md next to it said 168 and every
    # gate was green: doc_lint skips docs/book/ as generated output, and this
    # function only ever globbed one directory level.
    #
    # Only the renders of pages already scanned above are added, so this checks
    # freshness of the render and does not quietly widen what counts as a claim.
    # Absent (a clean checkout, or CI, where the directory does not exist) it
    # adds nothing rather than reporting a pass it did not perform.
    # A source excluded as history stays excluded in its render: the exclusion
    # is a fact about the CONTENT, and COUNT_HISTORY matches file names, so
    # `collaboration-log.md` would otherwise come straight back in as
    # `collaboration-log.html` carrying every count it ever recorded.
    book = REPO / "docs" / "book"
    if book.is_dir():
        targets += [b for p in sorted(REPO.glob("docs/*.md"))
                    if not any(h in p.name for h in COUNT_HISTORY)
                    and (b := book / f"{p.stem}.html").exists()]
    # A named target that does not exist is a check that silently stopped
    # running. `ARCHITECTURE_NOTES.md` was listed at the repo root, where it has
    # not lived for some time, and the PROSE_CLAIMS comment above still credits
    # that entry with catching its drift; the docs/*.md glob was doing the work.
    for path in named:
        if not path.exists() and path.name != "ARCHITECTURE_NOTES.md":
            hits.append(f"{path.relative_to(REPO)}: named in verify_prose_counts "
                        f"but does not exist (renamed or deleted?)")
    if not (REPO / "ARCHITECTURE_NOTES.md").exists() and not (
            REPO / "docs" / "ARCHITECTURE_NOTES.md").exists():
        hits.append("ARCHITECTURE_NOTES.md: named in verify_prose_counts at both "
                    "the repo root and docs/ and present at neither")
    for path in targets:
        if not path.exists() or any(h in path.name for h in COUNT_HISTORY):
            continue
        body = path.read_text(encoding="utf-8", errors="replace")
        body = re.sub(r"<script.*?</script>|<style.*?</style>", "", body, flags=re.S)
        # Backticks come out with the tags. A markdown code span is invisible to
        # a reader of the rendered page, so it must be invisible to the claim
        # patterns too: "138 total in `/openapi.json`" is the same assertion as
        # "138 total in /openapi.json", and requiring the pattern to know which
        # one the author typed is how a phrase list rots.
        flat = re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", body).replace("`", ""))
        for key, pat in PROSE_CLAIMS:
            want = CANON.get(key)
            if want is None:
                continue
            for m in re.finditer(pat, flat, re.I):
                # A sentence that explicitly frames the number as past tense is
                # a record, not a claim: "70 of the then-102 tools".
                lead = flat[max(0, m.start() - 24):m.start()].lower()
                if "then-" in lead or "was " in lead or "were " in lead:
                    continue
                # The whitepaper carries a "what v1 said -> what is true now"
                # table. The left cell QUOTES the superseded spec and is
                # correct forever; only the right cell is a claim about this
                # responder. A quoted cell opens with a section citation, so
                # find the cell this match sits in and skip it if it does.
                cell = flat.rfind("|", 0, m.start())
                if cell != -1 and re.match(r"\|\s*§", flat[cell:cell + 6]):
                    continue
                got = int(m.group(1))
                if got != want:
                    ctx = flat[max(0, m.start() - 40):m.end() + 20].strip()
                    hits.append(
                        f"{path.relative_to(REPO)}: says {got} for {key}, "
                        f"the responder answers {want} ({ctx[:70]!r})")
    return hits


# Patterns that PROSE_CLAIMS carries and the script scan below does not.
#
# `N core tools` matches one string this task could not reach: the JSON-LD FAQ
# in web/index.html answers "How do I connect Claude, Cursor, ...?" with
# "tools/list advertises the 15 core tools ... have all 105 registered up
# front", against a responder answering 16 and 107. That is real drift, in a
# block Google renders as a rich result. It is excluded here only because
# web/ was outside the write scope of the change that added this pattern
# (2026-08-10), not because the claim is acceptable.
#
# Delete this set the moment that FAQ answer is corrected. Two numbers, one
# line, `<script type=application/ld+json>` near the top of web/index.html:
# "the 15 core tools" -> 16, "all 105 registered" -> 107. web/*.html is
# include_str!-baked, so it needs a rebuild and a restart to reach the site.
PROSE_ONLY_CLAIMS = frozenset([r"(\d{1,3})\s+core tools\b"])

# The trailing-number form. PROSE_CLAIMS reads "168 algorithms"; the arcade
# writes "archetype algorithms - 168 content-addressed recipes", with the noun
# first, which none of those patterns can match.
SCRIPT_PROSE_CLAIMS = tuple(
    (k, p) for k, p in PROSE_CLAIMS if p not in PROSE_ONLY_CLAIMS
) + (
    ("algorithms", r"(\d{2,4})\s+content-addressed recipes\b"),
    ("algorithms", r"\balgorithms\s*[—–:-]\s*(\d{2,4})\b"),
)


def _script_prose_hits(claims) -> list[str]:
    """verify_script_prose_counts, over an arbitrary claim table.

    Split out so PROSE_ONLY_CLAIMS can be asked the one question that keeps an
    exclusion honest: would removing it change anything today.
    """
    hits: list[str] = []
    lit = re.compile(r"'((?:[^'\\\n]|\\.)*)'|\"((?:[^\"\\\n]|\\.)*)\"|`([^`\\]*)`")
    for path in sorted(REPO.glob("web/*.html")):
        if any(h in path.name for h in COUNT_HISTORY):
            continue
        body = path.read_text(encoding="utf-8", errors="replace")
        for block in re.findall(r"<script(?![^>]*\bsrc=)[^>]*>(.*?)</script>",
                                body, re.S):
            for m in lit.finditer(block):
                s = m.group(1) or m.group(2) or m.group(3) or ""
                if s.count(" ") < 6:
                    continue
                # Two patterns can reach the same number from opposite sides;
                # keyed on where the number sits, that is one finding.
                said: set[tuple[str, int]] = set()
                for key, pat in claims:
                    want = CANON.get(key)
                    if want is None:
                        continue
                    for c in re.finditer(pat, s, re.I):
                        lead = s[max(0, c.start() - 24):c.start()].lower()
                        if "then-" in lead or "was " in lead or "were " in lead:
                            continue
                        got = int(c.group(1))
                        if got == want or (key, c.start(1)) in said:
                            continue
                        said.add((key, c.start(1)))
                        ctx = s[max(0, c.start() - 40):c.end() + 20].strip()
                        hits.append(
                            f"{path.relative_to(REPO)}: script prose says "
                            f"{got} for {key}, the responder answers {want} "
                            f"({ctx[:70]!r})")
    return hits


def verify_script_prose_counts() -> list[str]:
    """The same assertion, for prose that lives inside a <script> block.

    verify_prose_counts strips <script> before it scans, which is right for a
    page whose script is machinery. web/arcade.html is not that page: nearly
    all of its agent-facing prose is JS string literals, so the largest such
    surface in the repo was guarded by nothing at all. It served "163
    content-addressed recipes" against a responder answering 168, and a stale
    "15 of 105 tools", and this script reported no drift the whole time.

    Only literals shaped like prose are read: six spaces or more. That is what
    separates a sentence from minified three.js, which shares the same script
    tag and would otherwise produce a wall of noise.
    """
    return _script_prose_hits(SCRIPT_PROSE_CLAIMS)


def verify_tool_counts() -> list[str]:
    """Assert the CURRENT numbers, rather than listing yesterday's stale ones.

    The burn-down table below (`STALE`) only catches phrases somebody thought
    to add after they went stale, so it always lags by exactly one change. On
    2026-08-06 the tool surface moved 105 -> 107 and core 15 -> 16, and four
    machine surfaces kept the old numbers while `--check` reported no drift:
    server.json told every directory the server had 15 core tools, and
    web/skills.md still said 102. This reads the numbers out and compares them
    to CANON, so a surface cannot drift silently even the first time.
    """
    problems: list[str] = []
    for rel in TOOL_COUNT_SURFACES:
        f = REPO / rel
        if not f.exists():
            problems.append(f"{rel}: missing")
            continue
        text = f.read_text(encoding="utf-8")
        for pattern, key in _TOOL_CLAIMS:
            for m in re.finditer(pattern, text):
                got = int(m.group(1))
                want = CANON[key]
                if got != want:
                    line = text[: m.start()].count("\n") + 1
                    problems.append(
                        f"{rel}:{line}: says {m.group(0)!r} but {key} is {want}"
                    )
    return problems


def verify_canon() -> list[str]:
    """Assert CANON matches what the repo (and, if reachable, the responder) say."""
    drift = []
    off = compute_offline()
    for k, v in off.items():
        if k in CANON and CANON[k] != v:
            drift.append(f"CANON[{k}]={CANON[k]} but repo computes {v}")

    responder = os.environ.get("EMEM_RESPONDER", "https://emem.dev")
    live = fetch_live(responder)
    if live:
        for k, v in live.items():
            if v is not None and k in CANON and CANON[k] != v:
                drift.append(f"CANON[{k}]={CANON[k]} but live /v1/agent_card says {v}")
    drift.extend(verify_security_limits(responder))
    drift.extend(verify_band_classes())
    # The surfaces a directory reads. Checked by reading their numbers out and
    # comparing to CANON, not by listing phrases that have already gone stale.
    drift.extend(verify_tool_counts())
    drift.extend(verify_prose_counts())
    drift.extend(verify_script_prose_counts())
    drift.extend(verify_prose_version())
    drift.extend(verify_package_versions())
    drift.extend(verify_count_history())
    drift.extend(verify_prose_only_claims())
    return drift


# ---------------------------------------------------------------------------
# Patch the machine twins. Field-name-targeted regex keeps formatting/diff small.
# ---------------------------------------------------------------------------
def _set_num(text: str, key: str, val: int) -> str:
    return re.sub(rf'("{re.escape(key)}"\s*:\s*)\d+', rf"\g<1>{val}", text)


def write_agent(check_only: bool) -> list[str]:
    f = REPO / "web" / "agent.json"
    text = f.read_text()
    repl = {
        "primitive_count": CANON["mcp_tools"],
        "algorithm_count": CANON["algorithms"],
        "band_cube_slot_count": CANON["cube_slots"],
        "materializer_wired_band_names": CANON["materializer_wired"],
        "rest_paths_in_openapi": CANON["rest_paths_openapi_total"],
        "rest_paths_v1_in_openapi": CANON["rest_paths_v1"],
        "source_scheme_count": CANON["source_schemes"],
        "topic_count_declared": CANON["topics"],
    }
    new = text
    for k, v in repl.items():
        new = _set_num(new, k, v)
    # Free-text count tokens in the tagline + description strings. These are
    # value-agnostic regexes (match whatever number is there now, set it to
    # canonical) so the description can't silently rot the way the old one-shot
    # string migrations did.
    #
    # That claim was not enforced, and it failed: one pattern carried a
    # literal `10 core`, so when core moved to 13 it matched nothing and
    # re.sub returned the input unchanged, leaving a stale description
    # beside a freshly-written number. A rewrite that matches nothing is
    # always a bug here, either the surface changed shape or the pattern
    # rotted, so it is now loud rather than silent.
    misses: list[str] = []

    def _sub(pattern: str, repl: str, s: str) -> str:
        out, n = re.subn(pattern, repl, s)
        if n == 0:
            misses.append(pattern)
        return out

    # The core count is canonical on both sides. It used to be literal
    # `10` in the pattern AND the replacement, which is from the 81-tool
    # era: once core moved to 13 the pattern stopped matching, `_sub`
    # rewrote nothing, and agent.json kept a stale description next to a
    # freshly-written `primitive_count`. A --write that silently no-ops is
    # worse than no --write, so every count here is a capture, never a
    # literal.
    new = _sub(r"\d+ MCP tools \(\d+ core, \d+ extended\)",
               f'{CANON["mcp_tools"]} MCP tools ({CANON["mcp_core"]} core, {CANON["mcp_extended"]} extended)', new)
    # The structured mcp_tools object: it sat at 102/87 while line 6 of the
    # same file said 104/89, because only the prose regexes were maintained.
    new = _sub(r'"mcp_tools":\s*\{\s*"core":\s*\d+,\s*"extended":\s*\d+,\s*"total":\s*\d+\s*\}',
               f'"mcp_tools": {{\n    "core": {CANON["mcp_core"]},\n    "extended": {CANON["mcp_extended"]},\n    "total": {CANON["mcp_tools"]}\n  }}', new)
    new = _sub(r"\d+ static MCP resources \+ \d+ URI templates",
               f'{CANON["mcp_resources"]} static MCP resources + {CANON["mcp_uri_templates"]} URI templates', new)
    # The structured mcp_resources object, for exactly the reason the mcp_tools
    # object above got one: only the prose regex was maintained, so line 6 of
    # this file said "19 static MCP resources + 8 URI templates" while the
    # object a machine actually parses, at the bottom of the same file, said
    # `"static": 18`. The live responder's resources/list serves 19.
    new = _sub(r'"mcp_resources":\s*\{\s*"static":\s*\d+,\s*"uri_templates":\s*\d+\s*\}',
               f'"mcp_resources": {{\n    "static": {CANON["mcp_resources"]},\n'
               f'    "uri_templates": {CANON["mcp_uri_templates"]}\n  }}', new)
    new = _sub(r"\d+ algorithms in a content-addressed registry",
               f'{CANON["algorithms"]} algorithms in a content-addressed registry', new)
    new = _sub(r"\d+ materializer-wired band names across \d+ cube slots",
               f'{CANON["materializer_wired"]} materializer-wired band names across {CANON["cube_slots"]} cube slots', new)
    new = _sub(r"\d+ cube slots; \d+ \(slot",
               f'{CANON["cube_slots"]} cube slots; {CANON["materializer_wired"]} (slot', new)
    out = _apply(f, text, new, check_only)
    if misses:
        rel = f.relative_to(REPO)
        out += [
            f"{rel}: pattern matched nothing, so nothing was rewritten: {p}" for p in misses
        ]
    return out


def _apply(f: Path, old: str, new: str, check_only: bool) -> list[str]:
    if old == new:
        return []
    rel = f.relative_to(REPO)
    if check_only:
        return [f"{rel}: count fields drift from canonical (run --write)"]
    f.write_text(new)
    return [f"{rel}: rewritten to canonical counts"]


# Prose surfaces: token scan for known-stale count phrases. Flags only; prose is
# fixed by hand (reviewed for voice), this just keeps it from silently rotting.
STALE_PHRASES = {
    "README.md": ["75 MCP tools", "87 documented REST", "118 live materializer",
                  "118 materializer-wired", "across 35 cube", "14 workspace crates",
                  "124 paths", "(102 tools)"],
    "web/index.html": ["75 MCP tools", "87 documented", "87 REST", "87 paths", "118 materializer", "35 cube slots",
                       "all 91", "other 77", "all 102", "all 104", "124 paths"],
    "web/how-it-works.html": ["75 MCP tools", "118 materializer", "35 cube slots", "41 cube"],
    "web/solutions.html": ["75 MCP tools", "118 materializer", "35 cube slots"],
    "web/reference.html": ["75 MCP tools", "87 documented", "118 materializer", "35 cube slots",
                           "108 documented", "91 (14 core"],
    "docs/intro.md": ["75 MCP", "87 ", "118 materializer", "35 cube", "41 cube"],
    # v2 only. whitepaper-v1.md is deliberately absent from this table: it
    # is archived unedited as the DOI-cited version, so its stale counts are
    # a historical record, not drift to sweep. v2 §16 lists what it got wrong.
    "docs/whitepaper-v2.md": ["Forty-one band cube", "41 cube", "35 cube",
                              "118 materializer", "75 MCP", "All 91"],
    # The canonical merged paper carries the same counts as v2 and rots the
    # same way; watch it with the same phrase list.
    "docs/whitepaper.md": ["Forty-one band cube", "41 cube", "35 cube",
                           "118 materializer", "75 MCP", "All 91"],
    "docs/registries.md": ["160 composition", "| 111 ", "| 160 ",
                           "118 materializer", "(86)", "nine manifests",
                           "not yet served by"],
    "AGENTS.md": ["version 1.0.0"],
    "web/ai-plugin.json": ["91 MCP tools", "77 extended", "102 MCP tools", "87 extended", "104 MCP tools", "89 extended"],
    "claude-skills/emem-locate-and-recall/SKILL.md": ["35 bands"],
    "web/skills.md": ["75 tools", "71 paths", "87 paths"],
    "web/llms.txt": ["75 MCP", "71 paths", "87 paths", "118 materializer",
                     "91 MCP", "108 documented", "160 algorithms",
                     "160 composition", "124 documented", "162 composition",
                     "all 104", "104 MCP tools"],
    "docs/agents.md": ["160 composition", "91 MCP", "77 extended",
                       "162 composition", "all 102", "all 104", "Version 1.2.1",
                       "129 total in"],
    "huggingface-space/README.md": ["70 MCP", "159 algorithms", "68-recipe",
                                    "42 bands"],
    # The registry-facing artifacts that rotted a full generation unseen
    # because nothing watched them at all.
    "server.json": ["14-tool", "all 94", "205 KB", "any of the 94", "all 104", "any of the 104"],
    "docs/mcp-directory.md": ["1.2.1", "the full 102", "one of the 94"],
    "web/a2a.html": ["Four entries", "all 102"],
    # channel.html itself stays in COUNT_HISTORY below, and that exemption is
    # correct: the page embeds signed note bodies verbatim, and an agent's note
    # titled "the headline number is too high, it is 6 of 9, not 7 of 10" is a
    # true record of what was written. Editing a signed body to satisfy a count
    # gate would falsify the thing the page exists to publish.
    #
    # The generator's OWN prose is a different matter. Nothing signed it, and it
    # is where the drift actually happened: the social cards shipped "7 of 10
    # corrections" as a string literal while the panel underneath computed 10 of
    # 13 from the rows, so every share of this page quoted a figure the page
    # contradicted. Both numbers are derived now. These pins are what stops the
    # next person reaching for a literal because it is one line shorter.
    "scripts/build_channel.py": ["Three AI agents", "three AI agents",
                                 "all three run on the same machine",
                                 "Three agents agreeing",
                                 "7 of 10 corrections were made",
                                 "A week of agents"],
}


def verify_prose_only_claims() -> list[str]:
    """PROSE_ONLY_CLAIMS has to still be buying something, and still be real.

    This is the only exemption in this file that keeps a pattern away from a
    surface on purpose, so it gets the treatment every other exemption here
    gets: it fails if it names a pattern PROSE_CLAIMS does not carry, and it
    fails once the drift it was written for is gone. An exemption that stops
    exempting anything is worse than none, because the next reader takes it as
    evidence the surface was considered and found fine.
    """
    hits = []
    for pat in sorted(PROSE_ONLY_CLAIMS):
        if not any(p == pat for _, p in PROSE_CLAIMS):
            hits.append(f"PROSE_ONLY_CLAIMS holds {pat!r}, which PROSE_CLAIMS does "
                        f"not carry; the exclusion excludes nothing.")
    still_needed = [(k, p) for k, p in PROSE_CLAIMS if p in PROSE_ONLY_CLAIMS]
    if still_needed and not _script_prose_hits(still_needed):
        hits.append("PROSE_ONLY_CLAIMS no longer suppresses any finding: the "
                    "script-prose surfaces it was written around are clean. "
                    "Delete the set and let the patterns run everywhere.")
    return hits


def verify_count_history() -> list[str]:
    """A history exemption that matches no scanned file is not an exemption.

    COUNT_HISTORY is consulted with `substring in path.name` against three
    scans: docs/*.md, docs/**/*.md and web/*.html, plus the docs/book renders.
    An entry that matches nothing in any of them suppresses no claim and is
    read by the next person as a decision about a file this gate covers.
    `upstream-readme.md` and `AGENT_HANDOFF` were both like that: real files,
    at the repo root, that no scan here has ever reached.
    """
    scanned = {p.name for p in REPO.glob("docs/*.md")}
    scanned |= {p.name for p in REPO.glob("docs/**/*.md")}
    scanned |= {p.name for p in REPO.glob("web/*.html")}
    scanned |= {"README.md", "AGENTS.md", "CHANGELOG.md"}
    book = REPO / "docs" / "book"
    if book.is_dir():
        scanned |= {p.name for p in book.glob("*.html")}
    return [f"COUNT_HISTORY entry {h!r} matches no file this script scans; it "
            f"exempts nothing, so drop it or fix the name."
            for h in COUNT_HISTORY if not any(h in n for n in scanned)]


def scan_prose() -> list[str]:
    hits = []
    for rel, phrases in STALE_PHRASES.items():
        p = REPO / rel
        if not p.exists():
            # A rename must not silently disable a check. This table is the
            # only thing catching these phrases, and a `continue` here is how
            # a file gets renamed and its drift guard quietly stops running.
            hits.append(f"{rel}: listed in STALE_PHRASES but does not exist "
                        f"(renamed or deleted? update the table)")
            continue
        body = p.read_text()
        for ph in phrases:
            if ph in body:
                hits.append(f"{rel}: stale count phrase present: {ph!r}")
    return hits


# Byte figures for the two tools/list surfaces, as published in prose.
#
# These are not counts, so `PROSE_CLAIMS` cannot see them, and they drifted
# across SIX surfaces before anyone noticed: README, agents.md, intro.md,
# mcp-directory.md, integrations.md and the whitepaper variously claimed
# 39/51 KB core and 204/210/266 KB full, against 66 and 288 measured on the
# wire 2026-08-11. A descriptor edit moves them, which is exactly why a human
# never re-measures: nothing was watching, and the numbers read as facts.
#
# Tolerance is wide on purpose. The claim being defended is "core is a
# fraction of full", not a byte count, and a check that reddens because
# somebody improved a tool description gets switched off. It fails only when
# a published figure is off by more than half, which is what "stale" looks
# like here.
MCP_BYTES_TOLERANCE = 0.5
MCP_BYTES_CLAIMS = [
    # (file, regex capturing the KB figure, which surface it describes)
    ("README.md", r"core loop in one page, about (\d+) KB of context", "core"),
    ("README.md", r"all 107 descriptors costs about (\d+) KB", "full"),
    ("docs/agents.md", r"in one page \(about\s+(\d+) KB of descriptors\)", "core"),
    ("docs/agents.md", r"advertises all 107 \(about (\d+) KB over 7", "full"),
    ("docs/intro.md", r"carries about (\d+) KB of descriptors instead of (?:\d+) KB", "core"),
    ("docs/intro.md", r"instead of (\d+) KB", "full"),
    ("docs/mcp-directory.md", r"in a single page \(about (\d+) KB\)", "core"),
    ("docs/mcp-directory.md", r"the full 107 cost about (\d+) KB", "full"),
    ("docs/integrations.md", r"registers about (\d+) KB of descriptors", "core"),
    ("docs/integrations.md", r"rather than about (\d+) KB for all 107", "full"),
    ("docs/whitepaper.md", r"All 107 cost about (\d+) KB", "full"),
    ("docs/whitepaper.md", r"core loop in a single page, about (\d+) KB", "core"),
]


def measure_mcp_list_bytes(responder: str) -> dict | None:
    """Whole HTTP body of a no-cursor tools/list on /mcp, and of every page of
    /mcp/full. The body is what the host measures, so that is what is compared
    against the prose."""
    import urllib.request

    def page(path, cursor=None):
        body = {"jsonrpc": "2.0", "id": 1, "method": "tools/list",
                "params": {} if cursor is None else {"cursor": cursor}}
        req = urllib.request.Request(
            responder + path, data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json",
                     "Accept": "application/json, text/event-stream"})
        with urllib.request.urlopen(req, timeout=60) as r:
            raw = r.read()
        return raw, json.loads(raw)["result"]

    try:
        raw, res = page("/mcp")
        if res.get("nextCursor"):
            return None  # core split across pages; a different check owns that
        core = len(raw)
        total, cursor, pages = 0, None, 0
        while True:
            raw, res = page("/mcp/full", cursor)
            total += len(raw)
            pages += 1
            cursor = res.get("nextCursor")
            if not cursor or pages > 20:
                break
        return {"core": core, "full": total}
    except Exception:
        return None


def verify_mcp_byte_claims(responder: str) -> list[str]:
    measured = measure_mcp_list_bytes(responder)
    if measured is None:
        # Unreachable, or core is paginated. Neither is this check's business,
        # and reporting a live figure as drift when the responder was simply
        # down is how a gate earns a reputation for crying wolf.
        return []
    hits = []
    for rel, pat, which in MCP_BYTES_CLAIMS:
        f = REPO / rel
        if not f.exists():
            hits.append(f"{rel}: named in MCP_BYTES_CLAIMS but missing")
            continue
        m = re.search(pat, f.read_text(encoding="utf-8"))
        if not m:
            hits.append(f"{rel}: the {which} byte claim no longer matches its "
                        f"pattern; the prose moved and this check went blind.")
            continue
        claimed_kb = int(m.group(1))
        real_kb = measured[which] / 1024
        if abs(claimed_kb - real_kb) > real_kb * MCP_BYTES_TOLERANCE:
            hits.append(f"{rel}: claims ~{claimed_kb} KB for the {which} "
                        f"tools/list surface; measured {real_kb:.0f} KB live.")
    return hits


def main() -> int:
    mode = sys.argv[1] if len(sys.argv) > 1 else "--show"

    print("Canonical counts (source of truth):")
    for k, v in CANON.items():
        print(f"  {k:28} {v}")
    print()

    drift = verify_canon()
    if drift:
        print("CANON DRIFT — registries/responder no longer match CANON:")
        for d in drift:
            print(f"  ! {d}")
        print("  -> update CANON in this file, then re-run --write.\n")
    else:
        print("CANON verified against repo registries"
              + (" and live responder.\n" if os.environ.get("EMEM_RESPONDER", "https://emem.dev") else ".\n"))

    if mode == "--check":
        problems = drift[:]
        problems += write_agent(check_only=True)
        problems += scan_prose()
        problems += verify_mcp_byte_claims(
            os.environ.get("EMEM_RESPONDER", "https://emem.dev").rstrip("/"))
        if problems:
            print("DRIFT DETECTED:")
            for p in problems:
                print(f"  ✗ {p}")
            return 1
        print("All surfaces match canonical counts. No drift.")
        return 0

    if mode == "--write":
        changes = write_agent(check_only=False)
        for c in changes:
            print(f"  ✓ {c}")
        print("\nMachine twins synced. Prose surfaces (README/docs/homepage):")
        for h in scan_prose():
            print(f"  ⚠ {h}")
        print("  (fix prose by hand for voice, then re-run --check)")
        return 1 if drift else 0

    return 1 if drift else 0


if __name__ == "__main__":
    raise SystemExit(main())
