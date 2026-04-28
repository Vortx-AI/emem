import {
  Bot,
  BrainCircuit,
  Check,
  ChevronRight,
  Copy,
  Crosshair,
  Fingerprint,
  Grid3X3,
  Grip,
  Layers3,
  LocateFixed,
  MapPin,
  Maximize2,
  MessageSquareText,
  Radio,
  Search,
  Share2,
  Sparkles
} from "lucide-react";
import maplibregl, { GeoJSONSource, LngLatLike, Map as MapLibreMap, Marker } from "maplibre-gl";
import {
  CSSProperties,
  FormEvent,
  PointerEvent as ReactPointerEvent,
  ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState
} from "react";
import { FAMILIES, FAMILY_LABEL, type BandFamily } from "./lib/bands.js";
import type {
  Coordinates,
  Geotessera1792,
  GridSection,
  Suggestion,
  TesseraAddress
} from "./lib/geotessera.js";

type SearchResponse = {
  results: TesseraAddress[];
};

type AutosuggestResponse = {
  suggestions: Suggestion[];
};

type AgentResponse = {
  intent: string;
  answer: string;
  tessera: TesseraAddress | null;
  actions: Array<{ type: string; coordinates?: Coordinates; zoom?: number; value?: string; label?: string }>;
  suggestions: string[];
  toolCalls?: Array<{ name: string; args: Record<string, unknown> }>;
  toolResults?: Array<{ name: string; result: unknown }>;
};

type ProviderStatus = {
  name: string;
  kind: "procedural" | "remote";
  live: boolean;
  reason: string | null;
  liveFamilies: number;
  endpoint: string | null;
};

type HealthResponse = {
  ok: boolean;
  product: string;
  mode: string;
  provider: ProviderStatus;
};

type ChatMessage = {
  role: "user" | "assistant";
  text: string;
};

type Mode = "human" | "agent";

type PaneSize = {
  width: number;
  height: number;
};

type PaneConstraints = {
  minWidth: number;
  minHeight: number;
  maxWidth: number;
  maxHeight: number;
};

type ResizeAnchor = "se" | "nw";

const DEFAULT_COORDINATES: Coordinates = { lat: 12.9716, lng: 77.5946 };
const EMPTY_COLLECTION = { type: "FeatureCollection", features: [] } as GeoJSON.FeatureCollection;
const MAP_STYLE = "https://tiles.openfreemap.org/styles/bright";

