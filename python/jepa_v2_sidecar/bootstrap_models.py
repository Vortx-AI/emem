"""One-shot HF cache seed for the inference sidecar.

Downloads Clay v1.5, Prithvi-EO-2.0-300M-TL, and Galileo (variant set
by `EMEM_GALILEO_VARIANT`) into the same cache the sidecar reads from
(`$EMEM_DATA/hf_cache/hub` or `$EMEM_HF_CACHE`). Once seeded the
sidecar can run with `HF_HUB_OFFLINE=1` and never touch the network
again — every loader resolves paths via
`huggingface_hub.try_to_load_from_cache`, which is read-only.

Usage:

    /home/ubuntu/emem/python/jepa_v2_sidecar/.venv/bin/python \
        /home/ubuntu/emem/python/jepa_v2_sidecar/bootstrap_models.py

    # subset:
    .venv/bin/python bootstrap_models.py clay prithvi
    .venv/bin/python bootstrap_models.py galileo

Idempotent: re-running with everything cached is a no-op (HF cache
short-circuits to the local snapshot dir).
"""

from __future__ import annotations

import os
import sys
import urllib.request
from pathlib import Path

EMEM_DATA = Path(os.environ.get("EMEM_DATA", "/home/ubuntu/emem/var/emem"))
HF_CACHE = Path(os.environ.get("EMEM_HF_CACHE", str(EMEM_DATA / "hf_cache/hub")))

CLAY_REPO = "made-with-clay/Clay"
CLAY_CKPT_FILENAME = "v1.5/clay-v1.5.ckpt"
CLAY_METADATA_DST = Path(__file__).parent / "clay_metadata.yaml"
CLAY_METADATA_URL = (
    "https://raw.githubusercontent.com/Clay-foundation/model/main/"
    "configs/metadata.yaml"
)

PRITHVI_REPO = "ibm-nasa-geospatial/Prithvi-EO-2.0-300M-TL"
# Pull the python helpers + config + checkpoint; skip the example
# notebooks and large eval rasters that are not needed at runtime.
PRITHVI_PATTERNS = ["*.py", "*.json", "*.txt", "*.pt"]

GALILEO_REPO = "nasaharvest/galileo"
GALILEO_VARIANT = os.environ.get("EMEM_GALILEO_VARIANT", "base")
GALILEO_PATTERNS = [f"models/{GALILEO_VARIANT}/*"]


def _ensure_clay() -> None:
    from huggingface_hub import hf_hub_download

    print(f"→ Clay v1.5: resolving {CLAY_REPO}/{CLAY_CKPT_FILENAME}", flush=True)
    path = hf_hub_download(
        repo_id=CLAY_REPO,
        filename=CLAY_CKPT_FILENAME,
        cache_dir=str(HF_CACHE),
    )
    size_gb = Path(path).stat().st_size / 1e9
    print(f"  ckpt: {path} ({size_gb:.2f} GB)")
    if not CLAY_METADATA_DST.exists():
        print(f"→ fetching metadata.yaml → {CLAY_METADATA_DST}")
        urllib.request.urlretrieve(CLAY_METADATA_URL, CLAY_METADATA_DST)
    print(f"✓ Clay v1.5 ready (metadata vendored at {CLAY_METADATA_DST.name})")


def _ensure_prithvi() -> None:
    from huggingface_hub import snapshot_download

    print(f"→ Prithvi-EO-2.0: snapshot_download {PRITHVI_REPO}", flush=True)
    path = snapshot_download(
        repo_id=PRITHVI_REPO,
        allow_patterns=PRITHVI_PATTERNS,
        cache_dir=str(HF_CACHE),
    )
    print(f"✓ Prithvi snapshot at {path}")


def _ensure_galileo() -> None:
    from huggingface_hub import snapshot_download

    print(
        f"→ Galileo-{GALILEO_VARIANT}: snapshot_download {GALILEO_REPO}",
        flush=True,
    )
    path = snapshot_download(
        repo_id=GALILEO_REPO,
        allow_patterns=GALILEO_PATTERNS,
        cache_dir=str(HF_CACHE),
    )
    print(f"✓ Galileo-{GALILEO_VARIANT} snapshot at {path}")


HANDLERS = {
    "clay": _ensure_clay,
    "prithvi": _ensure_prithvi,
    "galileo": _ensure_galileo,
}


def main(argv: list[str]) -> int:
    HF_CACHE.mkdir(parents=True, exist_ok=True)
    targets = argv[1:] or list(HANDLERS)
    unknown = [t for t in targets if t not in HANDLERS]
    if unknown:
        print(
            f"unknown bootstrap target(s): {unknown}; valid: {list(HANDLERS)}",
            file=sys.stderr,
        )
        return 2
    for t in targets:
        HANDLERS[t]()
    print(f"\nDone. Cache lives at {HF_CACHE}.")
    print(
        "Set `HF_HUB_OFFLINE=1` in the systemd unit to enforce no further\n"
        "network calls; the sidecar will then refuse to fetch anything not\n"
        "already cached and fail fast on missing models."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
