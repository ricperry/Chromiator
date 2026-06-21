import { hsv255ToRgb, rgbToHsv255 } from "./effect";

export type RGB = { r: number; g: number; b: number };
export type RGBA = RGB & { a: number };
export type ColorSpace = "rgb" | "hsv";

export type RgbSiteCoord = { r: number; g: number; b: number };
export type HsvSiteCoord = { h: number; s: number; v: number };
export type SiteCoord = RgbSiteCoord | HsvSiteCoord;

export type SourceColor = {
  key: number;
  rgba: RGBA;
  count: number;
  hsvBins?: HsvSiteCoord;
};

export type AnalyzedImage = {
  width: number;
  height: number;
  imageData: ImageData;
  sourceColors: SourceColor[];
  visiblePixelCount: number;
  transparentPixelCount: number;
};

export type Site = {
  id: string;
  groupId: string;
  colorSpace: ColorSpace;
  coord: SiteCoord;
  samplePoint?: { x: number; y: number };
  mass: number;
  order: number;
};

export type QuantizationGroup = {
  id: string;
  name: string;
  paletteColor: RGB;
  outputCoords: Partial<Record<ColorSpace, SiteCoord>>;
  outputFollowsSite: Partial<Record<ColorSpace, boolean>>;
  sites: Site[];
};

export type BezierNode = {
  id: string;
  x: number;
  y: number;
};

export type ProjectionPanelId = "rg" | "rb" | "gb" | "hs" | "hv" | "sv";

export type BezierProjection = {
  panel: ProjectionPanelId;
  nodes: BezierNode[];
  closed: boolean;
};

export type BezierRegion = {
  id: string;
  name: string;
  colorSpace: ColorSpace;
  outputColor: RGB;
  mass: number;
  priority: number;
  projections: Partial<Record<ProjectionPanelId, BezierProjection>>;
  order: number;
};

export type ColorAssignment = {
  color: SourceColor;
  siteId: string | null;
  groupId: string | null;
};

export type AssignmentStats = {
  ready: boolean;
  assignedPixels: number;
  groupPixels: Map<string, number>;
  assignments: Map<number, ColorAssignment>;
};

let nextSiteOrder = 1;
let nextRegionOrder = 1;

export function clampByte(value: number): number {
  return Math.max(0, Math.min(255, Math.round(value)));
}

export function normalizeHue(hue: number): number {
  return ((hue % 360) + 360) % 360;
}

export function rgbToHex({ r, g, b }: RGB): string {
  return `#${[r, g, b]
    .map((channel) => clampByte(channel).toString(16).padStart(2, "0"))
    .join("")}`;
}

export function hexToRgb(hex: string): RGB {
  const normalized = hex.replace("#", "").trim();
  const value = Number.parseInt(normalized, 16);
  if (!Number.isFinite(value) || normalized.length !== 6) {
    return { r: 0, g: 0, b: 0 };
  }
  return {
    r: (value >>> 16) & 255,
    g: (value >>> 8) & 255,
    b: value & 255,
  };
}

export function packRgba({ r, g, b, a }: RGBA): number {
  return (
    ((clampByte(r) << 24) |
      (clampByte(g) << 16) |
      (clampByte(b) << 8) |
      clampByte(a)) >>>
    0
  );
}

function unpackRgba(key: number): RGBA {
  return {
    r: (key >>> 24) & 255,
    g: (key >>> 16) & 255,
    b: (key >>> 8) & 255,
    a: key & 255,
  };
}

export function hsvBinsFromRgb(rgb: RGB): HsvSiteCoord {
  const [h, s, v] = rgbToHsv255(rgb.r, rgb.g, rgb.b);
  return {
    h: Math.min(359, Math.floor((h / 255) * 360)),
    s,
    v,
  };
}

export function hsvCoordToRgb(coord: HsvSiteCoord): RGB {
  const [r, g, b] = hsv255ToRgb(
    (normalizeHue(coord.h) / 360) * 255,
    coord.s,
    coord.v,
  );
  return { r, g, b };
}

export function siteCoordFromRgb(colorSpace: ColorSpace, rgb: RGB): SiteCoord {
  return colorSpace === "rgb" ? { ...rgb } : hsvBinsFromRgb(rgb);
}

export function outputCoordFromRgb(
  colorSpace: ColorSpace,
  color: RGB,
): SiteCoord {
  return siteCoordFromRgb(colorSpace, color);
}

