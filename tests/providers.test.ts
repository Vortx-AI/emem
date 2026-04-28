import { describe, expect, it } from "vitest";
import { coordinatesToTessera } from "../src/lib/geotessera.js";
import { resolveNearestPlace } from "../src/lib/gazetteer.js";
import {
  createAgriRemoteProvider,
  createProceduralProvider,
  createProvider,
  enrichTessera
} from "../src/lib/providers/index.js";

describe("procedural provider", () => {
  it("labels itself as procedural and offline without env", () => {
    const provider = createProvider({} as NodeJS.ProcessEnv);
    const status = provider.status();

    expect(status.kind).toBe("procedural");
    expect(status.live).toBe(false);
    expect(status.reason).toMatch(/AGRI_CUBE_URL/);
  });

  it("produces a 1792D intelligence object with honest status per band", async () => {
    const provider = createProceduralProvider();
    const base = coordinatesToTessera({ lat: 12.9716, lng: 77.5946 }, resolveNearestPlace);
    const enriched = await enrichTessera(base, provider);
    const intel = enriched.intelligence;

    expect(intel).toBeDefined();
    expect(intel?.dimensions).toBe(1792);
    expect(intel?.liveDims).toBe(0);
    expect((intel?.proceduralDims ?? 0) + (intel?.deferredDims ?? 0) + (intel?.unavailableDims ?? 0)).toBe(1792);

    const geotessera = intel?.coverage.find((band) => band.key === "geotessera");
    expect(geotessera?.status).toBe("procedural");

    const sentinel2 = intel?.coverage.find((band) => band.key === "sentinel2_raw");
    expect(sentinel2?.status).toBe("deferred");

    const reserved = intel?.coverage.find((band) => band.key === "reserved");
    expect(reserved?.status).toBe("unavailable");
  });
});

describe("agri remote provider", () => {
  const originalFetch = globalThis.fetch;

  it("falls back to procedural when the remote is unreachable", async () => {
    globalThis.fetch = (async () => {
      throw new Error("network down");
    }) as typeof fetch;

    try {
      const provider = createAgriRemoteProvider({ url: "http://not-a-real-host.local", apiKey: null });
      const base = coordinatesToTessera({ lat: 40.758, lng: -73.9855 }, resolveNearestPlace);
      const enriched = await enrichTessera(base, provider);

      expect(enriched.intelligence?.liveDims).toBe(0);
      expect(provider.status().live).toBe(false);
      expect(provider.status().reason).toMatch(/network down/);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });

  it("merges live coverage when the remote returns band data", async () => {
    const originalFetchImpl = globalThis.fetch;
    globalThis.fetch = (async () =>
      new Response(
        JSON.stringify({
          cellId: "mock",
          capturedAt: "2026-04-25T12:00:00Z",
          checksum: "deadbeef",
          coverage: [
            {
              key: "sentinel2_raw",
              status: "live",
              sampleValues: [0.1, 0.2, 0.3],
              note: "live S2 tile"
            }
          ]
        }),
        { status: 200, headers: { "content-type": "application/json" } }
      )) as typeof fetch;

    try {
      const provider = createAgriRemoteProvider({ url: "http://mock.local", apiKey: null });
      const base = coordinatesToTessera({ lat: 0, lng: 0 }, resolveNearestPlace);
      const enriched = await enrichTessera(base, provider);

      const s2 = enriched.intelligence?.coverage.find((band) => band.key === "sentinel2_raw");
      expect(s2?.status).toBe("live");
      expect(enriched.intelligence?.liveDims).toBeGreaterThan(0);
      expect(enriched.intelligence?.capturedAt).toBe("2026-04-25T12:00:00Z");
      expect(enriched.intelligence?.provider).toBe("agri-remote");
    } finally {
      globalThis.fetch = originalFetchImpl;
    }
  });
});
