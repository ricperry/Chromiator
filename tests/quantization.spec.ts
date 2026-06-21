import { describe, expect, it } from "vitest";

import {
  analyzeImageData,
  computeBezierAssignments,
  computeVoronoiAssignments,
  createBezierNode,
  createBezierRegion,
  createQuantizationGroup,
  createSite,
  projectionForRegion,
  renderBezierImage,
  renderVoronoiImage,
} from "../src/quantization";

class TestImageData {
  data: Uint8ClampedArray;
  width: number;
  height: number;

  constructor(
    dataOrWidth: Uint8ClampedArray | number,
    width?: number,
    height?: number,
  ) {
    if (typeof dataOrWidth === "number") {
      this.width = dataOrWidth;
      this.height = width ?? 1;
      this.data = new Uint8ClampedArray(this.width * this.height * 4);
    } else {
      this.data = dataOrWidth;
      this.width = width ?? 1;
      this.height = height ?? 1;
    }
  }
}

globalThis.ImageData = TestImageData as typeof ImageData;

function imageData(data: number[]): ImageData {
  return new ImageData(new Uint8ClampedArray(data), data.length / 4, 1);
}

describe("Quantiator assignment kernels", () => {
  it("assigns source colors to weighted Voronoi groups and preserves transparency", () => {
    const source = analyzeImageData(
      imageData([250, 0, 0, 255, 0, 0, 250, 255, 12, 34, 56, 0]),
    );
    const red = createQuantizationGroup("Red", { r: 255, g: 16, b: 16 });
    const blue = createQuantizationGroup("Blue", { r: 16, g: 16, b: 255 });
    red.sites.push(createSite(red.id, "rgb", { r: 255, g: 0, b: 0 }));
    blue.sites.push(createSite(blue.id, "rgb", { r: 0, g: 0, b: 255 }));

    const stats = computeVoronoiAssignments(source, [red, blue], "rgb");
    const output = renderVoronoiImage(source, [red, blue], stats);

    expect(stats.ready).toBe(true);
    expect(Array.from(output.data)).toEqual([
      255, 16, 16, 255, 16, 16, 255, 255, 0, 0, 0, 0,
    ]);
  });

  it("uses closed Bezier projections as color-space volumes", () => {
    const source = analyzeImageData(
      imageData([100, 100, 100, 255, 240, 240, 240, 255]),
    );
    const region = createBezierRegion("Mid", "rgb", { r: 10, g: 20, b: 30 });

    for (const panel of ["rg", "rb", "gb"] as const) {
      const projection = projectionForRegion(region, panel);
      projection.nodes.push(
        createBezierNode(50, 50),
        createBezierNode(150, 50),
        createBezierNode(150, 150),
        createBezierNode(50, 150),
      );
      projection.closed = true;
    }

    const stats = computeBezierAssignments(source, [region], "rgb");
    const output = renderBezierImage(source, [region], stats);

    expect(stats.ready).toBe(true);
    expect(Array.from(output.data)).toEqual([10, 20, 30, 255, 42, 48, 66, 255]);
  });
});
