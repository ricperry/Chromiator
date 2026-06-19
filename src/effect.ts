/**
* Minimal per-channel posterization (no band widths or hue yet).
* This is a stub you can extend into full Super Posterize logic.
*/
export type Levels = { R: number; G: number; B: number };
export type ChannelThresholds = Partial<Record<keyof Levels, number[]>>;
export type ChannelOutputs = Partial<Record<keyof Levels, number[]>>;

export type HSVLevels = { H: number; S: number; V: number };
export type HsvThresholds = Partial<Record<keyof HSVLevels, number[]>>;
export type HsvOutputs = Partial<Record<keyof HSVLevels, number[]>>;

/**
 * Clamp a numeric value into the 0–255 range used by 8-bit RGB channels.
 */
const clampChannel = (value: number) =>
  Math.min(255, Math.max(0, Math.round(value)));

const clampUnit = (value: number) => Math.min(1, Math.max(0, value));

export function rgbToHsv255(
  r: number,
  g: number,
  b: number,
): [number, number, number] {
  const rn = r / 255;
  const gn = g / 255;
  const bn = b / 255;

  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const delta = max - min;

  let h = 0;
  if (delta !== 0) {
    if (max === rn) {
      h = (gn - bn) / delta;
    } else if (max === gn) {
      h = (bn - rn) / delta + 2;
    } else {
      h = (rn - gn) / delta + 4;
    }
    h /= 6;
    if (h < 0) h += 1;
  }

  const s = max === 0 ? 0 : delta / max;
  const v = max;

  return [
    clampChannel(h * 255),
    clampChannel(s * 255),
    clampChannel(v * 255),
  ];
}

export function hsv255ToRgb(
  h: number,
  s: number,
  v: number,
): [number, number, number] {
  const hn = (h / 255) % 1;
  const sn = clampUnit(s / 255);
  const vn = clampUnit(v / 255);

  const c = vn * sn;
  const h6 = hn * 6;
  const x = c * (1 - Math.abs((h6 % 2) - 1));

  let r1 = 0;
  let g1 = 0;
  let b1 = 0;

  const sector = Math.floor(h6);
  switch (sector) {
    case 0:
      r1 = c;
      g1 = x;
      b1 = 0;
      break;
    case 1:
      r1 = x;
      g1 = c;
      b1 = 0;
      break;
    case 2:
      r1 = 0;
      g1 = c;
      b1 = x;
      break;
    case 3:
      r1 = 0;
      g1 = x;
      b1 = c;
      break;
    case 4:
      r1 = x;
      g1 = 0;
      b1 = c;
      break;
    default:
      r1 = c;
      g1 = 0;
      b1 = x;
      break;
  }

  const m = vn - c;
  return [
    clampChannel((r1 + m) * 255),
    clampChannel((g1 + m) * 255),
    clampChannel((b1 + m) * 255),
  ];
}

function quantize(
  value: number,
  levels: number,
  thresholds?: number[],
  outputs?: number[]
): number {
  /**
   * Quantize a single channel value into a band defined by the supplied
   * thresholds and optional explicit output levels. When thresholds or
   * outputs are missing we fall back to equally spaced bands and their
   * midpoints.
   */
  const L = Math.max(1, Math.floor(levels));
  if (L === 1) return value < 128 ? 0 : 255;

  if (!thresholds || thresholds.length !== L - 1) {
    const step = 256 / L; // band size
    const bandIndex = Math.min(L - 1, Math.floor(value / step));
    const lower = bandIndex * step;
    const upper = bandIndex === L - 1 ? 255 : (bandIndex + 1) * step;
    const bandCenter = (lower + upper) / 2;
    const custom = outputs?.[bandIndex];
    return clampChannel(custom ?? bandCenter);
  }

  let lower = 0;
  for (let i = 0; i < thresholds.length; i += 1) {
    const upper = thresholds[i];
    if (value <= upper) {
      const fallback = (lower + upper) / 2;
      const custom = outputs?.[i];
      return clampChannel(custom ?? fallback);
    }
    lower = upper;
  }

  const fallback = (lower + 255) / 2;
  const custom = outputs?.[thresholds.length];
  return clampChannel(custom ?? fallback);
}

