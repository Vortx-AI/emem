#!/usr/bin/env python3
"""Check every on-screen claim a film's film.json names against emem.dev.

    film_claims.py FILM.json

The half of docs/media/films/STANDARD.md that film_qc.py cannot do offline. For each claim:

  emem:fact:<cell>:<cid>          POST /v1/memory_token/resolve; it must resolve with its cell, and a `value`
                                   in the claim must equal the signed value rounded to the precision shown.
  emem:tree:<file_cid>#row=<i>    a number no single fact holds, pinned as one unit of the author's signed
                                   pointer.v1 index. The claim names the note in `index_note` (a path under
                                   /memories/by_attester/). GET /v1/tree/<file_cid>?row=<i>, the note and the
                                   source it indexes (on emem.dev only), then tools/verifiers/.../tree_proof.py
                                   check: note, leaf, root and unit must MATCH. Then the unit's `- value:` line
                                   must equal the claim's `value`.

  a computed claim (explainers only)  `reproduce` names a public script that computes the value offline, and
                                   `output` the file it writes; the file must hash to `output_sha256`.

  a pass label                     "Sentinel-2, 30 May 2026, 11:06 UTC" names the picture on screen, and a wrong
                                   date misleads as much as a wrong count. `stac_item` is the item's URL on one
                                   of the two public catalogues below, `shown_utc` the date or minute shown
                                   (2026-05-30 or 2026-05-30T11:06Z). The item's `datetime`, cut to that
                                   precision as a clock shows it, must equal it; the text must name the same day;
                                   and when film.json has segments, an evidence segment must list the item id, or
                                   an illustration segment (the catalogue's own preview) must name the item URL in
                                   `source.stac_item`. `cloud`, when the screen shows the catalogue's cloud figure,
                                   must equal the item's eo:cloud_cover, and the text must say it is the tile's.
  emem:bundle:<cid> + `rule`        a share or count over an area, such as "72 % of the park reads as snow". The
                                   bundle binds the signed per-cell facts; `rule` is {band, op, threshold} with op
                                   one of ge, gt, le, lt; `statistic` is count, share or percent. The gate resolves
                                   every member, applies the rule and recounts. All members must resolve, read the
                                   rule's band on one date, and carry no erratum. `total`, when shown, must equal
                                   the number of members.
  `rasters` + `index` + `rule`      a share or count over more pixels than a bundle holds (256): the emem:raster
                                   tokens of one scene over one box, from POST /v1/band_raster. `index` is value
                                   (one raster), ndvi, ndwi, mndwi or ndsi; `rule` is {op, threshold}. The gate
                                   fetches each artifact, checks its bytes against the cid (needs the blake3
                                   module), computes the index on the finest grid by nearest pixel, and counts
                                   again. The denominator is the pixels where every input has a value.
    or `rules`                     several rules that must all hold, such as snow: [{index: ndsi, op: ge,
                                   threshold: 0.4}, {index: value, band: s2.B08, op: gt, threshold: 1100}]. A
                                   value rule names its band. The rules must read exactly the rasters given.
    + `mask`                       the denominator is the pixels inside a mask: {rasters, index + rule or
                                   rules} from one scene, which may be another date, over the same box. A sea
                                   mask from a June pass and a count of ice on a January pass is one claim.
  `absence`                        no pass between two passes, such as "no clear optical pass for 95 days":
                                   {collection, lng, lat, after, before, max_cloud?}, `after` and `before` the
                                   days (YYYY-MM-DD) of the passes on each side. The gate searches both public
                                   catalogues: neither may hold a pass (under max_cloud % cloud, when given)
                                   strictly between, and one must hold a pass on each of the two days. `value`,
                                   when shown, is the gap, before minus after, in days. A record that has no
                                   reading for a day does not show that no satellite passed.
  emem:fact (a chain) + `fused`     a verdict that joins an observation and the reanalysis since, such as "whether
                                   the snow seen on 31 October lasted to 3 December: UNKNOWN", from POST
                                   /v1/persist. `token` is the chain's emem:fact; `fused` is {verdict?, field?,
                                   value?}: the verdict shown, and a number shown with the JSON pointer of the
                                   record field it is (/snowfall/total_cm). The gate resolves the chain, fetches
                                   the upstream series it read (GET /v1/artifacts/<cid>, bytes checked against the
                                   cid, needs the blake3 module), runs persistence_snow@1 again on those bytes and
                                   requires every number and the verdict to match; the observation and the depth
                                   fact it cites must resolve to the values it used. The text must say it is a
                                   model or reanalysis reading.
  a conversion                     "99 football pitches" from "71 hectares": `of` is the index of the claim it
                                   converts, `multiply` and `divide` the factors, `formula` the words. The gate
                                   converts the SIGNED value of that claim, not the rounded one on screen.

Exit 0 when every claim passes, 1 when one fails, 2 on bad input. Reads emem.dev, the catalogue item a pass
label names and, for an absence, the search of the two catalogues, and nothing else.
"""
import base64, hashlib, json, math, re, struct, subprocess, sys, tempfile, urllib.parse, urllib.request
from datetime import date, timedelta
from decimal import ROUND_HALF_EVEN, ROUND_HALF_UP, Decimal, InvalidOperation
from pathlib import Path