export function defaultPaletteColorForCoord(
  colorSpace: ColorSpace,
  coord: SiteCoord,
): RGB {
  return colorSpace === "rgb"
    ? { ...(coord as RgbSiteCoord) }
    : hsvCoordToRgb(coord as HsvSiteCoord);
}

export function analyzeImageData(imageData: ImageData): AnalyzedImage {
  const counts = new Map<number, number>();
  const data = imageData.data;
  let visiblePixelCount = 0;
  let transparentPixelCount = 0;

  for (let index = 0; index < data.length; index += 4) {
    const rgba = {
      r: data[index],
      g: data[index + 1],
      b: data[index + 2],
      a: data[index + 3],
    };
    if (rgba.a === 0) {
      transparentPixelCount += 1;
    } else {
      visiblePixelCount += 1;
    }
    const key = packRgba(rgba);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }

  const sourceColors = [...counts.entries()].map(([key, count]) => {
    const rgba = unpackRgba(key);
    return {
      key,
      rgba,
      count,
      hsvBins: rgba.a === 0 ? undefined : hsvBinsFromRgb(rgba),
    };
  });

  return {
    width: imageData.width,
    height: imageData.height,
    imageData,
    sourceColors,
    visiblePixelCount,
    transparentPixelCount,
  };
}

export function createQuantizationGroup(
  name = "Group 1",
  paletteColor: RGB = { r: 0, g: 150, b: 255 },
): QuantizationGroup {
  return {
    id: crypto.randomUUID(),
    name,
    paletteColor,
    outputCoords: {},
    outputFollowsSite: {},
    sites: [],
  };
}

export function createSite(
  groupId: string,
  colorSpace: ColorSpace,
  coord: SiteCoord,
): Site {
  return {
    id: crypto.randomUUID(),
    groupId,
    colorSpace,
    coord,
    mass: 1,
    order: nextSiteOrder++,
  };
}

export function completedSites(
  groups: QuantizationGroup[],
  colorSpace: ColorSpace,
): Site[] {
  return groups
    .flatMap((group) => group.sites)
    .filter((site) => site.colorSpace === colorSpace);
}

export function computeVoronoiAssignments(
  image: AnalyzedImage,
  groups: QuantizationGroup[],
  colorSpace: ColorSpace,
): AssignmentStats {
  const assignments = new Map<number, ColorAssignment>();
  const groupPixels = new Map(groups.map((group) => [group.id, 0]));
  const sites = completedSites(groups, colorSpace);

  if (sites.length < 2) {
    return { ready: false, assignedPixels: 0, groupPixels, assignments };
  }

  let assignedPixels = 0;
  for (const color of image.sourceColors) {
    if (color.rgba.a === 0) continue;
    const site = nearestSite(color, sites, colorSpace);
    assignments.set(color.key, {
      color,
      siteId: site.id,
      groupId: site.groupId,
    });
    assignedPixels += color.count;
    groupPixels.set(
      site.groupId,
      (groupPixels.get(site.groupId) ?? 0) + color.count,
    );
  }

  return { ready: true, assignedPixels, groupPixels, assignments };
}

export function renderVoronoiImage(
  image: AnalyzedImage,
  groups: QuantizationGroup[],
  stats: AssignmentStats,
): ImageData {
  if (!stats.ready) return cloneImageData(image.imageData);
  const output = new ImageData(image.width, image.height);
  const source = image.imageData.data;
  const target = output.data;
  const groupById = new Map(groups.map((group) => [group.id, group]));

  for (let index = 0; index < source.length; index += 4) {
    const alpha = source[index + 3];
    if (alpha === 0) {
      target[index + 3] = 0;
      continue;
    }
    const key = packRgba({
      r: source[index],
      g: source[index + 1],
      b: source[index + 2],
      a: alpha,
    });
    const groupId = stats.assignments.get(key)?.groupId;
    const color = groupId
      ? (groupById.get(groupId)?.paletteColor ?? sourceRgbAt(source, index))
      : sourceRgbAt(source, index);
    target[index] = color.r;
    target[index + 1] = color.g;
    target[index + 2] = color.b;
    target[index + 3] = alpha;
  }

  return output;
}

export function createBezierRegion(
  name: string,
  colorSpace: ColorSpace,
  outputColor: RGB,
): BezierRegion {
  return {
    id: crypto.randomUUID(),
    name,
    colorSpace,
    outputColor,
    mass: 1,
    priority: 0,
    projections: {},
    order: nextRegionOrder++,
  };
}

