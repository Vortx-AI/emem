"""Download the Clay v1.5 checkpoint into the emem HF cache.

Idempotent: re-running with the model already cached is a no-op. The
sidecar's `_resolve_clay_ckpt()` reads from the same cache_dir so this
script is the canonical bootstrap path.

Run inside the sidecar venv:

    /home/ubuntu/emem/python/jepa_v2_sidecar/.venv/bin/python \
        /home/ubuntu/emem/python/jepa_v2_sidecar/bootstrap_clay.py

Env overrides:
    EMEM_HF_CACHE   — HF cache dir (default: $EMEM_DATA/hf_cache/hub)
    EMEM_DATA       — emem data root (default: /home/ubuntu/emem/var/emem)
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

EMEM_DATA = Path(os.environ.get("EMEM_DATA", "/home/ubuntu/emem/var/emem"))
HF_CACHE = Path(os.environ.get("EMEM_HF_CACHE", str(EMEM_DATA / "hf_cache/hub")))
REPO = "made-with-clay/Clay"
FILENAME = "v1.5/clay-v1.5.ckpt"
METADATA_DST = Path(__file__).parent / "clay_metadata.yaml"
METADATA_URL = (
    "https://raw.githubusercontent.com/Clay-foundation/model/main/"
    "configs/metadata.yaml"
)


def main() -> int:
    HF_CACHE.mkdir(parents=True, exist_ok=True)

    from huggingface_hub import hf_hub_download
    from huggingface_hub.utils import HfHubHTTPError

    print(f"→ resolving {REPO}/{FILENAME} into {HF_CACHE}", flush=True)
    try:
        path = hf_hub_download(
            repo_id=REPO,
            filename=FILENAME,
            cache_dir=str(HF_CACHE),
        )
    except HfHubHTTPError as e:
        print(f"  hf_hub_download failed: {e}", file=sys.stderr)
        return 2
    print(f"  ckpt: {path}")

    if METADATA_DST.exists():
        print(f"→ metadata.yaml already vendored at {METADATA_DST}")
    else:
        print(f"→ fetching metadata.yaml → {METADATA_DST}")
        import urllib.request

        urllib.request.urlretrieve(METADATA_URL, METADATA_DST)

    size_gb = Path(path).stat().st_size / 1e9
    print(f"✓ Clay v1.5 ready ({size_gb:.2f} GB checkpoint, metadata vendored)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
