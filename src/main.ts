import {
  applyAlphaPosterize,
  applyBasicPosterize,
  applyHsvPosterize,
  rgbToHsv255,
} from "./effect";
import { PRESETS } from "./presets";
import { drawImageToCanvas } from "./util";

const sourceCanvas = document.getElementById(
  "originalCanvas",
) as HTMLCanvasElement | null;
const sourceCtx = sourceCanvas?.getContext("2d") ?? null;
const outputCanvas = document.getElementById(
  "processedCanvas",
) as HTMLCanvasElement | null;
const outputCtx = outputCanvas?.getContext("2d") ?? null;
const comparisonStage = document.getElementById(
  "comparisonStage",
) as HTMLElement | null;
const comparisonSlider = document.getElementById(
  "comparisonSlider",
) as HTMLInputElement | null;
const fileInput = document.getElementById(
  "fileInput",
) as HTMLInputElement | null;
const fileInputBtn = document.getElementById(
  "fileInputBtn",
) as HTMLButtonElement | null;
const selectedFileName = document.getElementById(
  "selectedFileName",
) as HTMLDivElement | null;
const resetBtn = document.getElementById(
  "resetBtn",
) as HTMLButtonElement | null;
// Histogram bars canvases
const histogramCanvasRed = document.getElementById(
  "histogramCanvasRed",
) as HTMLCanvasElement | null;
const histogramCtxRed = histogramCanvasRed?.getContext("2d") ?? null;
const histogramCanvasGreen = document.getElementById(
  "histogramCanvasGreen",
) as HTMLCanvasElement | null;
const histogramCtxGreen = histogramCanvasGreen?.getContext("2d") ?? null;
const histogramCanvasBlue = document.getElementById(
  "histogramCanvasBlue",
) as HTMLCanvasElement | null;
const histogramCtxBlue = histogramCanvasBlue?.getContext("2d") ?? null;
const histogramCanvasAlpha = document.getElementById(
  "histogramCanvasAlpha",
) as HTMLCanvasElement | null;
const histogramCtxAlpha = histogramCanvasAlpha?.getContext("2d") ?? null;

// Control canvases (for thresholds and levels)
const controlsCanvasRed = document.getElementById(
  "histogramControlsRed",
) as HTMLCanvasElement | null;
const controlsCtxRed = controlsCanvasRed?.getContext("2d") ?? null;
const controlsCanvasGreen = document.getElementById(
  "histogramControlsGreen",
) as HTMLCanvasElement | null;
const controlsCtxGreen = controlsCanvasGreen?.getContext("2d") ?? null;
const controlsCanvasBlue = document.getElementById(
  "histogramControlsBlue",
) as HTMLCanvasElement | null;
const controlsCtxBlue = controlsCanvasBlue?.getContext("2d") ?? null;
const controlsCanvasAlpha = document.getElementById(
  "histogramControlsAlpha",
) as HTMLCanvasElement | null;
const controlsCtxAlpha = controlsCanvasAlpha?.getContext("2d") ?? null;

export type Channel = "red" | "green" | "blue";
type UiChannel = Channel | "alpha";
const COLOR_CHANNELS: Channel[] = ["red", "green", "blue"];

const channelTitles: Record<UiChannel, HTMLElement | null> = {
  red: document.querySelector('.channel-title[data-channel="red"]'),
  green: document.querySelector('.channel-title[data-channel="green"]'),
  blue: document.querySelector('.channel-title[data-channel="blue"]'),
  alpha: document.querySelector('.channel-title[data-channel="alpha"]'),
};

export type ColorSpace = "rgb" | "hsv";
const COLOR_SPACE_STORAGE_KEY = "super-posterize-color-space";
const ALPHA_STORAGE_KEY = "super-posterize-posterize-alpha";

export type ChannelSettings = {
  levels: number;
  thresholds: number[];
  outputs: number[];
};

export type ExportSettings = {
  version: number;
  colorSpace?: ColorSpace;
  lockChannels: boolean;
  posterizeAlpha?: boolean;
  alpha?: ChannelSettings;
  channels: Record<Channel, ChannelSettings>;
};

const levelLabels: Record<UiChannel, HTMLElement | null> = {
  red: document.getElementById("redLevelsValue"),
  green: document.getElementById("greenLevelsValue"),
  blue: document.getElementById("blueLevelsValue"),
  alpha: document.getElementById("alphaLevelsValue"),
};

const bandControlContainers: Record<UiChannel, HTMLElement | null> = {
  red: document.querySelector('.band-levels[data-channel="red"]'),
  green: document.querySelector('.band-levels[data-channel="green"]'),
  blue: document.querySelector('.band-levels[data-channel="blue"]'),
  alpha: document.querySelector('.band-levels[data-channel="alpha"]'),
};

const lockToggle = document.getElementById(
  "lockChannelsToggle",
) as HTMLInputElement | null;
const colorSpaceSelect = document.getElementById(
  "colorSpaceSelect",
) as HTMLSelectElement | null;
const posterizeAlphaToggle = document.getElementById(
  "posterizeAlphaToggle",
) as HTMLInputElement | null;
const saveImageBtn = document.getElementById(
  "saveImageBtn",
) as HTMLButtonElement | null;
const exportSettingsBtn = document.getElementById(
  "exportSettingsBtn",
) as HTMLButtonElement | null;
const importSettingsBtn = document.getElementById(
  "importSettingsBtn",
) as HTMLButtonElement | null;
const settingsFileInput = document.getElementById(
  "settingsFileInput",
) as HTMLInputElement | null;
const presetsSelect = document.getElementById(
  "presetsSelect",
) as HTMLSelectElement | null;
const presetDescription = document.querySelector(
  ".preset-description",
) as HTMLElement | null;
let lockChannelsEnabled = false;
let activeColorSpace: ColorSpace = "rgb";
let posterizeAlphaEnabled = false;

function channelLabelFor(channel: UiChannel, colorSpace: ColorSpace): string {
  if (channel === "alpha") return "Alpha Channel";
  if (colorSpace === "hsv") {
    return (
      {
        red: "Hue Channel",
        green: "Saturation Channel",
        blue: "Value Channel",
      } as const
    )[channel as Channel];
  }
  return (
    {
      red: "Red Channel",
      green: "Green Channel",
      blue: "Blue Channel",
    } as const
  )[channel as Channel];
}

function syncedChannelLabel(colorSpace: ColorSpace): string {
  return colorSpace === "hsv" ? "HSV" : "RGB";
}

function applyChannelTitles() {
  if (lockChannelsEnabled) {
    channelTitles.red?.classList.add("synced");
    if (channelTitles.red)
      channelTitles.red.textContent = syncedChannelLabel(activeColorSpace);
  } else {
    channelTitles.red?.classList.remove("synced");
    if (channelTitles.red)
      channelTitles.red.textContent = channelLabelFor("red", activeColorSpace);
  }

  if (channelTitles.green)
    channelTitles.green.textContent = channelLabelFor("green", activeColorSpace);
  if (channelTitles.blue)
    channelTitles.blue.textContent = channelLabelFor("blue", activeColorSpace);
  if (channelTitles.alpha)
    channelTitles.alpha.textContent = channelLabelFor("alpha", activeColorSpace);
}

