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
    s.paper()

    # ---- geometry of the three acts: ledger (left), receipt (centre), parties
    bands = spec["bands"]
    px, py, pw = 88, 232, 458
    ph = 62 + len(bands) * 44
    CX, CY = 858, 470          # central signed receipt
    LR = 118                   # lotus radius (bigger, lusher)
    cons = spec["consumers"]
    rcx = 1276                 # right column roundel anchor (labels run right)
    cell_cx, cell_cy = px + pw / 2, py + ph + 198

    # keep-clear discs so the dense ground never lands under busy zones / text
    avoid = [
        (CX, CY, LR * 1.7),                       # central lotus + label
        (px + pw / 2, py + ph / 2, pw * 0.62),    # ledger card
        (cell_cx, cell_cy, 132),                  # the cell glyph + caption
        (rcx + 110, CY, 220),                     # the parties column + labels
        (W / 2, 150, 560),                        # the title band
        (W / 2, H - 96, 560),                     # the footer band
    ]
    cys = [CY - (len(cons) - 1) * 116 / 2 + i * 116 for i in range(len(cons))]
    for cy in cys:
        avoid.append((rcx + 110, cy, 150))

    s.fill_ground(avoid=avoid, top=176)
    s.border()

    # ---- header (serif title, mono tagline, bud-vine frieze) -------------
    s.text(78, 108, "emem for " + spec["industry"], 30, INK, font=SERIF)
    s.text(80, 142, spec["tagline"], 13, INK_SOFT, font=MONO)
    s.frieze(168, 78, W - 78)

    # storytelling motifs in the open zones, so the empty paper reads as a
    # painting. They sit in the gaps between the three acts and the parties.
    s.sun(CX, 250, 30)                               # sun centred over the receipt
    s.bird(CX - 250, 262, 40, col=TEAL, angle=0.1)   # peacock left of the sun
    s.bird(CX + 250, 262, 40, col=LAC, angle=-0.1)   # mirrored peacock right
    s.vine(96, 196, 96, py - 6, col=LEAF, n=3, amp=12)
    # fish shoal + vine swimming across the lower band, below the receipt
    s.fish(150, H - 150, 28, col=INDIGO, angle=-0.15)
    s.fish(205, H - 118, 22, col=TEAL, angle=0.12)
    s.fish(112, H - 108, 18, col=LAC, angle=0.25)
    s.vine(255, H - 200, 560, H - 210, col=LEAF, n=5, amp=20)
    s.fish(660, H - 196, 24, col=VERMIL, angle=0.1)
    s.fish(720, H - 220, 18, col=TURMERIC, angle=-0.1)
    s.fish(900, H - 200, 22, col=INDIGO, angle=0.08)
    s.bud(1010, H - 208, 22, col=VERMIL)
    s.vine(W - 110, 196, W - 110, py - 6, col=LEAF, n=3, amp=12)

    # faint field rings under the receipt (kept subtle)
    for R in (LR + 70, LR + 150):
        s.dot_ring(CX, CY, R, int(R / 6.2), 1.1, INK, op=0.06)

    # ---- left: live fact bundle ledger (bigger type, bolder card) --------
    s.rect(px, py, pw, ph, fill=PAPER, op=0.78, stroke=INK, sw=2.2, rx=12)
    s.rect(px + 5, py + 5, pw - 10, ph - 10, stroke=INK, sw=0.8, rx=9, op=0.5)
    nrv = int(pw / 16)
    for k in range(nrv):
        xx = px + (k + 0.5) * pw / nrv
        s.dot(xx, py, 1.1, INK, op=0.5); s.dot(xx, py + ph, 1.1, INK, op=0.5)
    s.seal(px + 28, py + 30, 8, fill=LEAF)
    s.text(px + 48, py + 36, "live fact bundle", 14, INK, font=MONO, weight="bold")
    s.line(px + 18, py + 52, px + pw - 18, py + 52, INK_SOFT, 1.0, op=0.6)
    for i, (name, val) in enumerate(bands):
        ry = py + 86 + i * 44
        s.dot(px + 30, ry - 5, 4.5, DYES[i % len(DYES)])
        s.text(px + 46, ry, name, 13, INK, font=MONO)
        s.text(px + pw - 22, ry, val, 13, DYES[i % len(DYES)], font=MONO, anchor="end", weight="bold")
    s.text(px + pw / 2, py + ph + 30, "each value a signed fact at one cell", 11.5,
           INK_SOFT, font=MONO, anchor="middle")

    # ---- the place those facts come from: a geolocation cell -------------
    s.flow((cell_cx, cell_cy - 64), (cell_cx, py + ph + 44), INK_SOFT, 1.0, op=0.55, beads=False)
    sz = 62
    s.rect(cell_cx - sz, cell_cy - sz, 2 * sz, 2 * sz, fill=PAPER, op=0.7, stroke=INK, sw=2.0, rx=10)
    for k in (1, 2):
        s.line(cell_cx - sz + k * 2 * sz / 3, cell_cy - sz + 6, cell_cx - sz + k * 2 * sz / 3,
               cell_cy + sz - 6, INK_SOFT, 0.8, op=0.45)
        s.line(cell_cx - sz + 6, cell_cy - sz + k * 2 * sz / 3, cell_cx + sz - 6,
               cell_cy - sz + k * 2 * sz / 3, INK_SOFT, 0.8, op=0.45)
    s.rect(cell_cx - sz / 3, cell_cy - sz / 3, 2 * sz / 3, 2 * sz / 3, fill=LEAF, op=0.22, stroke=INK, sw=1.4)
    s.circle(cell_cx, cell_cy, 7, stroke=INK, sw=1.2); s.dot(cell_cx, cell_cy, 3.6, VERMIL)
    s.text(cell_cx, cell_cy + sz + 28, spec.get("cell", "cell defi.zb5cf.nura.zd83c"), 11.5,
           INK, font=MONO, anchor="middle")
    s.text(cell_cx, cell_cy + sz + 46, "one cell · about 9.55 m", 11, INK_SOFT, font=MONO, anchor="middle")

    # bundle -> receipt (one bowed flow per band)
    for i in range(len(bands)):
        ry = py + 86 + i * 44 - 5
        s.flow((px + pw + 6, ry), (CX - LR - 8, CY), INK_SOFT, 1.0, op=0.5,
               bow=20 * math.sin(i * 1.4), beads=False)

    # ---- right: the parties who act on it (bigger roundels + flows) ------
    inners = ["fish", "leaf", "hatch", "ring"]
    for i, (name, sub) in enumerate(cons):
        y = cys[i]
        s.flow((CX + LR + 8, CY), (rcx - 46, y), LEAF, 1.6, op=0.85,
               bow=26 * (i - (len(cons) - 1) / 2), glow=True)
        s.roundel(rcx, y, 30, DYES[(i + 2) % len(DYES)], petal=(i % 2 == 0),
                  inner=inners[i % len(inners)])
        s.text(rcx + 48, y - 4, name, 14, INK, font=MONO, weight="bold")
        if sub:
            s.text(rcx + 48, y + 18, sub, 11, INK_SOFT, font=MONO)
    s.text(rcx, cys[0] - 76, "acts on the receipt", 12, INK, font=MONO, anchor="middle")

    # ---- centre: the signed receipt (big lush lotus + ribbon) ------------
    s.lotus(CX, CY, LR)
    s.seal(CX + LR * 0.84, CY - LR * 0.84, 13, fill=LEAF)
    rw = max(230, 11 * len(spec["receipt"]) + 64)
    ry0 = CY + LR + 78
    s.rect(CX - rw / 2, ry0 - 22, rw, 44, fill=GOLD_PALE, op=0.55, stroke=INK, sw=1.8, rx=7)
    s.rect(CX - rw / 2 + 4, ry0 - 18, rw - 8, 36, stroke=INK, sw=0.7, rx=5, op=0.5)
    s.dot(CX - rw / 2, ry0, 3.4, INK); s.dot(CX + rw / 2, ry0, 3.4, INK)
    s.text(CX, ry0 + 6, spec["receipt"], 15, INK, font=MONO, anchor="middle", weight="bold")
    s.text(CX, ry0 + 36, "signed · one 26-char handle", 11.5, INK_SOFT, font=MONO, anchor="middle")

    # ---- insight + verify footer (serif italic, mono note) ---------------
    s.line(220, H - 128, W - 220, H - 128, INK_SOFT, 0.8, op=0.45)
    s.dot(220, H - 128, 2.4, VERMIL); s.dot(W - 220, H - 128, 2.4, VERMIL)
    s.text(W / 2, H - 90, spec["insight"], 16, INK, font=SERIF, anchor="middle", italic=True)
    s.text(W / 2, H - 58, "anyone re-checks it at /verify · no trust in the server", 11.5,
           INK_SOFT, font=MONO, anchor="middle")

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

