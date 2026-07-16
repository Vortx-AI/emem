"""ememdev: Python client for the emem.dev Earth memory protocol.

The distribution and the module are both named ``ememdev``, on purpose:
the shorter name ``emem`` on PyPI belongs to an unrelated project by
another company, and a module named ``emem`` would collide with it on
disk if both were installed.

This package wraps the REST surface of the hosted instance at
https://emem.dev in a thin, typed client. Every call returns the raw JSON
the server emitted; nothing is reshaped, so the ed25519-signed receipts
and content-addressed CIDs are preserved verbatim for citation and
offline verification.

The responder enumerates its own surface: GET /openapi.json lists every
documented path and POST /mcp `tools/list` every MCP tool. Those are the
counts to quote, since a number written down here goes stale the next
time a route lands.

Quick start:

    from ememdev import Client

    em = Client()  # defaults to https://emem.dev
    cell = em.locate("Mount Fuji")["cell64"]
    facts = em.recall(cell, bands=["copdem30m.elevation_mean"])
    print(facts["facts"][0]["value"])
"""

from importlib.metadata import PackageNotFoundError, version as _pkg_version

from .client import AsyncClient, Client, EmemError, EmemHTTPError

__all__ = ["AsyncClient", "Client", "EmemError", "EmemHTTPError"]

# Single source of truth: the installed distribution's version, which comes
# from pyproject.toml. Keeps __version__ and the client User-Agent from
# drifting out of sync with what actually ships to PyPI.
try:
    __version__ = _pkg_version("ememdev")
except PackageNotFoundError:  # running from a source checkout, not installed
    __version__ = "0+unknown"
