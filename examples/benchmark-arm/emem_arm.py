#!/usr/bin/env python3
"""The canonical emem arm for memory benchmarks: a closed dereference loop.

WHY THIS FILE EXISTS
--------------------
Third parties benchmark emem against RAG and agent-memory products, and they
implement the emem arm themselves. If they implement it as "hand the model a
token and hope", they measure a loop emem does not claim, and they publish it.
This is the arm emem would defend, in the repo emem owns, so the comparison is
against the design rather than against a guess at it.

THE MEASUREMENT THAT SHAPED IT
------------------------------
The navigatable_worlds agent measured a real dereference arm on 2026-07-20
(n=56, two sites, gemma-4-12B + Qwen2.5-7B, greedy). Against a responder
WITHOUT the fixes below:

    token copied verbatim -> resolved     46/56   82.1%
    of resolves, value copied verbatim    36/46   78.3%
    END-TO-END byte-identical             36/56   64.3%

emem never served a wrong byte. Every loss was in the last mile, between the
signed fact and the model's answer, and nothing verified that step. Two
distinct failures:

  1. HEAD DROPPED. The model keeps the opaque base32 tail and discards the
     `emem:fact:<descriptor>:` head, so nothing resolves (17.9%).
  2. VALUE RETYPED. The model re-emits a JSON float from its own tokenizer,
     `0.2411` for `0.241103` (21.7% of successful resolves).

THE LOOP THAT CLOSES BOTH
-------------------------
    resolve  -> take `value_verbatim`, an exact decimal STRING, not a float
    answer   -> instruct the model to QUOTE that string
    echo     -> POST /v1/echo_verify {token, claimed_value}
    correct  -> on drift, substitute the verbatim value and re-verify

The claim this arm supports is deliberately narrow and is about the OUTPUT,
not the model: a single model pass is not reliable, but a loop that verifies
its own transcription and corrects on mismatch publishes a byte-identical
value every time, because `echo_verify` is the gate. Drift becomes a caught
event rather than a silent wrong number.

Failures it cannot fix are reported, never hidden: if the responder does not
hold the fact, or the model refuses, the arm returns `ok=False` with a reason.
An arm that silently substituted the truth for a model that never produced it
would be measuring emem's HTTP client, not the loop.

Usage:
    from emem_arm import EmemArm
    arm = EmemArm(answer_fn=my_model)          # answer_fn(prompt) -> str
    out = arm.answer(token, "What is the NDVI here?")
    out["value"], out["verified"], out["corrected"], out["drift"]

No dependencies beyond the standard library.
"""
from __future__ import annotations

import json
import re
import urllib.request
from dataclasses import dataclass, field
from typing import Callable, Optional

BASE = "https://emem.dev"


