"""Record a playground benchmark run as a run-manifest v1.1 + sidecar.

Schema is normative in AGENT_HANDOFF_playground_manifest.md; the viewer side is
UI_CONTRACT_playground_replay.md. This is the recorder: it runs the arms and
writes down what happened in a form a stranger can check without trusting it.

Three rules it holds to, each of which costs something:

  Every cell is a TRUE 10 m cell recalled from the responder, and every question
  is asked at that cell's OWN coordinates, derived from its cell64. See
  corpus.py for why this does not read a scene digest: those anchor signed
  scalars on a 212 m grid and inherit them across 4 tiles up to 151 m apart, so
  a question asked at a tile centre gets answered from up to 74 m away.

  It records no metric and no verdict. Inter-model agreement is computed by
  whoever reads the manifest, from the recorded answers. A manifest that carries
  its own conclusion is a manifest nobody should trust, and that binds the
  recorder as much as the viewer.

  Every arm records context_cids[] at the same granularity: exactly what it was
  given, in the order it was given. Without that, "RAG top-k drifts" stays an
  assertion in a project whose pitch is that assertions are not enough. If RAG
  is stable, the recording says so and the claim dies in public.

Usage:
    <venv>/bin/python bench/record_run.py --n 12 --queries 4 --out var/playground/lahaul
"""

from __future__ import annotations

import argparse
import json
import math
import pathlib
import subprocess
import time
import urllib.request
from typing import Any

from blake3 import blake3
from canonical import b32_nopad_lc, dumps, run_cid
from corpus import (
    DEG_10M_LAT,
    RESPONDER,
    build_patch,
    cell_latlng,
    cell_prose,
    fact_line_context,
    fact_line_emem,
)

LLM_URL = "http://127.0.0.1:5014/v1/chat/completions"

MODELS = [
    ("google/gemma-4-12B-it", "gemma"),
    ("Qwen/Qwen2.5-7B-Instruct", "qwen"),
]

BANDS = ["indices.ndvi", "indices.ndwi", "copdem30m.elevation_mean"]

#: Keylong, Lahaul, Himachal Pradesh. Coordinates, not the name: "Lahaul"
#: geocodes to "Lahage (village), Occitanie, France". Elevation ~3100 m here
#: independently confirms the anchor; the French hit sits at ~200 m.
LAHAUL = (32.5713, 77.0345)

#: P2: greedy, recorded, never inherited. At temperature > 0 an inter-model
#: disagreement is indistinguishable from two samples of one distribution,
#: which makes the headline unfalsifiable in the direction that flatters us.
DECODING = {"temperature": 0.0, "top_p": 1.0, "seed": 0, "max_tokens": 512}


def sidecar_put(sidecar: dict[str, str], text: str) -> str:
    """Content-address `text` and keep the exact bytes. Returns the cid.

    P1: the answers are what the headline metric measures, so they are hashed
    into the manifest and their bytes live here keyed by that hash. A verifier
    can then check that the answer shown is the answer generated.
    """
    cid = b32_nopad_lc(blake3(text.encode("utf-8")).digest())
    sidecar[cid] = text
    return cid


def call_model(base_model: str, family: str, prompt: str) -> tuple[str, int]:
    """One greedy completion. Returns (answer, elapsed_ms)."""
    body = {
        "base_model": base_model,
        "family": family,
        "messages": [{"role": "user", "content": prompt}],
        **DECODING,
    }
    t0 = time.monotonic()
    out = subprocess.run(
        ["curl", "-s", "-m", "300", "-X", "POST", LLM_URL,
         "-H", "Content-Type: application/json", "-d", json.dumps(body)],
        capture_output=True, text=True,
    ).stdout
    ms = int((time.monotonic() - t0) * 1000)
    try:
        return json.loads(out)["choices"][0]["message"]["content"], ms
    except Exception:
        # An empty answer is recorded rather than dropped: a step that failed is
        # data, and omitting it would bias the run toward whichever arm happens
        # to be more reliable today.
        return "", ms


