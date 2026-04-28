import { FAMILIES, FAMILY_LABEL } from "./bands.js";
import {
  Coordinates,
  TesseraAddress,
  coordinatesToTessera,
  generateSuggestions,
  looksLikeWords,
  parseCoordinates,
  wordsToTessera
} from "./geotessera.js";
import { resolveNearestPlace, searchPlaces } from "./gazetteer.js";
import { enrichTessera, type VectorProvider } from "./providers/index.js";

export type AgentIntent =
  | "decode_words"
  | "encode_coordinates"
  | "place_search"
  | "intelligence_report"
  | "band_inspect"
  | "guidance";

export type AgentAction =
  | { type: "fly_to"; coordinates: Coordinates; zoom: number }
  | { type: "copy"; value: string; label: string }
  | { type: "show_intelligence"; cellId: string }
  | { type: "expand_family"; family: string };

export type ToolName =
  | "resolve_cell"
  | "fetch_bands"
  | "similar_cells"
  | "explain_confidence";

export type ToolCall =
  | { name: "resolve_cell"; args: { query: string } }
  | { name: "fetch_bands"; args: { cellId: string; family?: string } }
  | { name: "similar_cells"; args: { cellId: string; family?: string; k?: number } }
  | { name: "explain_confidence"; args: { cellId: string } };

export type ToolResult =
  | { name: "resolve_cell"; result: { tessera: TesseraAddress | null; note?: string } }
  | {
      name: "fetch_bands";
      result: {
        cellId: string;
        family: string | null;
        bands: Array<{
          key: string;
          label: string;
          status: string;
          dims: number;
          note?: string;
          sampleValues?: number[];
        }>;
      };
    }
  | {
      name: "similar_cells";
      result: {
        cellId: string;
        family: string | null;
        note: string;
        neighbors: Array<{ cellId: string; words: string; distanceKm: number }>;
      };
    }
  | {
      name: "explain_confidence";
      result: {
        cellId: string;
        liveDims: number;
        proceduralDims: number;
        deferredDims: number;
        unavailableDims: number;
        confidence: "live" | "partial" | "placeholder";
      };
    };

export type AgentResponse = {
  intent: AgentIntent;
  answer: string;
  tessera: TesseraAddress | null;
  actions: AgentAction[];
  suggestions: string[];
  toolCalls: ToolCall[];
  toolResults: ToolResult[];
};

export type AgentEvent =
  | { type: "status"; message: string }
  | { type: "intent"; intent: AgentIntent }
  | { type: "tool_call"; call: ToolCall }
  | { type: "tool_result"; result: ToolResult }
  | { type: "token"; text: string }
  | { type: "final"; response: AgentResponse };

export async function resolveAgentMessage(
  message: string,
  current: TesseraAddress | null,
  provider: VectorProvider
): Promise<AgentResponse> {
  const events: AgentEvent[] = [];

  for await (const event of streamAgentMessage(message, current, provider)) {
    events.push(event);
  }

  const final = events.find((event): event is Extract<AgentEvent, { type: "final" }> => event.type === "final");
  return final?.response ?? guidanceResponse(current);
}

export async function* streamAgentMessage(
  message: string,
  current: TesseraAddress | null,
  provider: VectorProvider
): AsyncGenerator<AgentEvent> {
  const trimmed = message.trim();

  yield { type: "status", message: "interpreting" };

  const plan = planIntent(trimmed, current);
  yield { type: "intent", intent: plan.intent };

  const toolCalls: ToolCall[] = [];
  const toolResults: ToolResult[] = [];
  let tessera: TesseraAddress | null = current;

  for (const call of plan.toolCalls) {
    toolCalls.push(call);
    yield { type: "tool_call", call };

    const result = await executeTool(call, provider, tessera);
    toolResults.push(result);
    yield { type: "tool_result", result };

    if (result.name === "resolve_cell" && result.result.tessera) {
      tessera = result.result.tessera;
    }
  }

  const answer = narrate(plan.intent, tessera, toolResults, trimmed);

  for (const chunk of chunkText(answer)) {
    yield { type: "token", text: chunk };
  }

  const response: AgentResponse = {
    intent: plan.intent,
    answer,
    tessera,
    actions: buildActions(plan.intent, tessera),
    suggestions: buildSuggestions(plan.intent, tessera),
    toolCalls,
    toolResults
  };

  yield { type: "final", response };
}

type IntentPlan = {
  intent: AgentIntent;
  toolCalls: ToolCall[];
};

