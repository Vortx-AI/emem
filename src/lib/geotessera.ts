import type { BandFamily, BandTempo } from "./bands.js";

export type Coordinates = {
  lat: number;
  lng: number;
};

export type TesseraBounds = {
  southwest: Coordinates;
  northeast: Coordinates;
};

export type GeotesseraVector = {
  model: "geotessera-128d-v1";
  dimensions: 128;
  values: number[];
  checksum: string;
};

export type BandStatus = "live" | "procedural" | "deferred" | "unavailable";

export type BandCoverage = {
  key: string;
  label: string;
  family: BandFamily;
  tempo: BandTempo;
  source: string;
  offset: number;
  dims: number;
  status: BandStatus;
  note?: string;
  summary?: Array<{ name: string; value: number | string; unit?: string }>;
  sampleValues?: number[];
};

export type Geotessera1792 = {
  model: string;
  dimensions: 1792;
  provider: string;
  capturedAt: string | null;
  liveDims: number;
  proceduralDims: number;
  deferredDims: number;
  unavailableDims: number;
  checksum: string;
  coverage: BandCoverage[];
};

export type TesseraAddress = {
  country: string;
  square: TesseraBounds;
  nearestPlace: string;
  coordinates: Coordinates;
  words: string;
  language: "en";
  locale: null;
  map: string;
  cellId: string;
  geotessera128: GeotesseraVector;
  intelligence?: Geotessera1792;
};

export type GridSection = {
  type: "FeatureCollection";
  features: Array<{
    type: "Feature";
    properties: {
      axis: "lat" | "lng";
      cell?: number;
      stepMeters: number;
    };
    geometry: {
      type: "LineString";
      coordinates: Array<[number, number]>;
    };
  }>;
  properties: {
    cellSizeMeters: number;
    step: number;
    lineCount: number;
  };
};

export type Suggestion = Pick<
  TesseraAddress,
  "country" | "nearestPlace" | "words" | "language" | "locale"
> & {
  distanceToFocusKm: number | null;
  rank: number;
};

type PlaceResolver = (coordinates: Coordinates) => {
  country: string;
  nearestPlace: string;
};

const EARTH_RADIUS_METERS = 6378137;
const HALF_WORLD_METERS = Math.PI * EARTH_RADIUS_METERS;
const WORLD_METERS = HALF_WORLD_METERS * 2;
const CELL_SIZE_METERS = 3;
const MAX_MERCATOR_LAT = 85.05112878;
const BASE = 65536n;
const WORD_SPACE = BASE ** 3n;
const WORD_MASK = BASE - 1n;
const AFFINE_A = 25214903917n;
const AFFINE_B = 1442695040888963407n % WORD_SPACE;
const AFFINE_A_INV = modularInverse(AFFINE_A, WORD_SPACE);

export const GEOTESSERA = {
  cellSizeMeters: CELL_SIZE_METERS,
  maxMercatorLat: MAX_MERCATOR_LAT,
  columns: Math.ceil(WORLD_METERS / CELL_SIZE_METERS),
  rows: Math.ceil(WORLD_METERS / CELL_SIZE_METERS)
} as const;

const TOTAL_CELLS = BigInt(GEOTESSERA.columns) * BigInt(GEOTESSERA.rows);
const DOT_LIKE = /[.｡。･・︒។։။۔።।]+/g;
const SYLLABLES = [
  "ba",
  "be",
  "ci",
  "da",
  "fo",
  "ga",
  "hi",
  "jo",
  "lu",
  "me",
  "ni",
  "po",
  "ra",
  "se",
  "tu",
  "va"
] as const;

const SYLLABLE_INDEX = new Map<string, number>(
  SYLLABLES.map((syllable, index) => [syllable, index])
);

const DEFAULT_PLACE = {
  country: "XZ",
  nearestPlace: "Open world grid"
};

