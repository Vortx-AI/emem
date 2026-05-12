/**
 * Thin HTTP client for emem.dev — wraps the public REST surface.
 *
 * Coverage map (REST → method):
 *
 *   POST /v1/locate            → Client.locate
 *   POST /v1/recall            → Client.recall
 *   POST /v1/recall_many       → Client.recallMany
 *   POST /v1/recall_polygon    → Client.recallPolygon
 *   POST /v1/find_similar      → Client.findSimilar
 *   POST /v1/compare           → Client.compare
 *   POST /v1/compare_bands     → Client.compareBands
 *   POST /v1/trajectory        → Client.trajectory
 *   POST /v1/diff              → Client.diff
 *   POST /v1/query_region      → Client.queryRegion
 *   POST /v1/verify            → Client.verify
 *   POST /v1/ask               → Client.ask
 *   POST /v1/fetch             → Client.fetch
 *   POST /v1/backfill          → Client.backfill
 *   POST /v1/intent            → Client.intent
 *   POST /v1/heat_solve        → Client.heatSolve
 *   POST /v1/wave_solve        → Client.waveSolve
 *   POST /v1/jepa_predict      → Client.jepaPredict
 *   POST /v1/jepa_predict_v2   → Client.jepaPredictV2
 *   GET  /v1/bands             → Client.bands
 *   GET  /v1/algorithms        → Client.algorithms
 *   GET  /v1/sources           → Client.sources
 *   GET  /v1/schema            → Client.schema
 *   GET  /v1/manifests         → Client.manifests
 *   GET  /v1/topics            → Client.topics
 *   GET  /v1/grid_info         → Client.gridInfo
 *   GET  /v1/coverage_matrix   → Client.coverageMatrix
 *   GET  /v1/agent_card        → Client.agentCard
 *   GET  /v1/discover          → Client.discover
 *   GET  /openapi.json         → Client.openapi
 *   GET  /health               → Client.health
 *
 * Boring lat/lng shortcuts (skip locate→recall): ndvi, elevation, air,
 * lst, soil, water, forest, weather.
 */

import type {
  AskRequest,
  BackfillRequest,
  BoringQuery,
  ClientOptions,
  CompareBandsRequest,
  CompareRequest,
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

// Read env vars without depending on @types/node — works in Node/Bun/Deno
// where `process.env` exists, and silently falls back in browsers/edge
// runtimes where it does not.
const _env: Record<string, string | undefined> =
  (globalThis as { process?: { env?: Record<string, string | undefined> } }).process?.env ?? {};

const DEFAULT_BASE_URL = _env["EMEM_BASE_URL"] ?? "https://emem.dev";

const DEFAULT_TIMEOUT_MS = (() => {
  const raw = _env["EMEM_TIMEOUT_SECS"];
  const n = raw ? Number(raw) : NaN;
  return Number.isFinite(n) && n > 0 ? n * 1000 : 180_000;
})();

const USER_AGENT = "emem-ts/0.0.6 (+https://emem.dev)";

export class EmemError extends Error {
  override readonly name = "EmemError";
}

export class EmemHTTPError extends EmemError {
  override readonly name = "EmemHTTPError";
  constructor(
    readonly status: number,
    readonly url: string,
    readonly body: unknown,
  ) {
    super(`emem responder returned ${status} for ${url}: ${stringify(body)}`);
  }
}

function stringify(v: unknown): string {
  try {
    return typeof v === "string" ? v : JSON.stringify(v);
  } catch {
    return String(v);
  }
}

function stripUndefined<T extends Record<string, unknown>>(o: T): Partial<T> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(o)) {
    if (v !== undefined) out[k] = v;
  }
  return out as Partial<T>;
}

export class Client {
  readonly baseUrl: string;
  private readonly timeoutMs: number;
  private readonly fetchImpl: typeof fetch;
  private readonly extraHeaders: Record<string, string>;

