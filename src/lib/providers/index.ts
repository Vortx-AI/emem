import type { BANDS } from "../bands.js";
import type {
  BandCoverage,
  Coordinates,
  Geotessera1792,
  GeotesseraVector,
  TesseraAddress
} from "../geotessera.js";
import { buildProceduralIntelligence, proceduralVector128 } from "./procedural.js";

export type ProviderStatus = {
  name: string;
  kind: "procedural" | "remote";
  live: boolean;
  reason: string | null;
  liveFamilies: number;
  endpoint: string | null;
};

export type IntelligenceResult = {
  intelligence: Geotessera1792;
  vector128: GeotesseraVector;
  providerStatus: ProviderStatus;
};

export type VectorProvider = {
  status: () => ProviderStatus;
  getIntelligence: (cellId: string, coordinates: Coordinates) => Promise<IntelligenceResult>;
};

type BandRegistry = typeof BANDS;

export function createProvider(env: NodeJS.ProcessEnv = process.env): VectorProvider {
  const url = (env.AGRI_CUBE_URL ?? "").trim();
  const apiKey = (env.AGRI_CUBE_KEY ?? "").trim() || null;

  if (!url) {
    return createProceduralProvider();
  }

  return createAgriRemoteProvider({ url, apiKey });
}

export function createProceduralProvider(): VectorProvider {
  const providerStatus: ProviderStatus = {
    name: "emem-procedural",
    kind: "procedural",
    live: false,
    reason: "AGRI_CUBE_URL not configured — serving procedural placeholder bands honestly labeled.",
    liveFamilies: 0,
    endpoint: null
  };

  return {
    status: () => providerStatus,
    async getIntelligence(cellId, coordinates) {
      const intelligence = buildProceduralIntelligence(cellId, coordinates);
      const vector128 = proceduralVector128(cellId, coordinates);
      return { intelligence, vector128, providerStatus };
    }
  };
}

type AgriRemoteConfig = {
  url: string;
  apiKey: string | null;
};

type AgriRemotePayload = {
  cellId: string;
  capturedAt?: string;
  checksum?: string;
  coverage?: Partial<BandCoverage>[];
  vector128?: number[];
};

export function createAgriRemoteProvider(config: AgriRemoteConfig): VectorProvider {
  const procedural = createProceduralProvider();
  const baseStatus: ProviderStatus = {
    name: "agri-remote",
    kind: "remote",
    live: true,
    reason: null,
    liveFamilies: 0,
    endpoint: config.url
  };
  let lastError: string | null = null;

  return {
    status: () => ({
      ...baseStatus,
      live: lastError === null,
      reason: lastError
    }),
    async getIntelligence(cellId, coordinates) {
      try {
        const payload = await fetchAgri(config, cellId, coordinates);
        const merged = mergeAgriResponse(cellId, coordinates, payload);
        lastError = null;
        return merged;
      } catch (error) {
        lastError = error instanceof Error ? error.message : "agri remote failed";
        const fallback = await procedural.getIntelligence(cellId, coordinates);
        return {
          ...fallback,
          providerStatus: {
            ...baseStatus,
            live: false,
            reason: `${lastError} — falling back to procedural`
          }
        };
      }
    }
  };
}

async function fetchAgri(
  config: AgriRemoteConfig,
  cellId: string,
  coordinates: Coordinates
): Promise<AgriRemotePayload> {
  const endpoint = new URL("/cell", config.url).toString();
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 6000);

  try {
    const response = await fetch(endpoint, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(config.apiKey ? { Authorization: `Bearer ${config.apiKey}` } : {})
      },
      body: JSON.stringify({ cellId, lat: coordinates.lat, lng: coordinates.lng }),
      signal: controller.signal
    });

    if (!response.ok) {
      throw new Error(`agri remote responded ${response.status}`);
    }

    return (await response.json()) as AgriRemotePayload;
  } finally {
    clearTimeout(timer);
  }
}

function mergeAgriResponse(
  cellId: string,
  coordinates: Coordinates,
  payload: AgriRemotePayload
): IntelligenceResult {
  const proceduralIntelligence = buildProceduralIntelligence(cellId, coordinates);
  const agriCoverage = new Map<string, Partial<BandCoverage>>();

  for (const entry of payload.coverage ?? []) {
    if (entry.key) {
      agriCoverage.set(entry.key, entry);
    }
  }

  const coverage: BandCoverage[] = proceduralIntelligence.coverage.map((band) => {
    const live = agriCoverage.get(band.key);

    if (!live) {
      return band;
    }

    return {
      ...band,
      status: live.status ?? "live",
      note: live.note,
      summary: live.summary,
      sampleValues: live.sampleValues ?? band.sampleValues
    };
  });

  const dims = (status: BandCoverage["status"]) =>
    coverage.filter((band) => band.status === status).reduce((sum, band) => sum + band.dims, 0);

  const intelligence: Geotessera1792 = {
    model: "vortx-agri-1792d",
    dimensions: 1792,
    provider: "agri-remote",
    capturedAt: payload.capturedAt ?? new Date().toISOString(),
    liveDims: dims("live"),
    proceduralDims: dims("procedural"),
    deferredDims: dims("deferred"),
    unavailableDims: dims("unavailable"),
    checksum: payload.checksum ?? proceduralIntelligence.checksum,
    coverage
  };

  const vector128: GeotesseraVector =
    Array.isArray(payload.vector128) && payload.vector128.length === 128
      ? {
          model: "geotessera-128d-v1",
          dimensions: 128,
          values: payload.vector128,
          checksum: proceduralIntelligence.checksum
        }
      : proceduralVector128(cellId, coordinates);

  const liveFamilies = new Set(
    coverage.filter((band) => band.status === "live").map((band) => band.family)
  );

  return {
    intelligence,
    vector128,
    providerStatus: {
      name: "agri-remote",
      kind: "remote",
      live: true,
      reason: null,
      liveFamilies: liveFamilies.size,
      endpoint: null
    }
  };
}

export async function enrichTessera(
  base: TesseraAddress,
  provider: VectorProvider
): Promise<TesseraAddress> {
  const { intelligence, vector128 } = await provider.getIntelligence(base.cellId, base.coordinates);
  return { ...base, geotessera128: vector128, intelligence };
}

export type { BandRegistry };