"11-insurance-pc": dict(industry="property & casualty underwriting", seed=11,
    tagline="price every parcel from the same signed truth",
    bands=[("copdem30m.elevation_mean", "12 m"), ("surface_water.recurrence", "4 %"),
           ("indices.ndwi", "-0.10"), ("firms.brightness", "no fire"),
           ("worldpop.population", "1,240")],
    receipt="PARCEL-RISK RECEIPT",
    consumers=[("underwriter", "prices the parcel"), ("actuary", "reserves"),
               ("regulator", "Solvency II Pillar 3")],
    insight="Every parcel priced from the same signed facts, audit-ready under NAIC and Solvency II."),

"12-reinsurance-cat": dict(industry="reinsurance catastrophe", seed=12,
    tagline="cat layers priced on facts, not on a black-box curve",
    bands=[("surface_water.recurrence", "22 %"), ("weather.wind_speed_10m", "38 m/s"),
           ("tide.surge_residual", "+1.2 m"), ("copdem30m.elevation_mean", "3 m"),
           ("firms.brightness", "active")],
    receipt="CAT-LAYER RECEIPT",
    consumers=[("reinsurer", "cedes the cat"), ("cat modeler", "replays the layer"),
               ("rating agency", "model-agnostic")],
    insight="Two modelers can disagree on a 1-in-250 layer and cite exactly where."),