  constructor(options: ClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? DEFAULT_BASE_URL).replace(/\/+$/, "");
    this.timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    const fImpl = options.fetch ?? globalThis.fetch;
    if (!fImpl) {
      throw new EmemError(
        "No fetch implementation found. Pass `options.fetch` or run on Node 18+.",
      );
    }
    this.fetchImpl = fImpl.bind(globalThis);
    this.extraHeaders = options.headers ?? {};
  }

  private async request<T = Json>(
    method: "GET" | "POST",
    path: string,
    init: { body?: unknown; query?: Record<string, unknown> } = {},
  ): Promise<T> {
    const url = new URL(path.startsWith("/") ? path : `/${path}`, `${this.baseUrl}/`);
    if (init.query) {
      for (const [k, v] of Object.entries(init.query)) {
        if (v !== undefined && v !== null) url.searchParams.set(k, String(v));
      }
    }

    const headers: Record<string, string> = {
      accept: "application/json",
      "user-agent": USER_AGENT,
      ...this.extraHeaders,
    };
    let body: BodyInit | undefined;
    if (init.body !== undefined) {
      headers["content-type"] = "application/json";
      body = JSON.stringify(init.body);
    }

    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), this.timeoutMs);
    let resp: Response;
    try {
      resp = await this.fetchImpl(url.toString(), {
        method,
        headers,
        body,
        signal: ctrl.signal,
      });
    } finally {
      clearTimeout(timer);
    }

    const ct = resp.headers.get("content-type") ?? "";
    const parsed: unknown = ct.startsWith("application/json")
      ? await resp.json()
      : await resp.text();
    if (!resp.ok) {
      throw new EmemHTTPError(resp.status, url.toString(), parsed);
    }
    return parsed as T;
  }

  private post<T = Json>(path: string, body: Record<string, unknown>): Promise<T> {
    return this.request<T>("POST", path, { body: stripUndefined(body) });
  }

  private get<T = Json>(path: string, query?: Record<string, unknown>): Promise<T> {
    return this.request<T>("GET", path, { query: query ? stripUndefined(query) : undefined });
  }

  // ── Geocoder ──────────────────────────────────────────────────────────
  locate(req: LocateRequest): Promise<Json> {
    return this.post("/v1/locate", { ...req });
  }

  // ── Read primitives ──────────────────────────────────────────────────
  recall(req: RecallRequest): Promise<Json> {
    return this.post("/v1/recall", { ...req });
  }

  recallMany(req: RecallManyRequest): Promise<Json> {
    return this.post("/v1/recall_many", { ...req });
  }

  recallPolygon(req: RecallPolygonRequest): Promise<Json> {
    return this.post("/v1/recall_polygon", { ...req });
  }

  findSimilar(req: FindSimilarRequest): Promise<Json> {
    return this.post("/v1/find_similar", { ...req });
  }

  compare(req: CompareRequest): Promise<Json> {
    return this.post("/v1/compare", { ...req });
  }

  compareBands(req: CompareBandsRequest): Promise<Json> {
    return this.post("/v1/compare_bands", { ...req });
  }

  trajectory(req: TrajectoryRequest): Promise<Json> {
    return this.post("/v1/trajectory", { ...req });
  }

  diff(req: DiffRequest): Promise<Json> {
    return this.post("/v1/diff", { ...req });
  }

  queryRegion(req: QueryRegionRequest): Promise<Json> {
    return this.post("/v1/query_region", { ...req });
  }

  verify(req: VerifyRequest): Promise<Json> {
    return this.post("/v1/verify", { ...req });
  }

  ask(req: AskRequest): Promise<Json> {
    return this.post("/v1/ask", { ...req });
  }

  fetch(req: FetchRequest): Promise<Json> {
    return this.post("/v1/fetch", { ...req });
  }

  backfill(req: BackfillRequest): Promise<Json> {
    return this.post("/v1/backfill", { ...req });
  }

  intent(req: IntentRequest): Promise<Json> {
    return this.post("/v1/intent", { ...req });
  }

  // ── Physics solvers ─────────────────────────────────────────────────
  heatSolve(req: HeatSolveRequest): Promise<Json> {
    return this.post("/v1/heat_solve", { ...req });
  }

  waveSolve(req: WaveSolveRequest): Promise<Json> {
    return this.post("/v1/wave_solve", { ...req });
  }

  jepaPredict(req: JepaPredictRequest): Promise<Json> {
    return this.post("/v1/jepa_predict", { ...req });
  }

  jepaPredictV2(req: JepaPredictV2Request): Promise<Json> {
    return this.post("/v1/jepa_predict_v2", { ...req });
  }

  // ── Boring lat/lng shortcuts ───────────────────────────────────────
  private boring(path: string, q: BoringQuery): Promise<Json> {
    return this.get(path, { lat: q.lat, lon: q.lng, place: q.place });
  }
  ndvi(q: BoringQuery): Promise<Json> { return this.boring("/v1/ndvi", q); }
  elevation(q: BoringQuery): Promise<Json> { return this.boring("/v1/elevation", q); }
  air(q: BoringQuery): Promise<Json> { return this.boring("/v1/air", q); }
  lst(q: BoringQuery): Promise<Json> { return this.boring("/v1/lst", q); }
  soil(q: BoringQuery): Promise<Json> { return this.boring("/v1/soil", q); }
  water(q: BoringQuery): Promise<Json> { return this.boring("/v1/water", q); }
  forest(q: BoringQuery): Promise<Json> { return this.boring("/v1/forest", q); }
  weather(q: BoringQuery): Promise<Json> { return this.boring("/v1/weather", q); }

  // ── Introspection ─────────────────────────────────────────────────
  bands(): Promise<Json> { return this.get("/v1/bands"); }
  algorithms(key?: string): Promise<Json> {
    return this.get(key ? `/v1/algorithms/${encodeURIComponent(key)}` : "/v1/algorithms");
  }
  sources(): Promise<Json> { return this.get("/v1/sources"); }
  schema(): Promise<Json> { return this.get("/v1/schema"); }
  manifests(): Promise<Json> { return this.get("/v1/manifests"); }
  topics(): Promise<Json> { return this.get("/v1/topics"); }
  gridInfo(): Promise<Json> { return this.get("/v1/grid_info"); }
  coverageMatrix(): Promise<Json> { return this.get("/v1/coverage_matrix"); }
  agentCard(): Promise<Json> { return this.get("/v1/agent_card"); }
  discover(): Promise<Json> { return this.get("/v1/discover"); }
  openapi(): Promise<Json> { return this.get("/openapi.json"); }
  health(): Promise<Json> { return this.get("/health"); }
}