EMEM = "https://emem.dev"
REPO = Path(__file__).resolve().parent.parent
TREE_PROOF = Path(__file__).resolve().parent / "verifiers" / "emem-tokenise-files" / "tree_proof.py"
FACT = re.compile(r"^emem:fact:[A-Za-z0-9.]+:[a-z2-7]{20,64}$")
TREE = re.compile(r"^emem:tree:([a-z2-7]{20,64})#row=(\d{1,6})$")
BUNDLE = re.compile(r"^emem:bundle:[a-z2-7]{20,64}$")
RASTER = re.compile(r"^emem:raster:[a-z2-7]{20,64}:[A-Za-z0-9._]{1,64}:\d{1,9}:[a-z2-7]{20,64}$")
# Normalised differences over harmonised DN; the 1e-4 scale cancels in the ratio.
INDEX_BANDS = {"ndvi": ("s2.B08", "s2.B04"), "ndwi": ("s2.B03", "s2.B08"),
               "mndwi": ("s2.B03", "s2.B11"), "ndsi": ("s2.B03", "s2.B11")}
RULE_OPS = {"ge": lambda v, t: v >= t, "gt": lambda v, t: v > t, "le": lambda v, t: v <= t, "lt": lambda v, t: v < t}
NOTE_PATH = re.compile(r"^/memories/by_attester/[a-z2-7]{8}/[A-Za-z0-9._/-]{1,200}\.md$")
STAC_HOSTS = ("planetarycomputer.microsoft.com", "earth-search.aws.element84.com")
SHOWN_UTC = re.compile(r"^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2})Z)?$")
MONTHS = ("jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec")


