"""`EmemToolSpec`: LlamaIndex tools over the emem responder.

Four tools, in the order an agent uses them: ground a place, read the signed
facts there, resolve a citation someone handed you, check a receipt.

What separates this from any other place-data tool is what comes back with the
number. Every value carries a `fact_cid` and an `emem:fact:` token, and the
token resolves to the byte-identical signed record later, in another session or
another process, verifiable without calling back here. So the agent is told, in
each docstring, to quote the token rather than the number: a number in prose is
indistinguishable from a number someone made up, and the token is not.

Responses are trimmed, with one exception. A raw `/v1/recall` for a single band
is about 5 KB, most of it band documentation written for a human reading the
API; putting that in an agent's context on every call buys nothing and crowds
out the conversation. So facts come back as value, unit, address and citation,
and `band_help=True` pulls the prose back in when the agent needs to interpret
an unfamiliar band rather than just report it.

Two fields are trimmed harder than the rest because measurement said so, and
both numbers are from https://emem.dev rather than from a fixture:

`locate`'s `data_at_this_cell` is not a band list. It is a 19 KB briefing on
what the responder can do, of which `live_bands_by_topic` (2,951 chars, the
same on every cell sampled) is the part that answers "what can I read here".
Passing the whole thing through made a 20,058-character tool result; passing
the topic map makes 3,712.

A fact's `value` is normally under 100 characters, but the embedding bands
return vectors: `clay_v1` alone is 22,581 characters of floats. A recall at
Bengaluru with `bands` omitted returned 103 facts, and seven embedding vectors
were 131,246 of its 162,365 characters. No agent reads a 768-float vector out
of a tool result; it can only cite it. So a value over `VALUE_CHAR_BUDGET` is
replaced by its length, its first few elements and the citation that resolves
the whole thing, and the fact keeps every field needed to quote it.

What is left after that is mostly citations, and citations repeat themselves.
A `bands`-less recall at Bengaluru came back at 51,637 characters, of which
19,456 were the same strings written twice: the top-level `cite` list is every
per-fact `cite` again (9,160), and `fact_cid` and `cell` are the two halves of
the `emem:fact:<cell>:<fact_cid>` token sitting beside them (6,968 and 3,328).
None of that is dropped content, so all three go: 51,637 characters became
31,971, which is 19,666 once the punctuation that held the fields goes with
them.

Dropping a citation's parts is only safe while the token still yields them, so
neither is dropped on faith. `fact_cid` is the token's last colon-separated
segment in both grammars, the `cell64` anchor and the descriptor anchor alike,
because neither anchor may contain a colon. `cell` is only the anchor in the
cell64 form: a descriptor anchor is `<lat>,<lng>@<date>@<band>` and reaches the
cell by quantisation, not by string surgery. So each field is compared against
what the token yields and carried whenever the token does not yield it, which
covers a descriptor citation, a missing token, and any future grammar this
client has not been taught.

The receipt is the exception and is passed through whole, even though its
`fact_cids` array is a third copy of every cid and the largest single block
left (5,837 of the 7,480 characters the receipt costs at Bengaluru). As of
receipt preimage v2 the signature covers the inclusion proof, so a receipt with
any field removed does not fail loudly, it verifies as `signature_valid: false`.
An agent handed a trimmed receipt would report honest data as forged. Saving
5.8 KB is not worth a tool that accuses its own responder.
"""

from __future__ import annotations

import json
import os
from typing import Any, Dict, List, Optional

import httpx
from llama_index.core.tools.tool_spec.base import BaseToolSpec

DEFAULT_BASE_URL = "https://emem.dev"
DEFAULT_TIMEOUT = 60.0

# Where a readable value stops and a vector begins. Measured at Bengaluru over
# the 103 facts a `bands`-less recall returns: the largest scalar or short
# array was 74 characters (`geotessera.bin128`), the smallest embedding was
# 2,535 (`geotessera`). 512 sits in that gap with room on both sides, so a
# monthly series stays readable and a 768-float vector does not.
VALUE_CHAR_BUDGET = 512


def _token_parts(token: Any) -> Optional[tuple]:
    """Split `emem:fact:<anchor>:<fact_cid>` into `(anchor, fact_cid)`.

    Returns None for anything this client cannot read with certainty, which is
    what makes it safe to drop a field the token is supposed to carry: an
    unrecognised token yields nothing, so nothing is dropped.

    The last colon always separates the two halves. Both anchor grammars
    forbid a colon: a `cell64` is four `.`-separated bigrams, and a descriptor
    is `<lat>,<lng>@<date>@<band~render>` over digits, `-`, `.`, `,`, `@` and
    `~`. The responder's own parser splits the same way.
    """
    if not isinstance(token, str):
        return None
    for prefix in ("emem:fact:", "memt:"):
        if token.startswith(prefix):
            anchor, separator, fact_cid = token[len(prefix):].rpartition(":")
            if separator and anchor and fact_cid:
                return anchor, fact_cid
            return None
    return None