def _post(path: str, body: dict) -> dict[str, Any]:
    req = urllib.request.Request(
        RESPONDER + path, data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Connection": "close"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


def mint_token(cell64: str, band: str, fact: dict) -> str:
    """A descriptor token: the reader here is a model.

    Two models segment a cell64 identically 0% of the time and the same place
    as 5dp coordinates 100% of the time.
    """
    date = (fact.get("sources") or [{}])[0].get("captured_at", "")[:10]
    body = {"cell": cell64, "fact_cid": fact["fact_cid"], "band": band}
    if date:
        body["observed_on"] = date
    m = _post("/v1/memory_token", body)
    return m.get("descriptor_token") or m["memory_token"]


def receipt_cid(receipt: dict) -> str:
    """Key receipts{} by the receipt's own digest: one receipt covers many
    facts, so a fact-keyed map would store it once per fact and invite two
    copies drifting apart."""
    return b32_nopad_lc(blake3(dumps(receipt)).digest())


# ── the arms ──────────────────────────────────────────────────────────────


def build_emem(question, entry, tokens):
    """Addressed. Each observation arrives as a token that resolves."""
    body = "\n".join(fact_line_emem(b, tokens[b], f) for b, f in entry["facts"].items())
    prompt = (
        f"{question}\n\nYou hold these signed observations. Each line is an emem "
        f"citation token followed by its value:\n\n{body}\n\n"
        "Answer the question with the number, then cite the exact token you used."
    )
    bodies = [json.dumps(f, sort_keys=True) for f in entry["facts"].values()]
    return prompt, bodies


def build_context(question, entry):
    """The control: identical observations, addressing removed."""
    body = "\n".join(fact_line_context(b, f) for b, f in entry["facts"].items())
    prompt = (
        f"{question}\n\nYou hold these observations:\n\n{body}\n\n"
        "Answer the question with the number."
    )
    bodies = [json.dumps(f, sort_keys=True) for f in entry["facts"].values()]
    return prompt, bodies


def build_rag(question, retrieve, k=5):
    """Retrieved. What top-k actually returns, in the order it returns it."""
    chunks = retrieve(question, k)
    body = "\n\n".join(f"[{i + 1}] {c}" for i, c in enumerate(chunks))
    prompt = (
        f"{question}\n\nRetrieved context:\n\n{body}\n\n"
        "Answer the question with the number."
    )
    return prompt, chunks


def build_retriever(chunks: list[str]):
    """Dense retrieval over the whole patch, bge-small-en-v1.5, cosine top-k.

    Over the whole patch rather than a shortlist: retrieving from a pre-narrowed
    pool would measure a search the recorder had already done for it.
    """
    from sentence_transformers import SentenceTransformer, util

    m = SentenceTransformer("BAAI/bge-small-en-v1.5", device="cpu")
    emb = m.encode(chunks, convert_to_tensor=True, normalize_embeddings=True,
                   show_progress_bar=False)

    def retrieve(q: str, k: int) -> list[str]:
        qe = m.encode(q, convert_to_tensor=True, normalize_embeddings=True,
                      show_progress_bar=False)
        return [chunks[h["corpus_id"]] for h in util.semantic_search(qe, emb, top_k=k)[0]]

    return retrieve


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=12, help="patch is n x n TRUE 10 m cells")
    ap.add_argument("--queries", type=int, default=4)
    ap.add_argument("--arms", default="emem,context,rag")
    ap.add_argument("--world", default="world_lahaul")
    ap.add_argument("--out", default="var/playground/lahaul")
    args = ap.parse_args()
    arms = args.arms.split(",")

    # The patch is anchored at its SW corner so the named centre stays the
    # centre; a lattice built from the centre outward would shift the world.
    lat_c, lng_c = LAHAUL
    half = args.n * 10.0 / 2.0
    lat0 = lat_c - half / 111_320.0
    lng0 = lng_c - half / (111_320.0 * math.cos(math.radians(lat_c)))

    print(f"patch {args.n}x{args.n} true 10 m cells at {lat_c},{lng_c} ({args.n * 10} m)")
    t0 = time.monotonic()
    patch = build_patch(lat0, lng0, args.n, BANDS)
    print(f"  {len(patch)} distinct cell64 carry facts  ({time.monotonic() - t0:.0f}s)")
    if not patch:
        print("no cells carry facts: refusing to record an empty run")
        return 1

    # Each cell's OWN coordinates, from its cell64. This is the step the scene
    # digest got wrong by up to 74 m.
    for c64, entry in patch.items():
        band, fact = next(iter(entry["facts"].items()))
        entry["lat"], entry["lng"] = cell_latlng(
            c64, fact["fact_cid"], band,
            (fact.get("sources") or [{}])[0].get("captured_at", "")[:10])

    chunks = [cell_prose(e) for e in patch.values()]
    retrieve = build_retriever(chunks) if "rag" in arms else None

    entries = list(patch.values())
    picks = entries[:: max(1, len(entries) // args.queries)][: args.queries]
    sidecar: dict[str, str] = {}
    receipts: dict[str, dict] = {}
    steps: list[dict] = []
    clock = time.monotonic()

    for qi, entry in enumerate(picks):
        if "indices.ndvi" not in entry["facts"]:
            continue
        question = (
            f"What is the NDVI (vegetation greenness) of the 10 m cell at "
            f"latitude {entry['lat']:.5f}, longitude {entry['lng']:.5f}?"
        )
        tokens = ({b: mint_token(entry["cell64"], b, f) for b, f in entry["facts"].items()}
                  if "emem" in arms else {})

        for arm in arms:
            # Retrieval runs ONCE and its result is what both the prompt and
            # context_cids are built from. A second call would record a second
            # retrieval, not the one the model saw.
            if arm == "emem":
                prompt, bodies = build_emem(question, entry, tokens)
            elif arm == "context":
                prompt, bodies = build_context(question, entry)
            elif arm == "rag":
                prompt, bodies = build_rag(question, retrieve)
            else:
                continue
            ctx = [sidecar_put(sidecar, b) for b in bodies]

            for model, family in MODELS:
                answer, ms = call_model(model, family, prompt)
                r_cids = []
                if arm == "emem":
                    r = _post("/v1/memory_token/resolve", {"token": tokens["indices.ndvi"]})
                    rc = receipt_cid(r["receipt"])
                    receipts[rc] = r["receipt"]
                    r_cids = [rc]
                steps.append({
                    "t_offset_ms": int((time.monotonic() - clock) * 1000),
                    "turn_id": f"q{qi}-{arm}-{family}",
                    "arm": arm,
                    "model": model,
                    "prompt_cid": sidecar_put(sidecar, prompt),
                    "answer_cid": sidecar_put(sidecar, answer),
                    "action": {"highlight": [entry["cell64"]]} if arm == "emem" else None,
                    "fact_cids": ([f["fact_cid"] for f in entry["facts"].values()]
                                  if arm != "rag" else []),
                    "context_cids": ctx,
                    "receipt_cids": r_cids,
                    "timing_ms": ms,
                    "decoding": None,
                })
                truth = entry["facts"]["indices.ndvi"]["value"]
                print(f"  q{qi}-{arm:7}-{family:5} {ms:6}ms  truth={truth:.4f}  {answer[:44]!r}")

    manifest = {
        "manifest_version": 1,
        "run": {
            "run_cid": "",
            "recorded_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "world": args.world,
            "albedo_date": time.strftime("%Y-%m-%d", time.gmtime()),
            "responder": {
                "pubkey_b32": _post("/v1/memory_token/resolve", {
                    "token": next(iter(tokens.values()))})["signer_b32"] if tokens else "",
                "base_url": RESPONDER,
            },
            "source_versions": {},
            "arms": arms,
            "models": [m for m, _ in MODELS],
            "decoding": DECODING,
            "patch": {"centre": [lat_c, lng_c], "n": args.n, "cell_m": 10,
                      "cells": len(patch)},
        },
        "receipts": receipts,
        "steps": steps,
    }
    manifest["run"]["run_cid"] = run_cid(manifest)

    out = pathlib.Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.with_suffix(".json").write_text(json.dumps(manifest, indent=1))
    out.with_suffix(".sidecar.json").write_text(json.dumps(sidecar, indent=1))
    print(f"\nrun_cid : {manifest['run']['run_cid']}")
    print(f"steps   : {len(steps)}   sidecar: {len(sidecar)}")
    print(f"wrote   : {out.with_suffix('.json')}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