export function App() {
  const [selected, setSelected] = useState<TesseraAddress | null>(null);
  const [query, setQuery] = useState("");
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [mode, setMode] = useState<Mode>("human");
  const [isSearching, setIsSearching] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const [providerStatus, setProviderStatus] = useState<ProviderStatus | null>(null);
  const [expandedFamily, setExpandedFamily] = useState<BandFamily | null>(null);
  const [chatInput, setChatInput] = useState("");
  const [chat, setChat] = useState<ChatMessage[]>([
    {
      role: "assistant",
      text: "emem.dev is live. Every memory cell now carries a 1792-dimension intelligence contract."
    }
  ]);
  const commandPane = useResizablePane(
    "emem-command-pane",
    { width: 448, height: 792 },
    { minWidth: 360, minHeight: 520, maxWidth: 720, maxHeight: 900 },
    "se"
  );
  const assistantPane = useResizablePane(
    "emem-assistant-pane",
    { width: 392, height: 356 },
    { minWidth: 340, minHeight: 288, maxWidth: 620, maxHeight: 680 },
    "nw"
  );

  const agentPayload = useMemo(() => {
    if (!selected) {
      return "";
    }

    const intelligence = selected.intelligence;

    return JSON.stringify(
      {
        emem: `emem://${selected.words}`,
        words: selected.words,
        coordinates: selected.coordinates,
        country: selected.country,
        nearestPlace: selected.nearestPlace,
        cellId: selected.cellId,
        geotessera128: {
          model: selected.geotessera128.model,
          dimensions: selected.geotessera128.dimensions,
          checksum: selected.geotessera128.checksum,
          values: selected.geotessera128.values.slice(0, 16)
        },
        intelligence: intelligence
          ? {
              model: intelligence.model,
              dimensions: intelligence.dimensions,
              provider: intelligence.provider,
              capturedAt: intelligence.capturedAt,
              liveDims: intelligence.liveDims,
              proceduralDims: intelligence.proceduralDims,
              deferredDims: intelligence.deferredDims,
              unavailableDims: intelligence.unavailableDims,
              checksum: intelligence.checksum,
              coverage: intelligence.coverage.map((band) => ({
                key: band.key,
                family: band.family,
                status: band.status,
                dims: band.dims
              }))
            }
          : null
      },
      null,
      2
    );
  }, [selected]);

  useEffect(() => {
    void convertCoordinates(DEFAULT_COORDINATES).then((address) => {
      setSelected(address);
      setQuery(address.words);
    });
  }, []);

  useEffect(() => {
    let cancelled = false;

    void fetch("/api/health")
      .then((response) => response.json() as Promise<HealthResponse>)
      .then((payload) => {
        if (!cancelled) {
          setProviderStatus(payload.provider);
        }
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!query || query.length < 3) {
      setSuggestions([]);
      return;
    }

    const timeout = window.setTimeout(async () => {
      try {
        const focus = selected
          ? `&focus=${selected.coordinates.lat},${selected.coordinates.lng}`
          : "";
        const response = await fetch(`/api/autosuggest?input=${encodeURIComponent(query)}${focus}`);
        const payload = (await response.json()) as AutosuggestResponse;
        setSuggestions(payload.suggestions ?? []);
      } catch {
        setSuggestions([]);
      }
    }, 160);

    return () => window.clearTimeout(timeout);
  }, [query, selected]);

  const selectAddress = useCallback((address: TesseraAddress) => {
    setSelected(address);
    setQuery(address.words);
    setSuggestions([]);
  }, []);

  const handleSearch = async (event?: FormEvent) => {
    event?.preventDefault();
    const trimmed = query.trim();

    if (!trimmed) {
      return;
    }

    setIsSearching(true);

    try {
      const response = await fetch(`/api/search?q=${encodeURIComponent(trimmed)}`);
      const payload = (await response.json()) as SearchResponse;
      const [result] = payload.results ?? [];

      if (result) {
        selectAddress(result);
      } else {
        setChat((messages) => [
          ...messages,
          { role: "assistant", text: "No exact emem memory found for that search." }
        ]);
      }
    } finally {
      setIsSearching(false);
    }
  };

  const handleSuggestion = async (words: string) => {
    const response = await fetch(
      `/api/convert-to-coordinates?words=${encodeURIComponent(words)}`
    );
    const address = (await response.json()) as TesseraAddress;
    selectAddress(address);
  };

  const handleMapPick = useCallback(async (coordinates: Coordinates) => {
    const address = await convertCoordinates(coordinates);
    selectAddress(address);
  }, [selectAddress]);

  const handleLocate = () => {
    navigator.geolocation?.getCurrentPosition(async (position) => {
      const address = await convertCoordinates({
        lat: position.coords.latitude,
        lng: position.coords.longitude
      });
      selectAddress(address);
    });
  };

  const handleCopy = async (label: string, value: string) => {
    await navigator.clipboard.writeText(value);
    setCopied(label);
    window.setTimeout(() => setCopied(null), 1200);
  };

  const handleChat = async (event?: FormEvent) => {
    event?.preventDefault();
    const message = chatInput.trim();

    if (!message) {
      return;
    }

    setChatInput("");
    setChat((messages) => [...messages, { role: "user", text: message }]);

    const response = await fetch("/api/agent/resolve", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ message, current: selected })
    });
    const payload = (await response.json()) as AgentResponse;

    if (payload.tessera) {
      selectAddress(payload.tessera);
    }

    setChat((messages) => [...messages, { role: "assistant", text: payload.answer }]);
  };

  return (
    <main className="app-shell">
      <MapCanvas selected={selected} onPick={handleMapPick} />

      <section
        className="command-panel resizable-pane"
        aria-label="emem.dev command panel"
        style={paneStyle(commandPane.size)}
      >
        <PanelChrome
          icon={<Layers3 size={17} />}
          title="Memory console"
          status={selected ? selected.geotessera128.checksum : "warming"}
          onReset={commandPane.reset}
        />

        <div className="panel-body command-body">
          <div className="brand-row">
            <div className="brand-mark">
              <Grid3X3 size={20} />
            </div>
            <div>
              <p className="eyebrow">emem.dev</p>
              <h1>Location memory for people and agents.</h1>
            </div>
          </div>

          <ProviderRibbon status={providerStatus} intelligence={selected?.intelligence} />

          <form className="search-box" onSubmit={handleSearch}>
            <Search size={20} />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search words, coordinates, or place"
              aria-label="Search emem.dev"
            />
            <button className="icon-button primary" type="submit" aria-label="Search">
              {isSearching ? <Sparkles size={18} /> : <ChevronRight size={18} />}
            </button>
          </form>

          {suggestions.length > 0 && (
            <div className="suggestion-list">
              {suggestions.map((suggestion) => (
                <button
                  key={suggestion.words}
                  type="button"
                  onClick={() => void handleSuggestion(suggestion.words)}
                >
                  <span>///{suggestion.words}</span>
                  <small>
                    {suggestion.nearestPlace}
                    {suggestion.distanceToFocusKm !== null
                      ? ` · ${suggestion.distanceToFocusKm} km`
                      : ""}
                  </small>
                </button>
              ))}
            </div>
          )}

          <div className="mode-switch" role="tablist" aria-label="Mode">
            <button
              className={mode === "human" ? "active" : ""}
              type="button"
              onClick={() => setMode("human")}
            >
              <MapPin size={16} />
              Human
            </button>
            <button
              className={mode === "agent" ? "active" : ""}
              type="button"
              onClick={() => setMode("agent")}
            >
              <Bot size={16} />
              Agent
            </button>
          </div>

          {selected && mode === "human" && (
            <HumanPanel selected={selected} copied={copied} onCopy={handleCopy} />
          )}

          {selected && mode === "agent" && (
            <AgentPanel
              selected={selected}
              payload={agentPayload}
              copied={copied}
              onCopy={handleCopy}
              expandedFamily={expandedFamily}
              onExpandFamily={setExpandedFamily}
            />
          )}
        </div>
        <ResizeHandle anchor="se" onPointerDown={commandPane.beginResize} label="Resize memory console" />
      </section>

      <section
        className="assistant-panel resizable-pane"
        aria-label="emem.dev assistant"
        style={paneStyle(assistantPane.size)}
      >
        <div className="assistant-head">
          <div>
            <p className="eyebrow">Native LLM</p>
            <h2>emem copilot</h2>
          </div>
          <div className="pane-actions">
            <button className="icon-button ghost" type="button" onClick={assistantPane.reset} aria-label="Reset assistant pane size">
              <Maximize2 size={16} />
            </button>
            <BrainCircuit size={22} />
          </div>
        </div>

        <div className="chat-log">
          {chat.map((message, index) => (
            <div className={`chat-bubble ${message.role}`} key={`${message.role}-${index}`}>
              {message.text}
            </div>
          ))}
        </div>

        <form className="chat-box" onSubmit={handleChat}>
          <MessageSquareText size={18} />
          <input
            value={chatInput}
            onChange={(event) => setChatInput(event.target.value)}
            placeholder="Ask for a place, memory, or vector"
            aria-label="Ask emem copilot"
          />
          <button className="icon-button" type="submit" aria-label="Send">
            <ChevronRight size={18} />
          </button>
        </form>
        <ResizeHandle anchor="nw" onPointerDown={assistantPane.beginResize} label="Resize emem copilot" />
      </section>

      <div className="map-actions" aria-label="Map actions">
        <button className="icon-button" type="button" onClick={handleLocate} aria-label="Locate">
          <LocateFixed size={19} />
        </button>
        <button
          className="icon-button"
          type="button"
          onClick={() =>
            selected && void handleCopy("share", `https://emem.dev/${selected.words}`)
          }
          aria-label="Share"
        >
          <Share2 size={19} />
        </button>
      </div>
    </main>
  );
}