class EmemToolSpec(BaseToolSpec):
    """Signed, content-addressed facts about physical places.

    Reads need no key and no account, so the default construction talks to the
    public responder at https://emem.dev. Point `base_url` at your own node to
    use it instead; a receipt minted on one verifies against the other.
    """

    spec_functions = ["locate", "recall", "resolve_token", "verify_receipt"]

    def __init__(
        self,
        base_url: Optional[str] = None,
        timeout: float = DEFAULT_TIMEOUT,
        client: Optional[httpx.Client] = None,
    ) -> None:
        self.base_url = (base_url or os.environ.get("EMEM_URL") or DEFAULT_BASE_URL).rstrip("/")
        self.timeout = timeout
        self._client = client

    def _post(self, path: str, body: Dict[str, Any]) -> Dict[str, Any]:
        if self._client is not None:
            response = self._client.post(f"{self.base_url}{path}", json=body, timeout=self.timeout)
        else:
            response = httpx.post(f"{self.base_url}{path}", json=body, timeout=self.timeout)
        response.raise_for_status()
        return response.json()

    @staticmethod
    def _trim_fact(fact: Dict[str, Any], band_help: bool) -> Dict[str, Any]:
        """One fact, reduced to what an agent needs to answer and to cite."""
        token = fact.get("memory_token")
        parts = _token_parts(token)
        anchor, token_cid = parts if parts else (None, None)
        out: Dict[str, Any] = {
            "band": fact.get("band"),
            "value": fact.get("value"),
            "unit": fact.get("unit"),
            "tslot": fact.get("tslot"),
        }
        # `cell` and `fact_cid` are the token's two halves. Written out beside
        # it they were 10,296 of a 51,637-character result at Bengaluru, so
        # they are carried only when `cite` does not already spell them out:
        # a descriptor token anchors on coordinates rather than the cell, a
        # fact can arrive with a cid and no token at all, and a grammar this
        # client cannot parse yields neither. Comparing rather than assuming
        # is what keeps those cases intact.
        if fact.get("cell") is not None and fact.get("cell") != anchor:
            out["cell"] = fact["cell"]
        if fact.get("fact_cid") is not None and fact.get("fact_cid") != token_cid:
            out["fact_cid"] = fact["fact_cid"]
        out["cite"] = token
        out["signed_at"] = fact.get("signed_at")
        # An embedding is a citation, not a reading. Withhold the body and say
        # so, explicitly enough that a model reports "the tool did not give me
        # the vector" rather than "the value is null".
        encoded = json.dumps(out["value"], default=str)
        if len(encoded) > VALUE_CHAR_BUDGET:
            value = out["value"]
            out["value"] = None
            out["value_omitted"] = {
                "reason": (
                    "this band's value is too large for a tool result and was "
                    "withheld by the client, not missing from the record; "
                    "resolve `cite` to read it in full"
                ),
                "chars": len(encoded),
                "length": len(value) if isinstance(value, (list, str)) else None,
                "head": value[:4] if isinstance(value, list) else None,
            }
        if band_help:
            metadata = fact.get("band_metadata") or {}
            out["band_help"] = {
                "description": metadata.get("description"),
                "interpretation": metadata.get("interpretation"),
                "pitfalls": metadata.get("pitfalls"),
            }
        return out

    def locate(self, place: str) -> Dict[str, Any]:
        """Resolve a place name to its canonical emem address, and report which
        measurements are readable there.

        Call this first when you have a name rather than a cell. The returned
        `cell64` is the address every agent resolves the same name to, so two
        agents grounded on it are talking about the same ~10 m square rather
        than two paraphrases of a location.

        Check `disambiguation_required`: when it is true the name matched more
        than one place, and `alternatives` lists them. Ask the user which one
        they meant instead of guessing.

        `bands_available_here` maps a topic to the bands recallable at this
        cell, so pick the topic that matches the question and pass those names
        to `recall`.

        Args:
            place: A place name, or "lat,lng".
        """
        body = self._post("/v1/locate", {"q": place})
        # `data_at_this_cell` is a capability briefing, not a band list: on
        # every cell sampled it is a 19 KB object whose siblings describe
        # algorithm recipes, GPU availability and unmaterialised cube slots.
        # Only `live_bands_by_topic` answers the question this tool asks.
        # A responder that sends a bare list is taken at its word.
        inventory = body.get("data_at_this_cell")
        if isinstance(inventory, dict):
            inventory = inventory.get("live_bands_by_topic")
        return {
            "cell64": body.get("cell64"),
            "place_label": body.get("place_label"),
            "centre": body.get("centre"),
            "disambiguation_required": body.get("disambiguation_required"),
            "alternatives": body.get("alternatives"),
            "bands_available_here": inventory,
            "advice": body.get("advice"),
        }

    def recall(
        self,
        place: str,
        bands: Optional[List[str]] = None,
        band_help: bool = False,
    ) -> Dict[str, Any]:
        """Read signed measurements at a place, with a citation for each.

        Every returned fact carries a `cite` field holding an `emem:fact:`
        token. **Quote that token in your answer alongside the value.** It
        resolves to the byte-identical signed record for anyone who receives
        it, which a number written into prose does not: keep the token and the
        claim stays checkable after this conversation is compacted or handed to
        another model.

        The token is `emem:fact:<cell>:<fact_cid>`, so it already spells out
        both. That is why a fact normally shows no `cell` and no `fact_cid`
        beside its `cite`: they are the token's own halves, not missing fields.
        When one of them does appear, the token did not carry it and the field
        is the only copy.

        Args:
            place: A place name, "lat,lng", or a cell64 address from `locate`.
            bands: Measurement names, e.g. ["copdem30m.elevation_mean"]. Name
                the ones the question needs. Omitting this returns everything
                the responder holds at the cell, which at Bengaluru is 104
                facts and about 32 KB, nearly all of it citations for bands
                nobody asked about. Call `locate` first and pick from
                `bands_available_here` rather than omitting this.
            band_help: Include each band's description, interpretation and
                pitfalls. Off by default because it is verbose; turn it on when
                the band is unfamiliar and you need to interpret the number
                rather than just report it.
        """
        request: Dict[str, Any] = {"place": place}
        if bands:
            request["bands"] = bands
        body = self._post("/v1/recall", request)

        raw_facts = body.get("facts") or []
        facts = [self._trim_fact(f, band_help) for f in raw_facts]
        # `resolved_from.cell` describes how the name was matched: label, lat,
        # lng, confidence. It carries no address, and it is absent entirely
        # when `place` was already a cell64. The address lives on the facts,
        # which all share it. Reading it off `resolved_from` returned None on
        # every live call while the fixture supplied a `cell64` there.
        #
        # Read off the untrimmed facts, because the trim drops a `cell` the
        # fact's own token already carries.
        resolved = (body.get("resolved_from") or {}).get("cell") or {}
        cell = next((f["cell"] for f in raw_facts if f.get("cell")), None)
        # There is no top-level `cite` list. It was every fact's `cite` a
        # second time, 9,160 characters of a 51,637-character result at
        # Bengaluru, and a list of tokens detached from the readings they cite
        # is the harder of the two to quote correctly anyway.
        return {
            "place_label": resolved.get("label"),
            "cell": cell or resolved.get("cell64") or resolved.get("cell"),
            "facts": facts,
            # Passed through whole, and it must stay that way. The signature
            # covers the inclusion proof as of receipt preimage v2, so a
            # receipt missing any field does not fail loudly: it verifies as
            # `signature_valid: false`, which reads as "this data was tampered
            # with" rather than "the caller dropped a field". Trimming this to
            # save context would make the tool report forgery on its own
            # honest answers.
            "receipt": body.get("receipt") or {},
        }

    def resolve_token(self, token: str) -> Dict[str, Any]:
        """Resolve an `emem:fact:` token back to the record it names.

        Use this when a token arrives from somewhere else: another agent, an
        earlier session, a document. It returns the same bytes the token was
        minted over, so you can read a cited value rather than trusting whoever
        quoted it.

        Args:
            token: A full token, "emem:fact:<cell64>:<fact_cid>".
        """
        body = self._post("/v1/memory_token/resolve", {"token": token})
        # The record sits under `fact`; the envelope around it carries the
        # resolution outcome and the signer. The envelope also echoes `band`,
        # `kind`, `unit` and `value` at the top level for callers who reached
        # for them there, but they are copies of the same JSON nodes. Read the
        # signed body, so a value this tool reports is one the signature
        # covers.
        fact = body.get("fact") or {}
        return {
            "resolved": body.get("resolved"),
            "band": fact.get("band"),
            "value": fact.get("value"),
            "unit": fact.get("unit"),
            "tslot": fact.get("tslot"),
            "cell": body.get("cell"),
            "fact_cid": body.get("fact_cid"),
            "signed_at": fact.get("signed_at"),
            "signer_b32": body.get("signer_b32"),
        }

    def verify_receipt(self, receipt: Dict[str, Any]) -> Dict[str, Any]:
        """Check a receipt's signature against the responder's published key.

        Pass the `receipt` object from a `recall` result. `signature_valid`
        true means the responder really signed those bytes.

        One limit worth stating when you report the result: this proves what a
        single responder signed, not that the measurement is correct and not
        that any network agreed with it.

        Args:
            receipt: The receipt object returned by `recall`.
        """
        body = self._post("/v1/verify_receipt", {"receipt": receipt})
        return {
            "signature_valid": body.get("signature_valid"),
            "merkle_proof_valid": body.get("merkle_proof_valid"),
            "merkle_proof_error": body.get("merkle_proof_error"),
            "fact_cids_count": body.get("fact_cids_count"),
        }
