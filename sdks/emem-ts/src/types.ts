/** Recursive JSON value type returned by every emem responder. */
export type Json = string | number | boolean | null | Json[] | { [k: string]: Json };

export interface ClientOptions {
  /** Responder base URL. Defaults to `EMEM_BASE_URL` env or `https://emem.dev`. */
  baseUrl?: string;
  /** HTTP timeout in milliseconds. Defaults to 180_000 (matches the responder's gateway timeout). */
  timeoutMs?: number;
  /** Optional fetch implementation, e.g. for testing. Defaults to globalThis.fetch. */
  fetch?: typeof fetch;
  /** Extra HTTP headers to merge into every request. */
  headers?: Record<string, string>;
}

export interface LocateRequest {
  /** Free-text place name. REQUIRED unless `lat`+`lng` are provided. */
  place?: string;
  /** WGS-84 latitude. REQUIRED with `lng` unless `place` is provided. */
  lat?: number;
  /** WGS-84 longitude. REQUIRED with `lat` unless `place` is provided. */
  lng?: number;
}

export interface RecallRequest {
  cell: string;
  bands?: string[];
  tslot?: number;
}

export interface RecallManyRequest {
  cells: string[];
  bands?: string[];
}

export interface RecallPolygonRequest {
  place?: string;
  polygon_bbox?: [number, number, number, number];
  polygon_geojson?: Json;
  bands?: string[];
  max_cells?: number;
  cells_per_sqkm?: number;
  drill_on_water?: boolean;
}

export interface FindSimilarRequest {
  /** cell64 string OR `inline:[x,y,...]` literal vector. */
  key: string;
  k?: number;
  band?: string;
  mode?: "cosine" | "hamming" | "hamming_then_rerank";
}

export interface CompareRequest {
  a: string;
  b: string;
  family?: string;
}

export type ConsistencyPredicate =
  | { kind: "abs_diff_le"; threshold: number }
  | { kind: "abs_diff_lt"; threshold: number }
  | { kind: "cosine_ge"; threshold: number }
  | { kind: "cosine_gt"; threshold: number }
  | { kind: "l2_distance_le"; threshold: number };

export interface CompareBandsRequest {
  cell: string;
  a: string;
  b: string;
  tslot_a?: number;
  tslot_b?: number;
  predicate?: ConsistencyPredicate;
}

export interface TrajectoryRequest {
  cell: string;
  band: string;
  window: [number, number];
}

export interface DiffRequest {
  cell: string;
  band: string;
  tslot_a: number;
  tslot_b: number;
}

export interface QueryRegionRequest {
  geometry?: string;
  bbox?: [number, number, number, number];
  max_cells?: number;
  bands?: string[];
  agg?: "mean" | "median" | "p90" | "vector_centroid";
}

export interface VerifyRequest {
  cell: string;
  claim: Json;
  mode?: "fast" | "resolve";
}

export interface AskRequest {
  q: string;
  place?: string;
  cell?: string;
  lat?: number;
  lng?: number;
  include_image?: boolean;
  verbose?: boolean;
}

export interface FetchRequest {
  cid?: string;
  cell?: string;
  band?: string;
  tslot?: number;
}

export interface BackfillRequest {
  cell: string;
  band: string;
  start_unix?: number;
  end_unix?: number;
  max_facts?: number;
}

export interface IntentRequest {
  q: string;
  place?: string;
  cell?: string;
}

export interface HeatSolveRequest {
  cell: string;
  hours_ahead?: number;
  diffusivity_m2_per_s?: number;
}

export interface WaveSolveRequest {
  coastal_cell: string;
  offshore_height_m: number;
  period_s: number;
  n_offshore_cells?: number;
}

export interface JepaPredictRequest {
  cell: string;
  band?: string;
  lookback_months?: number;
  forecast_horizon_months?: number;
}

export interface JepaPredictV2Request {
  cell: string;
  band?: string;
  k_history?: number;
}

export interface BoringQuery {
  lat?: number;
  lng?: number;
  place?: string;
}
