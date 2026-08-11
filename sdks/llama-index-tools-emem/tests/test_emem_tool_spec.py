"""Tests for `EmemToolSpec`.

Everything here runs against `tests/fixtures/`, which holds verbatim response
bodies written by `tests/capture_fixtures.py` and not edited afterwards. The
previous version of this file carried hand-written dicts under a header that
called them real, and that is how two shipped tools stayed broken through a
green suite: `recall()["cell"]` read `resolved_from.cell.cell64`, a key
production does not send, and the fixture supplied it; `locate()` returned
`data_at_this_cell` believing it was a band list, and the fixture said it was
one band name instead of the 19 KB briefing it is.

`test_live_contract.py` is the other half: it re-fetches each fixture's
recorded request from production and fails if a key the fixture claims has
stopped being sent. A fixture nobody re-checks is a test that certifies the
code's assumptions back to itself.
"""

from __future__ import annotations

import json
import os

import httpx
import pytest
from llama_index.core.tools.tool_spec.base import BaseToolSpec

from llama_index.tools.emem import EmemToolSpec

FIXTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")


def fixture(name: str) -> dict:
    """A captured body, minus the capture note the responder never sends."""
    with open(os.path.join(FIXTURES, f"{name}.json")) as handle:
        body = json.load(handle)
    body.pop("_capture", None)
    return body


LOCATE = fixture("locate_bengaluru")
LOCATE_AMBIGUOUS = fixture("locate_springfield")
RECALL = fixture("recall_bengaluru")
RECALL_BY_CELL = fixture("recall_by_cell")
RECALL_EMBEDDING = fixture("recall_embedding")
RESOLVE = fixture("resolve_token")
VERIFY = fixture("verify_receipt")

CELL = "defi.zb493.xuqA.zcb5f"

# Measured over 24 places against https://emem.dev, including both poles, open
# ocean and the five-way ambiguous names. `locate` came back between 3,712 and
# 4,867 characters; the spread is `alternatives`, which the responder caps at
# five entries. 6,000 leaves 23 percent over the worst measured, roughly 25
# more band names at the 45 characters per entry `live_bands_by_topic` costs.
# If this trips, the question to ask is whether the field still earns its place
# in a context window, not whether the number should go up.
LOCATE_CHAR_BUDGET = 6_000

# Measured over 179 live facts at Bengaluru, Paris and Tokyo: the largest
# trimmed fact was 608 characters (`geotessera.multi_year`, whose value is
# withheld) and the smallest 296. Before the value budget the largest was
# 22,872. This bound does not depend on how many facts came back, which is
# what makes it worth asserting.
FACT_CHAR_BUDGET = 800


def _spec(handler) -> EmemToolSpec:
    transport = httpx.MockTransport(handler)
    return EmemToolSpec(client=httpx.Client(transport=transport))


def _serving(*bodies) -> EmemToolSpec:
    """A spec that answers each call with the next body in turn."""
    remaining = iter(bodies)
    return _spec(lambda request: httpx.Response(200, json=next(remaining)))


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


def test_every_fixture_was_captured_rather_than_written():
    """The failure this whole file is a response to. A fixture without a
    capture note is one somebody typed, and a typed fixture proves only that
    the code agrees with itself."""
    for name in ("locate_bengaluru", "locate_springfield", "recall_bengaluru",
                 "recall_by_cell", "recall_embedding", "resolve_token", "verify_receipt"):
        with open(os.path.join(FIXTURES, f"{name}.json")) as handle:
            body = json.load(handle)
        capture = body.get("_capture")
        assert capture, f"{name}.json has no _capture note; re-run tests/capture_fixtures.py"
        assert capture["origin"] and capture["path"] and capture["captured_at"]


def test_recall_returns_the_citation_token():
    out = _serving(RECALL).recall("Bengaluru", bands=["copdem30m.elevation_mean"])

    assert out["facts"][0]["value"] == 918.0
    assert out["facts"][0]["unit"] == "m"
    assert out["cite"] == [
        f"emem:fact:{CELL}:yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala"
    ]
    assert out["receipt"]["signature_b32"]


