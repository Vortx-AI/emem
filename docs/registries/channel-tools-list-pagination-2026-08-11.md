# Why hosts and directories show 12 tools, not 107

*Measured against `https://emem.dev` on 2026-08-11. Every number below is one
`tools/list` call away from being reproduced.*

The symptom people report is "the tool list does not load", on Smithery and in
MCP hosts generally. It is not a connection fault and not a client bug. There
are two distinct failures, and `/mcp` produces one or the other for every
client depending on whether it follows cursors.

## Measured

```
/mcp        8 pages   [43596, 20570, 43307, 43261, 42355, 45463, 45286, 8401] B
                      = 292,239 B total, 107 tools, 8 round trips, 6.4 s
/mcp/full   7 pages   = 289,937 B total, 107 tools, 7 round trips
```

Page 1 of `/mcp` carries **12 tools** and `nextCursor: core@12`. The cursor
chain then runs `core@12` → `tier:extended` → `extended@14` → … → null.

## Failure 1: a client that ignores `nextCursor` sees 12 tools

Most directory scanners, and several hosts, take page one and stop. They get 12
of 107, and not even the whole core profile, which is 16. The response says so
itself, in two fields that disagree:

```json
"_discovery": { "showing_count": 12, "showing": "core", "total_tools": 107 }
"_meta": { "dev.emem/profiles": { "core": 16, "all": 107 } }
```

The four remaining core tools are on page 2. So a non-paginating client cannot
see the advertised core surface at all, by construction.

## Failure 2: a client that does follow cursors gets all 107 from `/mcp`

`/mcp` does not stop at the core tier. When core is exhausted it hands the
reader on to extended:

`crates/emem-api-rest/src/lib.rs:20931`

```rust
let next_cursor = if more_in_tier {
    Some(format!("{effective_tier}@{next_index}"))
} else if effective_tier == "core" {
    Some("tier:extended".to_string())   // <- core readers are handed the rest
} else {
    None
};
```

So a well-behaved client walking the chain from `/mcp` pays **8 round trips,
292 KB and 6.4 seconds** before it can call anything. Cold-connect handshakes
and directory scanners routinely time out well inside that, which is what the
connect-then-drop cycling looks like from the outside.

## The claim in `server.json` is false, and inverted

`x-emem.directoryCompliance.toolListing` currently reads:

> tools/list at the advertised /mcp returns a 16-tool core surface (~51 KB);
> /mcp/full returns all 107 (about 266 KB). … so a host connecting cold from a
> directory spends 51 KB of context rather than 266 KB.

Measured, `/mcp` serves 292 KB over 8 pages and `/mcp/full` serves 290 KB over
7. `/mcp` is the **more** expensive endpoint, not the cheaper one. The split
does not bound anything; it only orders core first. A directory reading that
sentence and choosing `/mcp` gets the opposite of what it was promised.

This is the same class of defect as the dead documentation URL in the previous
log: a true-sounding sentence about behaviour outside the code, which no test
was watching.

## Fix

Two changes, and the second is what makes the first useful.

1. **Terminate the chain at the core tier when the endpoint's default is
   core.** Return `None` rather than `tier:extended`. A client that wants
   everything already has `/mcp/full`, and an explicit `{"tier":"all"}` still
   works from either endpoint, so nothing is lost. The current handoff means
   the `/mcp` default is indistinguishable from `/mcp/full` for any client
   patient enough to finish.

2. **Size the page budget so all 16 core tools fit page 1.** They are about
   64,166 bytes together (43,596 + 20,570), comfortably under the 102,400-byte
   client cap that `demos/stabilisation` already pins. The budget was halved in
   `c0b896b` so that each page fits the ceiling, and that is what splits core
   across two pages. Raising it to roughly 70 KB keeps every page legal and
   puts the whole core profile in one response.

Together: **one round trip, ~64 KB, 16 tools, and it works on clients that
never send a cursor at all.** That is the behaviour `server.json` already
claims.

3. **Correct the `directoryCompliance.toolListing` text** to whatever ships.

## A claim worth pinning

`demos/stabilisation` already asserts that every page of `/mcp/full` fits the
client cap, and it passes, because the pages are legal. What nothing asserts is the
property that actually matters to a directory:

> A no-cursor `tools/list` on `/mcp` returns the complete core profile, and the
> chain from `/mcp` ends at the core tier.

Both halves of that are false today and neither is red anywhere. The existing
`widest_tools_page` probe checks the wrong endpoint for this question, and
checks page size rather than page count.

## For anyone integrating right now

Until this changes:

- Do not judge the surface by page one. `/mcp` page one is 12 of 107.
- `emem_tools` maps the whole catalogue without loading it, and `tools/call`
  dispatches any of the 107 by name at either endpoint regardless of what
  `tools/list` showed you. A tool you cannot see is still callable.
- If you need everything up front and your client paginates, use `/mcp/full`:
  same tools, one fewer round trip.