"14-csrd-disclosure": dict(industry="CSRD / ESRS E1 climate disclosure", seed=14,
    tagline="disclose with receipts an auditor can verify on a flight",
    bands=[("copdem30m.elevation_mean", "8 m"), ("surface_water.recurrence", "6 %"),
           ("modis.lst_day_8day", "41 °C"), ("indices.ndvi", "0.22"),
           ("koppen.climate_class", "BWh")],
    receipt="DISCLOSURE RECEIPT",
    consumers=[("CFO", "ESRS E1 filing"), ("auditor", "assurance"),
               ("regulator", "42 jurisdictions")],
    insight="Each physical-risk claim carries the cell and the pixels behind it."),

"15-defense-geoint": dict(industry="defence & GEOINT", seed=15,
    tagline="fuse classified and open intel without losing chain of custody",
    bands=[("sar.vv_drop_db", "-6 dB"), ("indices.ndvi", "0.31"),
           ("modis.lst_day_8day", "33 °C"), ("esa_worldcover.lc_2021", "built"),
           ("copdem30m.elevation_mean", "220 m")],
    receipt="FUSION RECEIPT",
    consumers=[("analyst", "the read"), ("command", "the call"),
               ("coalition", "verifies offline")],
    insight="Three distinct attesters, one address; the verifier needs only the pubkey."),

"17-urban-planning": dict(industry="urban planning & permits", seed=17,
    tagline="a permit a citizen can audit on their phone",
    bands=[("surface_water.recurrence", "1-in-100"), ("copdem30m.elevation_mean", "5 m"),
           ("esa_worldcover.lc_2021", "urban"), ("indices.ndvi", "0.18"),
           ("worldpop.population", "3,400")],
    receipt="PERMIT RECEIPT",
    consumers=[("planner", "the conditions"), ("citizen", "verifies in 47 ms"),
               ("council", "the record")],
    insight="Eight facts attested; the citizen verifies offline with no callback to the issuer."),

"18-land-registry": dict(industry="land registry & title", seed=18,
    tagline="every parcel a content-addressed title, shared byte-for-byte",
    bands=[("field_boundaries.ftw", "polygon"), ("forest_change.lossyear", "none"),
           ("indices.ndvi", "0.66"), ("surface_water.recurrence", "2 %"),
           ("copdem30m.elevation_mean", "41 m")],
    receipt="TITLE RECEIPT",
    consumers=[("registrar", "the entry"), ("surveyor", "the boundary"),
               ("bank", "the collateral")],
    insight="Two offices resolve the same parcel handle to the same bytes."),

"19-esg-investing": dict(industry="ESG investing", seed=19,
    tagline="ESG scores with the receipts attached",
    bands=[("forest_change.lossyear", "flagged"), ("firms.brightness", "2 events"),
           ("wdpa.protected", "overlap"), ("modis.lst_day_8day", "+3 °C"),
           ("indices.ndvi", "0.44")],
    receipt="ESG-SCORE RECEIPT",
    consumers=[("portfolio mgr", "the score"), ("asset owner", "the mandate"),
               ("ratings provider", "replays")],
    insight="Each holding's score carries the cells and the pixels behind it."),

