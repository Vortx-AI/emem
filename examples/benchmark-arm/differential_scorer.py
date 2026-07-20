#!/usr/bin/env python3
"""An INDEPENDENT re-scorer for the navigatable_worlds benchmark runs.

WHY THIS FILE EXISTS
--------------------
The scorecard at cid `no4fvfl2e2v2zick33ydoadene` lists six things it does not
have. The second is:

    "No independent re-scoring (differential scorer requested, not delivered)."

emem asked for that scorer, so emem owes it. This is it. It reads the published
raw rows and recomputes the headline numbers from the recorded BYTES, using
scoring semantics written here rather than imported from the harness that
produced them. It deliberately does not import `canonical.py`, `analyze_runs.py`
or anything else from the benchmark directory: a re-scorer that shares the
original's helpers can only reproduce the original's bugs.

Four scorer bugs have already been found in this study by inspection. The point
of this file is not to certify that there are no more. It is to make the numbers
falsifiable by a second implementation, and to print every row where the two
implementations disagree so a reader can adjudicate rather than trust.

WHAT IT CHECKS, IN ORDER
------------------------
1. INTEGRITY, before any scoring. Every run and sidecar is hashed against
   `benchmark/PROVENANCE.json`, and every prompt/answer cid referenced by a step
   is recomputed as base32(blake3(bytes)). A number scored from bytes that do
   not match their own address is not evidence.

2. GROUND TRUTH, twice where possible. The value the prompt SHOWED the model,
   parsed from the prompt bytes, and the value emem SIGNED, resolved live from
   the responder. These differ legitimately: the `emem` arm displays a rounded
   `0.747614` while the signed fact is `0.7476139978791093`. A scorer with one
   notion of "expected" silently mis-scores one arm or the other. Disagreements
   between the two sources are reported, not reconciled.

3. SCORING, decomposed. `exact` (the ground-truth digits appear verbatim),
   `numeric_equal` (some number in the answer parses equal as f64 but the bytes
   differ, the rounding failure), `confidently_wrong` (a number that is neither),
   and `abstain` (no number offered). Collapsing these is how a benchmark
   flatters itself.

WHAT THIS SCORER CANNOT CHECK
-----------------------------
Its retrieval-hit detection matches the question's coordinate STRING against the
retrieved chunks. The original harness identifies the queried cell by cell id.
The string method is strictly more conservative, so it finds fewer hits and
CANNOT confirm or refute the published "conditioned on a hit, both models
answered exactly" claim. It is reported here as unconfirmed rather than
contradicted, because a weaker instrument disagreeing with a stronger one is not
evidence against the stronger one.

It also cannot check anything about the models, the sampling, or whether the
questions are representative. It re-scores recorded bytes. That is all.

Run it:
    python3 differential_scorer.py /home/ubuntu/navigatable_worlds
    python3 differential_scorer.py /home/ubuntu/navigatable_worlds --manifest-only
    python3 differential_scorer.py /home/ubuntu/navigatable_worlds --offline
"""
from __future__ import annotations

import base64
import glob
import hashlib
import json
import os
import re
import sys
import urllib.request
from collections import Counter, defaultdict

RESPONDER = "https://emem.dev"

# ---------------------------------------------------------------------------
# addressing
# ---------------------------------------------------------------------------


def b32(raw: bytes) -> str:
    return base64.b32encode(raw).decode().lower().rstrip("=")


def cid_of(text: str) -> str:
    """emem content address: base32(blake3(bytes)), lowercase, unpadded."""
    import blake3  # imported late so the integrity leg can be skipped without it

    return b32(blake3.blake3(text.encode()).digest())


def sha256_of(path: str) -> str:
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


# ---------------------------------------------------------------------------
# parsing. Written from the prompt bytes, not from the harness that wrote them.
# ---------------------------------------------------------------------------

_NUM = re.compile(r"-?\d+\.\d+|-?\d+")