export function applyBasicPosterize(
  imageData: ImageData,
  levels: Levels,
  thresholds: ChannelThresholds = {},
  outputs: ChannelOutputs = {}
): ImageData {
  /**
   * Iterate over the supplied ImageData and posterize each channel according
   * to the provided level counts, optional thresholds, and optional per-band
   * output values. All operations mutate the incoming ImageData in place to
   * avoid extra allocations for callers that are already working on copies.
   */
  const { data } = imageData;
  const Lr = Math.max(1, Math.floor(levels.R));
  const Lg = Math.max(1, Math.floor(levels.G));
  const Lb = Math.max(1, Math.floor(levels.B));

  const thresholdsR = thresholds.R && thresholds.R.length === Lr - 1 ? thresholds.R : undefined;
  const thresholdsG = thresholds.G && thresholds.G.length === Lg - 1 ? thresholds.G : undefined;
  const thresholdsB = thresholds.B && thresholds.B.length === Lb - 1 ? thresholds.B : undefined;

  const outputsR = outputs.R && outputs.R.length === Lr ? outputs.R : undefined;
  const outputsG = outputs.G && outputs.G.length === Lg ? outputs.G : undefined;
  const outputsB = outputs.B && outputs.B.length === Lb ? outputs.B : undefined;

  for (let i = 0; i < data.length; i += 4) {
    data[i] = quantize(data[i], Lr, thresholdsR, outputsR); // R
    data[i + 1] = quantize(data[i + 1], Lg, thresholdsG, outputsG); // G
    data[i + 2] = quantize(data[i + 2], Lb, thresholdsB, outputsB); // B
    // data[i + 3] = alpha channel, untouched
  }

  return imageData;
}

export function applyHsvPosterize(
  imageData: ImageData,
  levels: HSVLevels,
  thresholds: HsvThresholds = {},
  outputs: HsvOutputs = {},
): ImageData {
  const { data } = imageData;
  const Lh = Math.max(1, Math.floor(levels.H));
  const Ls = Math.max(1, Math.floor(levels.S));
  const Lv = Math.max(1, Math.floor(levels.V));

  const thresholdsH = thresholds.H && thresholds.H.length === Lh - 1 ? thresholds.H : undefined;
  const thresholdsS = thresholds.S && thresholds.S.length === Ls - 1 ? thresholds.S : undefined;
  const thresholdsV = thresholds.V && thresholds.V.length === Lv - 1 ? thresholds.V : undefined;

  const outputsH = outputs.H && outputs.H.length === Lh ? outputs.H : undefined;
  const outputsS = outputs.S && outputs.S.length === Ls ? outputs.S : undefined;
  const outputsV = outputs.V && outputs.V.length === Lv ? outputs.V : undefined;

  for (let i = 0; i < data.length; i += 4) {
    const r = data[i];
    const g = data[i + 1];
    const b = data[i + 2];

    const [h, s, v] = rgbToHsv255(r, g, b);

    const qh = quantize(h, Lh, thresholdsH, outputsH);
    const qs = quantize(s, Ls, thresholdsS, outputsS);
    const qv = quantize(v, Lv, thresholdsV, outputsV);

    const [nr, ng, nb] = hsv255ToRgb(qh, qs, qv);
    data[i] = nr;
    data[i + 1] = ng;
    data[i + 2] = nb;
  }

  return imageData;
}

export function applyAlphaPosterize(
  imageData: ImageData,
  levels: number,
  thresholds?: number[],
  outputs?: number[],
): ImageData {
  const { data } = imageData;
  const L = Math.max(1, Math.floor(levels));

  const thresholdsA = thresholds && thresholds.length === L - 1 ? thresholds : undefined;
  const outputsA = outputs && outputs.length === L ? outputs : undefined;

  for (let i = 0; i < data.length; i += 4) {
    data[i + 3] = quantize(data[i + 3], L, thresholdsA, outputsA);
  }

  return imageData;
}