export function coordinatesToTessera(
  coordinates: Coordinates,
  resolvePlace: PlaceResolver = () => DEFAULT_PLACE
): TesseraAddress {
  const cell = coordinatesToCell(coordinates);
  return cellToTessera(cell.cellId, resolvePlace);
}

export function wordsToTessera(
  words: string,
  resolvePlace: PlaceResolver = () => DEFAULT_PLACE
): TesseraAddress {
  const cellId = wordsToCellId(words);
  return cellToTessera(cellId, resolvePlace);
}

export function normalizeWords(input: string): string {
  return input
    .trim()
    .toLowerCase()
    .replace(/^\/{3}/, "")
    .replace(DOT_LIKE, ".")
    .replace(/\s+/g, ".")
    .replace(/\.+/g, ".")
    .replace(/^\./, "")
    .replace(/\.$/, "");
}

export function looksLikeWords(input: string): boolean {
  return normalizeWords(input).split(".").length === 3;
}

export function parseCoordinates(input: string): Coordinates | null {
  const match = input
    .trim()
    .match(/^\s*(-?\d+(?:\.\d+)?)\s*[, ]\s*(-?\d+(?:\.\d+)?)\s*$/);

  if (!match) {
    return null;
  }

  const lat = Number(match[1]);
  const lng = Number(match[2]);

  if (!Number.isFinite(lat) || !Number.isFinite(lng)) {
    return null;
  }

  if (lat < -90 || lat > 90 || lng < -180 || lng > 180) {
    return null;
  }

  return { lat, lng };
}

export function generateSuggestions(
  input: string,
  focus?: Coordinates,
  limit = 6,
  resolvePlace: PlaceResolver = () => DEFAULT_PLACE
): Suggestion[] {
  const normalized = normalizeWords(input);
  const parts = normalized.split(".").filter(Boolean);

  if (parts.length === 0) {
    return [];
  }

  const focusCell = focus ? coordinatesToCell(focus) : coordinatesToCell({ lat: 0, lng: 0 });
  const candidates = new Map<string, TesseraAddress>();

  collectLocalSuggestions(parts, focusCell.x, focusCell.y, candidates, limit * 3, resolvePlace);
  collectDecodedSuggestions(parts, candidates, limit * 3, resolvePlace);

  return Array.from(candidates.values())
    .map((address) => ({
      country: address.country,
      nearestPlace: address.nearestPlace,
      words: address.words,
      language: address.language,
      locale: address.locale,
      distanceToFocusKm: focus
        ? round(distanceKm(focus, address.coordinates), 2)
        : null,
      rank: 0
    }))
    .sort((left, right) => {
      const distanceLeft = left.distanceToFocusKm ?? Number.POSITIVE_INFINITY;
      const distanceRight = right.distanceToFocusKm ?? Number.POSITIVE_INFINITY;
      return distanceLeft - distanceRight || left.words.localeCompare(right.words);
    })
    .slice(0, limit)
    .map((suggestion, index) => ({ ...suggestion, rank: index + 1 }));
}

export function gridSectionFromBoundingBox(
  south: number,
  west: number,
  north: number,
  east: number
): GridSection {
  const sw = coordinatesToCell({ lat: south, lng: west });
  const ne = coordinatesToCell({ lat: north, lng: east });
  const minX = Math.min(sw.x, ne.x);
  const maxX = Math.max(sw.x, ne.x);
  const minY = Math.min(sw.y, ne.y);
  const maxY = Math.max(sw.y, ne.y);
  const spanX = Math.max(1, maxX - minX + 1);
  const spanY = Math.max(1, maxY - minY + 1);
  const step = Math.max(1, Math.ceil(Math.max(spanX, spanY) / 96));
  const features: GridSection["features"] = [];

  for (let x = minX; x <= maxX + 1; x += step) {
    const lng = mercatorToLngLat(cellBoundaryToMercatorX(x), 0).lng;
    features.push({
      type: "Feature",
      properties: { axis: "lng", cell: x, stepMeters: step * CELL_SIZE_METERS },
      geometry: {
        type: "LineString",
        coordinates: [
          [lng, south],
          [lng, north]
        ]
      }
    });
  }

  for (let y = minY; y <= maxY + 1; y += step) {
    const lat = mercatorToLngLat(0, cellBoundaryToMercatorY(y)).lat;
    features.push({
      type: "Feature",
      properties: { axis: "lat", cell: y, stepMeters: step * CELL_SIZE_METERS },
      geometry: {
        type: "LineString",
        coordinates: [
          [west, lat],
          [east, lat]
        ]
      }
    });
  }

  return {
    type: "FeatureCollection",
    features,
    properties: {
      cellSizeMeters: CELL_SIZE_METERS,
      step,
      lineCount: features.length
    }
  };
}

