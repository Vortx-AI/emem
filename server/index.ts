import cors from "cors";
import express from "express";
import path from "node:path";
import { BANDS, FAMILIES, FAMILY_LABEL, bandsByFamily } from "../src/lib/bands.js";
import { resolveAgentMessage, streamAgentMessage } from "../src/lib/agent.js";
import {
  coordinatesToTessera,
  generateSuggestions,
  gridSectionFromBoundingBox,
  looksLikeWords,
  parseCoordinates,
  wordsToTessera
} from "../src/lib/geotessera.js";
import { resolveNearestPlace, searchPlaces } from "../src/lib/gazetteer.js";
import { createProvider, enrichTessera } from "../src/lib/providers/index.js";

const PORT = Number(process.env.PORT ?? 8787);
const app = express();
const provider = createProvider();

app.use(cors());
app.use(express.json({ limit: "1mb" }));

app.get("/api/health", (_request, response) => {
  response.json({
    ok: true,
    product: "emem.dev",
    mode: process.env.NODE_ENV ?? "development",
    provider: provider.status()
  });
});

app.get("/api/bands", (_request, response) => {
  response.json({
    totalDims: BANDS.reduce((sum, band) => sum + band.dims, 0),
    families: FAMILIES.map((family) => ({
      key: family,
      label: FAMILY_LABEL[family],
      dims: bandsByFamily(family).reduce((sum, band) => sum + band.dims, 0),
      bands: bandsByFamily(family)
    })),
    provider: provider.status()
  });
});

app.get("/api/convert-to-3wa", async (request, response) => {
  try {
    const coordinates = parseCoordinates(String(request.query.coordinates ?? ""));

    if (!coordinates) {
      response.status(400).json({
        error: {
          code: "BadCoordinates",
          message: "coordinates must be latitude,longitude"
        }
      });
      return;
    }

    const base = coordinatesToTessera(coordinates, resolveNearestPlace);
    const enriched = await enrichTessera(base, provider);
    response.json(enriched);
  } catch (error) {
    sendError(response, error);
  }
});

app.get("/api/convert-to-coordinates", async (request, response) => {
  try {
    const words = String(request.query.words ?? "");

    if (!looksLikeWords(words)) {
      response.status(400).json({
        error: {
          code: "BadWords",
          message: "words must be a three-word emem address"
        }
      });
      return;
    }

    const base = wordsToTessera(words, resolveNearestPlace);
    const enriched = await enrichTessera(base, provider);
    response.json(enriched);
  } catch (error) {
    sendError(response, error, "BadWords");
  }
});

app.get("/api/autosuggest", (request, response) => {
  try {
    const input = String(request.query.input ?? "");
    const focus = parseCoordinates(String(request.query.focus ?? ""));
    const suggestions = generateSuggestions(input, focus ?? undefined, 6, resolveNearestPlace);
    response.json({ suggestions });
  } catch (error) {
    sendError(response, error);
  }
});

app.get("/api/grid-section", (request, response) => {
  try {
    const raw = String(request.query["bounding-box"] ?? "");
    const values = raw.split(",").map((value) => Number(value.trim()));

    if (values.length !== 4 || values.some((value) => !Number.isFinite(value))) {
      response.status(400).json({
        error: {
          code: "BadBoundingBox",
          message: "bounding-box must be south,west,north,east"
        }
      });
      return;
    }

    const [south, west, north, east] = values;
    response.json(gridSectionFromBoundingBox(south, west, north, east));
  } catch (error) {
    sendError(response, error);
  }
});

app.get("/api/search", async (request, response) => {
  try {
    const query = String(request.query.q ?? "");
    const coordinates = parseCoordinates(query);

    if (coordinates) {
      const base = coordinatesToTessera(coordinates, resolveNearestPlace);
      response.json({ results: [await enrichTessera(base, provider)] });
      return;
    }

    if (looksLikeWords(query)) {
      const base = wordsToTessera(query, resolveNearestPlace);
      response.json({ results: [await enrichTessera(base, provider)] });
      return;
    }

    const results = await Promise.all(
      searchPlaces(query).map(async (place) => {
        const base = coordinatesToTessera(place.coordinates, resolveNearestPlace);
        return enrichTessera(base, provider);
      })
    );

    response.json({ results });
  } catch (error) {
    sendError(response, error);
  }
});

app.post("/api/agent/resolve", async (request, response) => {
  const wantsStream = /text\/event-stream/.test(String(request.headers.accept ?? ""));

  try {
    const message = String(request.body?.message ?? "");
    const currentWords = request.body?.current?.words;
    const base =
      typeof currentWords === "string" ? wordsToTessera(currentWords, resolveNearestPlace) : null;
    const current = base ? await enrichTessera(base, provider) : null;

    if (!wantsStream) {
      response.json(await resolveAgentMessage(message, current, provider));
      return;
    }

    response.setHeader("Content-Type", "text/event-stream");
    response.setHeader("Cache-Control", "no-cache, no-transform");
    response.setHeader("Connection", "keep-alive");
    response.flushHeaders?.();

    for await (const event of streamAgentMessage(message, current, provider)) {
      response.write(`event: ${event.type}\n`);
      response.write(`data: ${JSON.stringify(event)}\n\n`);
    }

    response.end();
  } catch (error) {
    if (wantsStream && !response.headersSent) {
      response.setHeader("Content-Type", "text/event-stream");
    }

    if (wantsStream) {
      response.write(
        `event: error\ndata: ${JSON.stringify({
          type: "error",
          message: error instanceof Error ? error.message : "agent error"
        })}\n\n`
      );
      response.end();
      return;
    }

    sendError(response, error);
  }
});

if (process.env.NODE_ENV === "production") {
  const distPath = path.resolve(process.cwd(), "dist");
  app.use(express.static(distPath));
  app.get(/.*/, (_request, response) => {
    response.sendFile(path.join(distPath, "index.html"));
  });
}

app.listen(PORT, () => {
  const status = provider.status();
  console.log(`emem.dev API listening on http://localhost:${PORT}`);
  console.log(
    `  intelligence provider: ${status.name} (${status.kind})${
      status.endpoint ? ` → ${status.endpoint}` : ""
    }`
  );

  if (!status.live && status.reason) {
    console.log(`  note: ${status.reason}`);
  }
});

function sendError(
  response: express.Response,
  error: unknown,
  code = "BadRequest"
): void {
  response.status(400).json({
    error: {
      code,
      message: error instanceof Error ? error.message : "Request could not be processed"
    }
  });
}