function PanelChrome({
  icon,
  title,
  status,
  onReset
}: {
  icon: ReactNode;
  title: string;
  status: string;
  onReset: () => void;
}) {
  return (
    <div className="panel-chrome">
      <div className="panel-chip">
        {icon}
        <span>{title}</span>
      </div>
      <div className="panel-status">
        <Crosshair size={14} />
        <span>{status}</span>
      </div>
      <button className="icon-button ghost" type="button" onClick={onReset} aria-label="Reset pane size">
        <Maximize2 size={16} />
      </button>
    </div>
  );
}

function useResizablePane(
  storageKey: string,
  defaultSize: PaneSize,
  constraints: PaneConstraints,
  anchor: ResizeAnchor
) {
  const [size, setSize] = useState<PaneSize>(() => {
    if (typeof window === "undefined") {
      return defaultSize;
    }

    try {
      const stored = window.localStorage.getItem(storageKey);

      if (!stored) {
        return defaultSize;
      }

      const parsed = JSON.parse(stored) as Partial<PaneSize>;
      return constrainPaneSize(
        {
          width: Number(parsed.width) || defaultSize.width,
          height: Number(parsed.height) || defaultSize.height
        },
        constraints
      );
    } catch {
      return defaultSize;
    }
  });

  useEffect(() => {
    window.localStorage.setItem(storageKey, JSON.stringify(size));
  }, [size, storageKey]);

  const beginResize = useCallback(
    (event: ReactPointerEvent<HTMLButtonElement>) => {
      if (window.matchMedia("(max-width: 980px)").matches) {
        return;
      }

      event.preventDefault();
      const startX = event.clientX;
      const startY = event.clientY;
      const startSize = size;

      document.documentElement.classList.add("is-resizing-pane");

      const handleMove = (moveEvent: PointerEvent) => {
        const dx = moveEvent.clientX - startX;
        const dy = moveEvent.clientY - startY;
        const nextSize = {
          width: anchor === "nw" ? startSize.width - dx : startSize.width + dx,
          height: anchor === "nw" ? startSize.height - dy : startSize.height + dy
        };

        setSize(constrainPaneSize(nextSize, constraints));
      };

      const stopResize = () => {
        document.documentElement.classList.remove("is-resizing-pane");
        window.removeEventListener("pointermove", handleMove);
      };

      window.addEventListener("pointermove", handleMove);
      window.addEventListener("pointerup", stopResize, { once: true });
      window.addEventListener("pointercancel", stopResize, { once: true });
    },
    [anchor, constraints, size]
  );

  const reset = useCallback(() => setSize(defaultSize), [defaultSize]);

  return { size, beginResize, reset };
}

