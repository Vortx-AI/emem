"""Tests for `EmemToolSpec`.

Everything here runs against recorded shapes rather than the network. The
fixtures are trimmed copies of real responses from https://emem.dev, keeping
the fields the tools read and the ones they deliberately drop.
"""

from __future__ import annotations

import json

import httpx
import pytest
from llama_index.core.tools.tool_spec.base import BaseToolSpec

from llama_index.tools.emem import EmemToolSpec

RECALL_RESPONSE = {
    "resolved_from": {"cell": {"cell64": "defi.zb493.xuqA.zcb5f", "label": "Bengaluru, 19 IN"}},
    "facts": [
        {
            "band": "copdem30m.elevation_mean",
            "value": 918.0,
            "unit": "m",
            "tslot": 0,
            "cell": "defi.zb493.xuqA.zcb5f",
            "fact_cid": "yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala",
            "memory_token": "emem:fact:defi.zb493.xuqA.zcb5f:yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala",
            "signed_at": "2026-08-10T00:00:00Z",
            "signer_pubkey_b32": "abc",
            "band_metadata": {
                "description": "Copernicus GLO-30 DEM statistics at the cell.",
                "interpretation": "Default DEM source for emem.",
                "pitfalls": "GLO-30 is a surface model: it includes canopy height.",
                # A field the trimmer must not pass through, standing in for the
                # several kilobytes of documentation a real response carries.
                "unused_prose": "x" * 4000,
            },
        }
    ],
    "receipt": {
        "responder": "https://emem.dev",
        "responder_pubkey_b32": "key",
        "signature_b32": "sig",
        "fact_cids": ["yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala"],
        "served_at": "2026-08-10T00:00:00Z",
        "merkle_proof": {"big": "y" * 2000},
    },
}

LOCATE_RESPONSE = {
    "cell64": "defi.zb493.xuqA.zcb5f",
    "place_label": "Bengaluru, 19 IN",
    "centre": {"lat": 12.97, "lng": 77.59},
    "disambiguation_required": False,
    "alternatives": [],
    "data_at_this_cell": ["copdem30m.elevation_mean"],
    "advice": "Place names map to a single cell.",
    "polygon_geojson": {"dropped": "z" * 3000},
}


def _spec(handler) -> EmemToolSpec:
    transport = httpx.MockTransport(handler)
    return EmemToolSpec(client=httpx.Client(transport=transport))


def test_is_a_tool_spec():
    assert issubclass(EmemToolSpec, BaseToolSpec)
    assert EmemToolSpec.spec_functions == ["locate", "recall", "resolve_token", "verify_receipt"]


def test_every_declared_function_exists_and_is_documented():
    """A LlamaIndex tool's docstring is its contract with the model, so an
    undocumented spec function is a broken tool rather than an untidy one."""
    spec = EmemToolSpec()
    for name in EmemToolSpec.spec_functions:
        fn = getattr(spec, name, None)
        assert callable(fn), f"{name} is declared in spec_functions but not defined"
        assert (fn.__doc__ or "").strip(), f"{name} has no docstring"


def test_to_tool_list_builds():
    tools = EmemToolSpec().to_tool_list()
    assert {t.metadata.name for t in tools} == set(EmemToolSpec.spec_functions)


def test_recall_returns_the_citation_token():
    spec = _spec(lambda request: httpx.Response(200, json=RECALL_RESPONSE))
    out = spec.recall("Bengaluru", bands=["copdem30m.elevation_mean"])

    assert out["facts"][0]["value"] == 918.0
    assert out["facts"][0]["unit"] == "m"
    assert out["cell"] == "defi.zb493.xuqA.zcb5f"
    # The whole point of the tool: the agent is handed something to quote.
    assert out["cite"] == [
        "emem:fact:defi.zb493.xuqA.zcb5f:yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala"
    ]
    assert out["receipt"]["signature_b32"] == "sig"


def test_recall_trims_band_prose_but_never_the_receipt():
    """A raw recall for one band is about 5 KB, nearly all of it band prose
    written for a human. That goes. The receipt does not.

    The receipt has to survive intact because the signature covers the
    inclusion proof: a receipt missing `merkle_proof` verifies as
    `signature_valid: false` rather than erroring, so a tool that trimmed it
    would report its own honest answers as forged. That was a real bug here,
    caught only by verifying against the live responder, and this is the test
    that stops it coming back."""
    spec = _spec(lambda request: httpx.Response(200, json=RECALL_RESPONSE))
    out = spec.recall("Bengaluru")

    assert "unused_prose" not in json.dumps(out)
    assert out["receipt"] == RECALL_RESPONSE["receipt"], "receipt must pass through byte-for-byte"
    assert len(json.dumps(out)) < 4000, "tool result grew past the size the design assumes"