def test_recall_reports_the_address_it_read():
    """Shipped broken and tested green for the life of the package.

    The code read the cell out of `resolved_from.cell`, which describes how the
    name was matched (label, lat, lng, confidence) and has never carried an
    address. Live, `recall()["cell"]` was None on every call. The hand-written
    fixture put a `cell64` there, so the assertion passed.

    Worse on the second path: pass a cell64 as `place` and production omits
    `resolved_from` altogether, so there was nothing to read even in principle.
    The address is on the facts, which all share it."""
    assert "cell64" not in (RECALL["resolved_from"]["cell"]), \
        "production started sending cell64 under resolved_from; re-read the code before trusting this"
    assert "resolved_from" not in RECALL_BY_CELL, \
        "production started sending resolved_from for a cell64 lookup"

    assert _serving(RECALL).recall("Bengaluru")["cell"] == CELL
    assert _serving(RECALL_BY_CELL).recall(CELL)["cell"] == CELL


def test_recall_trims_band_prose_but_never_the_receipt():
    """A raw recall for one band is about 5 KB, nearly all of it band prose
    written for a human. That goes. The receipt does not.

    The receipt has to survive intact because the signature covers the
    inclusion proof: a receipt missing `merkle_proof` verifies as
    `signature_valid: false` rather than erroring, so a tool that trimmed it
    would report its own honest answers as forged. That was a real bug here,
    caught only by verifying against the live responder, and this is the test
    that stops it coming back."""
    out = _serving(RECALL).recall("Bengaluru")

    assert "pitfalls" not in json.dumps(out), "band prose survived the trim"
    assert out["receipt"] == RECALL["receipt"], "receipt must pass through byte-for-byte"
    # 5,268 characters of response became 2,201 live at the time of capture.
    assert len(json.dumps(out)) < 4_000, "tool result grew past the size the design assumes"


def test_band_help_is_opt_in():
    assert "band_help" not in _serving(RECALL).recall("Bengaluru")["facts"][0]

    helped = _serving(RECALL).recall("Bengaluru", band_help=True)["facts"][0]["band_help"]
    assert helped["pitfalls"].startswith("GLO-30 is a *surface* model")
    # Even with help on, only the three curated fields come through: the real
    # `band_metadata` also carries provenance, references, units and the cube
    # band it was inherited from.
    assert set(helped) == {"description", "interpretation", "pitfalls"}
    assert set(RECALL["facts"][0]["band_metadata"]) > set(helped)


def test_recall_sends_bands_only_when_given():
    seen = {}

    def handler(request: httpx.Request) -> httpx.Response:
        seen.clear()
        seen.update(json.loads(request.content))
        return httpx.Response(200, json=RECALL)

    _spec(handler).recall("Bengaluru")
    assert "bands" not in seen

    _spec(handler).recall("Bengaluru", bands=["a"])
    assert seen["bands"] == ["a"]


def test_an_embedding_value_is_withheld_and_says_so():
    """Seven of the 103 facts a `bands`-less recall returns at Bengaluru are
    embedding vectors, and they were 131,246 of the result's 162,365
    characters. A model cannot read a vector out of a tool result, only cite
    it, so the body goes and the citation stays.

    `value: null` on its own would be read as "no measurement". The note has
    to say the client withheld it."""
    raw = RECALL_EMBEDDING["facts"][0]
    assert len(json.dumps(raw["value"])) > 2_000, "fixture is no longer an embedding"

    fact = _serving(RECALL_EMBEDDING).recall(CELL, bands=["geotessera"])["facts"][0]
    assert fact["value"] is None
    assert fact["value_omitted"]["chars"] == len(json.dumps(raw["value"]))
    assert fact["value_omitted"]["length"] == len(raw["value"])
    assert "withheld by the client" in fact["value_omitted"]["reason"]
    # The point of withholding it: the citation still resolves the whole thing.
    assert fact["cite"] == raw["memory_token"]
    assert fact["band"] == raw["band"]
    # An embedding carries no `unit` at all, which the trimmer reports as None
    # rather than raising. Asserted because a KeyError here would be a crash in
    # the shipped tool, not a test detail.
    assert "unit" not in raw and fact["unit"] is None


def test_a_readable_value_is_left_alone():
    fact = _serving(RECALL).recall("Bengaluru")["facts"][0]
    assert fact["value"] == 918.0
    assert "value_omitted" not in fact


def test_a_fact_has_a_bounded_size():
    """The property the value budget buys: one fact costs about the same
    whatever band it came from, so a caller can reason about the result from
    the fact count. Without it a single `clay_v1` fact was 22,872 characters."""
    for body in (RECALL, RECALL_BY_CELL, RECALL_EMBEDDING):
        for fact in _serving(body).recall("x")["facts"]:
            size = len(json.dumps(fact))
            assert size <= FACT_CHAR_BUDGET, f"{fact['band']} trimmed to {size} chars"