function paneStyle(size: PaneSize): CSSProperties {
  return {
    "--pane-width": `${size.width}px`,
    "--pane-height": `${size.height}px`
  } as CSSProperties;
}

function constrainPaneSize(size: PaneSize, constraints: PaneConstraints): PaneSize {
  const viewportWidth = typeof window === "undefined" ? constraints.maxWidth : window.innerWidth;
  const viewportHeight = typeof window === "undefined" ? constraints.maxHeight : window.innerHeight;

  return {
    width: clamp(size.width, constraints.minWidth, Math.min(constraints.maxWidth, viewportWidth - 32)),
    height: clamp(size.height, constraints.minHeight, Math.min(constraints.maxHeight, viewportHeight - 32))
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max));
}

function ResizeHandle({
  anchor,
  label,
  onPointerDown
}: {
  anchor: ResizeAnchor;
  label: string;
  onPointerDown: (event: ReactPointerEvent<HTMLButtonElement>) => void;
}) {
  return (
    <button
      className={`resize-handle resize-${anchor}`}
      type="button"
      onPointerDown={onPointerDown}
      onDoubleClick={(event) => event.preventDefault()}
      aria-label={label}
    >
      <Grip size={15} />
    </button>
  );
}

function HumanPanel({
  selected,
  copied,
  onCopy
}: {
  selected: TesseraAddress;
  copied: string | null;
  onCopy: (label: string, value: string) => Promise<void>;
}) {
  return (
    <div className="data-panel">
      <div className="primary-address">
        <small>Human memory address</small>
        <button type="button" onClick={() => void onCopy("words", selected.words)}>
          <span>///{selected.words}</span>
          {copied === "words" ? <Check size={18} /> : <Copy size={18} />}
        </button>
      </div>

      <div className="metric-grid">
        <Metric label="Coordinates" value={`${selected.coordinates.lat}, ${selected.coordinates.lng}`} />
        <Metric label="Nearest place" value={selected.nearestPlace} />
        <Metric label="Country" value={selected.country} />
        <Metric label="Language" value={selected.language} />
      </div>

      <div className="bounds-box">
        <div>
          <small>Southwest</small>
          <span>
            {selected.square.southwest.lat}, {selected.square.southwest.lng}
          </span>
        </div>
        <div>
          <small>Northeast</small>
          <span>
            {selected.square.northeast.lat}, {selected.square.northeast.lng}
          </span>
        </div>
      </div>

      <MemoryStack selected={selected} />
    </div>
  );
}

