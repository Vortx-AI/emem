#!/usr/bin/env python3
"""Generate the industry gallery scenes in the Mithila SVG hand.

Each scene: a live bundle of the real emem bands this industry reads, signed
into one receipt, handed to the parties that act on it. One template, a data
table of accurate per-industry content. Writes docs/diagrams/<key>.svg and a
PNG preview under /tmp for visual checking.

Usage: python3 scripts/gen_industry.py [key ...]   (default: all)
"""
import os, sys, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _mithila_svg import (Svg, INK, INK_SOFT, TURMERIC, VERMIL, INDIGO, LAC,
                          LEAF, TEAL, GOLD_PALE, PAPER, PAPER2, DYES, SERIF, MONO)

OUT = "docs/diagrams"


def scene(spec):
    s = Svg(1600, 1040, seed=spec.get("seed", 7))
    W, H = s.W, s.H
    s.paper(); s.border()

    s.text(80, 56, "emem for " + spec["industry"], 30, INK, font=SERIF)
    s.text(82, 88, spec["tagline"], 13, INK_SOFT, font=MONO)

    CX, CY = 800, 470
    # faint field rings to anchor the centre, like the engram
    for R in (150, 232, 318):
        nn = int(R / 5.4)
        s.dot_ring(CX, CY, R, nn, 1.2, INK, op=0.06)
        s.dot_ring(CX, CY, R, max(8, nn // 6), 1.7, TURMERIC, phase=0.4, op=0.12)

    # ---- left: live fact bundle ledger
    bands = spec["bands"]
    px, py = 96, 250
    pw, ph = 410, 64 + len(bands) * 40
    s.rect(px, py, pw, ph, fill=PAPER, op=0.6, stroke=INK, sw=1.6, rx=10)
    n = int(pw / 14)
    for k in range(n):  # dotted top/bottom rivets
        xx = px + (k + 0.5) * pw / n
        s.dot(xx, py, 1.1, INK, op=0.5); s.dot(xx, py + ph, 1.1, INK, op=0.5)
    s.text(px + 20, py + 30, "live fact bundle", 13, INK, font=MONO, weight="bold")
    s.line(px + 18, py + 42, px + pw - 18, py + 42, INK_SOFT, 0.8, op=0.6)
    for i, (name, val) in enumerate(bands):
        ry = py + 70 + i * 40
        s.dot(px + 26, ry - 4, 4, DYES[i % len(DYES)])
        s.text(px + 40, ry, name, 13, INK, font=MONO)
        s.text(px + pw - 20, ry, val, 13, DYES[i % len(DYES)], font=MONO, anchor="end", weight="bold")
    s.text(px + pw / 2, py + ph + 26, "each value a signed fact", 11, INK_SOFT, font=MONO, anchor="middle")

    # bundle -> receipt
    for i in range(len(bands)):
        ry = py + 70 + i * 40 - 4
        s.flow((px + pw + 4, ry), (CX - 96, CY), INK_SOFT, 0.9, op=0.5, bow=14 * math.sin(i * 1.4), beads=False)

    # ---- centre: the signed receipt
    s.lotus(CX, CY, 78)
    s.seal(CX + 66, CY - 66, 11, fill=LEAF)
    s.text(CX, CY + 100, spec["receipt"], 13, INK, font=MONO, anchor="middle", weight="bold")
    s.text(CX, CY + 117, "signed · one 26-char handle", 11, INK_SOFT, font=MONO, anchor="middle")

    # ---- right: the parties who act on it
    cons = spec["consumers"]
    rx = 1340
    ys = [CY - (len(cons) - 1) * 78 / 2 + i * 78 for i in range(len(cons))]
    for i, (name, sub) in enumerate(cons):
        y = ys[i]
        s.flow((CX + 96, CY), (rx - 30, y), LEAF, 1.4, op=0.85, bow=18 * (i - (len(cons) - 1) / 2), glow=True)
        s.roundel(rx, y, 22, DYES[(i + 2) % len(DYES)], signed=False, petal=(i % 2 == 0))
        s.text(rx + 34, y - 2, name, 13, INK, font=MONO)
        if sub:
            s.text(rx + 34, y + 14, sub, 10, INK_SOFT, font=MONO)
    s.text(rx + 10, ys[0] - 44, "acts on the receipt", 11, INK, font=MONO, anchor="middle")

    # ---- insight line
    s.text(CX, H - 96, spec["insight"], 15, INK, font=SERIF, anchor="middle", italic=True)
    s.text(CX, H - 60, "anyone re-checks it at /verify · no trust in the server", 11, INK_SOFT, font=MONO, anchor="middle")

    path = f"{OUT}/{spec['key']}.svg"
    s.save(path)
    return path


# ---- data table: accurate per-industry content (bands are real emem keys) ----
SCENES = {
"30-maritime-ports": dict(industry="maritime & ports", seed=30,
    tagline="under-keel clearance and pilotage from one signed bundle",
    bands=[("tide.height_now", "+1.8 m"), ("weather.wind_speed_10m", "14 m/s"),
           ("marine.current_speed", "1.1 m/s"), ("marine.surge_residual", "+0.4 m"),
           ("ais.dwell_min_60", "17 vessels")],
    receipt="PILOTAGE RECEIPT",
    consumers=[("port authority", "berth window"), ("pilot", "under-keel calc"),
               ("vessel master", "go / no-go")],
    insight="The under-keel calc carries the tide, surge, and wind that justify it."),

"20-carbon-mrv": dict(industry="voluntary carbon-market MRV", seed=20,
    tagline="a carbon credit where the carbon is content-addressed",
    bands=[("forest_change.treecover2000", "82 %"), ("forest_change.lossyear", "none"),
           ("firms.brightness", "no fire"), ("esa_cci_biomass.agb_2020", "164 t/ha"),
           ("indices.ndvi", "0.81")],
    receipt="CARBON CERTIFICATE",
    consumers=[("registry", "issuance"), ("auditor", "10-yr re-recall"),
               ("buyer", "retires the tonne")],
    insight="Each tonne carries the pixels that justify it; a 10-year audit is just a recall."),

"16-disaster-response": dict(industry="disaster response", seed=16,
    tagline="one common operating picture, every responder on the same address",
    bands=[("firms.brightness", "active fire"), ("indices.ndwi", "flood +0.3"),
           ("sar.vv_drop_db", "-6 dB"), ("weather.wind_speed_10m", "22 m/s"),
           ("worldpop.population", "1,240 / cell")],
    receipt="SITUATION RECEIPT",
    consumers=[("federal EM", "situation report"), ("fire service", "tactical map"),
               ("utility ops", "restoration"), ("public portal", "shelter routing")],
    insight="Every responder is looking at the same cell, citing the same signed facts."),

"27-precision-agriculture": dict(industry="precision agriculture", seed=27,
    tagline="per-field decisions citing per-pixel facts",
    bands=[("indices.ndvi", "0.74"), ("indices.ndwi", "0.12"),
           ("soilgrids.phh2o_0_5", "6.3 pH"), ("weather.precip_24h", "8 mm"),
           ("modis.lst_day_8day", "29 °C")],
    receipt="SEASON RECEIPT",
    consumers=[("agronomist", "per-zone plan"), ("input supplier", "variable rate"),
               ("lender", "underwrites the yield")],
    insight="Per-zone math the input brand can audit and the bank can underwrite."),

"13-parametric-insurance": dict(industry="parametric insurance", seed=13,
    tagline="the trigger and the payout cite the same signed pixels",
    bands=[("weather.precip_72h", "11 mm"), ("indices.ndwi", "+0.34"),
           ("sar.vv_drop_db", "-7 dB"), ("modis.lst_day_8day", "41 °C"),
           ("tide.surge_residual", "+1.2 m")],
    receipt="TRIGGER RECEIPT",
    consumers=[("insurer", "pays on the index"), ("reinsurer", "cedes the cat"),
               ("farmer", "no claims adjuster")],
    insight="No basis-risk argument: the index payout cites the facts that set it off."),

"28-forestry-timber": dict(industry="forestry & timber", seed=28,
    tagline="legal-harvest provenance that travels with the log",
    bands=[("forest_change.lossyear", "2019"), ("jrc_tmf.deforestation_year", "none"),
           ("wdpa.protected", "outside"), ("indices.ndvi", "0.79"),
           ("esa_cci_biomass.agb_2020", "188 t/ha")],
    receipt="HARVEST RECEIPT",
    consumers=[("mill", "chain of custody"), ("certifier", "FSC / PEFC"),
               ("importer", "EUDR + Lacey")],
    insight="Each log lands with the stand it came from, signed and outside the protected line."),
}


def main():
    keys = sys.argv[1:] or list(SCENES)
    import cairosvg
    for k in keys:
        if k not in SCENES:
            print("unknown:", k); continue
        spec = dict(SCENES[k]); spec["key"] = k
        p = scene(spec)
        cairosvg.svg2png(url=p, write_to=f"/tmp/{k}.png", output_width=1000)
        print("wrote", p, "+ /tmp/" + k + ".png")


if __name__ == "__main__":
    main()