export function cellToWords(cellId: bigint): string {
  if (cellId < 0n || cellId >= TOTAL_CELLS) {
    throw new Error("Cell id is outside the emem.dev grid");
  }

  const mixed = (cellId * AFFINE_A + AFFINE_B) % WORD_SPACE;
  const first = Number(mixed & WORD_MASK);
  const second = Number((mixed >> 16n) & WORD_MASK);
  const third = Number((mixed >> 32n) & WORD_MASK);

  return `${indexToWord(first)}.${indexToWord(second)}.${indexToWord(third)}`;
}

export function wordsToCellId(words: string): bigint {
  const parts = normalizeWords(words).split(".");

  if (parts.length !== 3) {
    throw new Error("emem addresses must contain three dot-separated words");
  }

  const [first, second, third] = parts.map(wordToIndex);
  const mixed = BigInt(first) | (BigInt(second) << 16n) | (BigInt(third) << 32n);
  const cellId = positiveModulo((mixed - AFFINE_B) * AFFINE_A_INV, WORD_SPACE);

  if (cellId >= TOTAL_CELLS) {
    throw new Error("This three-word address is outside the active emem.dev grid");
  }

  return cellId;
}

export function geotesseraVector(cellId: bigint, coordinates: Coordinates): GeotesseraVector {
  const seed = fnv1a32(`${cellId}:${coordinates.lat.toFixed(7)}:${coordinates.lng.toFixed(7)}`);
  let state = seed || 0x9e3779b9;
  const values: number[] = [
    coordinates.lat / 90,
    coordinates.lng / 180,
    Math.sin(toRadians(coordinates.lat)),
    Math.cos(toRadians(coordinates.lat)),
    Math.sin(toRadians(coordinates.lng)),
    Math.cos(toRadians(coordinates.lng))
  ];

  while (values.length < 128) {
    state = xorshift32(state);
    values.push((state / 0xffffffff) * 2 - 1);
  }

  const magnitude = Math.sqrt(values.reduce((sum, value) => sum + value * value, 0));
  const normalized = values.map((value) => round(value / magnitude, 6));

  return {
    model: "geotessera-128d-v1",
    dimensions: 128,
    values: normalized,
    checksum: vectorChecksum(normalized)
  };
}

export function distanceKm(left: Coordinates, right: Coordinates): number {
  const deltaLat = toRadians(right.lat - left.lat);
  const deltaLng = toRadians(right.lng - left.lng);
  const lat1 = toRadians(left.lat);
  const lat2 = toRadians(right.lat);
  const a =
    Math.sin(deltaLat / 2) ** 2 +
    Math.cos(lat1) * Math.cos(lat2) * Math.sin(deltaLng / 2) ** 2;
  const c = 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a));
  return (EARTH_RADIUS_METERS * c) / 1000;
}

function cellToTessera(cellId: bigint, resolvePlace: PlaceResolver): TesseraAddress {
  const { x, y } = cellIdToXY(cellId);
  const bounds = cellBounds(x, y);
  const coordinates = {
    lat: round((bounds.southwest.lat + bounds.northeast.lat) / 2, 7),
    lng: round((bounds.southwest.lng + bounds.northeast.lng) / 2, 7)
  };
  const place = resolvePlace(coordinates);
  const words = cellToWords(cellId);

  return {
    country: place.country,
    square: bounds,
    nearestPlace: place.nearestPlace,
    coordinates,
    words,
    language: "en",
    locale: null,
    map: `https://emem.dev/${words}`,
    cellId: cellId.toString(),
    geotessera128: geotesseraVector(cellId, coordinates)
  };
}