def test_band_help_is_opt_in():
    spec = _spec(lambda request: httpx.Response(200, json=RECALL_RESPONSE))

    assert "band_help" not in spec.recall("Bengaluru")["facts"][0]

    helped = spec.recall("Bengaluru", band_help=True)["facts"][0]["band_help"]
    assert helped["pitfalls"].startswith("GLO-30 is a surface model")
    # Even with help on, only the three curated fields come through.
    assert set(helped) == {"description", "interpretation", "pitfalls"}


def test_recall_sends_bands_only_when_given():
    seen = {}

    def handler(request: httpx.Request) -> httpx.Response:
        seen.update(json.loads(request.content))
        return httpx.Response(200, json=RECALL_RESPONSE)

    _spec(handler).recall("Bengaluru")
    assert "bands" not in seen

    _spec(handler).recall("Bengaluru", bands=["a"])
    assert seen["bands"] == ["a"]


def test_locate_surfaces_disambiguation():
    ambiguous = dict(LOCATE_RESPONSE, disambiguation_required=True,
                     alternatives=[{"label": "Springfield, IL"}, {"label": "Springfield, MA"}])
    spec = _spec(lambda request: httpx.Response(200, json=ambiguous))
    out = spec.locate("Springfield")

    assert out["disambiguation_required"] is True
    assert len(out["alternatives"]) == 2
    assert "polygon_geojson" not in json.dumps(out)


def test_verify_receipt_reports_validity():
    spec = _spec(lambda request: httpx.Response(200, json={
        "signature_valid": True, "merkle_proof_valid": True,
        "merkle_proof_error": None, "fact_cids_count": 1, "noise": "n" * 500,
    }))
    out = spec.verify_receipt(RECALL_RESPONSE["receipt"])

    assert out["signature_valid"] is True
    assert out["fact_cids_count"] == 1
    assert "noise" not in out


def test_resolve_token_reads_a_citation_from_elsewhere():
    """The resolved record is nested under `fact`, not returned at the top
    level. Reading the top level looks like it works — every field simply
    comes back None — so this fixture mirrors the real envelope."""
    spec = _spec(lambda request: httpx.Response(200, json={
        "resolved": True,
        "cell": "defi.zb493.xuqA.zcb5f",
        "fact_cid": "cid",
        "signer_b32": "777er3yihgifqmv5hmc2",
        "value_verbatim": 918.0,
        "fact": {
            "band": "copdem30m.elevation_mean", "value": 918.0, "unit": "m",
            "tslot": 0, "signed_at": "2026-05-28T19:54:32Z",
        },
    }))
    out = spec.resolve_token("emem:fact:defi.zb493.xuqA.zcb5f:cid")
    assert out["resolved"] is True
    assert out["value"] == 918.0
    assert out["unit"] == "m"
    assert out["signer_b32"] == "777er3yihgifqmv5hmc2"


def test_resolve_token_round_trips_a_recall_citation():
    """A token minted by `recall` resolves back to the same value. This is the
    property the whole package exists to provide, so it is asserted rather than
    assumed."""
    responses = iter([RECALL_RESPONSE, {
        "resolved": True, "cell": "defi.zb493.xuqA.zcb5f",
        "fact_cid": "yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala",
        "signer_b32": "sig",
        "fact": {"band": "copdem30m.elevation_mean", "value": 918.0, "unit": "m"},
    }])
    spec = _spec(lambda request: httpx.Response(200, json=next(responses)))

    recalled = spec.recall("Bengaluru")
    resolved = spec.resolve_token(recalled["cite"][0])
    assert resolved["value"] == recalled["facts"][0]["value"]


def test_http_error_is_not_swallowed():
    spec = _spec(lambda request: httpx.Response(400, json={"error": "bad band"}))
    with pytest.raises(httpx.HTTPStatusError):
        spec.recall("Bengaluru", bands=["nope"])


def test_base_url_is_configurable():
    seen = {}

    def handler(request: httpx.Request) -> httpx.Response:
        seen["url"] = str(request.url)
        return httpx.Response(200, json=LOCATE_RESPONSE)

    transport = httpx.MockTransport(handler)
    EmemToolSpec(base_url="http://localhost:5051/", client=httpx.Client(transport=transport)).locate("x")
    assert seen["url"] == "http://localhost:5051/v1/locate"
