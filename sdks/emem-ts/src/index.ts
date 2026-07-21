/**
 * ememdev — TypeScript client for the emem.dev Earth memory protocol.
 *
 * This package wraps the REST surface of the hosted instance at
 * https://emem.dev in a single {@link Client} class. Every call returns
 * the parsed JSON the server emitted. Nothing is reshaped, so the
 * ed25519-signed receipts and content-addressed CIDs are preserved
 * verbatim for citation and offline verification.
 *
 * The responder enumerates its own surface: `GET /openapi.json` lists
 * every documented path and `POST /mcp` `tools/list` every MCP tool.
 * Those are the counts to quote, since a number written down here goes
 * stale the next time a route lands.
 *
 * Quick start:
 *
 * ```ts
 * import { Client } from "@vortxai/emem";
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

export { VERSION } from "./version.js";