function planIntent(trimmed: string, current: TesseraAddress | null): IntentPlan {
  const coordinates = parseCoordinates(trimmed);

  if (coordinates) {
    return {
      intent: "encode_coordinates",
      toolCalls: [{ name: "resolve_cell", args: { query: trimmed } }]
    };
  }

  if (looksLikeWords(trimmed)) {
    return {
      intent: "decode_words",
      toolCalls: [{ name: "resolve_cell", args: { query: trimmed } }]
    };
  }

  const lowered = trimmed.toLowerCase();
  const wantsIntelligence = /\b(intelligence|what('?s)?\s+here|explain|summary|analyze|analyse|confidence)\b/.test(
    lowered
  );
  const familyHint = FAMILIES.find((family) => new RegExp(`\\b${family}\\b`, "i").test(lowered));

  if (familyHint && current) {
    return {
      intent: "band_inspect",
      toolCalls: [
        { name: "fetch_bands", args: { cellId: current.cellId, family: familyHint } }
      ]
    };
  }

  if (wantsIntelligence && current) {
    return {
      intent: "intelligence_report",
      toolCalls: [
        { name: "fetch_bands", args: { cellId: current.cellId } },
        { name: "explain_confidence", args: { cellId: current.cellId } }
      ]
    };
  }

  if (trimmed.length > 0) {
    return {
      intent: "place_search",
      toolCalls: [{ name: "resolve_cell", args: { query: trimmed } }]
    };
  }

  return { intent: "guidance", toolCalls: [] };
}

async function executeTool(
  call: ToolCall,
  provider: VectorProvider,
  current: TesseraAddress | null
): Promise<ToolResult> {
  switch (call.name) {
    case "resolve_cell":
      return { name: "resolve_cell", result: await resolveCellTool(call.args.query, provider) };
    case "fetch_bands":
      return {
        name: "fetch_bands",
        result: fetchBandsTool(call.args.cellId, call.args.family ?? null, current)
      };
    case "similar_cells":
      return {
        name: "similar_cells",
        result: similarCellsTool(call.args.cellId, call.args.family ?? null, call.args.k ?? 4)
      };
    case "explain_confidence":
      return {
        name: "explain_confidence",
        result: explainConfidenceTool(current)
      };
  }
}

async function resolveCellTool(query: string, provider: VectorProvider) {
  const trimmed = query.trim();
  const coordinates = parseCoordinates(trimmed);

  if (coordinates) {
    const base = coordinatesToTessera(coordinates, resolveNearestPlace);
    return { tessera: await enrichTessera(base, provider) };
  }

  if (looksLikeWords(trimmed)) {
    try {
      const base = wordsToTessera(trimmed, resolveNearestPlace);
      return { tessera: await enrichTessera(base, provider) };
    } catch (error) {
      return {
        tessera: null,
        note: error instanceof Error ? error.message : "words did not resolve"
      };
    }
  }

  const place = searchPlaces(trimmed, 1)[0];

  if (place) {
    const base = coordinatesToTessera(place.coordinates, resolveNearestPlace);
    return { tessera: await enrichTessera(base, provider) };
  }

  return { tessera: null, note: "no matching place, coordinates, or active address" };
}

function fetchBandsTool(cellId: string, family: string | null, current: TesseraAddress | null) {
  if (!current || current.cellId !== cellId || !current.intelligence) {
    return { cellId, family, bands: [] };
  }

  const coverage = current.intelligence.coverage;
  const filtered = family ? coverage.filter((band) => band.family === family) : coverage;

  return {
    cellId,
    family,
    bands: filtered.map((band) => ({
      key: band.key,
      label: band.label,
      status: band.status,
      dims: band.dims,
      note: band.note,
      sampleValues: band.sampleValues
    }))
  };
}

function similarCellsTool(cellId: string, family: string | null, _k: number) {
  return {
    cellId,
    family,
    note:
      "similar_cells requires the agri remote data plane; procedural provider cannot compute true neighbors.",
    neighbors: [] as Array<{ cellId: string; words: string; distanceKm: number }>
  };
}

function explainConfidenceTool(current: TesseraAddress | null) {
  if (!current?.intelligence) {
    return {
      cellId: "",
      liveDims: 0,
      proceduralDims: 0,
      deferredDims: 0,
      unavailableDims: 0,
      confidence: "placeholder" as const
    };
  }

  const { liveDims, proceduralDims, deferredDims, unavailableDims } = current.intelligence;
  const informative = liveDims + proceduralDims;
  const total = informative + deferredDims + unavailableDims;
  const ratio = total === 0 ? 0 : liveDims / total;
  const confidence: "live" | "partial" | "placeholder" =
    ratio > 0.75 ? "live" : liveDims > 0 ? "partial" : "placeholder";

  return {
    cellId: current.cellId,
    liveDims,
    proceduralDims,
    deferredDims,
    unavailableDims,
    confidence
  };
}

function narrate(
  intent: AgentIntent,
  tessera: TesseraAddress | null,
  results: ToolResult[],
  trimmed: string
): string {
  if (intent === "encode_coordinates" && tessera) {
    return `Encoded ${formatCoordinatePair(tessera.coordinates)} as ///${tessera.words} near ${tessera.nearestPlace}. ${coverageSummary(tessera)}`;
  }

  if (intent === "decode_words" && tessera) {
    return `Decoded ///${tessera.words} to ${formatCoordinatePair(tessera.coordinates)} near ${tessera.nearestPlace}. ${coverageSummary(tessera)}`;
  }

  if (intent === "place_search" && tessera) {
    return `${tessera.nearestPlace} resolves to ///${tessera.words} at ${formatCoordinatePair(tessera.coordinates)}. ${coverageSummary(tessera)}`;
  }

  if (intent === "intelligence_report" && tessera) {
    const confidence = results.find((result) => result.name === "explain_confidence");
    const confidenceText =
      confidence?.name === "explain_confidence"
        ? `Confidence: ${confidence.result.confidence} (${confidence.result.liveDims} live / ${confidence.result.proceduralDims} procedural / ${confidence.result.deferredDims} deferred dims).`
        : "";
    return `Intelligence cell ${tessera.cellId} at ${tessera.nearestPlace}. ${coverageSummary(tessera)} ${confidenceText}`.trim();
  }

  if (intent === "band_inspect" && tessera) {
    const fetch = results.find((result) => result.name === "fetch_bands");
    if (fetch?.name === "fetch_bands") {
      const bands = fetch.result.bands;
      const liveCount = bands.filter((band) => band.status === "live").length;
      const deferredCount = bands.filter((band) => band.status === "deferred").length;
      return `${fetch.result.family ?? "all"} bands: ${bands.length} bands, ${liveCount} live, ${deferredCount} deferred. ${coverageSummary(tessera)}`;
    }
  }

  if (intent === "guidance" || !tessera) {
    return trimmed
      ? "Send coordinates, a three-word emem address, a known place, or ask for the intelligence report on the active cell."
      : "Pick any point on the map or search a place to open its memory cell. Ask me for 'intelligence' to see the 1792D band matrix.";
  }

  return coverageSummary(tessera);
}

function coverageSummary(tessera: TesseraAddress): string {
  const intelligence = tessera.intelligence;

  if (!intelligence) {
    return "";
  }

  const { liveDims, proceduralDims, deferredDims, unavailableDims } = intelligence;
  return `Provider ${intelligence.provider}: ${liveDims} live · ${proceduralDims} procedural · ${deferredDims} deferred · ${unavailableDims} reserved dims of ${intelligence.dimensions}.`;
}

function buildActions(intent: AgentIntent, tessera: TesseraAddress | null): AgentAction[] {
  if (!tessera) {
    return [];
  }

  const base: AgentAction[] = [
    { type: "fly_to", coordinates: tessera.coordinates, zoom: 19.2 }
  ];

  if (intent === "encode_coordinates" || intent === "place_search") {
    base.push({ type: "copy", value: tessera.words, label: "Copy words" });
  }

  if (intent === "decode_words") {
    base.push({ type: "copy", value: formatCoordinatePair(tessera.coordinates), label: "Copy coordinates" });
  }

  if (intent === "intelligence_report" || intent === "band_inspect") {
    base.push({ type: "show_intelligence", cellId: tessera.cellId });
  }

  return base;
}

function buildSuggestions(intent: AgentIntent, tessera: TesseraAddress | null): string[] {
  if (!tessera) {
    return ["Bengaluru", "40.758, -73.9855", "what's here"];
  }

  if (intent === "intelligence_report") {
    return FAMILIES.slice(0, 4).map((family) => FAMILY_LABEL[family].toLowerCase());
  }

  if (intent === "band_inspect") {
    return ["explain confidence", "show intelligence", "share link"];
  }

  return ["what's here", "show intelligence", "share link"];
}

function guidanceResponse(current: TesseraAddress | null): AgentResponse {
  return {
    intent: "guidance",
    answer:
      "Send coordinates, a three-word emem address, a known place, or ask for the intelligence report on the active cell.",
    tessera: current,
    actions: current ? [{ type: "fly_to", coordinates: current.coordinates, zoom: 19.2 }] : [],
    suggestions: buildSuggestions("guidance", current),
    toolCalls: [],
    toolResults: []
  };
}

function chunkText(text: string, size = 48): string[] {
  const chunks: string[] = [];

  for (let index = 0; index < text.length; index += size) {
    chunks.push(text.slice(index, index + size));
  }

  return chunks;
}

function formatCoordinatePair(coordinates: Coordinates): string {
  return `${coordinates.lat.toFixed(6)}, ${coordinates.lng.toFixed(6)}`;
}
