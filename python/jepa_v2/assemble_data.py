"""Assemble Tessera-vintage training data from a live emem responder.

Walks /v1/coverage_matrix to find cells with attested geotessera.YYYY
facts, fetches their 128-D vectors via /v1/recall, and saves a numpy
archive train.py consumes.

Usage:
    python assemble_data.py [--base-url URL] [--out PATH]

Defaults read from env:
    EMEM_BASE_URL  (default https://emem.dev)
    EMEM_DATA      (default /home/ubuntu/emem/var/emem)
"""

import argparse
import json
import os
import sys
import time
from collections import defaultdict
from pathlib import Path

import numpy as np
import requests

TESSERA_YEARS = list(range(2017, 2025))  # 2017..2024 inclusive
TESSERA_DIM = 128


def coverage_matrix(base_url: str, page_size: int = 200) -> list[dict]:
    """Walk /v1/coverage_matrix paginated. Returns a flat list of band entries."""
    out = []
    page = 1
    while True:
        r = requests.get(
            f"{base_url}/v1/coverage_matrix",
            params={"page": page, "page_size": page_size},
            timeout=30,
        )
        r.raise_for_status()
        body = r.json()
        out.extend(body.get("bands", []))
        pag = body.get("pagination", {})
        if not pag.get("has_next"):
            break
        page += 1
    return out


def cells_with_band(base_url: str, band: str) -> list[str]:
    """Best-effort cell enumeration for a band. Some emem deployments
    surface this; if not, we fall back to a /v1/recall_polygon over a
    representative grid."""
    # No direct enumeration endpoint today; use coverage_matrix's
    # facts_count to sanity-check we have data. Caller seeds cells.
    r = requests.get(f"{base_url}/v1/coverage_matrix", params={"band": band}, timeout=30)
    r.raise_for_status()
    return []  # Fallback: assemble.py caller seeds explicit cell list.


def fetch_vector(base_url: str, cell: str, band: str, tslot: int = 0) -> np.ndarray | None:
    """Fetch one Tessera vector via /v1/recall. Returns float32[128] or None."""
    r = requests.post(
        f"{base_url}/v1/recall",
        json={"cell": cell, "bands": [band], "tslot": tslot},
        timeout=60,
    )
    if r.status_code != 200:
        return None
    body = r.json()
    facts = body.get("facts", [])
    if not facts:
        return None
    fact = facts[0]
    if fact.get("kind") != "primary":
        return None
    val = fact.get("value")
    if not isinstance(val, list) or len(val) != TESSERA_DIM:
        return None
    try:
        return np.array(val, dtype=np.float32)
    except (TypeError, ValueError):
        return None