def test_locate_returns_bands_not_a_capability_briefing():
    """Shipped as a 20,058-character tool result.

    `data_at_this_cell` reads like a band list and is not one: it is the
    responder's briefing on what it can do here, and its siblings describe
    algorithm recipes, GPU availability and cube slots with no connector. The
    hand-written fixture set it to `["copdem30m.elevation_mean"]`, so the
    suite never saw the 19 KB."""
    assert isinstance(LOCATE["data_at_this_cell"], dict)
    assert len(json.dumps(LOCATE["data_at_this_cell"])) > 15_000, \
        "the field stopped being a briefing; re-read locate() before trusting this"

    out = _serving(LOCATE).locate("12.971899,77.593665")
    assert out["cell64"] == CELL
    assert out["bands_available_here"] == LOCATE["data_at_this_cell"]["live_bands_by_topic"]
    # The parts of the briefing that are not an answer to "what can I read".
    assert "algorithm_availability" not in json.dumps(out)
    assert "declared_but_no_materializer_at_this_responder" not in json.dumps(out)


def test_locate_takes_a_bare_list_at_its_word():
    """A responder that answers the question directly is not second-guessed."""
    body = dict(LOCATE, data_at_this_cell=["copdem30m.elevation_mean"])
    assert _serving(body).locate("x")["bands_available_here"] == ["copdem30m.elevation_mean"]


def test_locate_result_fits_a_tool_window():
    """`recall` has had a size assertion since it was written and `locate`
    never did, which is the only reason it shipped at five times the size."""
    for body, place in ((LOCATE, "12.971899,77.593665"), (LOCATE_AMBIGUOUS, "Springfield")):
        size = len(json.dumps(_serving(body).locate(place)))
        assert size <= LOCATE_CHAR_BUDGET, f"locate({place!r}) is {size} chars"


def test_locate_surfaces_disambiguation():
    out = _serving(LOCATE_AMBIGUOUS).locate("Springfield")

    assert out["disambiguation_required"] is True
    assert len(out["alternatives"]) == 5
    # 39 KB of coordinates in the captured body, and none of it reaches the model.
    assert len(json.dumps(LOCATE_AMBIGUOUS["polygon_geojson"])) > 20_000
    assert "polygon_geojson" not in json.dumps(out)


def test_verify_receipt_reports_validity():
    out = _serving(VERIFY).verify_receipt(RECALL["receipt"])

    assert out["signature_valid"] is True
    assert out["merkle_proof_valid"] is True
    assert out["fact_cids_count"] == 1
    # The captured body carries 20 further fields; four is what the caller needs.
    assert set(out) == {"signature_valid", "merkle_proof_valid",
                        "merkle_proof_error", "fact_cids_count"}
    assert len(VERIFY) > 20


def test_resolve_token_reads_a_citation_from_elsewhere():
    """The resolved record is nested under `fact`, not returned at the top
    level. Reading the top level looks like it works, every field simply comes
    back None, so this is asserted against the real envelope."""
    out = _serving(RESOLVE).resolve_token(f"emem:fact:{CELL}:cid")

    assert out["resolved"] is True
    assert out["value"] == 918.0
    assert out["unit"] == "m"
    assert out["cell"] == CELL
    assert out["signer_b32"] == RESOLVE["signer_b32"]
    assert "band" not in RESOLVE, "band moved to the top level; the nesting comment is stale"


def test_resolve_token_round_trips_a_recall_citation():
    """A token minted by `recall` resolves back to the same value. This is the
    property the whole package exists to provide, so it is asserted rather than
    assumed. Both bodies are captured, and the capture script resolves the
    token recall actually returned rather than one typed into it."""
    spec = _serving(RECALL, RESOLVE)

    recalled = spec.recall("Bengaluru")
    resolved = spec.resolve_token(recalled["cite"][0])

    assert recalled["cite"][0] == RESOLVE["token"]
    assert resolved["value"] == recalled["facts"][0]["value"]
    assert resolved["cell"] == recalled["cell"]


def test_http_error_is_not_swallowed():
    spec = _spec(lambda request: httpx.Response(400, json={"error": "bad band"}))
    with pytest.raises(httpx.HTTPStatusError):
        spec.recall("Bengaluru", bands=["nope"])


def test_base_url_is_configurable():
    seen = {}

    def handler(request: httpx.Request) -> httpx.Response:
        seen["url"] = str(request.url)
        return httpx.Response(200, json=LOCATE)

    transport = httpx.MockTransport(handler)
    EmemToolSpec(base_url="http://localhost:5051/", client=httpx.Client(transport=transport)).locate("x")
    assert seen["url"] == "http://localhost:5051/v1/locate"
