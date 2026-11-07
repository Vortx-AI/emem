# Why hosts and directories showed 12 tools, not 107

This is a record of a defect that emem.dev had on 11 August 2026, and of its
fix. The body of the record keeps the state of 11 August. Two later checks
come first.

## State on 7 November 2026

The fix still holds on version 2.4.4.

| Call | Pages | Bytes | Tools | `nextCursor` on the last page |
|---|---|---|---|---|
| `/mcp`, no cursor | 1 | 76,820 | 18 (the core profile) | none |
| `/mcp/full` | 9 | 354,705 | 116 | none |
| `/mcp` with `{"tier":"all"}` | 9 | 354,714 | 116 | none |

The pagination code in `crates/emem-api-rest/src/lib.rs` ends the chain when
the selected list ends. Find the `next_cursor` after the comment "The cursor
carries the selection". The code has no handoff to `tier:extended`.

The probe `core_profile_whole_on_page_one` in `demos/stabilisation/claims.py`
now pins the property in [A claim to pin](#a-claim-to-pin).

The text in `server.json` (`x-emem.directoryCompliance.toolListing`) gives the
correct count of 116 tools. Its byte and page figures are old. It says about
64 KB for `/mcp`, and 7 pages of about 290 KB for `/mcp/full`.

## State on 29 September 2026

The fix shipped. A `tools/list` call at `https://emem.dev/mcp` with no cursor
returned all 18 core tools in one page. The page had 76,766 bytes and no
`nextCursor`, so the chain ended at the core tier. `/mcp/full` listed all 114
tools across 8 pages. The `server.json` text that this record calls false then
described the single core page. Its byte figures were behind the measurement:
about 64 KB, and 7 pages for `/mcp/full`.

## The symptom on 11 August 2026

All numbers from here to the end are the state on 11 August 2026, measured
against `https://emem.dev`. One `tools/list` call reproduced each number on
that day.

Users reported that "the tool list does not load", on Smithery and in other
MCP hosts. The cause was not a connection fault or a client bug. `/mcp` had two
different failures. A client got one or the other: the result depended on
whether the client followed cursors.

## Measured

```
/mcp        8 pages   [43596, 20570, 43307, 43261, 42355, 45463, 45286, 8401] B
                      = 292,239 B total, 107 tools, 8 round trips, 6.4 s
/mcp/full   7 pages   = 289,937 B total, 107 tools, 7 round trips
```

Page 1 of `/mcp` had **12 tools** and `nextCursor: core@12`. The cursor chain
then ran `core@12`, `tier:extended`, `extended@14` and so on, until null.

## Failure 1: a client that ignores `nextCursor` sees 12 tools

Most directory scanners, and some hosts, take page one and stop. They got 12 of
107 tools. That was not even the full core profile of 16 tools. The response
showed this in two fields that disagreed:

```json
"_discovery": { "showing_count": 12, "showing": "core", "total_tools": 107 }
"_meta": { "dev.emem/profiles": { "core": 16, "all": 107 } }
```

The other four core tools were on page 2. Thus a client that did not paginate
could never see the full core surface that emem advertised.

## Failure 2: a client that follows cursors gets all 107 from `/mcp`

`/mcp` did not stop at the core tier. When the core tools ran out, the code
sent the reader on to the extended tier. The code was at
`crates/emem-api-rest/src/lib.rs:20931` on that day:

```rust
let next_cursor = if more_in_tier {
    Some(format!("{effective_tier}@{next_index}"))
} else if effective_tier == "core" {
    Some("tier:extended".to_string())   // <- core readers are handed the rest
} else {
    None
};
```

A client that walked the full chain from `/mcp` thus paid **8 round trips,
292 KB and 6.4 seconds** before it could call a tool. Cold-connect handshakes
and directory scanners often time out before that. From the outside, this
looked like a cycle of connect and drop.

## The claim in `server.json` was false, and inverted

On 11 August, `x-emem.directoryCompliance.toolListing` said:

> tools/list at the advertised /mcp returns a 16-tool core surface (~51 KB);
> /mcp/full returns all 107 (about 266 KB). … so a host connecting cold from a
> directory spends 51 KB of context rather than 266 KB.

The measurement was different. `/mcp` served 292 KB over 8 pages, and
`/mcp/full` served 290 KB over 7 pages. Thus `/mcp` was the **more** expensive
endpoint. The split did not limit the size; it only put the core tools first.
A directory that read the sentence and chose `/mcp` got the opposite of the
promise.

The dead documentation URL in the previous log was the same type of defect. In
both cases, a sentence about behaviour outside the code sounded true. No test
checked it.

## Fix

The record proposed three changes. The second change makes the first useful.

1. **Stop the chain at the core tier when the default of the endpoint is
   core.** Return `None`, not `tier:extended`. A client that wants all tools
   has `/mcp/full`. An explicit `{"tier":"all"}` works from both endpoints, so
   clients lose nothing. With the handoff, a patient client could not tell the
   `/mcp` default from `/mcp/full`.

2. **Set the page budget so that all 16 core tools fit on page 1.** Together
   they are about 64,166 bytes (43,596 + 20,570). That is well below the
   102,400-byte client cap that `demos/stabilisation` already pins. Commit
   `c0b896b` cut the budget in half so that each page fits the cap. That cut
   split the core tools across two pages. A budget of about 70 KB keeps each
   page below the cap and puts the full core profile in one response.

3. **Correct the `directoryCompliance.toolListing` text** to match what ships.

Changes 1 and 2 together give one round trip, about 64 KB and 16 tools. This
works on clients that never send a cursor. `server.json` already claimed this
behaviour.

## A claim to pin

On 11 August, `demos/stabilisation` checked that each page of `/mcp/full` fits
the client cap. That check passed, because the pages were within the cap. No
check covered the property that matters to a directory:

> A no-cursor `tools/list` on `/mcp` returns the complete core profile, and the
> chain from `/mcp` ends at the core tier.

Both parts of that property were false, and no check failed. The probe
`widest_tools_page` checked the wrong endpoint for this question. It also
checked page size, not page count.

## Advice to integrators on 11 August

Until the fix shipped:

- Do not judge the surface by page one. Page one of `/mcp` had 12 of 107 tools.
- `emem_tools` maps the full catalogue, and you do not have to load it.
  `tools/call` runs any of the 107 tools by name at either endpoint. A tool
  that `tools/list` did not show is still callable.
- If you need all tools at the start and your client paginates, use
  `/mcp/full`. It has the same tools and one fewer round trip.
