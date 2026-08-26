#!/usr/bin/env python3
"""Is the code answering the code we have?

    scripts/deploy_drift.py            check https://emem.dev
    scripts/deploy_drift.py --origin http://127.0.0.1:5051
    scripts/deploy_drift.py --require-head    fail unless prod is exactly HEAD

The axis every other gate here lacks
------------------------------------
The five gates in `gates.py` all compare source to source. Not one can see the
state where the code is correct and is not the code answering -- a deploy that
looked fine and left the old binary running, a restart that silently failed, a
build from a tree that no longer exists. The upstream has this axis (their live
service's fn_id against the contract computed in-process) and named it as the
stronger form of independence. I said we had none.

We did. `crates/emem-api-rest/build.rs` has been stamping EMEM_GIT_COMMIT and
EMEM_BUILD_TIMESTAMP into the binary and publishing them under
`operator_attestation` in /.well-known/emem.json, alongside a blake3 of the
binary itself. The capability existed and nothing compared it to anything, which
is the same shape as every defect this pair of systems found today: the
knowledge was in the tree and had not travelled to the place that needed it.

What is wrong and what is merely true
-------------------------------------
Being BEHIND HEAD is normal: work gets committed between deploys. This says how
far behind, and names the commits, because "three commits" is a fact and "up to
date" was an assumption.

What fails:

  unreproducible   the running commit is not in this repository at all, so
                   nothing here can rebuild what is serving. That is the state
                   worth stopping for.
  --require-head   asked for explicitly, e.g. by redeploy.sh right after a
                   restart, where "still on the old commit" means the deploy did
                   not take even though every step reported success.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def git(*args: str) -> str:
    try:
        return subprocess.run(["git", "-C", str(REPO), *args],
                              capture_output=True, text=True, timeout=60).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return ""


def baked_paths() -> set[str]:
    """Every file compiled INTO the binary, read from the source rather than
    listed by hand: `include_str!` targets plus the crates themselves.

    Why this matters. "N commits behind" is a fact about the repository and
    reads like a fact about the deployment. Most commits here touch scripts/ or
    docs prose, which the running binary never executes and never serves, so a
    non-zero count is a normal steady state rather than a deploy in flight. A
    reviewer who treats it as the second one chases a rebuild that would change
    nothing -- a neighbouring question answered by a field that looks like the
    one you asked. A peer operator warned me about this shape on their own
    surface before I could walk into it on mine.
    """
    import re
    out: set[str] = set()
    root = pathlib.Path(REPO)
    for rs in (root / "crates").rglob("*.rs"):
        try:
            body = rs.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for m in re.finditer(r'include_str!\(\s*"([^"]+)"', body):
            try:
                out.add(str((rs.parent / m.group(1)).resolve().relative_to(root)))
            except (ValueError, OSError):
                pass
    return out


def rebuild_relevant(commit_range: str) -> tuple[list[str], list[str]]:
    """Split the undeployed commits into the ones that change what the binary
    serves and the ones that cannot."""
    baked = baked_paths()
    changes, inert = [], []
    for line in git("log", "--format=%h %s", commit_range).split("\n"):
        if not line.strip():
            continue
        sha = line.split()[0]
        files = [f for f in git("show", "--name-only", "--format=", sha).split("\n") if f]
        hits = [f for f in files
                if f in baked or f.startswith("crates/")
                or f in ("Cargo.toml", "Cargo.lock")]
        (changes if hits else inert).append(line)
    return changes, inert


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--require-head", action="store_true",
                    help="fail unless the responder is serving exactly HEAD")
    args = ap.parse_args()

    try:
        with urllib.request.urlopen(f"{args.origin}/.well-known/emem.json", timeout=45) as r:
            doc = json.load(r)
    except Exception as e:
        # A responder we cannot reach is undetermined, not in sync.
        print(f"  {args.origin} unreachable: {e}")
        print("  Undetermined, not passing: this check exists to compare the")
        print("  running code to this tree, and there is no running code to read.")
        return 1

    att = doc.get("operator_attestation") or {}
    live_commit = att.get("git_commit") or ""
    built_at = att.get("build_timestamp") or "?"
    live_blake3 = att.get("binary_blake3") or ""

    head = git("rev-parse", "HEAD")
    print(f"  responder {args.origin}")
    print(f"    serving commit  {live_commit[:12] or '(none published)'}   built {built_at}")
    print(f"    this tree HEAD  {head[:12]}")

    if not live_commit or live_commit == "unknown":
        print("\n  The responder publishes no commit, so nothing here can say which")
        print("  source produced it. build.rs falls back to \"unknown\" outside a")
        print("  git checkout; a release build that lands in production this way")
        print("  is unreproducible by anyone, including its operator.")
        return 1

    known = git("cat-file", "-t", live_commit) == "commit"
    if not known:
        print(f"\n  UNREPRODUCIBLE: {live_commit[:12]} is not a commit in this")
        print("  repository. Something is serving that this tree cannot rebuild --")
        print("  a binary from uncommitted work, a branch that was force-pushed")
        print("  away, or a tree that no longer exists.")
        return 1

    if live_commit == head:
        print("\n  In sync: the code answering is the code here.")
    else:
        behind = git("rev-list", "--count", f"{live_commit}..{head}")
        ahead = git("rev-list", "--count", f"{head}..{live_commit}")
        gap = []
        if behind and behind != "0":
            gap.append(f"{behind} commit(s) behind")
        if ahead and ahead != "0":
            gap.append(f"{ahead} commit(s) AHEAD of this tree")
        print(f"\n  Drift: the responder is {', '.join(gap) or 'on a divergent commit'}.")
        changes, inert = rebuild_relevant(f"{live_commit}..{head}")
        for line in changes[:8]:
            print(f"    NOT DEPLOYED, changes the binary: {line[:70]}")
        for line in inert[:8]:
            print(f"    not deployed, cannot change it:   {line[:70]}")
        if not changes and inert:
            print("\n  None of the undeployed commits touches anything this binary")
            print("  compiles in or executes: scripts and prose only. The running")
            print("  code IS this tree for everything it can serve, and a rebuild")
            print("  would produce the same responder. Behind-HEAD is the steady")
            print("  state here, not a deploy in flight.")

    # The binary on disk is only comparable when it is the one that was
    # deployed; a rebuild since then legitimately differs. Reported, not judged.
    binary = REPO / "target" / "release" / "emem-server"
    if live_blake3 and binary.exists():
        try:
            import blake3 as _b3
            h = _b3.blake3(binary.read_bytes()).hexdigest()
            same = h == live_blake3
            print(f"    on-disk binary {'matches' if same else 'differs from'} the running one"
                  + ("" if same else " (a build has happened since the deploy)"))
        except ImportError:
            pass

    if args.require_head and live_commit != head:
        print("\n  --require-head: the responder is not serving HEAD. After a")
        print("  deploy that reported success, this means the deploy did not")
        print("  take -- the build, the restart, or both.")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