function coordinatesToCell(coordinates: Coordinates): { cellId: bigint; x: number; y: number } {
  const mercator = lngLatToMercator(coordinates);
  const x = clampInt(
    Math.floor((mercator.x + HALF_WORLD_METERS) / CELL_SIZE_METERS),
    0,
    GEOTESSERA.columns - 1
  );
  const y = clampInt(
    Math.floor((HALF_WORLD_METERS - mercator.y) / CELL_SIZE_METERS),
    0,
    GEOTESSERA.rows - 1
  );

  return {
    cellId: BigInt(y) * BigInt(GEOTESSERA.columns) + BigInt(x),
    x,
    y
  };
}

function cellIdToXY(cellId: bigint): { x: number; y: number } {
  return {
    x: Number(cellId % BigInt(GEOTESSERA.columns)),
    y: Number(cellId / BigInt(GEOTESSERA.columns))
  };
}

function cellBounds(x: number, y: number): TesseraBounds {
  const westX = cellBoundaryToMercatorX(x);
  const eastX = cellBoundaryToMercatorX(x + 1);
  const northY = cellBoundaryToMercatorY(y);
  const southY = cellBoundaryToMercatorY(y + 1);
  const southwest = mercatorToLngLat(westX, southY);
  const northeast = mercatorToLngLat(eastX, northY);

  return {
    southwest: {
      lat: round(southwest.lat, 7),
      lng: round(southwest.lng, 7)
    },
    northeast: {
      lat: round(northeast.lat, 7),
      lng: round(northeast.lng, 7)
    }
  };
}

function cellBoundaryToMercatorX(x: number): number {
  return x * CELL_SIZE_METERS - HALF_WORLD_METERS;
}

function cellBoundaryToMercatorY(y: number): number {
  return HALF_WORLD_METERS - y * CELL_SIZE_METERS;
}