"21-commodity-trading": dict(industry="soft-commodity trading desks", seed=21,
    tagline="trade a yield forecast where every field has a citation",
    bands=[("indices.ndvi", "0.74"), ("soilgrids.bdod_0_5cm", "1.32 g/cm³"),
           ("weather.temperature_2m", "28 °C"), ("weather.precip_24h", "8 mm"),
           ("modis.lst_day_8day", "31 °C")],
    receipt="DESK-NOTE RECEIPT",
    consumers=[("trader", "the position"), ("risk", "the limit"),
               ("back-office", "replays in 3 yrs")],
    insight="A weekly desk note the back office can replay in three years."),

"22-real-estate": dict(industry="real-estate due diligence", seed=22,
    tagline="every claim about the parcel has a 26-character handle",
    bands=[("surface_water.recurrence", "no exposure"), ("copdem30m.elevation_mean", "18 m"),
           ("indices.ndwi", "-0.08"), ("modis.lst_day_8day", "36 °C"),
           ("esa_worldcover.lc_2021", "built")],
    receipt="DUE-DILIGENCE RECEIPT",
    consumers=[("buyer", "the offer"), ("lender", "the LTV"),
               ("insurer", "the premium")],
    insight="Every claim about the parcel resolves to one signed handle."),

"23-mining-exploration": dict(industry="mining exploration", seed=23,
    tagline="the terrain and surface-change context layer for exploration",
    bands=[("copdem30m.elevation_mean", "1,420 m"), ("indices.ndvi", "0.08"),
           ("esa_worldcover.lc_2021", "bare"), ("surface_water.recurrence", "0 %"),
           ("sar.vv_drop_db", "-2 dB")],
    receipt="CONTEXT RECEIPT",
    consumers=[("geologist", "the basemap"), ("permitting", "the footprint"),
               ("community", "the disclosure")],
    insight="emem signs the terrain and surface-change context; the geophysics stays with the survey."),

"24-pipeline-monitoring": dict(industry="pipeline integrity monitoring", seed=24,
    tagline="every alert on the line carries its own evidence",
    bands=[("sar.vv_drop_db", "-7 dB"), ("indices.ndvi", "stress -0.2"),
           ("modis.lst_day_8day", "+4 °C"), ("surface_water.recurrence", "1 %"),
           ("copdem30m.elevation_mean", "96 m")],
    receipt="ALERT RECEIPT",
    consumers=[("control room", "the dispatch"), ("integrity eng.", "the dig"),
               ("regulator", "PHMSA part 191")],
    insight="Each alert ships with the SAR ground-drop and the thermal that raised it."),

"25-renewable-siting": dict(industry="renewable-energy siting", seed=25,
    tagline="a site score where every input has a public address",
    bands=[("copdem30m.slope_pct", "4 %"), ("esa_worldcover.lc_2021", "bare"),
           ("wdpa.protected", "clear"), ("surface_water.recurrence", "0 %"),
           ("worldpop.population", "120")],
    receipt="SITE-SCORE RECEIPT",
    consumers=[("developer", "the bid"), ("grid operator", "the interconnect"),
               ("lender", "the PPA")],
    insight="Every input to the site score has a public address the lender can re-pull."),

"26-grid-resilience": dict(industry="grid & transmission resilience", seed=26,
    tagline="transmission resilience facts that survive an audit",
    bands=[("weather.wind_speed_10m", "31 m/s"), ("modis.lst_day_8day", "44 °C"),
           ("firms.brightness", "near line"), ("surface_water.recurrence", "substation"),
           ("copdem30m.elevation_mean", "210 m")],
    receipt="RESILIENCE RECEIPT",
    consumers=[("grid operator", "the switching"), ("utility", "the hardening"),
               ("regulator", "the after-action")],
    insight="Each resilience fact at each substation survives the after-action audit."),

"29-eudr-supply-chain": dict(industry="EUDR supply chain", seed=29,
    cell="plot defi.zb3bb.zbb96.jUko",
    tagline="a deforestation-free plot, signed into one handle that clears customs",
    bands=[("jrc_gfc2020.forest_2020", "forest"), ("forest_change.lossyear", "none"),
           ("forest_change.treecover2000", "82 %"), ("sar.vv_drop_db", "-1 dB"),
           ("indices.ndvi", "0.80")],
    receipt="EUDR DDS",
    consumers=[("operator", "the statement"), ("customs", "TRACES NT"),
               ("importer", "clears the border")],
    insight="A geolocated plot, proven deforestation-free, in one signed handle the border can re-check."),
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
