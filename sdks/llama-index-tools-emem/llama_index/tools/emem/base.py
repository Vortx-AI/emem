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

The receipt is the exception and is passed through whole. As of receipt
preimage v2 the signature covers the inclusion proof, so a receipt with any
field removed does not fail loudly, it verifies as `signature_valid: false`.
An agent handed a trimmed receipt would report honest data as forged. Saving
1.7 KB is not worth a tool that accuses its own responder.
"""

from __future__ import annotations

import os
from typing import Any, Dict, List, Optional

import httpx
from llama_index.core.tools.tool_spec.base import BaseToolSpec

DEFAULT_BASE_URL = "https://emem.dev"
DEFAULT_TIMEOUT = 60.0


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
        out = {
            "band": fact.get("band"),
            "value": fact.get("value"),
            "unit": fact.get("unit"),
            "tslot": fact.get("tslot"),
            "cell": fact.get("cell"),
            "fact_cid": fact.get("fact_cid"),
            "cite": fact.get("memory_token"),
            "signed_at": fact.get("signed_at"),
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

        Args:
            place: A place name, or "lat,lng".
        """
        body = self._post("/v1/locate", {"q": place})
        return {
            "cell64": body.get("cell64"),
            "place_label": body.get("place_label"),
            "centre": body.get("centre"),
            "disambiguation_required": body.get("disambiguation_required"),
            "alternatives": body.get("alternatives"),
            "bands_available_here": body.get("data_at_this_cell"),
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

        Args:
            place: A place name, "lat,lng", or a cell64 address from `locate`.
            bands: Measurement names, e.g. ["copdem30m.elevation_mean"]. Omit
                to let the responder choose what it holds for this place. Call
                `locate` first if you need to know what is available.
            band_help: Include each band's description, interpretation and
                pitfalls. Off by default because it is verbose; turn it on when
                the band is unfamiliar and you need to interpret the number
                rather than just report it.
        """
        request: Dict[str, Any] = {"place": place}
        if bands:
            request["bands"] = bands
        body = self._post("/v1/recall", request)

        facts = [self._trim_fact(f, band_help) for f in body.get("facts") or []]
        resolved = (body.get("resolved_from") or {}).get("cell") or {}
        return {
            "place_label": resolved.get("label"),
            "cell": resolved.get("cell64") or resolved.get("cell"),
            "facts": facts,
            "cite": [f["cite"] for f in facts if f.get("cite")],
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
        # resolution outcome and the signer.
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