function lngLatToMercator(coordinates: Coordinates): { x: number; y: number } {
  const lat = clamp(coordinates.lat, -MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
  const lng = wrapLongitude(coordinates.lng);
  return {
    x: (lng / 180) * HALF_WORLD_METERS,
    y: Math.log(Math.tan(Math.PI / 4 + toRadians(lat) / 2)) * EARTH_RADIUS_METERS
  };
}

function mercatorToLngLat(x: number, y: number): Coordinates {
  return {
    lng: round((x / HALF_WORLD_METERS) * 180, 7),
    lat: round(toDegrees(2 * Math.atan(Math.exp(y / EARTH_RADIUS_METERS)) - Math.PI / 2), 7)
  };
}

function collectLocalSuggestions(
  parts: string[],
  focusX: number,
  focusY: number,
  candidates: Map<string, TesseraAddress>,
  limit: number,
  resolvePlace: PlaceResolver
): void {
  const maxRing = 36;

  for (let ring = 0; ring <= maxRing && candidates.size < limit; ring += 1) {
    for (let dx = -ring; dx <= ring && candidates.size < limit; dx += 1) {
      for (let dy = -ring; dy <= ring && candidates.size < limit; dy += 1) {
        if (Math.max(Math.abs(dx), Math.abs(dy)) !== ring) {
          continue;
        }

        const x = focusX + dx;
        const y = focusY + dy;

        if (x < 0 || y < 0 || x >= GEOTESSERA.columns || y >= GEOTESSERA.rows) {
          continue;
        }

        const cellId = BigInt(y) * BigInt(GEOTESSERA.columns) + BigInt(x);
        const words = cellToWords(cellId);

        if (wordsMatchParts(words, parts)) {
          candidates.set(words, cellToTessera(cellId, resolvePlace));
        }
      }
    }
  }
}

function collectDecodedSuggestions(
  parts: string[],
  candidates: Map<string, TesseraAddress>,
  limit: number,
  resolvePlace: PlaceResolver
): void {
  if (parts.length < 2 || candidates.size >= limit) {
    return;
  }

  try {
    const first = wordToIndex(parts[0]);
    const second = wordToIndex(parts[1]);
    const thirdPrefix = parts[2] ?? "";
    let scanned = 0;

    for (let third = 0; third < 65536 && candidates.size < limit; third += 1) {
      const thirdWord = indexToWord(third);

      if (thirdPrefix && !thirdWord.startsWith(thirdPrefix)) {
        continue;
      }

      scanned += 1;
      const mixed = BigInt(first) | (BigInt(second) << 16n) | (BigInt(third) << 32n);
      const cellId = positiveModulo((mixed - AFFINE_B) * AFFINE_A_INV, WORD_SPACE);

      if (cellId < TOTAL_CELLS) {
        const words = `${indexToWord(first)}.${indexToWord(second)}.${thirdWord}`;
        candidates.set(words, cellToTessera(cellId, resolvePlace));
      }

      if (scanned > 5000) {
        break;
      }
    }
  } catch {
    return;
  }
}

function wordsMatchParts(words: string, parts: string[]): boolean {
  const candidate = words.split(".");
  return parts.every((part, index) => candidate[index]?.startsWith(part));
}

function indexToWord(index: number): string {
  if (!Number.isInteger(index) || index < 0 || index > 65535) {
    throw new Error("Word index is outside the generated lexicon");
  }

  return [
    SYLLABLES[(index >> 12) & 15],
    SYLLABLES[(index >> 8) & 15],
    SYLLABLES[(index >> 4) & 15],
    SYLLABLES[index & 15]
  ].join("");
}

function wordToIndex(word: string): number {
  const clean = word.toLowerCase();

  if (!/^[a-z]{8}$/.test(clean)) {
    throw new Error(`"${word}" is not in the generated emem lexicon`);
  }

  let index = 0;

  for (let cursor = 0; cursor < 8; cursor += 2) {
    const syllable = clean.slice(cursor, cursor + 2);
    const value = SYLLABLE_INDEX.get(syllable);

    if (value === undefined) {
      throw new Error(`"${word}" is not in the generated emem lexicon`);
    }

    index = (index << 4) | value;
  }

  return index;
}

function modularInverse(value: bigint, modulus: bigint): bigint {
  let previousR = value;
  let r = modulus;
  let previousS = 1n;
  let s = 0n;

  while (r !== 0n) {
    const quotient = previousR / r;
    [previousR, r] = [r, previousR - quotient * r];
    [previousS, s] = [s, previousS - quotient * s];
  }

  if (previousR !== 1n) {
    throw new Error("Value is not invertible");
  }

  return positiveModulo(previousS, modulus);
}

function positiveModulo(value: bigint, modulus: bigint): bigint {
  return ((value % modulus) + modulus) % modulus;
}

function vectorChecksum(values: number[]): string {
  const hash = fnv1a32(values.map((value) => value.toFixed(4)).join(","));
  return hash.toString(16).padStart(8, "0");
}

function fnv1a32(input: string): number {
  let hash = 0x811c9dc5;

  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }

  return hash >>> 0;
}

function xorshift32(value: number): number {
  let x = value >>> 0;
  x ^= x << 13;
  x ^= x >>> 17;
  x ^= x << 5;
  return x >>> 0;
}

function toRadians(value: number): number {
  return (value * Math.PI) / 180;
}

function toDegrees(value: number): number {
  return (value * 180) / Math.PI;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function clampInt(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.trunc(value)));
}

function wrapLongitude(lng: number): number {
  if (lng === 180) {
    return 180;
  }

  return ((((lng + 180) % 360) + 360) % 360) - 180;
}

function round(value: number, decimals: number): number {
  const factor = 10 ** decimals;
  return Math.round(value * factor) / factor;
}
