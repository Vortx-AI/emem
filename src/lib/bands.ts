export type BandFamily =
  | "foundation"
  | "optical"
  | "radar"
  | "terrain"
  | "climate"
  | "soil"
  | "vegetation"
  | "landcover"
  | "water"
  | "human"
  | "vision"
  | "topology"
  | "encoding"
  | "reserved";

export type BandTempo = "static" | "slow" | "medium" | "fast" | "ultra_fast";

export type BandEntry = {
  key: string;
  offset: number;
  dims: number;
  family: BandFamily;
  label: string;
  source: string;
  nativeResolutionMeters: number | null;
  tempo: BandTempo;
};

export const GEOTESSERA1792_DIM = 1792;

export const BANDS: readonly BandEntry[] = [
  { key: "geotessera",        offset: 0,    dims: 128, family: "foundation", label: "GeoTessera base",           source: "Clay FM · 10m UTM seasonal composite",            nativeResolutionMeters: 10, tempo: "medium" },
  { key: "alphaearth",        offset: 128,  dims: 576, family: "foundation", label: "AlphaEarth 9y history",     source: "gs://alphaearth_foundations · 2017–2025 · 64D×9y",   nativeResolutionMeters: 10, tempo: "medium" },
  { key: "sentinel2_raw",     offset: 704,  dims: 10,  family: "optical",    label: "Sentinel-2 raw",            source: "Copernicus · 10 bands · 5-day revisit",               nativeResolutionMeters: 10, tempo: "fast" },
  { key: "sentinel1_raw",     offset: 714,  dims: 2,   family: "radar",      label: "Sentinel-1 SAR",            source: "Copernicus · VV/VH · 6–12d revisit",                  nativeResolutionMeters: 10, tempo: "fast" },
  { key: "dem",               offset: 716,  dims: 3,   family: "terrain",    label: "SRTM elevation",            source: "SRTM · elevation + slope + aspect",                    nativeResolutionMeters: 30, tempo: "static" },
  { key: "landcover",         offset: 719,  dims: 8,   family: "landcover",  label: "Landcover fusion",          source: "Overture → SAM3 → NDVI · 8-class annual",              nativeResolutionMeters: 10, tempo: "slow" },
  { key: "climate",           offset: 727,  dims: 4,   family: "climate",    label: "ERA5 summary",              source: "ERA5 · temp / precip / clay / OC normals",             nativeResolutionMeters: null, tempo: "slow" },
  { key: "indices",           offset: 731,  dims: 3,   family: "optical",    label: "NDVI / NDWI / BSI",         source: "Sentinel-2 derived · per revisit",                     nativeResolutionMeters: 10, tempo: "fast" },
  { key: "spatial_fourier",   offset: 734,  dims: 96,  family: "encoding",   label: "Spatial Fourier basis",     source: "sin/cos(lat,lng) multi-frequency · static",            nativeResolutionMeters: null, tempo: "static" },
  { key: "temporal_fourier",  offset: 830,  dims: 64,  family: "encoding",   label: "Temporal Fourier basis",    source: "sin/cos(day_of_year) multi-frequency · per revisit",   nativeResolutionMeters: null, tempo: "fast" },
  { key: "sam3_visual",       offset: 894,  dims: 192, family: "vision",     label: "SAM3 polygon features",     source: "Meta SAM3 · per detection",                            nativeResolutionMeters: 10, tempo: "slow" },
  { key: "qwen_visual",       offset: 1086, dims: 192, family: "vision",     label: "Qwen-VL tile features",     source: "Qwen-VL · per 1024px tile descriptor",                 nativeResolutionMeters: 10, tempo: "slow" },
  { key: "terrain_derived",   offset: 1278, dims: 32,  family: "terrain",    label: "Slope/TPI/TWI/aspect",      source: "Derived from SRTM · static",                           nativeResolutionMeters: 30, tempo: "static" },
  { key: "temporal_diff",     offset: 1310, dims: 64,  family: "vegetation", label: "AlphaEarth year-to-year",   source: "AE deltas · 2017→2025",                                nativeResolutionMeters: 10, tempo: "slow" },
  { key: "phenology",         offset: 1374, dims: 32,  family: "vegetation", label: "NDVI×SAR phenology",        source: "cross-band seasonal correlations",                     nativeResolutionMeters: 10, tempo: "medium" },
  { key: "topology",          offset: 1406, dims: 32,  family: "topology",   label: "Local neighborhood",        source: "local composition features · static",                  nativeResolutionMeters: 30, tempo: "static" },
  { key: "multiscale",        offset: 1438, dims: 64,  family: "vegetation", label: "Local vs neighborhood",     source: "multi-scale aggregates · per revisit",                 nativeResolutionMeters: 10, tempo: "fast" },
  { key: "nightlights",       offset: 1502, dims: 8,   family: "human",      label: "VIIRS night lights",        source: "VIIRS DNB · monthly composite",                        nativeResolutionMeters: 500, tempo: "fast" },
  { key: "ghsl",              offset: 1510, dims: 8,   family: "human",      label: "GHSL built-up",             source: "JRC GHSL · 5-year snapshot",                           nativeResolutionMeters: 30, tempo: "slow" },
  { key: "population",        offset: 1518, dims: 8,   family: "human",      label: "WorldPop density",          source: "WorldPop · annual",                                    nativeResolutionMeters: 100, tempo: "slow" },
  { key: "forest_change",     offset: 1526, dims: 12,  family: "landcover",  label: "Hansen forest change",      source: "Hansen Global · annual",                               nativeResolutionMeters: 30, tempo: "slow" },
  { key: "mangrove",          offset: 1538, dims: 4,   family: "landcover",  label: "Global Mangrove Watch",     source: "GMW · annual",                                         nativeResolutionMeters: 25, tempo: "slow" },
  { key: "protected",         offset: 1542, dims: 4,   family: "human",      label: "WDPA protected areas",      source: "WDPA · IUCN category · rarely changes",                nativeResolutionMeters: null, tempo: "static" },
  { key: "surface_water",     offset: 1546, dims: 12,  family: "water",      label: "JRC surface water",         source: "JRC GSW · occurrence/recurrence · monthly",            nativeResolutionMeters: 30, tempo: "fast" },
  { key: "ocean_chl",         offset: 1558, dims: 4,   family: "water",      label: "Ocean chlorophyll",         source: "MODIS OCx · daily",                                    nativeResolutionMeters: 4000, tempo: "fast" },
  { key: "koppen",            offset: 1562, dims: 32,  family: "climate",    label: "Köppen climate zone",       source: "Beck Köppen-Geiger · one-hot · millennial",            nativeResolutionMeters: 1000, tempo: "static" },
  { key: "terraclimate",      offset: 1594, dims: 20,  family: "climate",    label: "TerraClimate monthly",      source: "TerraClimate · PET/PDSI/PPT/TMIN/TMAX",                nativeResolutionMeters: 4000, tempo: "medium" },
  { key: "cop_dem",           offset: 1614, dims: 8,   family: "terrain",    label: "Copernicus GLO-30 DEM",     source: "Copernicus DEM · elevation/slope/aspect/TPI",          nativeResolutionMeters: 30, tempo: "static" },
  { key: "soilgrids",         offset: 1622, dims: 20,  family: "soil",       label: "ISRIC SoilGrids",           source: "SoilGrids · pH/OC/sand/silt/clay × 3 depths",          nativeResolutionMeters: 250, tempo: "slow" },
  { key: "ecoregions",        offset: 1642, dims: 20,  family: "landcover",  label: "WWF ecoregions",            source: "WWF Terrestrial Ecoregions · one-hot · static",        nativeResolutionMeters: null, tempo: "static" },
  { key: "admin",             offset: 1662, dims: 10,  family: "human",      label: "Admin & political",         source: "GeoNames + WDPA · country/admin1/urban flags",         nativeResolutionMeters: null, tempo: "slow" },
  { key: "reserved",          offset: 1672, dims: 120, family: "reserved",   label: "Reserved",                  source: "Future: wind / fire / traffic / AIS / air quality",   nativeResolutionMeters: null, tempo: "static" }
] as const;