export function createBezierNode(x: number, y: number): BezierNode {
  return { id: crypto.randomUUID(), x, y };
}

export function projectionPanelsForSpace(
  colorSpace: ColorSpace,
): ProjectionPanelId[] {
  return colorSpace === "rgb" ? ["rg", "rb", "gb"] : ["hs", "hv", "sv"];
}

export function projectionForRegion(
  region: BezierRegion,
  panel: ProjectionPanelId,
): BezierProjection {
  const existing = region.projections[panel];
  if (existing) return existing;
  const projection = { panel, nodes: [], closed: false };
  region.projections[panel] = projection;
  return projection;
}

export function projectCoordToPanelPoint(
  panel: ProjectionPanelId,
  coord: SiteCoord,
): { x: number; y: number } {
  if (panel === "rg")
    return { x: (coord as RgbSiteCoord).r, y: (coord as RgbSiteCoord).g };
  if (panel === "rb")
    return { x: (coord as RgbSiteCoord).r, y: (coord as RgbSiteCoord).b };
  if (panel === "gb")
    return { x: (coord as RgbSiteCoord).g, y: (coord as RgbSiteCoord).b };
  if (panel === "hs")
    return { x: (coord as HsvSiteCoord).h, y: (coord as HsvSiteCoord).s };
  if (panel === "hv")
    return { x: (coord as HsvSiteCoord).h, y: (coord as HsvSiteCoord).v };
  return { x: (coord as HsvSiteCoord).s, y: (coord as HsvSiteCoord).v };
}

export function validRegionProjections(
  region: BezierRegion,
): BezierProjection[] {
  return Object.values(region.projections).filter(
    (projection) => projection.closed && projection.nodes.length >= 3,
  );
}

export function computeBezierAssignments(
  image: AnalyzedImage,
  regions: BezierRegion[],
  colorSpace: ColorSpace,
): AssignmentStats {
  const activeRegions = regions.filter(
    (region) =>
      region.colorSpace === colorSpace &&
      validRegionProjections(region).length > 0,
  );
  const groupPixels = new Map(activeRegions.map((region) => [region.id, 0]));
  const assignments = new Map<number, ColorAssignment>();

  if (activeRegions.length === 0) {
    return { ready: false, assignedPixels: 0, groupPixels, assignments };
  }

  let assignedPixels = 0;
  for (const color of image.sourceColors) {
    if (color.rgba.a === 0) continue;
    const region = bestRegionForColor(color, activeRegions);
    if (!region) continue;
    assignments.set(color.key, { color, siteId: null, groupId: region.id });
    assignedPixels += color.count;
    groupPixels.set(region.id, (groupPixels.get(region.id) ?? 0) + color.count);
  }

  return { ready: true, assignedPixels, groupPixels, assignments };
}

export function renderBezierImage(
  image: AnalyzedImage,
  regions: BezierRegion[],
  stats: AssignmentStats,
): ImageData {
  if (!stats.ready) return cloneImageData(image.imageData);
  const output = new ImageData(image.width, image.height);
  const source = image.imageData.data;
  const target = output.data;
  const regionById = new Map(regions.map((region) => [region.id, region]));

  for (let index = 0; index < source.length; index += 4) {
    const alpha = source[index + 3];
    if (alpha === 0) {
      target[index + 3] = 0;
      continue;
    }
    const key = packRgba({
      r: source[index],
      g: source[index + 1],
      b: source[index + 2],
      a: alpha,
    });
    const regionId = stats.assignments.get(key)?.groupId;
    const color = regionId
      ? (regionById.get(regionId)?.outputColor ?? unassignedColor())
      : unassignedColor();
    target[index] = color.r;
    target[index + 1] = color.g;
    target[index + 2] = color.b;
    target[index + 3] = alpha;
  }

  return output;
}

function nearestSite(
  color: SourceColor,
  sites: Site[],
  colorSpace: ColorSpace,
): Site {
  let bestSite = sites[0];
  let bestDistance = influenceScore(color, bestSite, colorSpace);

  for (const site of sites.slice(1)) {
    const distance = influenceScore(color, site, colorSpace);
    if (
      distance < bestDistance ||
      (distance === bestDistance && site.order < bestSite.order)
    ) {
      bestSite = site;
      bestDistance = distance;
    }
  }
  return bestSite;
}

