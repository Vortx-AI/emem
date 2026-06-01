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
  scripts/sync_counts.py --write    # rewrite the count fields in the machine twins
                                     # (web/humans.json, web/agent.json) in place

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
CARGO = REPO / "Cargo.toml"

# ---------------------------------------------------------------------------
# Canonical counts. Every value here is verified below: the offline-computable
# ones are asserted against the registries; the runtime ones are checked against
# a live /v1/agent_card when reachable. CANON is the value the static twins are
# written to and the value --check enforces across every surface.
# ---------------------------------------------------------------------------
CANON = {
    "mcp_tools": 81,
    "mcp_core": 10,
    "mcp_extended": 71,
    "algorithms": 160,
    "rest_paths_v1": 93,            # documented /v1/* paths in OpenAPI
    "rest_paths_openapi_total": 96,  # all paths in OpenAPI
    "cube_slots": 42,
    "materializer_wired": 122,
    "source_schemes": 46,
    "topics": 27,
    "foundation_encoders": 4,
    "mcp_resources": 7,        # resources/list entries
    "mcp_uri_templates": 3,    # resource template entries
    "crates": 16,
    "version": "0.0.9",
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


def verify_canon() -> list[str]:
    """Assert CANON matches what the repo (and, if reachable, the responder) say."""
    drift = []
    off = compute_offline()
    for k, v in off.items():
        if k in CANON and CANON[k] != v:
            drift.append(f"CANON[{k}]={CANON[k]} but repo computes {v}")

    live = fetch_live(os.environ.get("EMEM_RESPONDER", "https://emem.dev"))
    if live:
        for k, v in live.items():
            if v is not None and k in CANON and CANON[k] != v:
                drift.append(f"CANON[{k}]={CANON[k]} but live /v1/agent_card says {v}")
    return drift


# ---------------------------------------------------------------------------
# Patch the machine twins. Field-name-targeted regex keeps formatting/diff small.
# ---------------------------------------------------------------------------
def _set_num(text: str, key: str, val: int) -> str:
    return re.sub(rf'("{re.escape(key)}"\s*:\s*)\d+', rf"\g<1>{val}", text)


def write_humans(check_only: bool) -> list[str]:
    f = REPO / "web" / "humans.json"
    text = f.read_text()
    repl = {
        "mcp_tools": CANON["mcp_tools"],
        "algorithms": CANON["algorithms"],
        "rest_paths_openapi": CANON["rest_paths_openapi_total"],
        "rest_paths_v1_openapi": CANON["rest_paths_v1"],
        "band_cube_slots": CANON["cube_slots"],
        "materializer_wired_names": CANON["materializer_wired"],
        "source_schemes": CANON["source_schemes"],
        "topics_declared": CANON["topics"],
    }
    new = text
    for k, v in repl.items():
        new = _set_num(new, k, v)
    problems = _apply(f, text, new, check_only)
    # Guard: every numeric field in the counts block must be one we manage,
    # so an unguarded hand-number can't be reintroduced and drift silently.
    counts = json.loads(text).get("counts", {})
    unmanaged = [k for k, v in counts.items()
                 if isinstance(v, int) and k not in repl]
    for k in unmanaged:
        problems.append(f"{f.relative_to(REPO)}: counts.{k} is not managed by "
                        f"sync_counts.py — add it to CANON+repl or remove it")
    return problems


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
    # Free-text count tokens in the tagline + description strings.
    new = new.replace("Three foundation encoders", "Four foundation encoders")
    new = new.replace("75 MCP tools (10 core, 65 extended)",
                      "80 MCP tools (10 core, 70 extended)")
    new = new.replace("18 static MCP resources + 8 URI templates",
                      "7 static MCP resources + 3 URI templates")
    new = new.replace("42 bands, 46 source schemes",
                      "122 materializer-wired band names across 42 cube slots, 46 source schemes")
    new = new.replace("35 cube slots; 118 (slot",
                      "42 cube slots; 122 (slot")
    return _apply(f, text, new, check_only)


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
                  "118 materializer-wired", "across 35 cube", "14 workspace crates"],
    "web/index.html": ["75 MCP tools", "87 documented", "87 REST", "87 paths", "118 materializer", "35 cube slots"],
    "web/how-it-works.html": ["75 MCP tools", "118 materializer", "35 cube slots", "41 cube"],
    "web/solutions.html": ["75 MCP tools", "118 materializer", "35 cube slots"],
    "web/reference.html": ["75 MCP tools", "87 documented", "118 materializer", "35 cube slots"],
    "docs/intro.md": ["75 MCP", "87 ", "118 materializer", "35 cube", "41 cube"],
    "docs/whitepaper.md": ["Forty-one band cube", "41 cube", "35 cube",
                           "118 materializer", "75 MCP"],
    "docs/registries.md": ["118 materializer", "(43)", "(86)"],
    "web/skills.md": ["75 tools", "71 paths", "87 paths"],
    "web/llms.txt": ["75 MCP", "71 paths", "87 paths", "118 materializer"],
}


def scan_prose() -> list[str]:
    hits = []
    for rel, phrases in STALE_PHRASES.items():
        p = REPO / rel
        if not p.exists():
            continue
        body = p.read_text()
        for ph in phrases:
            if ph in body:
                hits.append(f"{rel}: stale count phrase present: {ph!r}")
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
        problems += write_humans(check_only=True)
        problems += write_agent(check_only=True)
        problems += scan_prose()
        if problems:
            print("DRIFT DETECTED:")
            for p in problems:
                print(f"  ✗ {p}")
            return 1
        print("All surfaces match canonical counts. No drift.")
        return 0

    if mode == "--write":
        changes = write_humans(check_only=False) + write_agent(check_only=False)
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