function setColorSpace(
  next: ColorSpace,
  options: { saveToStorage?: boolean; rerender?: boolean } = {},
) {
  if (activeColorSpace === next) return;
  const saveToStorage = options.saveToStorage ?? true;
  const rerender = options.rerender ?? true;
  activeColorSpace = next;
  histogramCache = null;
  if (colorSpaceSelect) colorSpaceSelect.value = next;
  applyChannelTitles();
  if (saveToStorage) {
    localStorage.setItem(COLOR_SPACE_STORAGE_KEY, next);
  }
  if (rerender) {
    updateAllHistograms();
    renderAllBandControls();
    renderPosterized();
  }
}

if (comparisonStage && comparisonSlider) {
  const setReveal = (value: number) => {
    const clampedValue = Math.min(100, Math.max(0, value));
    comparisonStage.style.setProperty("--preview-reveal", `${clampedValue}%`);
  };

  comparisonSlider.addEventListener("input", (event) => {
    const newValue = Number((event.target as HTMLInputElement).value);
    setReveal(newValue);
  });

  setReveal(Number(comparisonSlider.value));
}

const DEFAULT_IMAGE = new URL("../test.png", import.meta.url).href;
const MIN_LEVELS = 2;
const MAX_LEVELS = 32;
const HIT_TARGET_PX = 6;

// Combined presets object that will include both built-in and PNG presets
interface PresetEntry {
  name: string;
  description: string;
  settings: ExportSettings;
}

const allPresets: Record<string, PresetEntry> = {};

// Preset Loading Functions
async function loadSinglePngPreset(fileName: string): Promise<boolean> {
  try {
    const response = await fetch(`/presets/${fileName}`);
    if (response.ok) {
      const arrayBuffer = await response.arrayBuffer();
      const uint8Array = new Uint8Array(arrayBuffer);

      // Check if this is a valid PNG with metadata
      try {
        const metadata = extractMetadataFromPng(uint8Array, "SuperPosterize");

        if (metadata) {
          const preset = JSON.parse(metadata) as ExportSettings;
          // Use filename without extension as preset key
          const presetKey = `png_${fileName.replace(".png", "").toLowerCase()}`;
          const presetName = fileName.replace(".png", "").replace(/_/g, " ");

          allPresets[presetKey] = {
            name: `📁 ${presetName}`,
            description: `PNG preset: ${presetName}`,
            settings: preset,
          };

          console.log(`✅ Loaded PNG preset: ${presetName} (${fileName})`);
          return true;
        } else {
          console.log(`⚠️ No SuperPosterize metadata found in ${fileName}`);
          return false;
        }
      } catch (pngError) {
        console.log(`❌ Error parsing PNG metadata in ${fileName}:`, pngError);
        return false;
      }
    } else {
      console.log(`❌ Failed to fetch ${fileName}: ${response.status}`);
      return false;
    }
  } catch (error) {
    console.log(`❌ Error loading ${fileName}:`, error);
    return false;
  }
}

async function initializePresets() {
  if (!presetsSelect || !presetDescription) return;

  // Load presets from manifest
  await loadPresetsFromManifest();

  // Populate dropdown with all presets
  Object.entries(allPresets).forEach(([key, preset]) => {
    const option = document.createElement("option");
    option.value = key;
    option.textContent = preset.name;
    presetsSelect.appendChild(option);
  });

  // Handle preset selection
  presetsSelect.addEventListener("change", () => {
    const selectedKey = presetsSelect.value;
    if (!selectedKey) {
      presetDescription.textContent = "";
      return;
    }

    const preset = allPresets[selectedKey];
    if (!preset) return;

    // Update description
    presetDescription.textContent = preset.description;

    // Apply preset settings
    applySettings(preset.settings);
  });
}

async function loadPresetsFromManifest() {
  try {
    // Load the manifest to get list of preset files
    const manifestResponse = await fetch("/presets/manifest.json");
    if (!manifestResponse.ok) {
      console.log(
        "No presets manifest found, falling back to built-in presets",
      );
      // Fallback to built-in presets if no manifest
      Object.entries(PRESETS).forEach(([key, preset]) => {
        allPresets[key] = preset;
      });
      return;
    }

    const manifest = await manifestResponse.json();
    const jsonFiles = manifest.jsonPresets || [];
    const pngFiles = manifest.pngPresets || [];

    console.log(
      `📁 Found ${jsonFiles.length} JSON presets and ${pngFiles.length} PNG presets in manifest`,
    );

    let loadedCount = 0;

    // Load JSON presets
    for (const fileName of jsonFiles) {
      const loaded = await loadJsonPreset(fileName);
      if (loaded) loadedCount++;
    }

    // Load PNG presets
    for (const fileName of pngFiles) {
      const loaded = await loadSinglePngPreset(fileName);
      if (loaded) loadedCount++;
    }

    console.log(`✅ Successfully loaded ${loadedCount} presets total`);
  } catch (error) {
    console.log("Error loading presets manifest:", error);
    // Fallback to built-in presets on error
    Object.entries(PRESETS).forEach(([key, preset]) => {
      allPresets[key] = preset;
    });
  }
}

async function loadJsonPreset(fileName: string): Promise<boolean> {
  try {
    const response = await fetch(`/presets/${fileName}`);
    if (response.ok) {
      const presetData = await response.json();

      // Use filename without extension as preset key
      const presetKey = fileName.replace(".json", "").toLowerCase();
      const presetName =
        presetData.name || fileName.replace(".json", "").replace(/_/g, " ");

      allPresets[presetKey] = {
        name: presetName,
        description: presetData.description || `JSON preset: ${presetName}`,
        settings: presetData.settings || presetData, // Handle both old and new format
      };

      console.log(`✅ Loaded JSON preset: ${presetName} (${fileName})`);
      return true;
    } else {
      console.log(`❌ Failed to fetch ${fileName}: ${response.status}`);
      return false;
    }
  } catch (error) {
    console.log(`❌ Error loading ${fileName}:`, error);
    return false;
  }
}

// Initialize presets dropdown
initializePresets();

const DEFAULT_LEVELS: Record<UiChannel, number> = {
  red: 8,
  green: 8,
  blue: 8,
  alpha: 8,
};

const channelLevels: Record<UiChannel, number> = { ...DEFAULT_LEVELS };

const channelThresholds: Record<UiChannel, number[]> = {
  red: [],
  green: [],
  blue: [],
  alpha: [],
};

const channelOutputs: Record<UiChannel, number[]> = {
  red: [],
  green: [],
  blue: [],
  alpha: [],
};

let originalImageData: ImageData | null = null;
let originalImageRevision = 0;

type HistogramCache = {
  revision: number;
  colorSpace: ColorSpace;
  perChannel: Record<Channel, Uint32Array>;
  combined: Uint32Array;
  alpha: Uint32Array;
};

let histogramCache: HistogramCache | null = null;

