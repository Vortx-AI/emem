import { BANDS, GEOTESSERA1792_DIM } from "../bands.js";
import type {
  BandCoverage,
  Coordinates,
  Geotessera1792,
  GeotesseraVector
} from "../geotessera.js";

export function buildProceduralIntelligence(
  cellId: string,
  coordinates: Coordinates
): Geotessera1792 {
  const seed = fnv1a32(`${cellId}:${coordinates.lat.toFixed(7)}:${coordinates.lng.toFixed(7)}`);
  const full = procedural1792(seed, coordinates);
  const coverage: BandCoverage[] = BANDS.map((band) => {
    const slice = full.slice(band.offset, band.offset + band.dims);
    const isProcedural = band.key === "geotessera" || band.family === "encoding";
    const isReserved = band.family === "reserved";

    const status: BandCoverage["status"] = isProcedural
      ? "procedural"
      : isReserved
        ? "unavailable"
        : "deferred";

    return {
      key: band.key,
      label: band.label,
      family: band.family,
      tempo: band.tempo,
      source: band.source,
      offset: band.offset,
      dims: band.dims,
      status,
      note:
        status === "deferred"
          ? "Real band deferred: data plane not connected. Connect agri remote to populate."
          : status === "unavailable"
            ? "Reserved for future sensors."
            : undefined,
      sampleValues: slice.slice(0, 8).map((value) => round(value, 6))
    };
  });

  const liveDims = 0;
  const proceduralDims = sumDimsWhere(coverage, "procedural");
  const deferredDims = sumDimsWhere(coverage, "deferred");
  const unavailableDims = sumDimsWhere(coverage, "unavailable");

  return {
    model: "emem-procedural-v1",
    dimensions: GEOTESSERA1792_DIM,
    provider: "procedural",
    capturedAt: null,
    liveDims,
    proceduralDims,
    deferredDims,
    unavailableDims,
    checksum: vectorChecksum(full),
    coverage
  };
}

export function proceduralVector128(
  cellId: string,
  coordinates: Coordinates
): GeotesseraVector {
  const seed = fnv1a32(`${cellId}:${coordinates.lat.toFixed(7)}:${coordinates.lng.toFixed(7)}`);
  const full = procedural1792(seed, coordinates);
  const values = normalize(full.slice(0, 128));
  return {
    model: "geotessera-128d-v1",
    dimensions: 128,
    values: values.map((value) => round(value, 6)),
    checksum: vectorChecksum(values)
  };
}

function procedural1792(seed: number, coordinates: Coordinates): number[] {
  const values: number[] = [
    coordinates.lat / 90,
    coordinates.lng / 180,
    Math.sin(toRadians(coordinates.lat)),
    Math.cos(toRadians(coordinates.lat)),
    Math.sin(toRadians(coordinates.lng)),
    Math.cos(toRadians(coordinates.lng))
  ];
  let state = seed || 0x9e3779b9;

  while (values.length < GEOTESSERA1792_DIM) {
    state = xorshift32(state);
    values.push((state / 0xffffffff) * 2 - 1);
  }

  return values;
}

function normalize(values: number[]): number[] {
  const magnitude = Math.sqrt(values.reduce((sum, value) => sum + value * value, 0)) || 1;
  return values.map((value) => value / magnitude);
}

function sumDimsWhere(coverage: BandCoverage[], status: BandCoverage["status"]): number {
  return coverage
    .filter((band) => band.status === status)
    .reduce((sum, band) => sum + band.dims, 0);
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

function round(value: number, decimals: number): number {
  const factor = 10 ** decimals;
  return Math.round(value * factor) / factor;
}
