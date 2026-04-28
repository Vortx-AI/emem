import { describe, expect, it } from "vitest";
import {
  BANDS,
  FAMILIES,
  GEOTESSERA1792_DIM,
  bandsByFamily,
  familyDims,
  totalDims,
  validateRegistry
} from "../src/lib/bands.js";

describe("band registry", () => {
  it("sums to exactly 1792 dimensions", () => {
    expect(totalDims()).toBe(GEOTESSERA1792_DIM);
  });

  it("has no gaps or overlaps in offsets", () => {
    expect(() => validateRegistry()).not.toThrow();
  });

  it("covers every 1792-dim offset through a family", () => {
    const covered = FAMILIES.reduce((sum, family) => sum + familyDims(family), 0);
    expect(covered).toBe(GEOTESSERA1792_DIM);
  });

  it("indexes the 128D foundation band at offset 0", () => {
    const foundation = bandsByFamily("foundation");
    expect(foundation[0].key).toBe("geotessera");
    expect(foundation[0].offset).toBe(0);
    expect(foundation[0].dims).toBe(128);
  });

  it("keeps reserved bands at the tail", () => {
    const last = BANDS[BANDS.length - 1];
    expect(last.key).toBe("reserved");
    expect(last.offset + last.dims).toBe(GEOTESSERA1792_DIM);
  });
});