function AgentPanel({
  selected,
  payload,
  copied,
  onCopy,
  expandedFamily,
  onExpandFamily
}: {
  selected: TesseraAddress;
  payload: string;
  copied: string | null;
  onCopy: (label: string, value: string) => Promise<void>;
  expandedFamily: BandFamily | null;
  onExpandFamily: (family: BandFamily | null) => void;
}) {
  return (
    <div className="data-panel agent-data">
      <div className="agent-headline">
        <Fingerprint size={22} />
        <div>
          <small>Agent memory cell</small>
          <strong>{selected.cellId}</strong>
        </div>
        <button
          className="icon-button"
          type="button"
          onClick={() => void onCopy("json", payload)}
          aria-label="Copy agent JSON"
        >
          {copied === "json" ? <Check size={17} /> : <Copy size={17} />}
        </button>
      </div>

      <div className="vector-strip">
        <span>{selected.intelligence?.model ?? selected.geotessera128.model}</span>
        <span>{selected.intelligence?.dimensions ?? 128}D</span>
        <span>{selected.intelligence?.checksum ?? selected.geotessera128.checksum}</span>
      </div>

      <BandMatrix
        intelligence={selected.intelligence ?? null}
        expandedFamily={expandedFamily}
        onExpandFamily={onExpandFamily}
      />

      <pre className="json-preview">{payload}</pre>
    </div>
  );
}

function ProviderRibbon({
  status,
  intelligence
}: {
  status: ProviderStatus | null;
  intelligence?: Geotessera1792 | null;
}) {
  const kind = status?.kind ?? "procedural";
  const live = status?.live ?? false;
  const name = status?.name ?? "emem-procedural";

  return (
    <div className={`provider-ribbon ${kind} ${live ? "live" : "offline"}`}>
      <Radio size={14} />
      <div className="provider-labels">
        <strong>{kind === "remote" ? "agri 1792D · remote" : "procedural · local"}</strong>
        <small>{name}</small>
      </div>
      {intelligence ? (
        <div className="provider-coverage" aria-label="Intelligence coverage">
          <span title="Real (remote) dims">{intelligence.liveDims}</span>
          <em>/</em>
          <span title="Procedural placeholder dims">{intelligence.proceduralDims}</span>
          <em>/</em>
          <span title="Deferred until data plane connects">{intelligence.deferredDims}</span>
          <em>/</em>
          <span title="Reserved dims">{intelligence.unavailableDims}</span>
        </div>
      ) : null}
    </div>
  );
}

