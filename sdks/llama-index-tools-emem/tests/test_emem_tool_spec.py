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

# Read off the capture rather than typed in. The elevation at this cell was
# 918.0 from `open_meteo_copdem90m@1` and, between two captures half an hour
# apart, 915.0712280273438 from `copernicus_dem_30m_aws_pixel@1`: production
# re-materialised the band from a better source and re-signed the fact. A
# literal here asserts what Bengaluru's elevation is, which is the responder's
# job and not this package's; what these tests owe is that the number and its
# citation cross the trim unaltered.
ELEVATION = RECALL["facts"][0]["value"]
ELEVATION_TOKEN = RECALL["facts"][0]["memory_token"]

# Measured over 24 places against https://emem.dev, including both poles, open
# ocean and the five-way ambiguous names. `locate` came back between 3,712 and
# 4,867 characters; the spread is `alternatives`, which the responder caps at
# five entries. 6,000 leaves 23 percent over the worst measured, roughly 25
# more band names at the 45 characters per entry `live_bands_by_topic` costs.
# If this trips, the question to ask is whether the field still earns its place
# in a context window, not whether the number should go up.
LOCATE_CHAR_BUDGET = 6_000

# Measured over 332 live facts at eleven places, from two-fact Jakarta to
# 104-fact Bengaluru: the largest trimmed fact was 507 characters
# (`geotessera.multi_year`, whose value is withheld) and the smallest 195.
# Before the value budget the largest was 22,872; before `cell` and `fact_cid`
# stopped being written out beside the token that already carries them, 608.
# This bound does not depend on how many facts came back, which is what makes
# it worth asserting.
FACT_CHAR_BUDGET = 600


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

    assert out["facts"][0]["value"] == ELEVATION
    assert out["facts"][0]["unit"] == "m"
    assert out["facts"][0]["cite"] == ELEVATION_TOKEN
    assert out["facts"][0]["cite"].startswith(f"emem:fact:{CELL}:")
    assert out["receipt"]["signature_b32"]


def test_there_is_no_top_level_citation_list():
    """It was every fact's `cite` written a second time: 9,160 characters of a
    51,637-character result at Bengaluru, for no information. Removed rather
    than kept as a convenience, because a list of tokens detached from the
    readings they cite is the harder of the two to quote correctly."""
    out = _serving(RECALL).recall("Bengaluru")

    assert "cite" not in out
    assert [f["cite"] for f in out["facts"]] == [ELEVATION_TOKEN]


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
    # 5,300 characters of captured response become 2,014. The receipt is 1,710
    # of what is left, which is the floor: it cannot be trimmed without making
    # it verify false, and its `fact_cids` array is a second copy of every cid
    # the facts already cite.
    assert len(json.dumps(out)) < 3_000, "tool result grew past the size the design assumes"


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


def test_a_fact_does_not_repeat_what_its_own_token_says():
    """`cell` and `fact_cid` are the two halves of `emem:fact:<cell>:<cid>`.
    Written out beside it they cost 10,296 characters of a 51,637-character
    result at Bengaluru and said nothing the token had not.

    The thing that must survive is the reverse direction, so it is asserted
    here rather than trusted: both halves come back out of the token."""
    raw = RECALL["facts"][0]
    fact = _serving(RECALL).recall("Bengaluru")["facts"][0]

    assert "cell" not in fact and "fact_cid" not in fact
    assert fact["cite"] == raw["memory_token"]

    anchor, cid = fact["cite"][len("emem:fact:"):].rsplit(":", 1)
    assert anchor == raw["cell"]
    assert cid == raw["fact_cid"]


def test_the_cid_is_kept_when_the_token_does_not_yield_it():
    """The trim is a comparison, not an assumption.

    Two token grammars resolve to the same fact. Both end in `fact_cid` after
    the last colon, so the cid always comes back. Only the cell64 one spells
    the cell out: a descriptor anchors on `<lat>,<lng>@<date>@<band~render>`
    and reaches the cell by quantisation, which no string surgery here can do.
    So a descriptor citation keeps its `cell` and drops its `fact_cid`, and
    `test_a_descriptor_token_still_resolves_and_still_hides_the_cell` in
    test_live_contract.py is the half that checks production agrees.

    The descriptor is built from the fixture's own fact so it cannot go stale
    against a re-signed one."""
    raw = RECALL["facts"][0]
    descriptor = (f"emem:fact:12.97190,77.59366"
                  f"@{raw['sources'][0]['captured_at'][:10]}"
                  f"@{raw['band'].replace('_', '~').replace('.', '~')}"
                  f":{raw['fact_cid']}")

    fact = _serving(dict(RECALL, facts=[dict(raw, memory_token=descriptor)])) \
        .recall("Bengaluru")["facts"][0]
    assert "fact_cid" not in fact, "the descriptor grammar still ends in the cid"
    assert fact["cell"] == CELL, "a descriptor anchor is not the cell; keep the cell"

    # A fact the responder cid'd but never tokenised. `enrich_facts_with_cid`
    # skips a fact that already carries a `fact_cid`, so it mints no token for
    # it, and dropping the cid there would drop the only copy.
    untokenised = dict(RECALL["facts"][0])
    untokenised.pop("memory_token")
    fact = _serving(dict(RECALL, facts=[untokenised])).recall("Bengaluru")["facts"][0]
    assert fact["cite"] is None
    assert fact["fact_cid"] == RECALL["facts"][0]["fact_cid"]
    assert fact["cell"] == CELL


def test_a_readable_value_is_left_alone():
    fact = _serving(RECALL).recall("Bengaluru")["facts"][0]
    assert fact["value"] == ELEVATION
    assert "value_omitted" not in fact
    assert len(json.dumps(fact["value"])) <= 512


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
    """The resolved record is nested under `fact`, and that is what the tool
    reads. Reading only the top level used to look like it worked, every field
    simply came back None, so this is asserted against the real envelope."""
    out = _serving(RESOLVE).resolve_token(f"emem:fact:{CELL}:cid")

    assert out["resolved"] is True
    assert out["value"] == ELEVATION
    assert out["unit"] == "m"
    assert out["cell"] == CELL
    assert out["signer_b32"] == RESOLVE["signer_b32"]


def test_resolve_reads_the_signed_body_even_though_the_envelope_now_echoes_it():
    """Production started echoing `band`, `kind`, `unit` and `value` at the top
    level of the resolve envelope, between two fixture captures half an hour
    apart. The tool keeps reading them out of `fact`, which is the signed
    record itself; the echo is a copy of the same JSON nodes and is convenient
    rather than authoritative. Asserted so a future drift between the two is a
    failure here rather than a value the tool reports from the wrong copy."""
    for key in ("band", "unit", "value"):
        assert key in RESOLVE, f"the envelope stopped echoing {key}"
        assert RESOLVE[key] == RESOLVE["fact"][key], \
            f"{key} disagrees between the envelope and the signed body"


def test_resolve_token_round_trips_a_recall_citation():
    """A token minted by `recall` resolves back to the same value. This is the
    property the whole package exists to provide, so it is asserted rather than
    assumed. Both bodies are captured, and the capture script resolves the
    token recall actually returned rather than one typed into it."""
    spec = _serving(RECALL, RESOLVE)

    recalled = spec.recall("Bengaluru")
    resolved = spec.resolve_token(recalled["facts"][0]["cite"])

    assert recalled["facts"][0]["cite"] == RESOLVE["token"]
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
