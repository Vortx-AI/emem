/**
 * @emem/client — TypeScript client for the emem.dev Earth memory protocol.
 *
 * The hosted instance at https://emem.dev exposes 138 REST routes (67
 * under `/v1/*`) and 34 MCP tools. This package wraps the REST surface
 * in a single {@link Client} class. Every call returns the parsed JSON
 * the server emitted — nothing is reshaped, so the ed25519-signed
 * receipts and content-addressed CIDs are preserved verbatim for
 * citation and offline verification.
 *
 * Quick start:
 *
 * ```ts
 * import { Client } from "@emem/client";
 *
 * const em = new Client();
 * const { cell64 } = await em.locate({ place: "Mount Fuji" });
 * const facts = await em.recall({ cell: cell64, bands: ["copdem30m.elevation_mean"] });
 * console.log(facts.facts[0].value);
 * ```
 */

export { Client, EmemError, EmemHTTPError } from "./client.js";
export type {
  AskRequest,
  BackfillRequest,
  ClientOptions,
  CompareBandsRequest,
  CompareRequest,
  ConsistencyPredicate,
  DiffRequest,
  FetchRequest,
  FindSimilarRequest,
  HeatSolveRequest,
  IntentRequest,
  JepaPredictRequest,
  JepaPredictV2Request,
  Json,
  LocateRequest,
  QueryRegionRequest,
  RecallManyRequest,
  RecallPolygonRequest,
  RecallRequest,
  TrajectoryRequest,
  VerifyRequest,
  WaveSolveRequest,
} from "./types.js";

export const VERSION = "0.0.4";