function BandMatrix({
  intelligence,
  expandedFamily,
  onExpandFamily
}: {
  intelligence: Geotessera1792 | null;
  expandedFamily: BandFamily | null;
  onExpandFamily: (family: BandFamily | null) => void;
}) {
  if (!intelligence) {
    return (
      <div className="band-matrix empty">
        <small>Intelligence pending. Select a cell to load the 1792D contract.</small>
      </div>
    );
  }

  const grouped = FAMILIES.map((family) => {
    const bands = intelligence.coverage.filter((band) => band.family === family);
    const dims = bands.reduce((sum, band) => sum + band.dims, 0);
    const live = bands.filter((band) => band.status === "live").length;
    const deferred = bands.filter((band) => band.status === "deferred").length;
    const procedural = bands.filter((band) => band.status === "procedural").length;
    const status = live > 0 ? "live" : procedural > 0 ? "procedural" : deferred > 0 ? "deferred" : "unavailable";

    return { family, label: FAMILY_LABEL[family], dims, bands, status };
  }).filter((entry) => entry.dims > 0);

  return (
    <div className="band-matrix">
      <div className="band-matrix-header">
        <span>Intelligence layer</span>
        <small>{intelligence.dimensions}D · {intelligence.coverage.length} bands</small>
      </div>
      <div className="band-grid">
        {grouped.map((entry) => (
          <button
            key={entry.family}
            type="button"
            className={`band-cell status-${entry.status} ${
              expandedFamily === entry.family ? "expanded" : ""
            }`}
            onClick={() =>
              onExpandFamily(expandedFamily === entry.family ? null : entry.family)
            }
          >
            <strong>{entry.label}</strong>
            <span>{entry.dims}D · {entry.status}</span>
          </button>
        ))}
      </div>
      {expandedFamily ? (
        <div className="band-detail">
          {intelligence.coverage
            .filter((band) => band.family === expandedFamily)
            .map((band) => (
              <div key={band.key} className={`band-row status-${band.status}`}>
                <div className="band-row-head">
                  <strong>{band.label}</strong>
                  <span>{band.dims}D · {band.tempo}</span>
                </div>
                <small className="band-source">{band.source}</small>
                {band.note ? <p className="band-note">{band.note}</p> : null}
                {band.summary?.length ? (
                  <div className="band-summary">
                    {band.summary.map((metric) => (
                      <span key={metric.name}>
                        <em>{metric.name}</em>
                        {String(metric.value)}{metric.unit ?? ""}
                      </span>
                    ))}
                  </div>
                ) : null}
              </div>
            ))}
        </div>
      ) : null}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric">
      <small>{label}</small>
      <span>{value}</span>
    </div>
  );
}

function MemoryStack({ selected }: { selected: TesseraAddress }) {
  const layers = [
    {
      icon: <MapPin size={15} />,
      label: "Human",
      value: `///${selected.words}`
    },
    {
      icon: <Grid3X3 size={15} />,
      label: "Square",
      value: "3m bounded cell"
    },
    {
      icon: <Fingerprint size={15} />,
      label: "Cell",
      value: selected.cellId
    },
    {
      icon: <BrainCircuit size={15} />,
      label: "Vector",
      value: selected.geotessera128.checksum
    }
  ];

  return (
    <div className="memory-stack" aria-label="emem memory layers">
      {layers.map((layer) => (
        <div className="memory-layer" key={layer.label}>
          <div className="memory-layer-icon">{layer.icon}</div>
          <div>
            <small>{layer.label}</small>
            <span>{layer.value}</span>
          </div>
        </div>
      ))}
    </div>
  );
}

