import { describe, expect, it } from "vitest";

import {
  applyAlphaPosterize,
  applyBasicPosterize,
  hsv255ToRgb,
  rgbToHsv255,
} from "../src/effect";

function imageData(data: number[]): ImageData {
  return {
    data: new Uint8ClampedArray(data),
    width: data.length / 4,
    height: 1,
  } as ImageData;
}

describe("effect kernels", () => {
  it("converts primary colors between RGB and HSV byte ranges", () => {
    expect(rgbToHsv255(255, 0, 0)).toEqual([0, 255, 255]);
    expect(rgbToHsv255(0, 255, 0)).toEqual([85, 255, 255]);
    expect(rgbToHsv255(0, 0, 255)).toEqual([170, 255, 255]);

    expect(hsv255ToRgb(0, 255, 255)).toEqual([255, 0, 0]);
    expect(hsv255ToRgb(85, 255, 255)).toEqual([0, 255, 0]);
    expect(hsv255ToRgb(170, 255, 255)).toEqual([0, 0, 255]);
  });

  it("maps RGB channels into threshold bands with explicit outputs", () => {
    const source = imageData([10, 130, 250, 255, 200, 70, 20, 128]);

    applyBasicPosterize(
      source,
      { R: 2, G: 2, B: 2 },
      { R: [127], G: [127], B: [127] },
      { R: [0, 255], G: [32, 224], B: [64, 192] },
    );

    expect(Array.from(source.data)).toEqual([
      0, 224, 192, 255, 255, 32, 64, 128,
    ]);
  });

  it("posterizes alpha without changing color channels", () => {
    const source = imageData([12, 34, 56, 10, 78, 90, 123, 240]);

    applyAlphaPosterize(source, 2, [127], [0, 255]);

    expect(Array.from(source.data)).toEqual([12, 34, 56, 0, 78, 90, 123, 255]);
  });
});
