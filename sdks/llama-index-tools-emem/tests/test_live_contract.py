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

import glob
import json
import os

import httpx
import pytest

FIXTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")
ORIGIN = os.environ.get("EMEM_URL", "https://emem.dev").rstrip("/")

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


def _fetch(capture: dict) -> dict:
    """Replay a recorded request against ORIGIN, or skip if nothing answers."""
    try:
        response = httpx.post(ORIGIN + capture["path"], json=capture["request"], timeout=180.0)
        response.raise_for_status()
        return response.json()
    except (httpx.HTTPError, json.JSONDecodeError) as error:
        pytest.skip(f"{ORIGIN} did not answer ({error!r}); fixture shapes NOT verified this run")


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