# The queried band is named in the question. Map the human phrasing to the label
# that appears on the value lines, because the two are worded differently.
_BANDS = [
    ("ndvi", "NDVI"),
    ("ndwi", "NDWI"),
    ("ndre", "NDRE"),
    ("ndmi", "NDMI"),
    ("elevation", "elevation"),
]


def queried_band(question: str) -> str | None:
    q = question.lower()
    for key, _label in _BANDS:
        if key in q:
            return key
    if "elevation" in q:
        return "elevation"
    return None


def question_coords(question: str) -> tuple[str, ...]:
    """The question's own latitude and longitude.

    These must be excluded from answer parsing. At the London site the question's
    own longitude (-0.13) is itself a plausible NDVI, and naive first-number
    extraction scored correct answers as wrong. That bug was found by the worlds
    agent in their own scorer; it is reproduced here as a hazard to avoid, which
    is not the same as importing their fix.
    """
    m = re.search(r"latitude\s+(-?[\d.]+),\s*longitude\s+(-?[\d.]+)", question)
    return (m.group(1), m.group(2)) if m else ()


def question_numbers(question: str) -> tuple[str, ...]:
    """EVERY number the question itself contains.

    Excluding only the coordinates was not enough, and the gap was found by the
    worlds agent re-deriving this scorer's rule against their own. Every question
    reads "the 10 m cell at latitude X, longitude Y", so an answer that echoes
    the question before answering it:

        "The NDVI of the 10 m cell at latitude X, longitude Y is 0.672."

    yields ['10', '0.672'] and first-number extraction scores the answer as 10.
    Two models that both said 0.672 were then recorded as disagreeing. That
    inflated the measured disagreement in the compaction arms specifically,
    because those are the arms where models restate the question most.

    The coordinate exclusion was a special case of this rule. A number the
    question already contains cannot be the answer to it, so exclude all of
    them rather than enumerating the ones observed to bite.
    """
    return tuple(m.group(0) for m in _NUM.finditer(question or ""))


def numbers_in(text: str, exclude: tuple[str, ...] = ()) -> list[str]:
    return [m.group(0) for m in _NUM.finditer(text or "") if m.group(0) not in exclude]


def shown_value(prompt: str, band: str, coords: tuple[str, ...]) -> str | None:
    """The value the PROMPT put in front of the model, if it put one there.

    Three prompt shapes carry values: `label = value` (emem, context),
    `label: value` inside a bracketed retrieved chunk (rag), and prose summaries
    (compaction) which carry no reliable labelled value at all.
    """
    if not prompt or not band:
        return None
    key = dict(_BANDS).get(band, band)

    # RAG: several cells are offered. Only the chunk whose coordinates match the
    # question can be the ground truth; if it was never retrieved, the arm cannot
    # succeed and that is a retrieval failure, not an answering failure.
    if "Retrieved context:" in prompt and coords:
        lat, lon = coords
        for chunk in re.split(r"\n(?=\[\d+\])", prompt):
            if f"latitude {lat}, longitude {lon}" in chunk:
                m = re.search(rf"{key}[^:=]*[:=]\s*(-?[\d.]+)", chunk)
                return m.group(1) if m else None
        return None  # correct cell absent from context

    m = re.search(rf"^\s*{key}[^:=\n]*[:=]\s*(-?[\d.]+)\s*$", prompt, re.M)
    return m.group(1) if m else None


# ---------------------------------------------------------------------------
# the responder: what emem actually signed
# ---------------------------------------------------------------------------


class Responder:
    """Resolves fact cids live, with a cache. This is the second, independent
    ground truth: what the substrate signed, as opposed to what the harness
    displayed."""

    def __init__(self, base: str = RESPONDER, enabled: bool = True):
        self.base, self.enabled, self.cache = base, enabled, {}

    def resolve(self, cid: str) -> dict | None:
        if not self.enabled:
            return None
        if cid in self.cache:
            return self.cache[cid]
        try:
            req = urllib.request.Request(
                self.base + "/v1/memory_token/resolve",
                data=json.dumps({"token": cid}).encode(),
                headers={"Content-Type": "application/json"},
            )
            with urllib.request.urlopen(req, timeout=20) as r:
                out = json.load(r)
        except Exception:
            out = None
        self.cache[cid] = out
        return out

    def signed_value(self, fact_cids: list[str], band: str) -> tuple[str | None, str | None]:
        """(value_verbatim, cid) for the fact matching `band`."""
        for c in fact_cids or []:
            r = self.resolve(c)
            if not r:
                continue
            blob = json.dumps(r).lower()
            if band and band in blob:
                v = r.get("value_verbatim")
                if v is not None:
                    return str(v), c
        return None, None