function influenceScore(
  color: SourceColor,
  site: Site,
  colorSpace: ColorSpace,
): number {
  return (
    distanceToSite(color, site, colorSpace) - normalizedMass(site.mass) * 0.055
  );
}

function distanceToSite(
  color: SourceColor,
  site: Site,
  colorSpace: ColorSpace,
): number {
  if (colorSpace === "rgb") {
    const coord = site.coord as RgbSiteCoord;
    const dr = (color.rgba.r - coord.r) / 255;
    const dg = (color.rgba.g - coord.g) / 255;
    const db = (color.rgba.b - coord.b) / 255;
    return dr * dr + dg * dg + db * db;
  }

  if (!color.hsvBins) return Number.POSITIVE_INFINITY;
  const source = hsvCylinderPoint(color.hsvBins);
  const target = hsvCylinderPoint(site.coord as HsvSiteCoord);
  const dx = source.x - target.x;
  const dy = source.y - target.y;
  const dz = source.z - target.z;
  return dx * dx + dy * dy + dz * dz;
}

function normalizedMass(mass: number): number {
  return Math.max(-10, Math.min(10, mass));
}

function hsvCylinderPoint({ h, s, v }: HsvSiteCoord): {
  x: number;
  y: number;
  z: number;
} {
  const radians = (normalizeHue(h) / 180) * Math.PI;
  const radius = s / 255;
  return {
    x: radius * Math.cos(radians),
    y: radius * Math.sin(radians),
    z: v / 255,
  };
}

function bestRegionForColor(
  color: SourceColor,
  regions: BezierRegion[],
): BezierRegion | null {
  let best: BezierRegion | null = null;
  for (const region of regions) {
    if (!regionContainsColor(region, color)) continue;
    if (
      !best ||
      region.mass > best.mass ||
      (region.mass === best.mass && region.priority > best.priority) ||
      (region.mass === best.mass &&
        region.priority === best.priority &&
        region.order < best.order)
    ) {
      best = region;
    }
  }
  return best;
}

function regionContainsColor(
  region: BezierRegion,
  color: SourceColor,
): boolean {
  const projections = validRegionProjections(region);
  return (
    projections.length > 0 &&
    projections.every((projection) =>
      projectionContainsColor(projection, color),
    )
  );
}

function projectionContainsColor(
  projection: BezierProjection,
  color: SourceColor,
): boolean {
  const point = sourceColorProjectionPoint(projection.panel, color);
  if (!point) return false;
  const huePanel = projection.panel === "hs" || projection.panel === "hv";
  const nodes = huePanel
    ? projection.nodes.map((node) => ({
        ...node,
        x: node.x + Math.round((point.x - node.x) / 360) * 360,
      }))
    : projection.nodes;
  return pointInPolygon(point, nodes);
}

function sourceColorProjectionPoint(
  panel: ProjectionPanelId,
  color: SourceColor,
): { x: number; y: number } | null {
  if (panel === "rg") return { x: color.rgba.r, y: color.rgba.g };
  if (panel === "rb") return { x: color.rgba.r, y: color.rgba.b };
  if (panel === "gb") return { x: color.rgba.g, y: color.rgba.b };
  if (!color.hsvBins) return null;
  if (panel === "hs") return { x: color.hsvBins.h, y: color.hsvBins.s };
  if (panel === "hv") return { x: color.hsvBins.h, y: color.hsvBins.v };
  return { x: color.hsvBins.s, y: color.hsvBins.v };
}

function pointInPolygon(
  point: { x: number; y: number },
  nodes: Array<{ x: number; y: number }>,
): boolean {
  let inside = false;
  for (
    let index = 0, previous = nodes.length - 1;
    index < nodes.length;
    previous = index, index += 1
  ) {
    const a = nodes[index];
    const b = nodes[previous];
    const crosses = a.y > point.y !== b.y > point.y;
    if (crosses) {
      const xAtY = ((b.x - a.x) * (point.y - a.y)) / (b.y - a.y || 1) + a.x;
      if (point.x < xAtY) inside = !inside;
    }
  }
  return inside;
}

function cloneImageData(imageData: ImageData): ImageData {
  return new ImageData(
    new Uint8ClampedArray(imageData.data),
    imageData.width,
    imageData.height,
  );
}

function sourceRgbAt(source: Uint8ClampedArray, index: number): RGB {
  return { r: source[index], g: source[index + 1], b: source[index + 2] };
}

function unassignedColor(): RGB {
  return { r: 42, g: 48, b: 66 };
}
