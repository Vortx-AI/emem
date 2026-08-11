"""Does production still send what the fixtures say it sends?

A captured fixture is only better than a hand-written one until the responder
changes. This re-runs each fixture's recorded request against the live origin
and fails if a key the fixture claims has stopped arriving, then checks the
handful of paths the tools actually read.

Run against a node of your choosing with `EMEM_URL=http://localhost:5051`.

Honest limit: when the origin cannot be reached these tests skip. A skip is
not a pass. It means the fixtures were not checked on this run, and the offline
suite is back to certifying that the code agrees with itself.
"""

from __future__ import annotations

import functools
import glob
import json
import os

import httpx
import pytest

from llama_index.tools.emem import EmemToolSpec

FIXTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")
ORIGIN = os.environ.get("EMEM_URL", "https://emem.dev").rstrip("/")

DENSE_CELL = "defi.zb493.xuqA.zcb5f"

# The worst case the tool ships: `bands` omitted at the densest cell the
# responder serves. Measured over eleven live places with `bands` omitted,
# Bengaluru worst at 31,990 characters over 104 facts, then Tokyo 13,445 and
# Paris 12,904. One more fact costs about 291 characters, so 40,000 is 25
# percent over the worst measured and roughly 27 more bands of room.
#
# This is asserted live rather than against a fixture because the fixture
# would be the 503 KB body that response is; `capture_fixtures.py` picks its
# queries to stay reviewable in a diff, and half a megabyte is not. The
# offline half of this bound is `FACT_CHAR_BUDGET`, which caps one fact
# whatever the fact count.
#
# If this trips, the question is what the responder started sending, not
# whether the number should go up: 40,000 characters is already more than
# some hosted clients will pass through, and `bands` exists so an agent never
# has to ask for all of it.
RECALL_CHAR_BUDGET = 40_000

# What the four tools read, path by path. A fixture that stopped exercising one
# of these would let it rot unnoticed, so this list is checked against live
# directly rather than inferred from the captures.
READS = {
    # `place_label` is only asked of the ambiguous fixture: production returns
    # null for it on a coordinate lookup, which is honest rather than broken.
    "locate_bengaluru": ["cell64", "centre", "advice",
                         "data_at_this_cell.live_bands_by_topic"],
    "locate_springfield": ["cell64", "place_label", "disambiguation_required",
                           "alternatives", "data_at_this_cell.live_bands_by_topic"],
    "recall_bengaluru": ["facts.band", "facts.value", "facts.unit", "facts.tslot",
                         "facts.cell", "facts.fact_cid", "facts.memory_token",
                         "facts.signed_at", "facts.band_metadata.description",
                         "facts.band_metadata.interpretation", "facts.band_metadata.pitfalls",
                         "receipt", "resolved_from.cell.label"],
    "recall_by_cell": ["facts.cell", "facts.memory_token", "receipt"],
    "recall_embedding": ["facts.value", "facts.memory_token"],
    "resolve_token": ["resolved", "cell", "fact_cid", "signer_b32",
                      "fact.band", "fact.value", "fact.unit", "fact.tslot", "fact.signed_at"],
    "verify_receipt": ["signature_valid", "merkle_proof_valid", "fact_cids_count"],
}

NAMES = sorted(os.path.basename(p)[:-5] for p in glob.glob(os.path.join(FIXTURES, "*.json")))


def _load(name: str) -> dict:
    with open(os.path.join(FIXTURES, f"{name}.json")) as handle:
        return json.load(handle)


def _post(path: str, body: dict) -> dict:
    """One live call, or skip if nothing answers."""
    try:
        response = httpx.post(ORIGIN + path, json=body, timeout=180.0)
        response.raise_for_status()
        return response.json()
    except (httpx.HTTPError, json.JSONDecodeError) as error:
        pytest.skip(f"{ORIGIN} did not answer ({error!r}); shapes NOT verified this run")


def _fetch(capture: dict) -> dict:
    """Replay a recorded request against ORIGIN, or skip if nothing answers."""
    return _post(capture["path"], capture["request"])


def _key_paths(node, prefix: str = ""):
    """Every dotted key path in a body.

    Descends dicts, and lists only through their first element, which is what
    keeps a polygon's 39 KB of coordinates out of the comparison while still
    walking into `facts[0]`.
    """
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "_capture":
                continue
            path = f"{prefix}.{key}" if prefix else key
            yield path
            yield from _key_paths(value, path)
    elif isinstance(node, list) and node:
        yield from _key_paths(node[0], prefix)


def _at(body, path: str):
    """Resolve a dotted path, stepping into the first element of any list."""
    node = body
    for part in path.split("."):
        if isinstance(node, list):
            if not node:
                return None
            node = node[0]
        if not isinstance(node, dict) or part not in node:
            return None
        node = node[part]
    return node


@pytest.mark.parametrize("name", NAMES)
def test_the_fixture_claims_nothing_production_stopped_sending(name):
    """The direction that matters.

    The fixture this replaced invented `resolved_from.cell.cell64`, and the
    suite passed because the code and the fixture shared one wrong assumption.
    Anything the fixture asserts, production has to still send."""
    fixture = _load(name)
    live = _fetch(fixture["_capture"])

    missing = sorted(set(_key_paths(fixture)) - set(_key_paths(live)))
    assert not missing, (
        f"{name}.json claims keys {ORIGIN} no longer sends: {missing}. "
        f"Re-run tests/capture_fixtures.py and check what the tools read."
    )