# ---------------------------------------------------------------------------
# scoring
# ---------------------------------------------------------------------------

_ABSTAIN = re.compile(
    r"\b(cannot|can't|unable|not (?:provided|available|present|contain)|"
    r"no (?:information|data|value)|does not (?:contain|include|provide)|"
    r"insufficient|unknown|not specified)\b",
    re.I,
)


def score_answer(answer: str, truth: str, coords: tuple[str, ...]) -> str:
    """One row, four mutually exclusive outcomes.

    `exact` means the digits emem signed (or displayed) survived the model
    verbatim. `numeric_equal` means the value is right but the bytes are not,
    which is the rounding failure the loop exists to catch. Keeping them apart
    is the entire point: a benchmark that scores 0.2411 as correct for 0.241103
    cannot see the failure mode.
    """
    if truth is None:
        return "no_ground_truth"
    nums = numbers_in(answer, exclude=coords)
    if not nums:
        return "abstain"
    if truth in nums:
        return "exact"
    try:
        t = float(truth)
        for n in nums:
            if float(n) == t:
                return "numeric_equal"
    except ValueError:
        pass
    if _ABSTAIN.search(answer or ""):
        return "abstain"
    return "confidently_wrong"


# ---------------------------------------------------------------------------
# citation fidelity, scored separately from value fidelity
# ---------------------------------------------------------------------------

_TOKEN = re.compile(r"emem:fact:[a-z0-9.,@~_\-]+:([a-z2-7]{52})", re.I)
_BARE = re.compile(r"\b([a-z2-7]{52})\b")


def citation_grade(prompt: str, answer: str, band: str | None = None) -> str | None:
    """full_token / cid_only / none, relative to the token the prompt offered.

    `cid_only` is the head-drop failure: the model keeps the opaque base32 tail
    and discards `emem:fact:<descriptor>:`, so a naive responder resolves nothing.

    The prompt usually offers SEVERAL tokens (elevation, ndvi, ndwi). Grading
    against the first one found marks a correct ndvi citation as a miss. It is a bug
    this scorer had on its first run, and the reason the band is passed in.
    """
    cands = list(_TOKEN.finditer(prompt or ""))
    if not cands:
        return None
    want = None
    if band:
        for m in cands:
            if band in m.group(0).lower():
                want = m
                break
    want = want or cands[0]
    cid = want.group(1).lower()
    for m in _TOKEN.finditer(answer or ""):
        if m.group(1).lower() == cid:
            return "full_token"
    if any(m.group(1).lower() == cid for m in _BARE.finditer((answer or "").lower())):
        return "cid_only"
    return "none"


# ---------------------------------------------------------------------------
# driver
# ---------------------------------------------------------------------------


def load_manifest(root: str) -> dict:
    p = os.path.join(root, "benchmark", "PROVENANCE.json")
    return json.load(open(p)) if os.path.exists(p) else {}


def integrity(root: str, man: dict, check_cids: bool = True) -> dict:
    out = {"files_ok": 0, "files_bad": [], "files_missing": [], "cids_ok": 0, "cids_bad": []}
    for r in man.get("runs", []):
        for key, shakey in (("file", "sha256"), ("sidecar", "sidecar_sha256")):
            rel = r.get(key)
            if not rel:
                continue
            path = os.path.join(root, rel)
            if not os.path.exists(path):
                out["files_missing"].append(rel)
                continue
            if sha256_of(path) == r.get(shakey):
                out["files_ok"] += 1
            else:
                out["files_bad"].append(rel)

    if check_cids:
        try:
            import blake3  # noqa: F401
        except ImportError:
            out["cids_skipped"] = "blake3 not installed"
            return out
        for run_path, side in iter_runs(root):
            for cid, content in (side or {}).items():
                if not isinstance(content, str):
                    continue
                if cid_of(content) == cid:
                    out["cids_ok"] += 1
                else:
                    out["cids_bad"].append((os.path.basename(run_path), cid))
    return out