function MapCanvas({
  selected,
  onPick
}: {
  selected: TesseraAddress | null;
  onPick: (coordinates: Coordinates) => void;
}) {
  const container = useRef<HTMLDivElement | null>(null);
  const map = useRef<MapLibreMap | null>(null);
  const marker = useRef<Marker | null>(null);

  useEffect(() => {
    if (!container.current || map.current) {
      return;
    }

    const instance = new maplibregl.Map({
      container: container.current,
      style: MAP_STYLE,
      center: [DEFAULT_COORDINATES.lng, DEFAULT_COORDINATES.lat],
      zoom: 15,
      pitch: 54,
      bearing: -18,
      attributionControl: false
    });

    map.current = instance;
    instance.addControl(new maplibregl.AttributionControl({ compact: true }), "bottom-right");
    instance.addControl(new maplibregl.NavigationControl({ visualizePitch: true }), "bottom-right");

    instance.on("load", () => {
      instance.addSource("grid-section", {
        type: "geojson",
        data: EMPTY_COLLECTION
      });
      instance.addLayer({
        id: "grid-section-line",
        type: "line",
        source: "grid-section",
        paint: {
          "line-color": "#f05d4f",
          "line-width": ["interpolate", ["linear"], ["zoom"], 17, 0.4, 21, 1.3],
          "line-opacity": ["interpolate", ["linear"], ["zoom"], 17, 0, 18.5, 0.84]
        }
      });

      instance.addSource("selected-square", {
        type: "geojson",
        data: EMPTY_COLLECTION
      });
      instance.addLayer({
        id: "selected-square-fill",
        type: "fill",
        source: "selected-square",
        paint: {
          "fill-color": "#11b5a4",
          "fill-opacity": 0.28
        }
      });
      instance.addLayer({
        id: "selected-square-line",
        type: "line",
        source: "selected-square",
        paint: {
          "line-color": "#082c2b",
          "line-width": 2.8
        }
      });

      void updateGrid(instance);
    });

    instance.on("click", (event) => {
      onPick({ lat: event.lngLat.lat, lng: event.lngLat.lng });
    });

    instance.on("moveend", () => {
      void updateGrid(instance);
    });

    return () => {
      instance.remove();
      map.current = null;
    };
  }, [onPick]);

  useEffect(() => {
    const instance = map.current;

    if (!instance || !selected) {
      return;
    }

    const center: LngLatLike = [selected.coordinates.lng, selected.coordinates.lat];

    if (!marker.current) {
      const element = document.createElement("div");
      element.className = "tessera-marker";
      marker.current = new maplibregl.Marker({ element, anchor: "center" })
        .setLngLat(center)
        .addTo(instance);
    } else {
      marker.current.setLngLat(center);
    }

    const square = squareFeature(selected);
    const source = instance.getSource("selected-square") as GeoJSONSource | undefined;
    source?.setData(square);

    instance.flyTo({
      center,
      zoom: Math.max(instance.getZoom(), 18.6),
      pitch: 58,
      speed: 0.72,
      curve: 1.2,
      essential: true
    });
  }, [selected]);

  return <div ref={container} className="map-canvas" aria-label="emem.dev map" />;
}

async function updateGrid(instance: MapLibreMap) {
  if (!instance.isStyleLoaded()) {
    return;
  }

  const source = instance.getSource("grid-section") as GeoJSONSource | undefined;

  if (!source) {
    return;
  }

  if (instance.getZoom() < 18.1) {
    source.setData(EMPTY_COLLECTION);
    return;
  }

  const bounds = instance.getBounds();
  const bbox = [
    bounds.getSouth(),
    bounds.getWest(),
    bounds.getNorth(),
    bounds.getEast()
  ].join(",");

  try {
    const response = await fetch(`/api/grid-section?bounding-box=${encodeURIComponent(bbox)}`);
    const grid = (await response.json()) as GridSection;
    source.setData(grid as GeoJSON.FeatureCollection);
  } catch {
    source.setData(EMPTY_COLLECTION);
  }
}

function squareFeature(selected: TesseraAddress): GeoJSON.FeatureCollection {
  const { southwest, northeast } = selected.square;

  return {
    type: "FeatureCollection",
    features: [
      {
        type: "Feature",
        properties: {},
        geometry: {
          type: "Polygon",
          coordinates: [
            [
              [southwest.lng, southwest.lat],
              [northeast.lng, southwest.lat],
              [northeast.lng, northeast.lat],
              [southwest.lng, northeast.lat],
              [southwest.lng, southwest.lat]
            ]
          ]
        }
      }
    ]
  };
}

async function convertCoordinates(coordinates: Coordinates): Promise<TesseraAddress> {
  const response = await fetch(
    `/api/convert-to-3wa?coordinates=${coordinates.lat},${coordinates.lng}`
  );
  return (await response.json()) as TesseraAddress;
}