def assemble(base_url: str, cells: list[str], out_path: Path) -> dict:
    """Pull (cell, year) → vector for every (cell, year) in TESSERA_YEARS,
    filter to cells with at least 4 consecutive years (the minimum a
    [t-2, t-1, t] → [t+1] dynamics model needs to produce one valid
    training pair), and save."""
    vectors = {}  # (cell, year) → vec
    print(f"fetching geotessera vintages for {len(cells)} candidate cells", flush=True)
    t0 = time.time()
    n_calls = 0
    for cell in cells:
        for year in TESSERA_YEARS:
            band = f"geotessera.{year}"
            v = fetch_vector(base_url, cell, band)
            n_calls += 1
            if v is not None:
                vectors[(cell, year)] = v
            if n_calls % 50 == 0:
                print(
                    f"  ... {n_calls} fetches, "
                    f"{len(vectors)} vectors landed, "
                    f"elapsed {time.time() - t0:.1f}s",
                    flush=True,
                )
    print(f"done fetching: {n_calls} calls, {len(vectors)} vectors in {time.time() - t0:.1f}s", flush=True)

    # Bucket by cell, find cells with the longest consecutive run.
    by_cell: dict[str, dict[int, np.ndarray]] = defaultdict(dict)
    for (cell, year), v in vectors.items():
        by_cell[cell][year] = v

    keep_cells: list[str] = []
    keep_arrays: list[np.ndarray] = []  # shape [num_kept_years, 128]
    keep_year_ranges: list[tuple[int, int]] = []
    for cell, year_vecs in by_cell.items():
        years = sorted(year_vecs.keys())
        if len(years) < 4:
            continue
        # Find longest consecutive run.
        best_start = years[0]
        best_end = years[0]
        cur_start = years[0]
        for i in range(1, len(years)):
            if years[i] == years[i - 1] + 1:
                if years[i] - cur_start > best_end - best_start:
                    best_start, best_end = cur_start, years[i]
            else:
                cur_start = years[i]
        if best_end - best_start + 1 < 4:
            continue
        run = list(range(best_start, best_end + 1))
        arr = np.stack([year_vecs[y] for y in run], axis=0)
        keep_cells.append(cell)
        keep_arrays.append(arr)
        keep_year_ranges.append((best_start, best_end))

    print(f"kept {len(keep_cells)} cells with ≥4 consecutive vintages", flush=True)
    # Pad to a common length so we can stack into one numpy array.
    max_run = max((arr.shape[0] for arr in keep_arrays), default=0)
    if max_run == 0:
        raise RuntimeError(
            "No cells have a 4+ consecutive Tessera vintage run on this responder. "
            "Materialize more cells (call /v1/recall geotessera.YYYY for a wider range) "
            "and re-run."
        )
    padded = np.zeros((len(keep_arrays), max_run, TESSERA_DIM), dtype=np.float32)
    mask = np.zeros((len(keep_arrays), max_run), dtype=bool)
    for i, (arr, (start, end)) in enumerate(zip(keep_arrays, keep_year_ranges)):
        n = arr.shape[0]
        padded[i, :n] = arr
        mask[i, :n] = True

    out_path.parent.mkdir(parents=True, exist_ok=True)
    np.savez(
        out_path,
        cells=np.array(keep_cells, dtype=object),
        year_starts=np.array([s for s, _ in keep_year_ranges], dtype=np.int32),
        year_ends=np.array([e for _, e in keep_year_ranges], dtype=np.int32),
        vectors=padded,
        mask=mask,
    )
    return {
        "n_cells": len(keep_cells),
        "max_run": int(max_run),
        "n_calls": n_calls,
        "out_path": str(out_path),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--base-url",
        default=os.environ.get("EMEM_BASE_URL", "https://emem.dev"),
    )
    parser.add_argument("--out", type=Path, default=None)
    parser.add_argument(
        "--cells",
        type=Path,
        default=None,
        help="Optional file of cell64 strings, one per line. If omitted, "
        "we sample 100 well-known cells from a global random grid.",
    )
    args = parser.parse_args()

    if args.out is None:
        emem_data = Path(os.environ.get("EMEM_DATA", "/home/ubuntu/emem/var/emem"))
        args.out = emem_data / "jepa_v2" / "training_data.npz"

    if args.cells:
        cells = [ln.strip() for ln in args.cells.read_text().splitlines() if ln.strip()]
    else:
        # Bootstrap sample: well-spread global grid points. Far from
        # comprehensive but covers enough biomes/latitudes that the
        # dynamics head sees real diversity. Each is "lat,lng" — emem
        # /v1/recall accepts either a cell64 or a "lat,lng" string and
        # resolves the latter to a cell64 internally.
        sample_lat_lng = []
        for lat in np.linspace(-60, 60, 7):
            for lng in np.linspace(-160, 160, 9):
                sample_lat_lng.append(f"{lat:.4f},{lng:.4f}")
        cells = sample_lat_lng

    summary = assemble(args.base_url, cells, args.out)
    print(json.dumps(summary, indent=2), flush=True)


if __name__ == "__main__":
    sys.exit(main())