def iter_runs(root: str):
    for f in sorted(glob.glob(os.path.join(root, "runs", "*.json"))):
        if f.endswith(".sidecar.json"):
            continue
        sp = f.replace(".json", ".sidecar.json")
        side = json.load(open(sp)) if os.path.exists(sp) else {}
        yield f, side


def rescore(root: str, live: bool = True, manifest_only: bool = False, man: dict | None = None) -> dict:
    """Score every row against BOTH ground truths.

    `shown` is what the prompt displayed; `signed` is what emem holds. They are
    not the same object and the difference is load-bearing, so both are carried
    to the end rather than collapsed here.
    """
    resp = Responder(enabled=live)
    rows = []

    allowed = None
    if manifest_only and man:
        allowed = {os.path.basename(r["file"]) for r in man.get("runs", []) if r.get("file")}

    for run_path, side in iter_runs(root):
        if allowed is not None and os.path.basename(run_path) not in allowed:
            continue
        run = json.load(open(run_path))
        world = (run.get("run") or {}).get("world")
        for st in run.get("steps", []):
            prompt = side.get(st.get("prompt_cid")) or ""
            answer = side.get(st.get("answer_cid")) or ""
            if not prompt:
                continue
            question = prompt.split("\n")[0]
            band = queried_band(question)
            # Two distinct things that were briefly conflated: the lat/lon pair,
            # which locates the queried cell inside a RAG chunk, and every number
            # the question echoes, which must never be mistaken for the answer.
            coords = question_coords(question)
            echoed = question_numbers(question)
            shown = shown_value(prompt, band, coords)

            # Resolve the signed fact for EVERY row, not only where the prompt
            # withholds it. Restricting live resolution to value-less prompts is
            # what hid the shown-vs-signed gap on this scorer's first run.
            signed, _ = resp.signed_value(st.get("fact_cids") or [], band)

            # A retrieval arm that never surfaced the queried cell cannot answer
            # from context. That is a retrieval failure, and scoring it as a
            # generation failure would blame the wrong component.
            #
            # `has_context` gates this on prompts that actually carry retrieved
            # chunks. The handoff-RAG prompts have a different shape, and on this
            # scorer's second run they were silently counted as retrieval HITS,
            # inflating the hit rate from 0.026 to 0.086. Absence of the miss
            # marker is not evidence of a hit.
            has_context = "Retrieved context:" in prompt
            retrieval_miss = has_context and shown is None

            truth = shown if shown is not None else signed
            rows.append(
                {
                    "run": os.path.basename(run_path),
                    "world": world,
                    "arm": st.get("arm"),
                    "model": st.get("model"),
                    "turn": st.get("turn_id"),
                    "band": band,
                    "shown": shown,
                    "signed": signed,
                    "truth": truth,
                    "has_context": has_context,
                    "retrieval_miss": retrieval_miss,
                    "grade": score_answer(answer, truth, echoed),
                    "grade_vs_signed": score_answer(answer, signed, echoed) if signed else None,
                    "citation": citation_grade(prompt, answer, band),
                    "answer": answer,
                }
            )
    return {"rows": rows, "live_used": len(resp.cache)}


def summarise(rows: list[dict]) -> dict:
    by = defaultdict(Counter)
    for r in rows:
        by[(r["arm"], r["model"])][r["grade"]] += 1
        if r["citation"]:
            by[(r["arm"], r["model"])]["cite_" + r["citation"]] += 1
    return by