function getHistogramCache(): HistogramCache | null {
  if (!originalImageData) return null;
  if (
    histogramCache &&
    histogramCache.revision === originalImageRevision &&
    histogramCache.colorSpace === activeColorSpace
  ) {
    return histogramCache;
  }

  const perChannel: Record<Channel, Uint32Array> = {
    red: new Uint32Array(256),
    green: new Uint32Array(256),
    blue: new Uint32Array(256),
  };
  const combined = new Uint32Array(256);
  const alpha = new Uint32Array(256);
  const data = originalImageData.data;

  if (activeColorSpace === "hsv") {
    for (let i = 0; i < data.length; i += 4) {
      const [h, s, v] = rgbToHsv255(data[i], data[i + 1], data[i + 2]);
      perChannel.red[h] += 1;
      perChannel.green[s] += 1;
      perChannel.blue[v] += 1;
      combined[h] += 1;
      combined[s] += 1;
      combined[v] += 1;
      alpha[data[i + 3]] += 1;
    }
  } else {
    for (let i = 0; i < data.length; i += 4) {
      const r = data[i];
      const g = data[i + 1];
      const b = data[i + 2];
      perChannel.red[r] += 1;
      perChannel.green[g] += 1;
      perChannel.blue[b] += 1;
      combined[r] += 1;
      combined[g] += 1;
      combined[b] += 1;
      alpha[data[i + 3]] += 1;
    }
  }

  histogramCache = {
    revision: originalImageRevision,
    colorSpace: activeColorSpace,
    perChannel,
    combined,
    alpha,
  };
  return histogramCache;
}

function createEvenThresholds(levels: number): number[] {
  if (levels <= 1) return [];
  const thresholds: number[] = [];
  for (let i = 1; i < levels; i += 1) {
    const raw = Math.floor((i * 256) / levels);
    const previous = thresholds[i - 1] ?? 0;
    const clamped = Math.min(254, Math.max(1, raw));
    thresholds.push(clamped <= previous ? previous + 1 : clamped);
  }
  return thresholds;
}

function createLinearOutputs(levels: number): number[] {
  if (levels <= 0) return [];
  const outputs: number[] = [];
  const step = 255 / levels;
  for (let i = 0; i < levels; i += 1) {
    const center = (i + 0.5) * step;
    outputs.push(Math.max(0, Math.min(255, Math.round(center))));
  }
  return outputs;
}

function normalizeChannelState(channel: UiChannel) {
  const level = channelLevels[channel];
  if (channelThresholds[channel].length !== Math.max(0, level - 1)) {
    channelThresholds[channel] = createEvenThresholds(level);
  }
  if (channelOutputs[channel].length !== level) {
    channelOutputs[channel] = createLinearOutputs(level);
  }
}

function syncOtherChannelsFrom(source: Channel) {
  normalizeChannelState(source);
  const level = channelLevels[source];
  const thresholds = [...channelThresholds[source]];
  const outputs = [...channelOutputs[source]];

  COLOR_CHANNELS.forEach((channel) => {
    if (channel === source) return;
    channelLevels[channel] = level;
    channelThresholds[channel] = [...thresholds];
    channelOutputs[channel] = [...outputs];
    syncChannelLabel(channel, level);
  });
}

function synchronizeChannels(base: Channel) {
  syncOtherChannelsFrom(base);
}

function applyLockStateToUI() {
  document.querySelectorAll<HTMLElement>(".histogram").forEach((panel) => {
    const channel = panel.dataset.channel as UiChannel | undefined;
    if (!channel) return;
    const hide =
      channel === "alpha"
        ? !posterizeAlphaEnabled
        : lockChannelsEnabled && channel !== "red";
    panel.classList.toggle("hidden", hide);
  });

  applyChannelTitles();
}

function toggleLockChannels(enabled: boolean) {
  lockChannelsEnabled = enabled;
  if (lockToggle && lockToggle.checked !== enabled) {
    lockToggle.checked = enabled;
  }
  if (lockChannelsEnabled) {
    synchronizeChannels("red");
  }
  applyLockStateToUI();
  renderAllBandControls();
  updateAllHistograms();
  renderPosterized();
}

function togglePosterizeAlpha(
  enabled: boolean,
  options: { saveToStorage?: boolean; rerender?: boolean } = {},
) {
  posterizeAlphaEnabled = enabled;
  if (posterizeAlphaToggle && posterizeAlphaToggle.checked !== enabled) {
    posterizeAlphaToggle.checked = enabled;
  }
  if (options.saveToStorage ?? true) {
    localStorage.setItem(ALPHA_STORAGE_KEY, enabled ? "true" : "false");
  }

  if (posterizeAlphaEnabled) {
    normalizeChannelState("alpha");
    syncChannelLabel("alpha", channelLevels.alpha);
  }

  applyLockStateToUI();
  if (options.rerender ?? true) {
    renderAllBandControls();
    updateAllHistograms();
    renderPosterized();
  }
}

function sanitizeThresholds(values: unknown, level: number): number[] {
  const fallback = createEvenThresholds(level);
  if (!Array.isArray(values)) return fallback;
  const result: number[] = [];
  let previous = 0;
  for (let i = 0; i < level - 1; i += 1) {
    const sourceValue = values[i];
    const defaultValue = fallback[i] ?? previous + 1;
    const raw = Math.round(
      typeof sourceValue === "number" ? sourceValue : defaultValue,
    );
    const clamped = Math.max(previous + 1, Math.min(254, raw));
    result.push(clamped);
    previous = clamped;
  }
  return result.length === level - 1 ? result : fallback;
}

function sanitizeOutputs(values: unknown, level: number): number[] {
  if (!Array.isArray(values) || values.length !== level) {
    return createLinearOutputs(level);
  }
  return values
    .slice(0, level)
    .map((value) => Math.max(0, Math.min(255, Math.round(Number(value)))));
}

function collectChannelSettings(channel: UiChannel): ChannelSettings {
  normalizeChannelState(channel);
  return {
    levels: channelLevels[channel],
    thresholds: [...channelThresholds[channel]],
    outputs: [...channelOutputs[channel]],
  };
}

function buildSettingsPayload(): ExportSettings {
  const channels: Record<Channel, ChannelSettings> = {
    red: collectChannelSettings("red"),
    green: collectChannelSettings("green"),
    blue: collectChannelSettings("blue"),
  };
  const payload: ExportSettings = {
    version: 3,
    colorSpace: activeColorSpace,
    lockChannels: lockChannelsEnabled,
    posterizeAlpha: posterizeAlphaEnabled,
    channels,
  };
  if (posterizeAlphaEnabled) {
    payload.alpha = collectChannelSettings("alpha");
  }
  return payload;
}

// PNG Metadata Functions
function findChunk(pngData: Uint8Array, chunkType: string): number {
  let pos = 8; // Skip PNG signature

  while (pos < pngData.length - 8) {
    const chunkLength =
      (pngData[pos] << 24) |
      (pngData[pos + 1] << 16) |
      (pngData[pos + 2] << 8) |
      pngData[pos + 3];
    const currentChunkType = String.fromCharCode(
      pngData[pos + 4],
      pngData[pos + 5],
      pngData[pos + 6],
      pngData[pos + 7],
    );

    if (currentChunkType === chunkType) {
      return pos;
    }

    pos += 8 + chunkLength + 4; // Skip chunk header + data + CRC
  }

  return -1;
}