def get(url, data=None):
    req = urllib.request.Request(url, data=json.dumps(data).encode() if data is not None else None,
                                 headers={"content-type": "application/json", "user-agent": "emem-film-claims"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def num(x):
    """Decimal, not float: 0.125 rounds the same way on screen and here."""
    t = str(x)
    for sep in (",", " ", "\u00a0", "\u202f"):
        t = t.replace(sep, "")
    try:
        return Decimal(t.strip().rstrip("%"))
    except InvalidOperation:
        return None


def same(shown, signed):
    """The value on screen is the signed value rounded to the precision shown: 3539.04 for 3539.0422 passes,
    1,285 for 1284 does not. A tolerance would let a wrong count through. At an exact half either common rule
    (half up, or half to even as Python's format does) passes."""
    a, b = num(shown), num(signed)
    if a is None or b is None or not a.is_finite() or not b.is_finite():
        return str(shown).strip() == str(signed).strip()
    digits = re.search(r"\.(\d+)", str(shown).replace(",", ""))
    q = Decimal(1).scaleb(-(len(digits.group(1)) if digits else 0))
    return a in (b.quantize(q, ROUND_HALF_UP), b.quantize(q, ROUND_HALF_EVEN))


def fact(c):
    r = json.loads(get(f"{EMEM}/v1/memory_token/resolve", {"token": c["token"]}))
    if not r.get("resolved"):
        return False, f"does not resolve: {r.get('message', 'no reason given')}"
    if r.get("cell_matches") is False:
        return False, "resolves only without its cell; cite it as emem:fact:<cell>:<cid>"
    if r.get("erratum"):
        return False, f"carries an erratum ({r['erratum'].get('id')}); cite the fact in replaced_by"
    if "value" in c and not same(c["value"], r.get("value")):
        return False, f"shown {c['value']!r}, signed {r.get('value')!r}"
    v = r.get("value")
    if isinstance(v, (dict, list)):
        return True, "signed record (a structure; no value compared)"
    return True, f"signed {v!r}", v


def tree(c, m):
    cid, row = m.group(1), int(m.group(2))
    note_path = str(c.get("index_note", ""))
    if not NOTE_PATH.match(note_path):
        return False, "a tree claim needs `index_note`, the path of its pointer.v1 note under /memories/by_attester/"
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        (d / "row.json").write_bytes(get(f"{EMEM}/v1/tree/{cid}?row={row}"))
        note = get(f"{EMEM}{note_path}")
        (d / "note.md").write_bytes(note)
        src = re.search(rb"^source:\s*(\S+)", note, re.M)
        if not src or not src.group(1).decode().startswith(EMEM + "/memories/"):
            return False, "the note's source is not a note on emem.dev, so its bytes cannot be read here"
        body = get(src.group(1).decode())
        (d / "source.md").write_bytes(body)
        r = subprocess.run([sys.executable, str(TREE_PROOF), "check", str(d / "row.json"), str(d / "note.md"),
                            str(d / "source.md")], capture_output=True, text=True)
        lines = [l for l in r.stdout.splitlines() if l.split()[:1] and l.split()[0] in ("note", "leaf", "root", "unit")]
        if r.returncode != 0 or len(lines) != 4 or any("MISMATCH" in l for l in lines):
            return False, "tree_proof: " + " / ".join(" ".join(l.split()[:2]) for l in lines)
        lf = re.search(r"offset=(\d+) length=(\d+)", r.stdout)
        unit = body[int(lf.group(1)):int(lf.group(1)) + int(lf.group(2))].decode("utf-8", "replace")
        v = re.search(r"^- value:\s*(.+)$", unit, re.M)
        if "value" in c:
            if not v:
                return False, "the unit has no `- value:` line to compare"
            if not same(c["value"], v.group(1)):
                return False, f"shown {c['value']!r}, the signed unit says {v.group(1).strip()!r}"
        unit_value = v.group(1).strip() if v else None
        return True, f"note, leaf, root and unit MATCH; unit value {unit_value or '(none)'}", unit_value


def bundle(c):
    rule, stat = c.get("rule") or {}, c.get("statistic")
    op, band, thr = rule.get("op"), rule.get("band"), num(rule.get("threshold"))
    if op not in RULE_OPS or not band or thr is None or stat not in ("count", "share", "percent"):
        return False, "a bundle claim needs `rule` {band, op: ge|gt|le|lt, threshold} and `statistic` count|share|percent"
    if "value" not in c:
        return False, "a bundle claim needs the `value` shown"
    b = json.loads(get(f"{EMEM}/v1/memory_bundle/{c['token']}"))
    cites = b.get("citations") or []
    if not cites or len(cites) > 256:
        return False, f"the bundle has {len(cites)} citations; a share needs 1 to 256"
    if b.get("resolved") != b.get("members") or b.get("members") != len(cites):
        return False, f"{b.get('resolved')} of {b.get('members')} members resolved; a share over part of a set says more than it holds"
    if {x.get("band") for x in cites} != {band}:
        return False, f"the bundle holds {sorted({str(x.get('band')) for x in cites})}, and the rule reads {band}"
    days = {x.get("resolved_tslot") for x in cites}
    if len(days) != 1:
        return False, f"the members span {len(days)} dates; a share is of one date"
    hits = 0
    for x in cites:
        r = json.loads(get(f"{EMEM}/v1/memory_token/resolve", {"token": x["memory_token"]}))
        if not r.get("resolved") or r.get("cell_matches") is False:
            return False, f"member {x['memory_token'][:60]} does not resolve with its cell"
        if r.get("erratum"):
            return False, f"member at {x.get('cell')} carries an erratum ({r['erratum'].get('id')}); bundle its replacement"
        v = num(r.get("value"))
        if v is None or not v.is_finite():
            return False, f"member at {x.get('cell')} has no numeric value ({r.get('value')!r})"
        hits += RULE_OPS[op](v, thr)
    n = len(cites)
    got = {"count": Decimal(hits), "share": Decimal(hits) / n, "percent": Decimal(100 * hits) / n}[stat]
    if "total" in c and num(c["total"]) != n:
        return False, f"shown total {c['total']!r}, the bundle has {n} members"
    rule_txt = f"{band} {op} {rule.get('threshold')}"
    if not same(c["value"], got):
        return False, f"shown {c['value']!r}; {hits} of {n} members meet {rule_txt}, so the {stat} is {got:.6g}"
    return True, f"{hits} of {n} members meet {rule_txt} on one date: {stat} {got:.6g}"


def raster_grid(token):
    """One resolved raster: (band, aoi_cid, scene id, x0, y0, dx, dy, width, height, values)."""
    r = json.loads(get(f"{EMEM}/v1/raster/resolve", {"token": token}))
    art = r.get("artifact") or {}
    if not art.get("present") or not art.get("url"):
        raise ValueError(f"the artifact is evicted: {art.get('if_evicted', 'POST /v1/band_raster again with the same request')}")
    try:
        import blake3
    except ImportError:
        raise ValueError("a raster claim needs the blake3 module (pip install blake3): without it the bytes are not tied to their cid")
    b = get(f"{EMEM}{art['url']}")
    cid = base64.b32encode(blake3.blake3(b).digest()).decode().lower().rstrip("=")
    if cid != art.get("artifact_cid"):
        raise ValueError(f"the artifact bytes hash to {cid}, and the signed record names {art.get('artifact_cid')}")
    if len(b) < 64 or b[:8] != b"EMEMGRD1":
        raise ValueError("the artifact is not an emem grid")
    w, h, _epsg, _nodata = struct.unpack_from("<IIII", b, 8)
    y0, x0, dy, dx = struct.unpack_from("<dddd", b, 24)
    (channels,) = struct.unpack_from("<I", b, 56)
    if channels > 1 or len(b) != 64 + 4 * w * h:
        raise ValueError("a raster claim reads a grid of one scalar channel")
    src = ((r.get("derivation") or {}).get("sources") or [{}])[0]
    return (r.get("band"), r.get("aoi_cid"), src.get("id"), x0, y0, dx, dy, w, h, struct.unpack_from(f"<{w * h}f", b, 64))


def conditions(spec):
    """[(index, band, op, threshold, shown)] from `rules`, or from one `index` and `rule`. A `value` rule names
    its `band`, except the single rule over a single raster that predates `rules`."""
    single = spec.get("rules") is None
    raw = [dict(spec.get("rule") or {}, index=spec.get("index"))] if single else spec["rules"]
    if not isinstance(raw, list) or not raw:
        raise ValueError("`rules` must be a list of {index, op, threshold}, with `band` when index is value")
    out = []
    for r in raw:
        index, op, thr, band = r.get("index"), r.get("op"), num(r.get("threshold")), r.get("band")
        if index != "value" and index not in INDEX_BANDS:
            raise ValueError(f"`index` must be value or one of {', '.join(INDEX_BANDS)}")
        if op not in RULE_OPS or thr is None or not thr.is_finite():
            raise ValueError("a rule needs {op: ge|gt|le|lt, threshold}")
        if index == "value" and not band and not (single and len(spec.get("rasters") or []) == 1):
            raise ValueError("a rule on index value names its `band`")
        out.append((index, band, op, float(thr), r.get("threshold")))
    return out


def grids_of(tokens, what):
    """The rasters of one scene over one box, by band."""
    if not tokens or not all(RASTER.match(str(t)) for t in tokens):
        raise ValueError(f"{what} needs `rasters`, a list of emem:raster tokens")
    grids = {}
    for t in tokens:
        g = raster_grid(t)
        if g[0] in grids:
            raise ValueError(f"{what} has two rasters of {g[0]}")
        grids[g[0]] = g
    if len({g[1] for g in grids.values()}) != 1 or len({g[2] for g in grids.values()}) != 1 \
            or None in {g[2] for g in grids.values()}:
        raise ValueError(f"the rasters of {what} must cover one box (aoi_cid) from one scene")
    return grids


def at(g, x, y):
    col, row = math.floor((x - g[3]) / g[5] + 0.5), math.floor((y - g[4]) / g[6] + 0.5)
    if 0 <= col < g[7] and 0 <= row < g[8]:
        v = g[9][row * g[7] + col]
        return v if math.isfinite(v) else None
    return None


def holds(conds, grids, x, y):
    """True or False when every input of every rule has a value here, else None (not in the denominator)."""
    ok = True
    for index, band, op, thr, _ in conds:
        if index == "value":
            v = at(grids[band], x, y)
            if v is None:
                return None
        else:
            a, b = (at(grids[k], x, y) for k in INDEX_BANDS[index])
            if a is None or b is None or a + b == 0:
                return None
            v = (a - b) / (a + b)
        ok = RULE_OPS[op](v, thr) and ok
    return ok


def bound(conds, grids, what):
    """Name the single raster of a legacy `value` rule, and check the rules read exactly the rasters given."""
    if len(conds) == 1 and conds[0][0] == "value" and not conds[0][1]:
        conds[0] = ("value", next(iter(grids))) + conds[0][2:]
    need = {b for i, band, *_ in conds for b in ((band,) if i == "value" else INDEX_BANDS[i])}
    if need != set(grids):
        raise ValueError(f"the rules of {what} read {', '.join(sorted(need))}; its rasters are "
                         f"{', '.join(sorted(map(str, grids)))}")
    return " and ".join(f"{i}{'(' + b + ')' if i == 'value' else ''} {op} {shown}" for i, b, op, _, shown in conds)


def raster(c):
    stat = c.get("statistic")
    if stat not in ("count", "share", "percent") or "value" not in c:
        return False, "a raster claim needs `statistic` count|share|percent and `value`"
    try:
        conds = conditions(c)
        grids = grids_of(c.get("rasters"), "the claim")
        rules_txt = bound(conds, grids, "the claim")
        mask = c.get("mask")
        if mask is not None:
            mconds = conditions(mask)
            mgrids = grids_of(mask.get("rasters"), "the mask")
            mask_txt = bound(mconds, mgrids, "the mask")
            if {g[1] for g in mgrids.values()} != {g[1] for g in grids.values()}:
                raise ValueError("the mask must cover the same box (aoi_cid) as the claim")
    except ValueError as e:
        return False, str(e)
    fine = min(grids.values(), key=lambda g: abs(g[5]))
    _, _, scene, fx0, fy0, fdx, fdy, fw, fh, _ = fine
    valid = hits = 0
    for row in range(fh):
        for col in range(fw):
            x, y = fx0 + col * fdx, fy0 + row * fdy
            if mask is not None and not holds(mconds, mgrids, x, y):
                continue
            h = holds(conds, grids, x, y)
            if h is None:
                continue
            valid += 1
            hits += h
    if valid == 0:
        return False, "no pixel has a value in every input" + (" inside the mask" if mask is not None else "")
    got = {"count": Decimal(hits), "share": Decimal(hits) / valid, "percent": Decimal(100 * hits) / valid}[stat]
    where = f" inside {mask_txt} of {next(iter(mgrids.values()))[2]}" if mask is not None else ""
    if "total" in c and num(c["total"]) != valid:
        return False, f"shown total {c['total']!r}, and {valid} pixels have a value in every input{where}"
    what = f"{rules_txt} on {fw}x{fh} px of {scene}{where}"
    if not same(c["value"], got):
        return False, f"shown {c['value']!r}; {hits} of {valid} pixels meet {what}, so the {stat} is {got:.6g}"
    return True, f"{hits} of {valid} pixels meet {what}: {stat} {got:.6g}"


STAC_SEARCH = ("https://planetarycomputer.microsoft.com/api/stac/v1/search",
               "https://earth-search.aws.element84.com/v1/search")
DAY = re.compile(r"^\d{4}-\d{2}-\d{2}$")


def absence(c):
    """No pass between two passes, in either public catalogue. A record that holds no reading for a day does
    not say whether the satellite passed: emem's dated recall says "nothing is signed for" a day that had three
    passes under cloud. So the gap is checked against the catalogues, not against emem."""
    a = c.get("absence") or {}
    coll, lng, lat, cloud = a.get("collection"), a.get("lng"), a.get("lat"), a.get("max_cloud")
    if not coll or not isinstance(lng, (int, float)) or not isinstance(lat, (int, float)) \
            or not DAY.match(str(a.get("after", ""))) or not DAY.match(str(a.get("before", ""))):
        return False, ("an absence claim needs `absence` {collection, lng, lat, after, before}, the two dates "
                       "YYYY-MM-DD of the passes on each side, and optionally max_cloud")
    if cloud is not None and (not isinstance(cloud, (int, float)) or not 0 < cloud <= 100):
        return False, "`max_cloud` is a percent above 0 and at most 100"
    d0, d1 = date.fromisoformat(a["after"]), date.fromisoformat(a["before"])
    if (d1 - d0).days < 2:
        return False, "`before` must be at least two days after `after`, or there is no gap between them"

    def passes(first, last):
        out = []
        for url in STAC_SEARCH:
            body = {"collections": [coll], "intersects": {"type": "Point", "coordinates": [lng, lat]},
                    "datetime": f"{first.isoformat()}T00:00:00Z/{last.isoformat()}T23:59:59Z", "limit": 100}
            if cloud is not None:
                body["query"] = {"eo:cloud_cover": {"lt": cloud}}
            for f in json.loads(get(url, body)).get("features") or []:
                out.append((urllib.parse.urlparse(url).hostname.split(".")[0], f.get("id"),
                            str((f.get("properties") or {}).get("datetime"))[:16]))
        return out
    under = f" under {cloud} % cloud" if cloud is not None else ""
    kind = f"{coll} pass{under}"
    inside = sorted(passes(d0 + timedelta(days=1), d1 - timedelta(days=1)), key=lambda p: p[2])
    if inside:
        return False, (f"{len(inside)} {coll} {'pass' if len(inside) == 1 else 'passes'}{under} between them, "
                       f"first {inside[0][2]} ({inside[0][0]}: {inside[0][1]})")
    ends = []
    for d in (d0, d1):
        on = passes(d, d)
        if not on:
            return False, f"no {kind} on {d} in either catalogue; `after` and `before` are the passes on each side"
        ends.append(f"{d} ({on[0][0]}: {on[0][1]})")
    gap = (d1 - d0).days
    if "value" in c and not same(c["value"], gap):
        return False, f"shown {c['value']!r}; from {d0} to {d1} is {gap} days"
    return True, f"no {kind} in either catalogue between {ends[0]} and {ends[1]}: a gap of {gap} days"


def label(c, meta):
    url = str(c.get("stac_item", ""))
    u = urllib.parse.urlparse(url)
    if u.scheme != "https" or u.hostname not in STAC_HOSTS:
        return False, "a pass label needs `stac_item`, the https URL of the item on " + " or ".join(STAC_HOSTS)
    m = SHOWN_UTC.match(str(c.get("shown_utc", "")))
    if not m:
        return False, "a pass label needs `shown_utc`, the date or minute shown: 2026-05-30 or 2026-05-30T11:06Z"
    y, mo, d, hh, mi = m.groups()
    if not 1 <= int(mo) <= 12:
        return False, f"`shown_utc` has no month {mo}"
    item = json.loads(get(url))
    dt = str((item.get("properties") or {}).get("datetime") or "")
    want = f"{y}-{mo}-{d}" + (f"T{hh}:{mi}" if hh else "")
    if not dt.startswith(want):
        return False, f"shown {want}, the catalogue item says {dt or 'no datetime'}"
    text = str(c.get("text", "")).lower()
    named = y in text and re.search(rf"(?<!\d)0?{int(d)}(?!\d)", text) and (MONTHS[int(mo) - 1] in text or f"-{mo}-" in text)
    if not named or (hh and f"{hh}:{mi}" not in text):
        return False, f"the text does not say {want}; `shown_utc` must be what the screen shows"
    cc = (item.get("properties") or {}).get("eo:cloud_cover")
    if "cloud" in c:
        if cc is None or not same(c["cloud"], cc):
            return False, f"shown {c['cloud']!r} % cloud, the catalogue item says {cc!r}"
        if "tile" not in text and "scene" not in text:
            return False, "the catalogue's cloud figure is the whole tile's, not the box's: the text must say tile or scene"
    segs = meta.get("segments") or []
    listed = {str(s) for g in segs if g.get("role") == "evidence" for s in (g.get("scenes") or [])}
    # A catalogue's own preview of a pass is an illustration (a rendering, not source pixels) that names its item.
    previews = {str((g.get("source") or {}).get("stac_item")) for g in segs if g.get("role") == "illustration"}
    if segs and item.get("id") not in listed and url not in previews:
        return False, (f"no evidence segment lists {item.get('id')!r}, and no illustration segment's source names "
                       "this item; name scenes by their catalogue item id")
    return True, f"catalogue datetime {dt}" + (f", {cc} % tile cloud" if "cloud" in c else "")


# persistence_snow@1, as crates/emem-api-rest/src/persist.rs declares it. A record whose rule differs is
# refused rather than recomputed under its own numbers.
PERSIST_RULE = {"ndsi_snow": 0.4, "ndsi_no_snow": 0.0, "melt_out_m": 0.01, "snow_on_ground_m": 0.05,
                "glacier_pack_m": 10.0, "ddf_mm_per_degc_day": 3.0, "start_density_kg_m3": 300.0,
                "snowfall_cm_per_mm": 0.7, "max_missing_share": 0.1}


def f64_round(v):
    """Rust's f64::round: half away from zero."""
    f = math.floor(v)
    d = v - f
    return f + 1 if d > 0.5 or (d == 0.5 and v > 0) else f


def r2(x):
    return None if x is None else f64_round(x * 100.0) / 100.0


def utc(s):
    from datetime import datetime, timezone
    s = s.rstrip("Z")
    fmt = "%Y-%m-%dT%H:%M:%S" if s.count(":") == 2 else "%Y-%m-%dT%H:%M"
    return int(datetime.strptime(s, fmt).replace(tzinfo=timezone.utc).timestamp())


def iso_hour(u):
    from datetime import datetime, timezone
    return datetime.fromtimestamp(u, timezone.utc).strftime("%Y-%m-%dT%H:00Z")


def iso_day(u):
    return iso_hour(u)[:10]


def persist_judge(ndsi, obs_unix, to_unix, body):
    """The same chain as persist.rs `judge`, hour by hour in the same order, so the floats agree bit for bit."""
    R = PERSIST_RULE
    h = body["hourly"]
    hours = [utc(t) for t in h["time"]]
    T, S, D = h["temperature_2m_era5_land"], h["snowfall_era5"], h["snow_depth_era5_land"]
    obs_h, to_h = obs_unix // 3600 * 3600, to_unix // 3600 * 3600
    win = [i for i, x in enumerate(hours) if obs_h < x <= to_h]
    n = len(win)
    gaps = [sum(1 for i in win if v[i] is None) for v in (T, S, D)]
    start = hours.index(obs_h) if obs_h in hours else None
    d0 = D[start] if start is not None else None
    d_end = D[hours.index(to_h)] if to_h in hours else None
    days, thaw_h, pdd, total = {}, 0, 0.0, 0.0
    t_max = d_min = d_max = last_thaw = m_out = m_snow = dd_out = None
    glacier = d0 is not None and d0 >= R["glacier_pack_m"]
    swe = d0 * R["start_density_kg_m3"] if d0 is not None else 0.0
    swe0 = swe
    for i in win:
        x = hours[i]
        day = days.setdefault(iso_day(x), {"t_max": None, "thaw": 0, "pdd": 0.0, "fall": 0.0, "dmin": None, "dmax": None})
        t = T[i]
        if t is not None:
            day["t_max"] = t if day["t_max"] is None else max(day["t_max"], t)
            if t_max is None or t > t_max[0]:
                t_max = (t, x)
            if t > 0.0:
                thaw_h += 1; day["thaw"] += 1; pdd += t / 24.0; day["pdd"] += t / 24.0; last_thaw = x
        fall = S[i] if S[i] is not None else 0.0
        total += fall; day["fall"] += fall
        d = D[i]
        if d is not None:
            day["dmin"] = d if day["dmin"] is None else min(day["dmin"], d)
            day["dmax"] = d if day["dmax"] is None else max(day["dmax"], d)
            if d_min is None or d < d_min[0]:
                d_min = (d, x)
            d_max = d if d_max is None else max(d_max, d)
            glacier = glacier or d >= R["glacier_pack_m"]
            if m_out is None and d < R["melt_out_m"]:
                m_out = x
            if m_snow is None and d >= R["snow_on_ground_m"]:
                m_snow = x
        before = swe
        swe = max(swe + fall / R["snowfall_cm_per_mm"] - (max(t, 0.0) * R["ddf_mm_per_degc_day"] / 24.0 if t is not None else 0.0), 0.0)
        if dd_out is None and before > 0.0 and swe <= 0.0:
            dd_out = x
    state = "snow" if ndsi >= R["ndsi_snow"] else "no_snow" if ndsi < R["ndsi_no_snow"] else "snow_or_cloud"
    if n == 0 or glacier or max(gaps) > R["max_missing_share"] * n or state == "snow_or_cloud":
        verdict = "unknown"
    elif state == "snow":
        if d0 is None or d0 < R["melt_out_m"] or (m_out is None) != (dd_out is None):
            verdict = "unknown"
        elif m_out is None:
            verdict = "persisted_likely"
        else:
            verdict = "renewed" if d_end is not None and d_end >= R["snow_on_ground_m"] else "broken"
    elif d0 is not None and d0 >= R["snow_on_ground_m"]:
        verdict = "unknown"
    else:
        verdict = "broken" if m_snow is not None else "persisted_likely"
    fall_day = None
    for k, v in sorted(days.items()):  # Rust's max_by keeps the last of equal maxima
        if fall_day is None or v["fall"] >= fall_day[1]:
            fall_day = (k, v["fall"])
    return {
        "window": {"from": iso_hour(obs_h), "to": iso_hour(to_h), "hours": n},
        "verdict": verdict,
        "/thaw/hours_above_0c": thaw_h, "/thaw/days_with_thaw": sum(1 for v in days.values() if v["thaw"]),
        "/thaw/positive_degree_days": r2(pdd), "/thaw/max_t2m_c/value": r2(t_max[0]) if t_max else None,
        "/snowfall/total_cm": r2(total),
        "/snowfall/max_day/cm": r2(fall_day[1]) if fall_day and fall_day[1] > 0 else None,
        "/model_pack/depth_at_observation_m": r2(d0), "/model_pack/min_m/value": r2(d_min[0]) if d_min else None,
        "/model_pack/max_m": r2(d_max), "/model_pack/melt_out_at": iso_hour(m_out) if m_out is not None else None,
        "/model_pack/depth_at_date_m": r2(d_end),
        "/degree_day_pack/swe_start_mm": r2(swe0), "/degree_day_pack/swe_at_date_mm": r2(swe),
        "/degree_day_pack/melt_out_at": iso_hour(dd_out) if dd_out is not None else None,
        "/daily": [{"day": k, "t_max_c": r2(v["t_max"]), "thaw_hours": v["thaw"], "positive_degree_days": r2(v["pdd"]),
                    "snowfall_cm": r2(v["fall"]), "depth_min_m": r2(v["dmin"]), "depth_max_m": r2(v["dmax"])}
                   for k, v in sorted(days.items())],
    }


def pointer(doc, p):
    for k in p.strip("/").split("/"):
        if isinstance(doc, list) and k.isdigit() and int(k) < len(doc):
            doc = doc[int(k)]
        elif isinstance(doc, dict) and k in doc:
            doc = doc[k]
        else:
            return None
    return doc


def fused(c):
    r = json.loads(get(f"{EMEM}/v1/memory_token/resolve", {"token": c["token"]}))
    if not r.get("resolved"):
        return False, f"does not resolve: {r.get('message', 'no reason given')}"
    if r.get("cell_matches") is False:
        return False, "resolves only without its cell; cite it as emem:fact:<cell>:<cid>"
    rec = r.get("value")
    if not isinstance(rec, dict) or rec.get("schema") != "emem.persistence.v1":
        return False, "the token is not a persistence chain (schema emem.persistence.v1)"
    if rec.get("fn_key") != "persistence_snow@1":
        return False, f"rule {rec.get('fn_key')!r}: this gate recomputes persistence_snow@1 only"
    declared = {k: (rec.get("rule") or {}).get(k) for k in PERSIST_RULE}
    if declared != PERSIST_RULE:
        return False, f"the record's rule numbers differ from persistence_snow@1: {declared}"
    text = str(c.get("text", "")).lower()
    if "model" not in text and "reanalysis" not in text:
        return False, "the text must say this is a model or reanalysis reading"
    try:
        import blake3
    except ImportError:
        return False, "the blake3 module is needed to check the series artifact (pip install blake3)"
    art = (rec.get("series") or {}).get("artifact_cid", "")
    raw = get(f"{EMEM}/v1/artifacts/{art}")
    if base64.b32encode(blake3.blake3(raw).digest()).decode().rstrip("=").lower() != art:
        return False, f"the series bytes do not hash to {art}"
    ob = rec.get("observation") or {}
    cell = c["token"].split(":")[2]
    o = json.loads(get(f"{EMEM}/v1/memory_token/resolve", {"token": f"emem:fact:{cell}:{ob.get('fact_cid')}"}))
    if not o.get("resolved") or o.get("value") != ob.get("value"):
        return False, f"the observation {ob.get('fact_cid')} does not resolve to {ob.get('value')!r}"
    want = persist_judge(ob["value"], utc(ob["observed_at"]), utc(rec["window"]["to"]), json.loads(raw))
    bad = [k for k, v in want.items() if (rec.get(k) if not k.startswith("/") else pointer(rec, k)) != v]
    if bad:
        k = bad[0]
        return False, f"recomputed {k} = {want[k]!r}, the record says {(rec.get(k) if not k.startswith('/') else pointer(rec, k))!r}"
    dep = rec.get("depth_fact")
    if dep:
        d = json.loads(get(f"{EMEM}/v1/memory_token/resolve", {"token": f"emem:fact:{cell}:{dep.get('fact_cid')}"}))
        if not d.get("resolved") or r2(d.get("value")) != want["/model_pack/depth_at_date_m"]:
            return False, f"the depth fact {dep.get('fact_cid')} does not resolve to the depth at the hour asked"
    shown = c.get("fused") if isinstance(c.get("fused"), dict) else {}
    if shown.get("verdict") is not None and str(shown["verdict"]).lower().replace(" ", "_") != rec["verdict"]:
        return False, f"shown verdict {shown['verdict']!r}, the chain says {rec['verdict']!r}"
    if shown.get("field"):
        v = pointer(rec, shown["field"])
        if v is None or not same(shown.get("value"), v):
            return False, f"shown {shown.get('value')!r} for {shown['field']}, the chain says {v!r}"
    return True, f"recomputed {len(want)} links: {rec['verdict']} over {rec['window']['hours']} hours"


def convert(c, i, claims, signed):
    of, mul, div = c.get("of"), num(c.get("multiply", 1)), num(c.get("divide", 1))
    if not isinstance(of, int) or isinstance(of, bool) or not 0 <= of < len(claims) or of == i:
        return False, "a conversion needs `of`, the index of another claim in this film.json"
    if mul is None or div is None or not mul.is_finite() or not div.is_finite() or div == 0:
        return False, "`multiply` and `divide` must be finite numbers, `divide` not 0"
    if not str(c.get("formula", "")).strip():
        return False, "a conversion needs `formula`, the arithmetic in words"
    base = signed.get(of)
    lead = re.match(r"\s*([-+]?[\d,]*\.?\d+(?:[eE][-+]?\d+)?)", str(base)) if base is not None else None
    b = num(lead.group(1)) if lead else None
    if b is None:
        return False, f"claim {of} passed with no numeric signed value, so there is nothing to convert"
    if "value" not in c:
        return False, "a conversion needs the `value` shown"
    v = b * mul / div
    if not same(c["value"], v):
        return False, f"shown {c['value']!r}; {c['formula']} on the signed {base} gives {v:.8g}"
    return True, f"{c['formula']} on the signed {base} gives {v:.8g}"


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    try:
        meta = json.loads(Path(sys.argv[1]).read_text())
    except (OSError, ValueError) as e:
        print(f"cannot read film.json: {e}")
        return 2
    claims, bad, signed, results = meta.get("claims") or [], 0, {}, {}
    if not claims:
        print("FAIL no claims in film.json")
        return 1
    # Conversions last: each reads the signed value of the claim it converts.
    order = sorted(range(len(claims)), key=lambda k: "of" in claims[k])
    for i in order:
        c = claims[i]
        tok = str(c.get("token", ""))
        try:
            if "of" in c:
                ok, why = convert(c, i, claims, signed)
            elif c.get("rasters"):
                ok, why = raster(c)
            elif c.get("absence"):
                ok, why = absence(c)
            elif c.get("stac_item") or c.get("shown_utc"):
                ok, why = label(c, meta)
            elif not tok and c.get("reproduce") and c.get("output_sha256"):
                out = REPO / str(c.get("output", ""))
                if REPO not in out.resolve().parents or not out.is_file():
                    ok, why = False, "a computed claim needs `output`, a file in this repository"
                else:
                    h = hashlib.sha256(out.read_bytes()).hexdigest()
                    ok = h == c["output_sha256"]
                    why = f"computed by {c['reproduce']}; output {'matches' if ok else 'differs from'} its sha256"
            elif BUNDLE.match(tok):
                ok, why = bundle(c)
            elif FACT.match(tok) and c.get("fused") is not None:
                ok, why = fused(c)
            elif FACT.match(tok) or TREE.match(tok):
                m = TREE.match(tok)
                ok, why, *val = tree(c, m) if m else fact(c)
                if ok and val and val[0] is not None:
                    signed[i] = val[0]
            else:
                ok, why = False, "not an emem:fact, emem:tree#row or emem:bundle token, rasters, an absence, a pass label or a conversion"
        except Exception as e:  # noqa: BLE001
            ok, why = False, f"{type(e).__name__}: {str(e)[:160]}"
        bad += not ok
        cited = (tok or c.get("stac_item") or ",".join(str(t).split(":")[3] for t in c.get("rasters") or [])
                 or (f"absence {c['absence'].get('after')}..{c['absence'].get('before')}" if isinstance(c.get("absence"), dict) else ""))
        results[i] = f"{'pass' if ok else 'FAIL'}  {i:>2} {c.get('text', '')[:46]!r:50} {cited[:72]}  {why}"
    for i in range(len(claims)):
        print(results[i])
    print(f"{len(claims) - bad} of {len(claims)} claims pass")
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
