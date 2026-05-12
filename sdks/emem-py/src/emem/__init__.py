"""emem — Python client for the emem.dev Earth memory protocol.

The hosted instance at https://emem.dev exposes 139 REST routes (74 under
/v1/*) and 36 MCP tools. This package wraps the REST surface in a thin,
typed client. Every call returns the raw JSON the server emitted; nothing
is reshaped, so the ed25519-signed receipts and content-addressed CIDs
are preserved verbatim for citation and offline verification.

Quick start:

    from emem import Client

    em = Client()  # defaults to https://emem.dev
    cell = em.locate("Mount Fuji")["cell64"]
    facts = em.recall(cell, bands=["copdem30m.elevation_mean"])
    print(facts["facts"][0]["value"])
"""

from .client import AsyncClient, Client, EmemError, EmemHTTPError

__all__ = ["AsyncClient", "Client", "EmemError", "EmemHTTPError"]
__version__ = "0.0.6"