function embedMetadataInPng(
  pngData: Uint8Array,
  keyword: string,
  text: string,
): Uint8Array {
  // Find IEND chunk
  const iendPosition = findChunk(pngData, "IEND");
  if (iendPosition === -1) {
    throw new Error("Invalid PNG: IEND chunk not found");
  }

  const keywordBytes = new TextEncoder().encode(keyword);
  const textBytes = new TextEncoder().encode(text);
  const chunkData = new Uint8Array(keywordBytes.length + 1 + textBytes.length);

  // tEXt format: keyword + null separator + text
  chunkData.set(keywordBytes, 0);
  chunkData[keywordBytes.length] = 0; // null separator
  chunkData.set(textBytes, keywordBytes.length + 1);

  const chunkLength = chunkData.length;
  const chunkType = new TextEncoder().encode("tEXt");

  // Calculate CRC for chunk type + data
  const crcData = new Uint8Array(4 + chunkData.length);
  crcData.set(chunkType, 0);
  crcData.set(chunkData, 4);
  const crc = calculateCRC32(crcData);

  // Create new PNG with embedded metadata
  const newPngSize = pngData.length + 4 + 4 + chunkData.length + 4;
  const newPngData = new Uint8Array(newPngSize);

  // Copy everything before IEND
  newPngData.set(pngData.slice(0, iendPosition), 0);

  // Insert our tEXt chunk
  let insertPos = iendPosition;

  // Write chunk length (big-endian)
  newPngData[insertPos++] = (chunkLength >>> 24) & 0xff;
  newPngData[insertPos++] = (chunkLength >>> 16) & 0xff;
  newPngData[insertPos++] = (chunkLength >>> 8) & 0xff;
  newPngData[insertPos++] = chunkLength & 0xff;

  // Write chunk type
  newPngData.set(chunkType, insertPos);
  insertPos += 4;

  // Write chunk data
  newPngData.set(chunkData, insertPos);
  insertPos += chunkData.length;

  // Write CRC (big-endian)
  newPngData[insertPos++] = (crc >>> 24) & 0xff;
  newPngData[insertPos++] = (crc >>> 16) & 0xff;
  newPngData[insertPos++] = (crc >>> 8) & 0xff;
  newPngData[insertPos++] = crc & 0xff;

  // Copy IEND chunk and everything after
  newPngData.set(pngData.slice(iendPosition), insertPos);

  return newPngData;
}

function extractMetadataFromPng(
  pngData: Uint8Array,
  keyword: string,
): string | null {
  let pos = 8; // Skip PNG signature

  while (pos < pngData.length - 8) {
    const chunkLength =
      (pngData[pos] << 24) |
      (pngData[pos + 1] << 16) |
      (pngData[pos + 2] << 8) |
      pngData[pos + 3];
    const chunkType = String.fromCharCode(
      pngData[pos + 4],
      pngData[pos + 5],
      pngData[pos + 6],
      pngData[pos + 7],
    );

    if (chunkType === "tEXt") {
      const chunkDataStart = pos + 8;
      const chunkDataEnd = chunkDataStart + chunkLength;
      const chunkData = pngData.slice(chunkDataStart, chunkDataEnd);

      // Find null separator
      let nullPos = -1;
      for (let i = 0; i < chunkData.length; i++) {
        if (chunkData[i] === 0) {
          nullPos = i;
          break;
        }
      }

      if (nullPos !== -1) {
        const chunkKeyword = new TextDecoder().decode(
          chunkData.slice(0, nullPos),
        );
        if (chunkKeyword === keyword) {
          const text = new TextDecoder().decode(chunkData.slice(nullPos + 1));
          return text;
        }
      }
    }

    pos += 8 + chunkLength + 4; // Skip chunk header + data + CRC
  }

  return null;
}