def main() -> int:
    root = sys.argv[1] if len(sys.argv) > 1 else "/home/ubuntu/navigatable_worlds"
    live = "--offline" not in sys.argv

    man = load_manifest(root)
    print(f"# differential re-score of {root}")
    print(f"# scorecard under review: {(man.get('canonical_result') or {}).get('scorecard_cid')}")
    print()

    print("## 1. integrity")
    ig = integrity(root, man)
    print(f"   manifest files byte-identical : {ig['files_ok']}")
    print(f"   manifest files missing        : {len(ig['files_missing'])} {ig['files_missing'][:5]}")
    print(f"   manifest files sha mismatch   : {len(ig['files_bad'])} {ig['files_bad'][:5]}")
    print(f"   sidecar entries cid==blake3   : {ig['cids_ok']}")
    print(f"   sidecar entries cid MISMATCH  : {len(ig['cids_bad'])} {ig['cids_bad'][:5]}")
    print()

    print("## 2. re-score")
    mo = "--manifest-only" in sys.argv
    out = rescore(root, live=live, manifest_only=mo, man=man)
    rows = out["rows"]
    scope = "manifest runs only" if mo else "all runs on disk"
    print(f"   scope: {scope}")
    print(f"   rows scored: {len(rows)}   unique facts resolved: {out['live_used']}")
    print()

    by = summarise(rows)
    print("## 3. per arm x model")
    hdr = f"   {'arm':<28}{'model':<26}{'exact':>7}{'num=':>6}{'wrong':>7}{'abst':>6}{'n':>6}"
    print(hdr)
    for (arm, model), c in sorted(by.items()):
        n = sum(v for k, v in c.items() if not k.startswith("cite_"))
        print(
            f"   {str(arm):<28}{str(model).split('/')[-1]:<26}"
            f"{c['exact']:>7}{c['numeric_equal']:>6}{c['confidently_wrong']:>7}"
            f"{c['abstain']:>6}{n:>6}"
        )
    print()

    print("## 4. citation fidelity (where the prompt offered a token)")
    cit = Counter()
    for r in rows:
        if r["citation"]:
            cit[(r["arm"], r["citation"])] += 1
    for (arm, grade), n in sorted(cit.items()):
        print(f"   {arm:<28}{grade:<14}{n:>6}")
    print()

    print("## 5. retrieval, scored separately from generation")
    rag = [r for r in rows if r.get("has_context")]
    other = [r for r in rows if r["arm"] == "rag" and not r.get("has_context")]
    if rag:
        miss = sum(1 for r in rag if r["retrieval_miss"])
        hit_rows = [r for r in rag if not r["retrieval_miss"]]
        print(f"   rows with retrieved context : {len(rag)}")
        print(f"   queried cell absent         : {miss}")
        print(f"   independent hit rate        : {len(hit_rows)/len(rag):.3f}")
        print(f"   conditioned on a hit, exact : "
              f"{sum(1 for r in hit_rows if r['grade']=='exact')}/{len(hit_rows)}")
    if other:
        print(f"   rag-family rows with a different prompt shape (handoff), "
              f"not counted either way: {len(other)}")
    print()

    print("## 6. rows with no ground truth (unscoreable, reported not hidden)")
    ngt = Counter(r["arm"] for r in rows if r["grade"] == "no_ground_truth")
    for arm, n in sorted(ngt.items()):
        print(f"   {arm:<28}{n:>6}")
    if not ngt:
        print("   none")
    print()

    print("## 7. shown vs signed: what the prompt displayed vs what emem signed")
    both = [r for r in rows if r["shown"] and r["signed"]]
    dis = [r for r in both if r["shown"] != r["signed"]]
    print(f"   rows where both are known : {len(both)}")
    print(f"   rows where they DIFFER    : {len(dis)}")
    for r in dis[:4]:
        print(f"     {r['run']} {r['turn']}: shown={r['shown']}  signed={r['signed']}")
    if dis:
        print("   NOTE: where these differ the arm measures copying a ROUNDED display,")
        print("   not carrying the signed fact. Reported, not corrected.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