def _post(path: str, body: dict, base: str = BASE, timeout: int = 30) -> dict:
    req = urllib.request.Request(
        base + path,
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


# A value the model emitted. Deliberately permissive about surrounding prose
# and deliberately strict about the number itself: full precision is kept.
_NUM = re.compile(r"-?\d+\.\d+|-?\d+")


def extract_number(text: str, exclude: tuple[str, ...] = ()) -> Optional[str]:
    """First number in `text` that is not one of `exclude`.

    `exclude` exists because of a real scoring bug the worlds agent found: at a
    London site the question's own LONGITUDE (-0.13) is itself a plausible NDVI,
    so naive first-number extraction scored correct answers as wrong. Pass the
    question's coordinates here.
    """
    for m in _NUM.finditer(text or ""):
        tok = m.group(0)
        if tok not in exclude:
            return tok
    return None


@dataclass
class EmemArm:
    """The addressed-memory arm: resolve, quote, verify, correct."""

    answer_fn: Callable[[str], str]
    base: str = BASE
    #: Re-ask the model once with the exact string before substituting it.
    reask_on_drift: bool = True
    stats: dict = field(default_factory=lambda: {
        "n": 0, "resolved": 0, "degraded": 0,
        "verified_first_pass": 0, "verified_after_retry": 0,
        "corrected": 0, "failed": 0,
    })

    # -- step 1 -------------------------------------------------------------
    def resolve(self, token: str) -> dict:
        """Dereference a citation. Accepts a bare cid too (answers degraded)."""
        return _post("/v1/memory_token/resolve", {"token": token}, self.base)

    # -- step 3 -------------------------------------------------------------
    def echo_verify(self, token: str, claimed: str) -> dict:
        """Ask emem whether what we are about to publish matches what it holds."""
        return _post(
            "/v1/echo_verify", {"token": token, "claimed_value": claimed}, self.base
        )

    # -- the loop -----------------------------------------------------------
    def answer(self, token: str, question: str, exclude: tuple[str, ...] = ()) -> dict:
        self.stats["n"] += 1
        try:
            r = self.resolve(token)
        except Exception as exc:  # responder does not hold it, or is unreachable
            self.stats["failed"] += 1
            return {"ok": False, "reason": f"resolve failed: {exc}", "token": token}

        self.stats["resolved"] += 1
        if r.get("degraded"):
            # A bare cid resolved. The bytes are as authoritative as any other
            # resolve; only the CITATION was lossy, so record it and continue.
            self.stats["degraded"] += 1

        truth = r.get("value_verbatim")
        if truth is None:
            self.stats["failed"] += 1
            return {"ok": False, "reason": "fact has no scalar value", "token": token}

        # -- step 2: the instruction is the whole point. Quote, do not retype.
        prompt = (
            f"{question}\n\n"
            f"The signed value is exactly: {truth}\n"
            f"Answer with that value copied EXACTLY, digit for digit. "
            f"Do not round it, reformat it, or write it in your own words."
        )
        raw = self.answer_fn(prompt)
        claimed = extract_number(raw, exclude=exclude) or (raw or "").strip()

        # -- step 3: verify the transcription, not just the retrieval.
        ev = self.echo_verify(token, claimed)
        corrected = False
        # Whether the MODEL got it right unaided. A successful retry is not a
        # first pass, and conflating them would flatter the model at the
        # expense of the measurement.
        first_pass_ok = bool(ev.get("matches"))
        first_drift = ev.get("drift")

        # -- step 4: correct on drift, then re-verify. This is what makes the
        #    PUBLISHED value reliable even when a single pass is not.
        if not ev.get("matches"):
            if self.reask_on_drift:
                raw = self.answer_fn(
                    prompt + f"\n\nYour previous answer did not match "
                    f"({ev.get('drift')}). Reply with only: {truth}"
                )
                claimed = extract_number(raw, exclude=exclude) or (raw or "").strip()
                ev = self.echo_verify(token, claimed)
            if not ev.get("matches"):
                # The model would not produce it. Publish the signed value and
                # SAY the model did not, rather than pretending it did.
                claimed, corrected = truth, True
                ev = self.echo_verify(token, claimed)

        if corrected:
            self.stats["corrected"] += 1
        elif first_pass_ok:
            self.stats["verified_first_pass"] += 1
        elif ev.get("matches"):
            self.stats["verified_after_retry"] += 1

        return {
            "ok": True,
            "token": token,
            "canonical_token": r.get("canonical_token"),
            "fact_cid": r.get("fact_cid"),
            "value": claimed,
            "value_verbatim": truth,
            "verified": bool(ev.get("matches")),
            "drift": first_drift,
            "first_pass_ok": first_pass_ok,
            "corrected": corrected,
            "degraded_citation": bool(r.get("degraded")),
            "model_output": raw,
            "receipt": r.get("receipt"),
        }

    def report(self) -> dict:
        s = dict(self.stats)
        n = max(s["n"], 1)
        # The two numbers that must never be conflated: what the LOOP
        # publishes, and what the MODEL managed unaided.
        s["published_byte_identical"] = (
            s["verified_first_pass"] + s["verified_after_retry"] + s["corrected"]
        ) / n
        s["model_byte_identical_first_pass"] = s["verified_first_pass"] / n
        return s


if __name__ == "__main__":
    # Self-check against the live responder with a stub "model" that rounds,
    # i.e. reproduces the exact failure measured at 21.7%.
    TOKEN = (
        "emem:fact:defi.zb572.xoso.zb1ec:"
        "2p6sz3pv45ndkyqstir4nd6bjnzx63rrcb4pnhgahsnb2oczh5aq"
    )
    calls = {"n": 0}

    def rounding_model(prompt: str) -> str:
        calls["n"] += 1
        # First pass rounds (the real failure); the retry copies correctly.
        return "The NDVI delta is -0.0558." if calls["n"] == 1 else "-0.055822789005725904"

    arm = EmemArm(answer_fn=rounding_model)
    out = arm.answer(TOKEN, "What is the NDVI delta here?")
    print(json.dumps({k: out[k] for k in
                      ("value", "verified", "drift", "corrected")}, indent=1))
    print(json.dumps(arm.report(), indent=1))
