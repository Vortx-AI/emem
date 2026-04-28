import { describe, expect, it } from "vitest";
import {
  coordinatesToTessera,
  generateSuggestions,
  gridSectionFromBoundingBox,
  wordsToTessera
} from "../src/lib/geotessera.js";
import { resolveNearestPlace } from "../src/lib/gazetteer.js";

describe("emem spatial core", () => {
  it("round-trips coordinates through a three-word emem address", () => {
    const encoded = coordinatesToTessera(
      { lat: 12.9716, lng: 77.5946 },
      resolveNearestPlace
    );
    const decoded = wordsToTessera(encoded.words, resolveNearestPlace);

    expect(decoded.words).toBe(encoded.words);
    expect(decoded.cellId).toBe(encoded.cellId);
    expect(Math.abs(decoded.coordinates.lat - encoded.coordinates.lat)).toBeLessThan(0.000001);
    expect(Math.abs(decoded.coordinates.lng - encoded.coordinates.lng)).toBeLessThan(0.000001);
  });

  it("returns a normalized 128D agent vector", () => {
    const encoded = coordinatesToTessera(
      { lat: 40.758, lng: -73.9855 },
      resolveNearestPlace
    );
    const vector = encoded.geotessera128.values;
    const magnitude = Math.sqrt(vector.reduce((sum, value) => sum + value * value, 0));

    expect(encoded.geotessera128.dimensions).toBe(128);
    expect(vector).toHaveLength(128);
    expect(magnitude).toBeGreaterThan(0.999);
    expect(magnitude).toBeLessThan(1.001);
    expect(encoded.geotessera128.checksum).toMatch(/^[a-f0-9]{8}$/);
  });

  it("builds bounded GeoJSON grid sections", () => {
    const grid = gridSectionFromBoundingBox(12.971, 77.594, 12.972, 77.596);

    expect(grid.type).toBe("FeatureCollection");
    expect(grid.features.length).toBeGreaterThan(0);
    expect(grid.features.length).toBeLessThanOrEqual(220);
    expect(grid.properties.cellSizeMeters).toBe(3);
  });

  it("suggests completions around the focus point", () => {
    const encoded = coordinatesToTessera(
      { lat: 37.8199, lng: -122.4783 },
      resolveNearestPlace
    );
    const prefix = encoded.words.split(".").slice(0, 2).join(".") + ".";
    const suggestions = generateSuggestions(
      prefix,
      encoded.coordinates,
      6,
      resolveNearestPlace
    );

    expect(suggestions.some((suggestion) => suggestion.words === encoded.words)).toBe(true);
  });
});