export const BAND_BY_KEY: Readonly<Record<string, BandEntry>> = Object.freeze(
  BANDS.reduce<Record<string, BandEntry>>((map, band) => {
    map[band.key] = band;
    return map;
  }, {})
);

export const FAMILIES: readonly BandFamily[] = [
  "foundation",
  "optical",
  "radar",
  "terrain",
  "climate",
  "soil",
  "vegetation",
  "landcover",
  "water",
  "human",
  "vision",
  "topology",
  "encoding",
  "reserved"
];

export const FAMILY_LABEL: Readonly<Record<BandFamily, string>> = Object.freeze({
  foundation: "Foundation embeddings",
  optical: "Optical sensors",
  radar: "Radar sensors",
  terrain: "Terrain & topography",
  climate: "Climate",
  soil: "Soil",
  vegetation: "Vegetation & phenology",
  landcover: "Land cover & ecology",
  water: "Water",
  human: "Human presence",
  vision: "Native vision features",
  topology: "Local topology",
  encoding: "Positional encoding",
  reserved: "Reserved"
});

export function bandsByFamily(family: BandFamily): BandEntry[] {
  return BANDS.filter((band) => band.family === family);
}

export function familyDims(family: BandFamily): number {
  return bandsByFamily(family).reduce((sum, band) => sum + band.dims, 0);
}

export function totalDims(): number {
  return BANDS.reduce((sum, band) => sum + band.dims, 0);
}

export function validateRegistry(): void {
  const total = totalDims();

  if (total !== GEOTESSERA1792_DIM) {
    throw new Error(`band registry sums to ${total}, expected ${GEOTESSERA1792_DIM}`);
  }

  let cursor = 0;

  for (const band of BANDS) {
    if (band.offset !== cursor) {
      throw new Error(
        `band "${band.key}" starts at ${band.offset} but expected ${cursor} (gap or overlap)`
      );
    }

    cursor += band.dims;
  }
}

validateRegistry();