function calculateCRC32(data: Uint8Array): number {
  const crcTable = new Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let j = 0; j < 8; j++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    crcTable[i] = c;
  }

  let crc = 0xffffffff;
  for (let i = 0; i < data.length; i++) {
    crc = crcTable[(crc ^ data[i]) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

async function saveProcessedImage() {
  if (!outputCanvas) {
    alert("No processed image to save!");
    return;
  }

  const fileName = prompt(
    "Enter a filename for the image:",
    "posterized_image",
  );
  if (fileName === null) return;

  const sanitizedFileName = fileName.trim() || "posterized_image";

  try {
    const settings = buildSettingsPayload();
    const settingsJson = JSON.stringify(settings);

    // Get PNG data from canvas
    const originalBlob = await new Promise<Blob>((resolve, reject) => {
      outputCanvas.toBlob(
        (blob) => {
          if (blob) {
            resolve(blob);
          } else {
            reject(new Error("Failed to create PNG blob from canvas"));
          }
        },
        "image/png",
        1.0,
      );
    });

    const originalArrayBuffer = await originalBlob.arrayBuffer();
    const uint8Array = new Uint8Array(originalArrayBuffer);

    // Insert tEXt chunk with metadata
    const modifiedPngData = embedMetadataInPng(
      uint8Array,
      "SuperPosterize",
      settingsJson,
    );

    // Create download
    const downloadBlob = new Blob([new Uint8Array(modifiedPngData)], {
      type: "image/png",
    });
    const url = URL.createObjectURL(downloadBlob);
    const link = document.createElement("a");
    link.href = url;

    const cleanFileName = sanitizedFileName
      .replace(/[^a-zA-Z0-9\s-_]/g, "")
      .replace(/\s+/g, "_")
      .substring(0, 50);

    link.download = `${cleanFileName}.png`;
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    URL.revokeObjectURL(url);

    console.log("Successfully saved PNG with embedded metadata");
  } catch (error) {
    const errorMessage =
      error instanceof Error ? error.message : "Unknown error";
    console.error("Error saving image:", error);
    alert(`Failed to save image with metadata: ${errorMessage}`);
  }
}

async function handleSettingsFile(file: File) {
  try {
    if (
      file.type === "application/json" ||
      file.name.toLowerCase().endsWith(".json")
    ) {
      // Handle JSON file
      const text = await file.text();
      const parsed = JSON.parse(text) as ExportSettings;
      if (!parsed || typeof parsed !== "object" || !parsed.channels) {
        throw new Error("Invalid settings file");
      }
      applySettings(parsed);
    } else if (
      file.type === "image/png" ||
      file.name.toLowerCase().endsWith(".png")
    ) {
      // Handle PNG file with embedded metadata
      const arrayBuffer = await file.arrayBuffer();
      const uint8Array = new Uint8Array(arrayBuffer);

      const metadata = extractMetadataFromPng(uint8Array, "SuperPosterize");
      if (!metadata) {
        throw new Error("No SuperPosterize settings found in this PNG file");
      }

      const parsed = JSON.parse(metadata) as ExportSettings;
      if (!parsed || typeof parsed !== "object" || !parsed.channels) {
        throw new Error("Invalid settings data in PNG file");
      }

      applySettings(parsed);
      console.log("Successfully loaded settings from PNG file");
    } else {
      throw new Error("Please select a .json or .png file");
    }
  } catch (error) {
    const errorMessage =
      error instanceof Error ? error.message : "Unknown error";
    console.error("Error loading settings:", error);
    alert(`Failed to load settings: ${errorMessage}`);
  } finally {
    if (settingsFileInput) settingsFileInput.value = "";
  }
}

function downloadSettings() {
  const payload = buildSettingsPayload();
  const blob = new Blob([JSON.stringify(payload, null, 2)], {
    type: "application/json",
  });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = "SuperPosterSettings.json";
  document.body.appendChild(link);
  link.click();
  document.body.removeChild(link);
  URL.revokeObjectURL(url);
}

function applySettings(settings: ExportSettings) {
  if (settings.colorSpace === "hsv" || settings.colorSpace === "rgb") {
    setColorSpace(settings.colorSpace, { saveToStorage: false, rerender: false });
  } else {
    setColorSpace("rgb", { saveToStorage: false, rerender: false });
  }

  toggleLockChannels(false);

  COLOR_CHANNELS.forEach((channel) => {
    const channelConfig = settings.channels[channel];
    if (!channelConfig) return;
    const level = Math.max(
      MIN_LEVELS,
      Math.min(MAX_LEVELS, Math.floor(channelConfig.levels)),
    );
    channelLevels[channel] = level;
    channelThresholds[channel] = sanitizeThresholds(
      channelConfig.thresholds,
      level,
    );
    channelOutputs[channel] = sanitizeOutputs(channelConfig.outputs, level);
    syncChannelLabel(channel, level);
  });

  const alphaEnabled =
    settings.posterizeAlpha === true || typeof settings.alpha === "object";
  if (alphaEnabled) {
    const alphaConfig = settings.alpha;
    const level = Math.max(
      MIN_LEVELS,
      Math.min(MAX_LEVELS, Math.floor(alphaConfig?.levels ?? channelLevels.alpha)),
    );
    channelLevels.alpha = level;
    channelThresholds.alpha = sanitizeThresholds(alphaConfig?.thresholds, level);
    channelOutputs.alpha = sanitizeOutputs(alphaConfig?.outputs, level);
    syncChannelLabel("alpha", level);
  }

  togglePosterizeAlpha(alphaEnabled, { saveToStorage: false, rerender: false });
  toggleLockChannels(settings.lockChannels ?? false);
}

type DragState = {
  channel: UiChannel;
  index: number;
  pointerId: number;
  type: "threshold" | "level";
} | null;
let dragState: DragState = null;

type BandBounds = { start: number; end: number };

/**
 * Build an array of band bounds for the channel based on thresholds. When a
 * channel lacks custom thresholds we revert to the evenly spaced defaults so
 * downstream logic always receives a complete set of ranges.
 */
function getBandBounds(channel: UiChannel): BandBounds[] {
  const levels = channelLevels[channel];
  const thresholds = channelThresholds[channel];
  const effective =
    thresholds.length === levels - 1
      ? thresholds
      : createEvenThresholds(levels);

  const bounds: BandBounds[] = [];
  let lower = 0;
  for (let i = 0; i < effective.length; i += 1) {
    const upper = effective[i];
    bounds.push({ start: lower, end: upper });
    lower = upper;
  }
  bounds.push({ start: lower, end: 255 });
  return bounds;
}

/**
 * Ensure the channel has one output value per band and clamp entries to the
 * legal 0–255 range. We preserve existing values so user edits survive
 * threshold or level adjustments.
 */
function ensureChannelOutputs(channel: UiChannel) {
  const bounds = getBandBounds(channel);
  const existing = channelOutputs[channel];
  channelOutputs[channel] = bounds.map((band, index) => {
    const previous = existing[index];
    const fallback = Math.round((band.start + band.end) / 2);
    const value = typeof previous === "number" ? previous : fallback;
    return Math.max(0, Math.min(255, Math.round(value)));
  });
}

/**
 * Persist a band output adjustment and update the preview. The sliders already
 * clamp values, however we defensively clamp again for programmatic updates.
 */
function setBandOutput(channel: UiChannel, index: number, value: number) {
  const outputs = channelOutputs[channel];
  if (!outputs || index < 0 || index >= outputs.length) return;
  const clamped = Math.max(0, Math.min(255, Math.round(value)));
  outputs[index] = clamped;
  if (lockChannelsEnabled && channel !== "alpha") {
    syncOtherChannelsFrom(channel as Channel);
  }
  renderPosterized();
}

/**
 * Render the slider stack for a channel. Each slider feeds back into the
 * output store and triggers a live posterized redraw.
 */
function renderBandControls(channel: UiChannel) {
  const container = bandControlContainers[channel];
  if (!container) return;

  ensureChannelOutputs(channel);

  const outputs = channelOutputs[channel];

  container.innerHTML = "";

  outputs.forEach((value, index) => {
    const control = document.createElement("div");
    control.className = "band-control";

    const label = document.createElement("span");
    label.className = "band-label";
    label.textContent = `Band ${index + 1}`;

    const slider = document.createElement("input");
    slider.type = "range";
    slider.min = "0";
    slider.max = "255";
    slider.value = value.toString();
    slider.step = "1";
    slider.className = "band-slider";

    const valueDisplay = document.createElement("span");
    valueDisplay.className = "band-output-value";
    valueDisplay.textContent = value.toString();

    slider.addEventListener("input", () => {
      const next = Number(slider.value);
      valueDisplay.textContent = next.toString();
      setBandOutput(channel, index, next);
    });

    control.appendChild(label);
    control.appendChild(slider);
    control.appendChild(valueDisplay);
    container.appendChild(control);
  });
}

function renderAllBandControls() {
  if (lockChannelsEnabled) {
    renderBandControls("red");
  } else {
    COLOR_CHANNELS.forEach((channel) => {
      renderBandControls(channel);
    });
  }

  if (posterizeAlphaEnabled) {
    renderBandControls("alpha");
  } else if (bandControlContainers.alpha) {
    bandControlContainers.alpha.innerHTML = "";
  }
}

function updateChannelHistogram(channel: UiChannel) {
  const targets = {
    red: {
      bars: { canvas: histogramCanvasRed, ctx: histogramCtxRed },
      controls: { canvas: controlsCanvasRed, ctx: controlsCtxRed },
      color: "#ff4d4f",
    },
    green: {
      bars: { canvas: histogramCanvasGreen, ctx: histogramCtxGreen },
      controls: { canvas: controlsCanvasGreen, ctx: controlsCtxGreen },
      color: "#4dff7a",
    },
    blue: {
      bars: { canvas: histogramCanvasBlue, ctx: histogramCtxBlue },
      controls: { canvas: controlsCanvasBlue, ctx: controlsCtxBlue },
      color: "#6ba8ff",
    },
    alpha: {
      bars: { canvas: histogramCanvasAlpha, ctx: histogramCtxAlpha },
      controls: { canvas: controlsCanvasAlpha, ctx: controlsCtxAlpha },
      color: "#d0d0d0",
    },
  }[channel];

  if (
    !sourceCanvas ||
    !targets.bars.canvas ||
    !targets.bars.ctx ||
    !targets.controls.canvas ||
    !targets.controls.ctx
  )
    return;

  const cache = getHistogramCache();
  if (!cache) return;

  const thresholds = channelThresholds[channel];
  const counts =
    channel === "alpha"
      ? cache.alpha
      : lockChannelsEnabled && channel === "red"
        ? cache.combined
        : lockChannelsEnabled
          ? null
          : cache.perChannel[channel as Channel];

  if (!counts) return;

  const max = counts.reduce((acc, value) => (value > acc ? value : acc), 0);
  const { bars, controls } = targets;
  const barsCtx = bars.ctx;
  const barsCanvas = bars.canvas;
  const controlsCtx = controls.ctx;
  const controlsCanvas = controls.canvas;

  if (!barsCtx || !barsCanvas || !controlsCtx || !controlsCanvas) return;

  // Clear and draw histogram bars
  const canvasWidth = barsCanvas.width;
  const canvasHeight = barsCanvas.height;

  barsCtx.clearRect(0, 0, canvasWidth, canvasHeight);
  barsCtx.fillStyle = "#0f0f0f";
  barsCtx.fillRect(0, 0, canvasWidth, canvasHeight);

  if (max !== 0) {
    const barWidth = canvasWidth / 256;
    // Use white for combined histogram, original color for individual channels
    barsCtx.fillStyle =
      lockChannelsEnabled && channel === "red" ? "#ffffff" : targets.color;
    const maxLog = Math.log1p(max);

    for (let i = 0; i < 256; i += 1) {
      const value = counts[i];
      if (value === 0) continue;
      const normalized = Math.log1p(value) / maxLog;
      const barHeight = normalized * canvasHeight;
      const x = i * barWidth;
      const y = canvasHeight - barHeight;
      barsCtx.fillRect(x, y, Math.max(1, barWidth), barHeight);
    }
  }

  // Clear and draw both thresholds and levels on controls canvas
  controlsCtx.clearRect(0, 0, canvasWidth, canvasHeight);

  // Draw threshold lines
  if (thresholds.length > 0) {
    controlsCtx.shadowColor = "rgba(0, 0, 0, 0.5)";
    controlsCtx.shadowBlur = 3;
    controlsCtx.lineWidth = 2;
    controlsCtx.strokeStyle = "#ffffff";
    controlsCtx.globalAlpha = 0.9;

    thresholds.forEach((threshold) => {
      const x = (threshold / 255) * canvasWidth;
      controlsCtx.beginPath();
      controlsCtx.moveTo(x, 0);
      controlsCtx.lineTo(x, canvasHeight);
      controlsCtx.stroke();
    });
  }

  // Draw level bars
  const outputs = channelOutputs[channel];
  const bands = outputs.length;

  if (bands > 0) {
    // Calculate band boundaries
    const bandBoundaries = [0, ...thresholds, 255];

    controlsCtx.shadowColor = "rgba(0, 0, 0, 0.5)";
    controlsCtx.shadowBlur = 2;
    controlsCtx.lineWidth = 3;
    controlsCtx.strokeStyle = "#ffffff";
    controlsCtx.globalAlpha = 0.8;

    outputs.forEach((outputLevel, index) => {
      const bandStart = bandBoundaries[index];
      const bandEnd = bandBoundaries[index + 1];
      const bandCenterX = ((bandStart + bandEnd) / 2 / 255) * canvasWidth;
      const levelY = canvasHeight - (outputLevel / 255) * canvasHeight;

      // Draw horizontal level bar
      const barWidth = Math.min(
        40,
        ((bandEnd - bandStart) / 255) * canvasWidth * 0.8,
      );
      controlsCtx.beginPath();
      controlsCtx.moveTo(bandCenterX - barWidth / 2, levelY);
      controlsCtx.lineTo(bandCenterX + barWidth / 2, levelY);
      controlsCtx.stroke();
    });
  }
}

function updateAllHistograms() {
  if (lockChannelsEnabled) {
    // When synchronized, only update the red channel histogram and controls
    updateChannelHistogram("red");
    if (posterizeAlphaEnabled) {
      updateChannelHistogram("alpha");
    }

    // Clear green and blue histograms
    const greenBarsCtx = histogramCtxGreen;
    const greenControlsCtx = controlsCtxGreen;
    const blueBarsCtx = histogramCtxBlue;
    const blueControlsCtx = controlsCtxBlue;

    if (greenBarsCtx && histogramCanvasGreen) {
      greenBarsCtx.clearRect(
        0,
        0,
        histogramCanvasGreen.width,
        histogramCanvasGreen.height,
      );
      greenBarsCtx.fillStyle = "#0f0f0f";
      greenBarsCtx.fillRect(
        0,
        0,
        histogramCanvasGreen.width,
        histogramCanvasGreen.height,
      );
    }
    if (greenControlsCtx && controlsCanvasGreen) {
      greenControlsCtx.clearRect(
        0,
        0,
        controlsCanvasGreen.width,
        controlsCanvasGreen.height,
      );
    }

    if (blueBarsCtx && histogramCanvasBlue) {
      blueBarsCtx.clearRect(
        0,
        0,
        histogramCanvasBlue.width,
        histogramCanvasBlue.height,
      );
      blueBarsCtx.fillStyle = "#0f0f0f";
      blueBarsCtx.fillRect(
        0,
        0,
        histogramCanvasBlue.width,
        histogramCanvasBlue.height,
      );
    }
    if (blueControlsCtx && controlsCanvasBlue) {
      blueControlsCtx.clearRect(
        0,
        0,
        controlsCanvasBlue.width,
        controlsCanvasBlue.height,
      );
    }
  } else {
    updateChannelHistogram("red");
    updateChannelHistogram("green");
    updateChannelHistogram("blue");
    if (posterizeAlphaEnabled) {
      updateChannelHistogram("alpha");
    }
  }
}

/**
 * Render the processed canvas using the current channel settings, keeping the
 * original ImageData untouched so histograms continue to represent the source.
 */
function renderPosterized() {
  if (!originalImageData || !outputCanvas || !outputCtx) return;

  ensureChannelOutputs("red");
  ensureChannelOutputs("green");
  ensureChannelOutputs("blue");
  if (posterizeAlphaEnabled) {
    ensureChannelOutputs("alpha");
  }

  if (outputCanvas.width !== originalImageData.width) {
    outputCanvas.width = originalImageData.width;
  }
  if (outputCanvas.height !== originalImageData.height) {
    outputCanvas.height = originalImageData.height;
  }

  const working = new ImageData(
    new Uint8ClampedArray(originalImageData.data),
    originalImageData.width,
    originalImageData.height,
  );

  let posterized =
    activeColorSpace === "hsv"
      ? applyHsvPosterize(
          working,
          {
            H: channelLevels.red,
            S: channelLevels.green,
            V: channelLevels.blue,
          },
          {
            H:
              channelThresholds.red.length === channelLevels.red - 1
                ? channelThresholds.red
                : createEvenThresholds(channelLevels.red),
            S:
              channelThresholds.green.length === channelLevels.green - 1
                ? channelThresholds.green
                : createEvenThresholds(channelLevels.green),
            V:
              channelThresholds.blue.length === channelLevels.blue - 1
                ? channelThresholds.blue
                : createEvenThresholds(channelLevels.blue),
          },
          {
            H:
              channelOutputs.red.length === channelLevels.red
                ? channelOutputs.red
                : undefined,
            S:
              channelOutputs.green.length === channelLevels.green
                ? channelOutputs.green
                : undefined,
            V:
              channelOutputs.blue.length === channelLevels.blue
                ? channelOutputs.blue
                : undefined,
          },
        )
      : applyBasicPosterize(
          working,
          {
            R: channelLevels.red,
            G: channelLevels.green,
            B: channelLevels.blue,
          },
          {
            R:
              channelThresholds.red.length === channelLevels.red - 1
                ? channelThresholds.red
                : createEvenThresholds(channelLevels.red),
            G:
              channelThresholds.green.length === channelLevels.green - 1
                ? channelThresholds.green
                : createEvenThresholds(channelLevels.green),
            B:
              channelThresholds.blue.length === channelLevels.blue - 1
                ? channelThresholds.blue
                : createEvenThresholds(channelLevels.blue),
          },
          {
            R:
              channelOutputs.red.length === channelLevels.red
                ? channelOutputs.red
                : undefined,
            G:
              channelOutputs.green.length === channelLevels.green
                ? channelOutputs.green
                : undefined,
            B:
              channelOutputs.blue.length === channelLevels.blue
                ? channelOutputs.blue
                : undefined,
          },
        );

  if (posterizeAlphaEnabled) {
    const levels = channelLevels.alpha;
    const thresholds =
      channelThresholds.alpha.length === levels - 1
        ? channelThresholds.alpha
        : createEvenThresholds(levels);
    const outputs =
      channelOutputs.alpha.length === levels ? channelOutputs.alpha : undefined;
    posterized = applyAlphaPosterize(posterized, levels, thresholds, outputs);
  }
  outputCtx.putImageData(posterized, 0, 0);
}

function drawFromSource(
  src: string,
  cleanup?: () => void,
  saveToStorage = false,
) {
  if (!sourceCanvas || !sourceCtx) return;

  const img = new Image();
  img.onload = () => {
    drawImageToCanvas(img, sourceCanvas, sourceCtx);
    originalImageData = sourceCtx.getImageData(
      0,
      0,
      sourceCanvas.width,
      sourceCanvas.height,
    );
    originalImageRevision += 1;
    histogramCache = null;

    // Save image data to localStorage if requested
    if (saveToStorage && src !== DEFAULT_IMAGE) {
      try {
        const canvas = document.createElement("canvas");
        canvas.width = sourceCanvas.width;
        canvas.height = sourceCanvas.height;
        const ctx = canvas.getContext("2d");
        if (ctx) {
          ctx.putImageData(originalImageData, 0, 0);
          const dataUrl = canvas.toDataURL("image/png");
          localStorage.setItem("super-posterize-image", dataUrl);
          localStorage.setItem("super-posterize-has-image", "true");
        }
      } catch (e) {
        console.warn("Failed to save image to localStorage:", e);
      }
    }

    if (outputCanvas && outputCtx) {
      if (outputCanvas.width !== sourceCanvas.width) {
        outputCanvas.width = sourceCanvas.width;
      }
      if (outputCanvas.height !== sourceCanvas.height) {
        outputCanvas.height = sourceCanvas.height;
      }
      if (comparisonStage && sourceCanvas.width && sourceCanvas.height) {
        const aspect =
          sourceCanvas.width && sourceCanvas.height
            ? sourceCanvas.width / sourceCanvas.height
            : 1;
        comparisonStage.style.setProperty(
          "--image-aspect",
          aspect.toString(),
        );
      }
      renderPosterized();
    }
    updateAllHistograms();
    cleanup?.();
  };
  img.src = src;
}

function syncChannelLabel(channel: UiChannel, value: number) {
  const label = levelLabels[channel];
  if (label) {
    label.textContent = value.toString();
  }
}

function setChannelLevel(channel: UiChannel, value: number) {
  const clamped = Math.max(MIN_LEVELS, Math.min(MAX_LEVELS, Math.floor(value)));
  channelLevels[channel] = clamped;
  channelThresholds[channel] = createEvenThresholds(clamped);
  channelOutputs[channel] = createLinearOutputs(clamped);
  syncChannelLabel(channel, clamped);
  if (lockChannelsEnabled && channel !== "alpha") {
    syncOtherChannelsFrom(channel as Channel);
    renderAllBandControls();
    updateAllHistograms();
  } else {
    updateChannelHistogram(channel);
    renderBandControls(channel);
  }
  renderPosterized();
}

function resetChannelLevels() {
  (Object.keys(DEFAULT_LEVELS) as UiChannel[]).forEach((channel) => {
    const defaultValue = DEFAULT_LEVELS[channel];
    channelLevels[channel] = defaultValue;
    channelThresholds[channel] = createEvenThresholds(defaultValue);
    channelOutputs[channel] = createLinearOutputs(defaultValue);
    syncChannelLabel(channel, defaultValue);
    updateChannelHistogram(channel);
  });
  renderAllBandControls();
  renderPosterized();
}

function setChannelThreshold(channel: UiChannel, index: number, value: number) {
  const thresholds = channelThresholds[channel];
  if (!thresholds || index < 0 || index >= thresholds.length) return;

  const minValue = index === 0 ? 1 : thresholds[index - 1] + 1;
  const maxValue =
    index === thresholds.length - 1 ? 254 : thresholds[index + 1] - 1;
  const clamped = Math.max(minValue, Math.min(maxValue, Math.round(value)));
  if (thresholds[index] === clamped) return;

  thresholds[index] = clamped;
  if (lockChannelsEnabled && channel !== "alpha") {
    syncOtherChannelsFrom(channel as Channel);
    renderAllBandControls();
    updateAllHistograms();
  } else {
    updateChannelHistogram(channel);
    renderBandControls(channel);
  }
  renderPosterized();
}

function attachUnifiedDragHandlers(
  canvas: HTMLCanvasElement | null,
  channel: UiChannel,
) {
  if (!canvas) return;

  canvas.style.touchAction = "none";

  const isOverThreshold = (x: number): number => {
    const rect = canvas.getBoundingClientRect();
    const scaleX = canvas.width / rect.width;
    const canvasX = x * scaleX;

    const thresholds = channelThresholds[channel];
    const tolerance = HIT_TARGET_PX * scaleX;

    for (let i = 0; i < thresholds.length; i++) {
      const thresholdX = (thresholds[i] / 255) * canvas.width;
      if (Math.abs(canvasX - thresholdX) < tolerance) {
        return i;
      }
    }
    return -1;
  };

  const isOverLevelHandle = (x: number, y: number): number => {
    const rect = canvas.getBoundingClientRect();
    const canvasX = (x / rect.width) * canvas.width;
    const canvasY = (y / rect.height) * canvas.height;

    const thresholds = channelThresholds[channel];
    const outputs = channelOutputs[channel];
    const bandBoundaries = [0, ...thresholds, 255];

    for (let i = 0; i < outputs.length; i++) {
      const bandStart = bandBoundaries[i];
      const bandEnd = bandBoundaries[i + 1];
      const bandCenterX = ((bandStart + bandEnd) / 2 / 255) * canvas.width;
      const levelY = canvas.height - (outputs[i] / 255) * canvas.height;

      const barWidth = Math.min(
        40,
        ((bandEnd - bandStart) / 255) * canvas.width * 0.8,
      );

      if (
        Math.abs(canvasX - bandCenterX) <= barWidth / 2 + 10 &&
        Math.abs(canvasY - levelY) <= 10
      ) {
        return i;
      }
    }
    return -1;
  };

  const updateCursor = (x: number, y: number) => {
    const thresholdIndex = isOverThreshold(x);
    const levelIndex = isOverLevelHandle(x, y);

    if (thresholdIndex >= 0) {
      canvas.style.cursor = "col-resize";
    } else if (levelIndex >= 0) {
      canvas.style.cursor = "row-resize";
    } else {
      canvas.style.cursor = "crosshair";
    }
  };

  const handlePointerMove = (event: PointerEvent) => {
    const rect = canvas.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;

    if (
      dragState &&
      dragState.channel === channel &&
      dragState.pointerId === event.pointerId
    ) {
      if (dragState.type === "threshold") {
        const scaleX = canvas.width / rect.width;
        const canvasX = x * scaleX;
        const value = (canvasX / canvas.width) * 255;
        setChannelThreshold(channel, dragState.index, value);

        // If channels are synchronized, apply to all channels
        if (lockChannelsEnabled && channel !== "alpha") {
          synchronizeChannels(channel as Channel);
        }
      } else if (dragState.type === "level") {
        const normalizedY = 255 - (y / rect.height) * 255;
        const value = Math.max(0, Math.min(255, Math.round(normalizedY)));
        const outputs = channelOutputs[channel];
        if (outputs[dragState.index] !== undefined) {
          outputs[dragState.index] = value;

          // If channels are synchronized, apply to all channels
          if (lockChannelsEnabled && channel !== "alpha") {
            const currentIndex = dragState.index; // Store index before forEach
            COLOR_CHANNELS.forEach((ch) => {
              if (
                ch !== channel &&
                channelOutputs[ch][currentIndex] !== undefined
              ) {
                channelOutputs[ch][currentIndex] = value;
              }
            });
          }

          updateAllHistograms();
          renderPosterized();
        }
      }
      event.preventDefault();
      return;
    }

    // Only update cursor if this channel is active (not synchronized or is red channel)
    if (!lockChannelsEnabled || channel === "red" || channel === "alpha") {
      updateCursor(x, y);
    } else {
      canvas.style.cursor = "default";
    }
  };

  const handlePointerDown = (event: PointerEvent) => {
    if (dragState) return;

    // If channels are synchronized and this is not the red channel, ignore
    if (lockChannelsEnabled && channel !== "red" && channel !== "alpha") return;

    const rect = canvas.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;

    const thresholdIndex = isOverThreshold(x);
    const levelIndex = isOverLevelHandle(x, y);

    if (thresholdIndex >= 0) {
      dragState = {
        channel,
        index: thresholdIndex,
        pointerId: event.pointerId,
        type: "threshold",
      };
      canvas.setPointerCapture(event.pointerId);
      event.preventDefault();
    } else if (levelIndex >= 0) {
      dragState = {
        channel,
        index: levelIndex,
        pointerId: event.pointerId,
        type: "level",
      };
      canvas.setPointerCapture(event.pointerId);
      event.preventDefault();
    }
  };

  const endDrag = (event: PointerEvent) => {
    if (
      !dragState ||
      dragState.channel !== channel ||
      dragState.pointerId !== event.pointerId
    )
      return;

    if (canvas.hasPointerCapture(event.pointerId)) {
      canvas.releasePointerCapture(event.pointerId);
    }
    dragState = null;

    // Update cursor after drag ends
    const rect = canvas.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    updateCursor(x, y);

    event.preventDefault();
  };

  canvas.addEventListener("pointermove", handlePointerMove);
  canvas.addEventListener("pointerdown", handlePointerDown);
  canvas.addEventListener("pointerup", endDrag);
  canvas.addEventListener("pointercancel", endDrag);
}

// Attach unified drag handlers to control canvases
attachUnifiedDragHandlers(controlsCanvasRed, "red");
attachUnifiedDragHandlers(controlsCanvasGreen, "green");
attachUnifiedDragHandlers(controlsCanvasBlue, "blue");
attachUnifiedDragHandlers(controlsCanvasAlpha, "alpha");

lockToggle?.addEventListener("change", () => {
  toggleLockChannels(lockToggle.checked);
});

colorSpaceSelect?.addEventListener("change", () => {
  const raw = colorSpaceSelect.value;
  if (raw === "rgb" || raw === "hsv") {
    setColorSpace(raw);
  }
});

posterizeAlphaToggle?.addEventListener("change", () => {
  togglePosterizeAlpha(posterizeAlphaToggle.checked);
});

saveImageBtn?.addEventListener("click", () => {
  saveProcessedImage();
});

exportSettingsBtn?.addEventListener("click", () => {
  downloadSettings();
});

importSettingsBtn?.addEventListener("click", () => {
  settingsFileInput?.click();
});

settingsFileInput?.addEventListener("change", () => {
  const file = settingsFileInput.files?.[0];
  if (!file) return;
  handleSettingsFile(file);
});

document.querySelectorAll<HTMLButtonElement>(".band-step").forEach((button) => {
  const channel = button.dataset.channel as UiChannel | undefined;
  const direction = button.dataset.direction;
  if (!channel || !direction) return;

  const delta = direction === "increment" ? 1 : -1;
  button.addEventListener("click", () => {
    const next = channelLevels[channel] + delta;
    if (next < MIN_LEVELS || next > MAX_LEVELS) return;
    setChannelLevel(channel, next);
  });
});

fileInputBtn?.addEventListener("click", () => {
  fileInput?.click();
});

fileInput?.addEventListener("change", () => {
  const file = fileInput.files?.[0];
  if (!file) {
    if (selectedFileName) selectedFileName.textContent = "No file selected";
    return;
  }

  if (selectedFileName) selectedFileName.textContent = file.name;
  const objectUrl = URL.createObjectURL(file);
  drawFromSource(objectUrl, () => URL.revokeObjectURL(objectUrl), true);
});

resetBtn?.addEventListener("click", () => {
  if (fileInput) fileInput.value = "";
  if (selectedFileName) selectedFileName.textContent = "No file selected";
  localStorage.removeItem("super-posterize-image");
  localStorage.removeItem("super-posterize-has-image");
  localStorage.removeItem(ALPHA_STORAGE_KEY);
  togglePosterizeAlpha(false);
  resetChannelLevels();
  if (!originalImageData) {
    drawFromSource(DEFAULT_IMAGE);
  }
});

// Initialize the application
function initializeApp() {
  const savedColorSpace = localStorage.getItem(COLOR_SPACE_STORAGE_KEY);
  if (savedColorSpace === "rgb" || savedColorSpace === "hsv") {
    activeColorSpace = savedColorSpace;
  }
  if (colorSpaceSelect) {
    colorSpaceSelect.value = activeColorSpace;
  }

  const savedAlpha = localStorage.getItem(ALPHA_STORAGE_KEY);
  if (savedAlpha === "true" || savedAlpha === "false") {
    posterizeAlphaEnabled = savedAlpha === "true";
  }
  if (posterizeAlphaToggle) {
    posterizeAlphaToggle.checked = posterizeAlphaEnabled;
  }

  resetChannelLevels();

  // Try to restore saved image, otherwise use default
  const savedImage = localStorage.getItem("super-posterize-image");
  const hasImage = localStorage.getItem("super-posterize-has-image");

  if (savedImage && hasImage === "true") {
    drawFromSource(savedImage);
  } else {
    drawFromSource(DEFAULT_IMAGE);
  }

  applyLockStateToUI();
}

// Start the application
initializeApp();