@pytest.mark.parametrize("name", sorted(READS))
def test_live_still_serves_every_path_the_tools_read(name):
    fixture = _load(name)
    live = _fetch(fixture["_capture"])

    for path in READS[name]:
        assert _at(live, path) is not None, f"{ORIGIN} sends no {path} for {name}"


@functools.lru_cache(maxsize=1)
def _dense_recall() -> dict:
    """One `bands`-less recall at the dense cell, reused by both tests below.

    It is the largest call the package makes, so it is made once."""
    try:
        return EmemToolSpec(base_url=ORIGIN, timeout=300.0).recall(DENSE_CELL)
    except httpx.HTTPError as error:
        pytest.skip(f"{ORIGIN} did not answer ({error!r}); recall size NOT checked this run")


def test_recall_with_bands_omitted_still_fits_a_tool_result():
    """The size that started all of this, measured end to end through the
    shipped tool against the live responder rather than through a fixture."""
    out = _dense_recall()

    size = len(json.dumps(out))
    assert size <= RECALL_CHAR_BUDGET, (
        f"recall({DENSE_CELL!r}) with bands omitted is {size} chars over "
        f"{len(out['facts'])} facts"
    )


def test_every_citation_the_trim_leaves_behind_still_dereferences():
    """The trim drops `cell` and `fact_cid` from a fact because its `cite`
    already carries both. That is only true while the token still resolves, so
    it is checked against production rather than against the grammar: take a
    token out of a trimmed result, hand it back, and require the same signed
    reading.

    A trim that passes every offline test and yields a token nobody can
    dereference is the failure this whole package exists to prevent."""
    out = _dense_recall()
    spec = EmemToolSpec(base_url=ORIGIN, timeout=300.0)

    # A withheld embedding first: it is the fact with the most riding on its
    # citation, since resolving it is the only way to reach the value at all.
    facts = sorted(out["facts"], key=lambda f: "value_omitted" not in f)
    for fact in facts[:3]:
        assert "cell" not in fact and "fact_cid" not in fact, \
            "a cell64 citation stopped spelling out its own halves"
        try:
            resolved = spec.resolve_token(fact["cite"])
        except httpx.HTTPError as error:
            pytest.skip(f"{ORIGIN} did not answer ({error!r}); citations NOT checked")
        assert resolved["resolved"] is True, f"{fact['cite']} did not resolve"
        assert resolved["band"] == fact["band"]
        assert resolved["cell"] == out["cell"]
        assert resolved["fact_cid"] == fact["cite"].rsplit(":", 1)[-1]
        if "value_omitted" in fact:
            assert len(json.dumps(resolved["value"])) == fact["value_omitted"]["chars"], \
                "the withheld value did not come back whole"


def test_a_descriptor_token_still_resolves_and_still_hides_the_cell():
    """The premise for keeping `cell` on a descriptor citation.

    `/v1/memory_token` mints a second grammar for the same fact,
    `emem:fact:<lat>,<lng>@<date>@<band~render>:<fact_cid>`. It resolves to the
    identical record, so the cid still comes off the end, but its anchor is
    coordinates: the responder reaches the cell by quantising them, which the
    client cannot do. That is why the trim compares each half against the token
    instead of assuming both are in there. If this grammar ever started
    carrying the cell64, the comparison would simply stop keeping the field,
    but the reverse, a descriptor whose cell was dropped, is unrecoverable, so
    the premise is checked rather than remembered."""
    fixture = _load("recall_bengaluru")
    fact = fixture["facts"][0]
    minted = _post("/v1/memory_token", {
        "cell": fact["cell"],
        "fact_cid": fact["fact_cid"],
        "band": fact["band"],
        "observed_on": fact["sources"][0]["captured_at"][:10],
    })

    descriptor = minted.get("descriptor_token")
    assert descriptor, "the responder stopped minting descriptor_token"
    assert "@" in descriptor and fact["cell"] not in descriptor, \
        "a descriptor anchor now carries the cell64; re-read _trim_fact"
    assert descriptor.rsplit(":", 1)[-1] == fact["fact_cid"]

    resolved = _post("/v1/memory_token/resolve", {"token": descriptor})
    assert resolved["resolved"] is True
    assert resolved["cell"] == fact["cell"], "the descriptor resolved to another cell"
    assert resolved["fact_cid"] == fact["fact_cid"]


def test_locate_data_at_this_cell_is_still_a_briefing_not_a_band_list():
    """The premise of the `locate` trim. If the responder ever narrows this
    field to an actual band list, the trim should be removed rather than left
    reaching for a key that stopped existing."""
    live = _fetch(_load("locate_bengaluru")["_capture"])
    briefing = live["data_at_this_cell"]

    assert isinstance(briefing, dict), "data_at_this_cell became a list; simplify locate()"
    assert "live_bands_by_topic" in briefing
    assert len(json.dumps(briefing)) > len(json.dumps(briefing["live_bands_by_topic"])) * 3, \
        "the briefing stopped dwarfing the band map; re-measure before keeping the trim"
