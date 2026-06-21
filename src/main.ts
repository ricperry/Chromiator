import {
  applyAlphaPosterize,
  applyBasicPosterize,
  applyHsvPosterize,
  hsv255ToRgb,
  rgbToHsv255,
} from "./effect";
import { PRESETS, type PresetEntry } from "./presets";
import {
  analyzeImageData,
  completedSites,
  computeBezierAssignments,
  computeVoronoiAssignments,
  createBezierNode,
  createBezierRegion,
  createQuantizationGroup,
  createSite,
  hexToRgb,
  hsvCoordToRgb,
  outputCoordFromRgb,
  projectCoordToPanelPoint,
  projectionForRegion,
  projectionPanelsForSpace,
  renderBezierImage,
  renderVoronoiImage,
  rgbToHex,
  siteCoordFromRgb,
  validRegionProjections,
  type HsvSiteCoord,
  type AnalyzedImage,
  type BezierRegion,
  type QuantizationGroup,
  type RGB,
} from "./quantization";
import { drawImageToCanvas } from "./util";

const sourceCanvas = document.getElementById(
  "originalCanvas",
) as HTMLCanvasElement | null;
const sourceCtx = sourceCanvas?.getContext("2d") ?? null;
const outputCanvas = document.getElementById(
  "processedCanvas",
) as HTMLCanvasElement | null;
const outputCtx = outputCanvas?.getContext("2d") ?? null;
const quantizerOverlayCanvas = document.getElementById(
  "quantizerOverlayCanvas",
) as HTMLCanvasElement | null;
const quantizerOverlayCtx = quantizerOverlayCanvas?.getContext("2d") ?? null;
const comparisonStage = document.getElementById(
  "comparisonStage",
) as HTMLElement | null;
const floatingModeToolbar = document.querySelector(
  ".floating-mode-toolbar",
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
const METADATA_KEYWORD = "Threshiator";
const LEGACY_METADATA_KEYWORD = "SuperPosterize";
const COLOR_SPACE_STORAGE_KEY = "threshiator-color-space";
const ALPHA_STORAGE_KEY = "threshiator-posterize-alpha";
const SMOOTHING_STORAGE_KEY = "threshiator-smoothing";

export type ChannelSettings = {
  levels: number;
  thresholds: number[];
  outputs: number[];
};

export type ExportSettings = {
  version: number;
  colorSpace?: ColorSpace;
  processingMode?: ProcessingMode;
  smoothing?: number;
  lockChannels: boolean;
  posterizeAlpha?: boolean;
  alpha?: ChannelSettings;
  channels: Record<Channel, ChannelSettings>;
  quantizer?: {
    editMode?: VoronoiEditMode;
    sampleRadius?: number;
    groups: QuantizationGroup[];
    activeGroupId: string;
    activeSiteId: string | null;
    regions: BezierRegion[];
    activeRegionId: string | null;
  };
};

const levelLabels: Record<UiChannel, HTMLInputElement | null> = {
  red: document.getElementById("redLevelsValue") as HTMLInputElement | null,
  green: document.getElementById("greenLevelsValue") as HTMLInputElement | null,
  blue: document.getElementById("blueLevelsValue") as HTMLInputElement | null,
  alpha: document.getElementById("alphaLevelsValue") as HTMLInputElement | null,
};

const bandControlContainers: Record<UiChannel, HTMLElement | null> = {
  red: document.querySelector('.band-levels[data-channel="red"]'),
  green: document.querySelector('.band-levels[data-channel="green"]'),
  blue: document.querySelector('.band-levels[data-channel="blue"]'),
  alpha: document.querySelector('.band-levels[data-channel="alpha"]'),
};

const controlsPane = document.querySelector(".controls-pane");
const histogramsSection = document.getElementById("histograms");
const lockToggle = document.getElementById(
  "lockChannelsToggle",
) as HTMLInputElement | null;
const syncChannelsLabel = document.getElementById(
  "syncChannelsLabel",
) as HTMLSpanElement | null;
const canvasStatusOverlay = document.getElementById(
  "canvasStatusOverlay",
) as HTMLElement | null;
const colorSpaceSelect = document.getElementById(
  "colorSpaceSelect",
) as HTMLSelectElement | null;
const processingModeSelect = document.getElementById(
  "processingModeSelect",
) as HTMLSelectElement | null;
const quantizerPanel = document.getElementById(
  "quantizerPanel",
) as HTMLElement | null;
const quantizerTitle = document.getElementById(
  "quantizerTitle",
) as HTMLElement | null;
const voronoiActions = document.getElementById(
  "voronoiActions",
) as HTMLElement | null;
const bezierActions = document.getElementById(
  "bezierActions",
) as HTMLElement | null;
const addQuantizerGroupBtn = document.getElementById(
  "addQuantizerGroupBtn",
) as HTMLButtonElement | null;
const deleteQuantizerGroupBtn = document.getElementById(
  "deleteQuantizerGroupBtn",
) as HTMLButtonElement | null;
const voronoiSourceModeBtn = document.getElementById(
  "voronoiSourceModeBtn",
) as HTMLButtonElement | null;
const voronoiTargetModeBtn = document.getElementById(
  "voronoiTargetModeBtn",
) as HTMLButtonElement | null;
const addBezierRegionBtn = document.getElementById(
  "addBezierRegionBtn",
) as HTMLButtonElement | null;
const closeBezierRegionBtn = document.getElementById(
  "closeBezierRegionBtn",
) as HTMLButtonElement | null;
const deleteBezierRegionBtn = document.getElementById(
  "deleteBezierRegionBtn",
) as HTMLButtonElement | null;
const quantizerResetBtn = document.getElementById(
  "quantizerResetBtn",
) as HTMLButtonElement | null;
const quantizerEditor = document.getElementById(
  "quantizerEditor",
) as HTMLElement | null;
const quantizerList = document.getElementById(
  "quantizerList",
) as HTMLElement | null;
const quantizerStats = document.getElementById(
  "quantizerStats",
) as HTMLElement | null;
const smoothingInput = document.getElementById(
  "smoothingInput",
) as HTMLInputElement | null;
const smoothingValue = document.getElementById(
  "smoothingValue",
) as HTMLInputElement | null;
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
let lockChannelsEnabled = false;
let activeColorSpace: ColorSpace = "rgb";
let posterizeAlphaEnabled = false;
let smoothingAmount = 0;
let floatingOverlayVisible = true;
let activePresetDescription = "";
type ProcessingMode = "threshold" | "voronoi" | "bezier";
type VoronoiEditMode = "source" | "target";
let activeProcessingMode: ProcessingMode = "voronoi";
let activeVoronoiEditMode: VoronoiEditMode = "source";
type EngineSettingsCache = {
  colorSpace: ColorSpace;
  smoothing: number;
  posterizeAlpha: boolean;
  lockChannels: boolean;
};
const engineSettingsCache: Partial<Record<ProcessingMode, EngineSettingsCache>> =
  {
    threshold: {
      colorSpace: "rgb",
      smoothing: 0,
      posterizeAlpha: false,
      lockChannels: false,
    },
    voronoi: {
      colorSpace: "rgb",
      smoothing: 0,
      posterizeAlpha: false,
      lockChannels: false,
    },
  };
let quantizerGroups: QuantizationGroup[] = [
  createQuantizationGroup("Index 1", { r: 0, g: 150, b: 255 }),
];
let activeQuantizerGroupId = quantizerGroups[0].id;
let activeQuantizerSiteId: string | null = null;
let bezierRegions: BezierRegion[] = [];
let activeBezierRegionId: string | null = null;
let quantizerRevision = 0;
let voronoiSampleRadius = 1;
let draggedImageSite: {
  siteId: string;
  pointerId: number;
} | null = null;

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

function syncGroupForColorSpace(colorSpace: ColorSpace): Channel[] {
  return colorSpace === "hsv" ? ["green", "blue"] : COLOR_CHANNELS;
}

function syncBaseChannelForColorSpace(colorSpace: ColorSpace): Channel {
  return colorSpace === "hsv" ? "green" : "red";
}

function isSyncableChannel(channel: UiChannel): channel is Channel {
  return (
    channel !== "alpha" &&
    syncGroupForColorSpace(activeColorSpace).includes(channel)
  );
}

function isVisibleWhenSynced(channel: UiChannel): boolean {
  if (activeProcessingMode !== "threshold") {
    return channel !== "alpha";
  }
  if (channel === "alpha") return posterizeAlphaEnabled;
  if (!lockChannelsEnabled) return true;
  if (activeColorSpace === "hsv") {
    return channel === "red" || channel === "green";
  }
  return channel === "red";
}

function syncedChannelLabel(colorSpace: ColorSpace): string {
  return colorSpace === "hsv" ? "Saturation / Value" : "RGB";
}

function usesHueDegreeControls(channel: UiChannel): boolean {
  return activeColorSpace === "hsv" && channel === "red";
}

function rawToControlValue(channel: UiChannel, value: number): number {
  if (!usesHueDegreeControls(channel)) return Math.round(value);
  return Math.round((Math.max(0, Math.min(255, value)) / 255) * 360);
}

function controlToRawValue(channel: UiChannel, value: number): number {
  if (!usesHueDegreeControls(channel)) return Math.round(value);
  return Math.round((Math.max(0, Math.min(360, value)) / 360) * 255);
}

function updateModeLabels() {
  if (syncChannelsLabel) {
    syncChannelsLabel.textContent =
      activeColorSpace === "hsv" ? "Sync S/V" : "Sync RGB";
  }
  if (canvasStatusOverlay) {
    const messages: string[] = [];
    if (activePresetDescription && activeProcessingMode !== "voronoi") {
      messages.push(activePresetDescription);
    }
    if (activeColorSpace === "hsv") {
      messages.push(
        "Hue uses 0-360 degree controls and is not synced with Saturation / Value.",
      );
    }
    if (activeProcessingMode === "threshold") {
      messages.push(
        "Thresholds mode: drag vertical threshold lines and horizontal output handles in the channel graphs.",
      );
    } else if (activeProcessingMode === "voronoi") {
      messages.push(
        activeVoronoiEditMode === "source"
          ? `Source mode: click the image to set the active source sample. Samples average a ${voronoiSampleRadius}px radius in the active space.`
          : "Target mode: drag channel color bars to set the active index target color.",
      );
    } else if (activeProcessingMode === "bezier") {
      messages.push(
        "Click visible image pixels to add linked source boundary points. Drag channel histograms to set the active target color, then close the source region.",
      );
    }
    canvasStatusOverlay.textContent = messages.join("  |  ");
    canvasStatusOverlay.hidden =
      !floatingOverlayVisible || messages.length === 0;
  }
}

function updateToolbarModeState() {
  floatingModeToolbar?.classList.toggle(
    "voronoi-toolbar",
    activeProcessingMode === "voronoi",
  );
}

function applyChannelTitles() {
  if (lockChannelsEnabled && activeColorSpace === "rgb") {
    channelTitles.red?.classList.add("synced");
    if (channelTitles.red)
      channelTitles.red.textContent = syncedChannelLabel("rgb");
  } else {
    channelTitles.red?.classList.remove("synced");
    if (channelTitles.red)
      channelTitles.red.textContent = channelLabelFor("red", activeColorSpace);
  }

  if (channelTitles.green) {
    channelTitles.green.classList.toggle(
      "synced",
      lockChannelsEnabled && activeColorSpace === "hsv",
    );
    channelTitles.green.textContent =
      lockChannelsEnabled && activeColorSpace === "hsv"
        ? syncedChannelLabel("hsv")
        : channelLabelFor("green", activeColorSpace);
  }
  if (channelTitles.blue)
    channelTitles.blue.textContent = channelLabelFor("blue", activeColorSpace);
  if (channelTitles.alpha)
    channelTitles.alpha.textContent = channelLabelFor(
      "alpha",
      activeColorSpace,
    );
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
  if (activeProcessingMode === "voronoi") {
    rehydrateVoronoiSitesForActiveColorSpace();
  }
  if (colorSpaceSelect) colorSpaceSelect.value = next;
  updateModeLabels();
  applyChannelTitles();
  if (lockChannelsEnabled) {
    synchronizeChannels(syncBaseChannelForColorSpace(activeColorSpace));
  }
  if (saveToStorage) {
    localStorage.setItem(COLOR_SPACE_STORAGE_KEY, next);
  }
  if (rerender) {
    activeQuantizerSiteId =
      activeQuantizerGroup()?.sites.find(
        (site) => site.colorSpace === activeColorSpace,
      )?.id ?? null;
    activeBezierRegionId =
      bezierRegions.find((region) => region.colorSpace === activeColorSpace)
        ?.id ?? null;
    updateAllHistograms();
    renderAllBandControls();
    renderQuantizerControls();
    renderPosterized();
  }
}

function sanitizeSmoothing(value: unknown): number {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return 0;
  return Math.max(MIN_SMOOTHING, Math.min(MAX_SMOOTHING, Math.round(numeric)));
}

function syncSmoothingUi() {
  const value = smoothingAmount.toString();
  if (smoothingInput && smoothingInput.value !== value) {
    smoothingInput.value = value;
  }
  if (smoothingValue) {
    smoothingValue.value = value;
  }
}

function commitNumberInputOnEnter(input: HTMLInputElement, commit: () => void) {
  input.addEventListener("keydown", (event) => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    commit();
  });
}

function setSmoothingAmount(
  next: number,
  options: { saveToStorage?: boolean; rerender?: boolean } = {},
) {
  const clamped = sanitizeSmoothing(next);
  const saveToStorage = options.saveToStorage ?? true;
  const rerender = options.rerender ?? true;
  if (smoothingAmount === clamped) {
    syncSmoothingUi();
    return;
  }

  smoothingAmount = clamped;
  preprocessedImageCache = null;
  histogramCache = null;
  syncSmoothingUi();

  if (saveToStorage) {
    localStorage.setItem(SMOOTHING_STORAGE_KEY, clamped.toString());
  }
  if (rerender) {
    updateAllHistograms();
    renderPosterized();
  }
}

function setComparisonReveal(value: number) {
  const clampedValue = Math.min(100, Math.max(0, value));
  comparisonStage?.style.setProperty("--preview-reveal", `${clampedValue}%`);
  if (comparisonSlider) {
    comparisonSlider.value = Math.round(clampedValue).toString();
  }
}

function isTextEntryTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const input = target.closest("input, select, textarea");
  if (!input) return false;
  if (input instanceof HTMLInputElement) {
    return input.type !== "range" && input.type !== "checkbox";
  }
  return true;
}

function setFloatingOverlayVisible(visible: boolean) {
  floatingOverlayVisible = visible;
  if (floatingModeToolbar) {
    floatingModeToolbar.hidden = !floatingOverlayVisible;
  }
  updateModeLabels();
}

function toggleFloatingToolbar() {
  setFloatingOverlayVisible(!floatingOverlayVisible);
}

if (comparisonStage && comparisonSlider) {
  let isDraggingComparison = false;

  const isOverComparisonHandle = (event: PointerEvent): boolean => {
    const rect = comparisonStage.getBoundingClientRect();
    if (rect.width <= 0) return false;
    const reveal = Number(comparisonSlider.value);
    const handleX = rect.left + (Math.max(0, Math.min(100, reveal)) / 100) * rect.width;
    return Math.abs(event.clientX - handleX) <= 18;
  };

  const revealFromPointer = (event: PointerEvent) => {
    const rect = comparisonStage.getBoundingClientRect();
    if (rect.width <= 0) return;
    const percent = ((event.clientX - rect.left) / rect.width) * 100;
    setComparisonReveal(percent);
  };

  comparisonSlider.addEventListener("input", (event) => {
    const newValue = Number((event.target as HTMLInputElement).value);
    setComparisonReveal(newValue);
  });

  comparisonStage.addEventListener("pointerdown", (event) => {
    const target = event.target as HTMLElement | null;
    if (target?.closest(".floating-mode-toolbar")) {
      return;
    }
    if (isOverComparisonHandle(event)) {
      isDraggingComparison = true;
      comparisonStage.setPointerCapture(event.pointerId);
      revealFromPointer(event);
      event.preventDefault();
      return;
    }
    if (handleQuantizerStagePointer(event)) {
      return;
    }
    isDraggingComparison = true;
    comparisonStage.setPointerCapture(event.pointerId);
    revealFromPointer(event);
    event.preventDefault();
  });

  comparisonStage.addEventListener("pointermove", (event) => {
    if (draggedImageSite?.pointerId === event.pointerId) {
      updateDraggedImageSite(event.clientX, event.clientY);
      event.preventDefault();
      return;
    }
    if (!isDraggingComparison) return;
    revealFromPointer(event);
    event.preventDefault();
  });

  const stopComparisonDrag = (event: PointerEvent) => {
    if (draggedImageSite?.pointerId === event.pointerId) {
      draggedImageSite = null;
      if (comparisonStage.hasPointerCapture(event.pointerId)) {
        comparisonStage.releasePointerCapture(event.pointerId);
      }
      event.preventDefault();
      return;
    }
    if (!isDraggingComparison) return;
    isDraggingComparison = false;
    if (comparisonStage.hasPointerCapture(event.pointerId)) {
      comparisonStage.releasePointerCapture(event.pointerId);
    }
    event.preventDefault();
  };

  comparisonStage.addEventListener("pointerup", stopComparisonDrag);
  comparisonStage.addEventListener("pointercancel", stopComparisonDrag);

  setComparisonReveal(Number(comparisonSlider.value));
}

if (comparisonStage && "ResizeObserver" in window) {
  const previewResizeObserver = new ResizeObserver(() => {
    schedulePreviewResize();
  });
  previewResizeObserver.observe(comparisonStage);
}

const DEFAULT_IMAGE = new URL("../test.png", import.meta.url).href;
const MIN_LEVELS = 2;
const MAX_LEVELS = 32;
const MIN_SMOOTHING = 0;
const MAX_SMOOTHING = 10;
const HIT_TARGET_PX = 6;

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
        const metadata = extractSettingsMetadataFromPng(uint8Array);

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
          console.log(`⚠️ No Threshiator metadata found in ${fileName}`);
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
  if (!presetsSelect) return;

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
      activePresetDescription = "";
      updateModeLabels();
      return;
    }

    const preset = allPresets[selectedKey];
    if (!preset) return;

    activePresetDescription = preset.description;
    updateModeLabels();

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

let fullResolutionImageData: ImageData | null = null;
let originalImageData: ImageData | null = null;
let originalImageRevision = 0;
let previewScale = 1;
let previewResizeFrame: number | null = null;

type PreprocessedImageCache = {
  revision: number;
  smoothingRadius: number;
  imageData: ImageData;
};

type HistogramCache = {
  revision: number;
  colorSpace: ColorSpace;
  smoothingRadius: number;
  perChannel: Record<Channel, Uint32Array>;
  combined: Uint32Array;
  alpha: Uint32Array;
};

let preprocessedImageCache: PreprocessedImageCache | null = null;
let histogramCache: HistogramCache | null = null;

function createGaussianKernel(radius: number): Float32Array {
  const sigma = Math.max(0.5, radius);
  const kernelRadius = Math.ceil(sigma * 3);
  const kernel = new Float32Array(kernelRadius * 2 + 1);
  const divisor = 2 * sigma * sigma;
  let total = 0;

  for (let i = -kernelRadius; i <= kernelRadius; i += 1) {
    const value = Math.exp(-(i * i) / divisor);
    kernel[i + kernelRadius] = value;
    total += value;
  }

  for (let i = 0; i < kernel.length; i += 1) {
    kernel[i] /= total;
  }

  return kernel;
}

function applyGaussianBlur(imageData: ImageData, radius: number): ImageData {
  if (radius <= 0) return imageData;

  const { width, height, data } = imageData;
  const kernel = createGaussianKernel(radius);
  const kernelRadius = Math.floor(kernel.length / 2);
  const horizontal = new Float32Array(data.length);
  const output = new Uint8ClampedArray(data.length);

  for (let y = 0; y < height; y += 1) {
    const rowOffset = y * width * 4;
    for (let x = 0; x < width; x += 1) {
      const target = rowOffset + x * 4;
      for (let channel = 0; channel < 4; channel += 1) {
        let sum = 0;
        for (let k = -kernelRadius; k <= kernelRadius; k += 1) {
          const sampleX = Math.max(0, Math.min(width - 1, x + k));
          sum +=
            data[rowOffset + sampleX * 4 + channel] * kernel[k + kernelRadius];
        }
        horizontal[target + channel] = sum;
      }
    }
  }

  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const target = (y * width + x) * 4;
      for (let channel = 0; channel < 4; channel += 1) {
        let sum = 0;
        for (let k = -kernelRadius; k <= kernelRadius; k += 1) {
          const sampleY = Math.max(0, Math.min(height - 1, y + k));
          sum +=
            horizontal[(sampleY * width + x) * 4 + channel] *
            kernel[k + kernelRadius];
        }
        output[target + channel] = Math.round(sum);
      }
    }
  }

  return new ImageData(output, width, height);
}

function getPreprocessedImageData(): ImageData | null {
  if (!originalImageData) return null;
  const smoothingRadius = smoothingAmount * previewScale;
  if (smoothingRadius <= 0) return originalImageData;

  if (
    preprocessedImageCache &&
    preprocessedImageCache.revision === originalImageRevision &&
    preprocessedImageCache.smoothingRadius === smoothingRadius
  ) {
    return preprocessedImageCache.imageData;
  }

  const imageData = applyGaussianBlur(originalImageData, smoothingRadius);
  preprocessedImageCache = {
    revision: originalImageRevision,
    smoothingRadius,
    imageData,
  };
  return imageData;
}

function getHistogramCache(): HistogramCache | null {
  const histogramSource = getPreprocessedImageData();
  if (!histogramSource) return null;
  if (
    histogramCache &&
    histogramCache.revision === originalImageRevision &&
    histogramCache.colorSpace === activeColorSpace &&
    histogramCache.smoothingRadius === smoothingAmount * previewScale
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
  const data = histogramSource.data;

  if (activeColorSpace === "hsv") {
    for (let i = 0; i < data.length; i += 4) {
      const [h, s, v] = rgbToHsv255(data[i], data[i + 1], data[i + 2]);
      perChannel.red[h] += 1;
      perChannel.green[s] += 1;
      perChannel.blue[v] += 1;
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
    smoothingRadius: smoothingAmount * previewScale,
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
  if (!isSyncableChannel(source)) return;
  normalizeChannelState(source);
  const level = channelLevels[source];
  const thresholds = [...channelThresholds[source]];
  const outputs = [...channelOutputs[source]];

  syncGroupForColorSpace(activeColorSpace).forEach((channel) => {
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
    panel.classList.toggle("hidden", !isVisibleWhenSynced(channel));
  });

  updateModeLabels();
  applyChannelTitles();
}

function toggleLockChannels(enabled: boolean) {
  lockChannelsEnabled = enabled;
  if (lockToggle && lockToggle.checked !== enabled) {
    lockToggle.checked = enabled;
  }
  if (lockChannelsEnabled) {
    synchronizeChannels(syncBaseChannelForColorSpace(activeColorSpace));
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

function captureEngineSettings(): EngineSettingsCache {
  return {
    colorSpace: activeColorSpace,
    smoothing: smoothingAmount,
    posterizeAlpha: posterizeAlphaEnabled,
    lockChannels: lockChannelsEnabled,
  };
}

function restoreEngineSettings(settings: EngineSettingsCache) {
  activeColorSpace = settings.colorSpace;
  smoothingAmount = settings.smoothing;
  posterizeAlphaEnabled = settings.posterizeAlpha;
  lockChannelsEnabled =
    activeProcessingMode === "voronoi" ? false : settings.lockChannels;

  if (colorSpaceSelect) colorSpaceSelect.value = activeColorSpace;
  syncSmoothingUi();
  if (posterizeAlphaToggle) {
    posterizeAlphaToggle.checked = posterizeAlphaEnabled;
  }
  if (lockToggle) {
    lockToggle.checked = lockChannelsEnabled;
  }
  preprocessedImageCache = null;
  histogramCache = null;
}

function setProcessingMode(next: ProcessingMode) {
  if (activeProcessingMode === next) {
    renderQuantizerControls();
    return;
  }
  engineSettingsCache[activeProcessingMode] = captureEngineSettings();
  activeProcessingMode = next;
  const cachedSettings = engineSettingsCache[next];
  if (cachedSettings) {
    restoreEngineSettings(cachedSettings);
  }
  if (activeProcessingMode === "voronoi") {
    rehydrateVoronoiSitesForActiveColorSpace();
  }
  if (processingModeSelect) processingModeSelect.value = next;
  updateModeLabels();
  applyLockStateToUI();
  renderAllBandControls();
  updateAllHistograms();
  renderQuantizerControls();
  renderPosterized();
}

function setVoronoiEditMode(next: VoronoiEditMode) {
  if (activeVoronoiEditMode === next) {
    renderQuantizerControls();
    return;
  }
  activeVoronoiEditMode = next;
  updateModeLabels();
  updateAllHistograms();
  renderQuantizerControls();
}

function touchQuantizerState() {
  quantizerRevision += 1;
}

function activeQuantizerGroup(): QuantizationGroup | undefined {
  return (
    quantizerGroups.find((group) => group.id === activeQuantizerGroupId) ??
    quantizerGroups[0]
  );
}

function activeBezierRegion(): BezierRegion | undefined {
  return bezierRegions.find((region) => region.id === activeBezierRegionId);
}

function resetQuantizerState() {
  const group = createQuantizationGroup("Index 1", { r: 0, g: 150, b: 255 });
  quantizerGroups = [group];
  activeQuantizerGroupId = group.id;
  activeQuantizerSiteId = null;
  bezierRegions = [];
  activeBezierRegionId = null;
  touchQuantizerState();
  updateAllHistograms();
  renderQuantizerControls();
  renderPosterized();
}

function addQuantizerGroup() {
  const group = createQuantizationGroup(
    `Index ${quantizerGroups.length + 1}`,
    randomReadableColor(),
  );
  quantizerGroups.push(group);
  activeQuantizerGroupId = group.id;
  activeQuantizerSiteId = null;
  touchQuantizerState();
  updateAllHistograms();
  renderQuantizerControls();
}

function deleteActiveQuantizerGroup() {
  if (quantizerGroups.length <= 1) {
    const group = quantizerGroups[0];
    group.sites = [];
    group.outputCoords = {};
    group.outputFollowsSite = {};
    activeQuantizerSiteId = null;
  } else {
    quantizerGroups = quantizerGroups.filter(
      (group) => group.id !== activeQuantizerGroupId,
    );
    activeQuantizerGroupId = quantizerGroups[0].id;
    activeQuantizerSiteId = null;
  }
  touchQuantizerState();
  updateAllHistograms();
  renderQuantizerControls();
  renderPosterized();
}

function addBezierRegion() {
  const region = createBezierRegion(
    `Index ${bezierRegions.length + 1}`,
    activeColorSpace,
    randomReadableColor(),
  );
  bezierRegions.push(region);
  activeBezierRegionId = region.id;
  touchQuantizerState();
  updateAllHistograms();
  renderQuantizerControls();
}

function deleteActiveBezierRegion() {
  if (!activeBezierRegionId) return;
  bezierRegions = bezierRegions.filter(
    (region) => region.id !== activeBezierRegionId,
  );
  activeBezierRegionId =
    bezierRegions.find((region) => region.colorSpace === activeColorSpace)
      ?.id ?? null;
  touchQuantizerState();
  updateAllHistograms();
  renderQuantizerControls();
  renderPosterized();
}

function closeActiveBezierRegion() {
  const region = activeBezierRegion();
  if (!region) return;
  projectionPanelsForSpace(activeColorSpace).forEach((panel) => {
    const projection = region.projections[panel];
    if (projection && projection.nodes.length >= 3) {
      projection.closed = true;
    }
  });
  touchQuantizerState();
  updateAllHistograms();
  renderQuantizerControls();
  renderPosterized();
}

function handleQuantizerStagePointer(event: PointerEvent): boolean {
  if (activeProcessingMode === "threshold") return false;
  const target = event.target as HTMLElement | null;
  if (target?.closest(".comparison-slider")) return false;

  if (activeProcessingMode === "voronoi") {
    if (activeVoronoiEditMode === "target") return false;

    const site = nearestImageSite(event.clientX, event.clientY);
    if (site) {
      activeQuantizerSiteId = site.id;
      activeQuantizerGroupId = site.groupId;
      draggedImageSite = { siteId: site.id, pointerId: event.pointerId };
      comparisonStage?.setPointerCapture(event.pointerId);
      renderQuantizerControls();
      renderQuantizerOverlay();
      event.preventDefault();
      return true;
    }
    const sample = samplePreviewAverage(event.clientX, event.clientY);
    if (sample) createSiteFromSample(sample);
  } else {
    const sample = samplePreviewPixel(event.clientX, event.clientY);
    if (sample) addBezierBoundaryPoint(sample.rgb);
  }
  event.preventDefault();
  return true;
}

function findQuantizerSite(siteId: string | null) {
  if (!siteId) return null;
  return (
    quantizerGroups
      .flatMap((group) => group.sites)
      .find((site) => site.id === siteId) ?? null
  );
}

function nearestImageSite(clientX: number, clientY: number) {
  if (activeProcessingMode !== "voronoi" || !originalImageData) {
    return null;
  }
  const layout = previewImageLayout();
  if (!layout) return null;
  const hitRadius = 12;
  const sites = completedSites(quantizerGroups, activeColorSpace).filter(
    (site) => site.samplePoint,
  );
  const sortedSites = [
    ...sites.filter((site) => site.groupId === activeQuantizerGroupId),
    ...sites.filter((site) => site.groupId !== activeQuantizerGroupId),
  ];

  let bestSite: (typeof sortedSites)[number] | null = null;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const site of sortedSites) {
    if (!site.samplePoint) continue;
    const x = layout.left + site.samplePoint.x * layout.width;
    const y = layout.top + site.samplePoint.y * layout.height;
    const distance = Math.hypot(clientX - x, clientY - y);
    if (distance <= hitRadius && distance < bestDistance) {
      bestSite = site;
      bestDistance = distance;
    }
  }
  return bestSite;
}

function updateDraggedImageSite(clientX: number, clientY: number) {
  if (!draggedImageSite) return;
  const site = findQuantizerSite(draggedImageSite.siteId);
  if (!site) return;
  const sample = samplePreviewAverage(clientX, clientY);
  if (!sample) return;
  updateSiteFromSample(site, sample);
}

type PreviewPixel = { rgb: RGB; x: number; y: number };
type VoronoiSample = {
  rgb: RGB;
  coord: ReturnType<typeof siteCoordFromRgb>;
  samplePoint: { x: number; y: number };
};

function previewPixelFromClient(
  clientX: number,
  clientY: number,
): { x: number; y: number } | null {
  if (!originalImageData) return null;
  const layout = previewImageLayout();
  if (!layout) return null;
  const x = Math.floor(
    ((clientX - layout.left) / layout.width) * originalImageData.width,
  );
  const y = Math.floor(
    ((clientY - layout.top) / layout.height) * originalImageData.height,
  );
  if (
    x < 0 ||
    y < 0 ||
    x >= originalImageData.width ||
    y >= originalImageData.height
  ) {
    return null;
  }
  return { x, y };
}

function previewImageLayout(): {
  left: number;
  top: number;
  width: number;
  height: number;
} | null {
  if (!originalImageData || !sourceCanvas) return null;
  const rect = sourceCanvas.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;

  const imageAspect = originalImageData.width / originalImageData.height;
  const boxAspect = rect.width / rect.height;
  let width = rect.width;
  let height = rect.height;
  let left = rect.left;
  let top = rect.top;

  if (boxAspect > imageAspect) {
    width = rect.height * imageAspect;
    left += (rect.width - width) / 2;
  } else if (boxAspect < imageAspect) {
    height = rect.width / imageAspect;
    top += (rect.height - height) / 2;
  }

  return { left, top, width, height };
}

function samplePreviewPixel(
  clientX: number,
  clientY: number,
): PreviewPixel | null {
  if (!originalImageData) return null;
  const point = previewPixelFromClient(clientX, clientY);
  if (!point) return null;
  const { x, y } = point;
  const index = (y * originalImageData.width + x) * 4;
  const data = originalImageData.data;
  if (data[index + 3] === 0) return null;
  return {
    x,
    y,
    rgb: { r: data[index], g: data[index + 1], b: data[index + 2] },
  };
}

function samplePreviewAverageAtPixel(
  center: { x: number; y: number },
): VoronoiSample | null {
  if (!originalImageData) return null;
  const { width, height, data } = originalImageData;
  const samplePoint = {
    x: width <= 1 ? 0 : center.x / (width - 1),
    y: height <= 1 ? 0 : center.y / (height - 1),
  };
  const radius = sanitizeVoronoiSampleRadius(voronoiSampleRadius) - 1;
  const radiusSquared = radius * radius;
  let count = 0;
  let red = 0;
  let green = 0;
  let blue = 0;
  let hueX = 0;
  let hueY = 0;
  let saturation = 0;
  let value = 0;

  for (
    let y = Math.max(0, center.y - radius);
    y <= Math.min(height - 1, center.y + radius);
    y += 1
  ) {
    for (
      let x = Math.max(0, center.x - radius);
      x <= Math.min(width - 1, center.x + radius);
      x += 1
    ) {
      const dx = x - center.x;
      const dy = y - center.y;
      if (dx * dx + dy * dy > radiusSquared) continue;

      const index = (y * width + x) * 4;
      if (data[index + 3] === 0) continue;

      const r = data[index];
      const g = data[index + 1];
      const b = data[index + 2];
      red += r;
      green += g;
      blue += b;
      count += 1;

      if (activeColorSpace === "hsv") {
        const [h, s, v] = rgbToHsv255(r, g, b);
        const radians = (h / 255) * Math.PI * 2;
        hueX += Math.cos(radians) * s;
        hueY += Math.sin(radians) * s;
        saturation += s;
        value += v;
      }
    }
  }

  if (count === 0) return null;

  if (activeColorSpace === "hsv") {
    const averagedS = Math.round(saturation / count);
    const averagedV = Math.round(value / count);
    const hueDegrees =
      hueX === 0 && hueY === 0
        ? 0
        : ((Math.atan2(hueY, hueX) * 180) / Math.PI + 360) % 360;
    const coord = {
      h: Math.min(359, Math.floor(hueDegrees)),
      s: averagedS,
      v: averagedV,
    };
    const [r, g, b] = hsv255ToRgb((coord.h / 360) * 255, coord.s, coord.v);
    return { rgb: { r, g, b }, coord, samplePoint };
  }

  const rgb = {
    r: Math.round(red / count),
    g: Math.round(green / count),
    b: Math.round(blue / count),
  };
  return { rgb, coord: siteCoordFromRgb("rgb", rgb), samplePoint };
}

function samplePreviewAverage(
  clientX: number,
  clientY: number,
): VoronoiSample | null {
  if (!originalImageData) return null;
  const center = previewPixelFromClient(clientX, clientY);
  if (!center) return null;
  return samplePreviewAverageAtPixel(center);
}

function samplePreviewAverageAtSamplePoint(
  samplePoint: { x: number; y: number },
): VoronoiSample | null {
  if (!originalImageData) return null;
  const x = Math.round(
    Math.max(0, Math.min(1, samplePoint.x)) * (originalImageData.width - 1),
  );
  const y = Math.round(
    Math.max(0, Math.min(1, samplePoint.y)) * (originalImageData.height - 1),
  );
  return samplePreviewAverageAtPixel({ x, y });
}

function rehydrateVoronoiSitesForActiveColorSpace() {
  if (!originalImageData) return;

  for (const group of quantizerGroups) {
    const sourceSite =
      group.sites.find(
        (site) => site.colorSpace === activeColorSpace && site.samplePoint,
      ) ?? group.sites.find((site) => site.samplePoint);
    if (!sourceSite?.samplePoint) continue;

    const sample = samplePreviewAverageAtSamplePoint(sourceSite.samplePoint);
    if (!sample) continue;

    const existingSite = group.sites.find(
      (site) => site.colorSpace === activeColorSpace,
    );
    if (existingSite) {
      existingSite.coord = sample.coord;
      existingSite.samplePoint = sample.samplePoint;
    } else {
      const site = createSite(group.id, activeColorSpace, sample.coord);
      site.samplePoint = sample.samplePoint;
      group.sites.push(site);
    }
  }

  touchQuantizerState();
}

function createSiteFromSample(sample: VoronoiSample) {
  const { rgb, coord } = sample;
  const duplicate = completedSites(quantizerGroups, activeColorSpace).find(
    (site) => sampledCoordDistance(site.coord, coord) <= 0.0008,
  );
  if (duplicate) {
    activeQuantizerSiteId = duplicate.id;
    activeQuantizerGroupId = duplicate.groupId;
    updateAllHistograms();
    renderQuantizerControls();
    return;
  }

  const group = activeQuantizerGroup() ?? quantizerGroups[0];
  const site = createSite(group.id, activeColorSpace, coord);
  site.samplePoint = sample.samplePoint;
  group.sites = group.sites.filter(
    (candidate) => candidate.colorSpace !== activeColorSpace,
  );
  group.sites.push(site);
  group.paletteColor = rgb;
  group.outputCoords[activeColorSpace] = coord;
  group.outputFollowsSite[activeColorSpace] = true;
  activeQuantizerSiteId = site.id;
  activeQuantizerGroupId = group.id;
  touchQuantizerState();
  updateAllHistograms();
  renderQuantizerControls();
  renderQuantizerOverlay();
  renderPosterized();
}

function updateSiteFromSample(
  site: NonNullable<ReturnType<typeof findQuantizerSite>>,
  sample: VoronoiSample,
) {
  const group = quantizerGroups.find(
    (candidate) => candidate.id === site.groupId,
  );
  site.coord = sample.coord;
  site.samplePoint = sample.samplePoint;
  if (group && group.outputFollowsSite[activeColorSpace] !== false) {
    group.paletteColor = sample.rgb;
    group.outputCoords[activeColorSpace] = sample.coord;
  }
  activeQuantizerSiteId = site.id;
  activeQuantizerGroupId = site.groupId;
  touchQuantizerState();
  updateAllHistograms();
  renderQuantizerControls();
  renderQuantizerOverlay();
  renderPosterized();
}

function activeIndexLabel(index: number): string {
  return `Index ${index + 1}`;
}

function groupIndex(groupId: string): number {
  return Math.max(
    0,
    quantizerGroups.findIndex((candidate) => candidate.id === groupId),
  );
}

function regionIndex(regionId: string): number {
  return Math.max(
    0,
    bezierRegions
      .filter((candidate) => candidate.colorSpace === activeColorSpace)
      .findIndex((candidate) => candidate.id === regionId),
  );
}

function targetChannelValue(channel: Channel): number | null {
  if (activeProcessingMode === "voronoi") {
    const group = activeQuantizerGroup();
    if (!group) return null;
    if (activeColorSpace === "rgb") {
      return (
        {
          red: group.paletteColor.r,
          green: group.paletteColor.g,
          blue: group.paletteColor.b,
        } as const
      )[channel];
    }
    const coord =
      (group.outputCoords.hsv as HsvSiteCoord | undefined) ??
      (siteCoordFromRgb("hsv", group.paletteColor) as HsvSiteCoord);
    return channel === "red"
      ? Math.round((coord.h / 360) * 255)
      : channel === "green"
        ? coord.s
        : coord.v;
  }

  if (activeProcessingMode === "bezier") {
    const region = activeBezierRegion();
    if (!region) return null;
    if (activeColorSpace === "rgb") {
      return (
        {
          red: region.outputColor.r,
          green: region.outputColor.g,
          blue: region.outputColor.b,
        } as const
      )[channel];
    }
    const [h, s, v] = rgbToHsv255(
      region.outputColor.r,
      region.outputColor.g,
      region.outputColor.b,
    );
    return channel === "red" ? h : channel === "green" ? s : v;
  }

  return null;
}

function setTargetChannelValue(channel: Channel, value: number) {
  if (activeProcessingMode === "threshold") return;
  const clamped = Math.max(0, Math.min(255, Math.round(value)));

  if (activeProcessingMode === "voronoi") {
    const group = activeQuantizerGroup();
    if (!group) return;

    if (activeColorSpace === "rgb") {
      const next = { ...group.paletteColor };
      if (channel === "red") next.r = clamped;
      if (channel === "green") next.g = clamped;
      if (channel === "blue") next.b = clamped;
      group.paletteColor = next;
      group.outputCoords.rgb = { ...next };
    } else {
      const coord = {
        ...(((group.outputCoords.hsv as HsvSiteCoord | undefined) ??
          siteCoordFromRgb("hsv", group.paletteColor)) as HsvSiteCoord),
      };
      if (channel === "red")
        coord.h = Math.min(359, Math.round((clamped / 255) * 360));
      if (channel === "green") coord.s = clamped;
      if (channel === "blue") coord.v = clamped;
      group.outputCoords.hsv = coord;
      group.paletteColor = hsvCoordToRgb(coord);
    }
    group.outputFollowsSite[activeColorSpace] = false;
  } else {
    const region = activeBezierRegion();
    if (!region) return;

    if (activeColorSpace === "rgb") {
      const next = { ...region.outputColor };
      if (channel === "red") next.r = clamped;
      if (channel === "green") next.g = clamped;
      if (channel === "blue") next.b = clamped;
      region.outputColor = next;
    } else {
      const hsv = rgbToHsv255(
        region.outputColor.r,
        region.outputColor.g,
        region.outputColor.b,
      );
      if (channel === "red") hsv[0] = clamped;
      if (channel === "green") hsv[1] = clamped;
      if (channel === "blue") hsv[2] = clamped;
      const [r, g, b] = hsv255ToRgb(hsv[0], hsv[1], hsv[2]);
      region.outputColor = { r, g, b };
    }
  }

  touchQuantizerState();
  updateAllHistograms();
  renderQuantizerControls();
  renderQuantizerOverlay();
  renderPosterized();
}

function usesHistogramTargetControls(): boolean {
  return (
    activeProcessingMode === "bezier" ||
    (activeProcessingMode === "voronoi" && activeVoronoiEditMode === "target")
  );
}

function addBezierBoundaryPoint(rgb: RGB) {
  let region = activeBezierRegion();
  if (!region || region.colorSpace !== activeColorSpace) {
    addBezierRegion();
    region = activeBezierRegion();
  }
  if (!region) return;

  const coord = siteCoordFromRgb(activeColorSpace, rgb);
  projectionPanelsForSpace(activeColorSpace).forEach((panel) => {
    const projection = projectionForRegion(region, panel);
    if (projection.closed) return;
    const point = projectCoordToPanelPoint(panel, coord);
    projection.nodes.push(createBezierNode(point.x, point.y));
  });
  region.outputColor = rgb;
  touchQuantizerState();
  renderQuantizerControls();
  renderPosterized();
}

function renderQuantizerControls() {
  comparisonStage?.classList.toggle(
    "quantizer-editing",
    activeProcessingMode === "bezier" ||
      (activeProcessingMode === "voronoi" &&
        activeVoronoiEditMode === "source"),
  );
  histogramsSection?.classList.toggle(
    "quantizer-mode",
    activeProcessingMode !== "threshold",
  );
  histogramsSection?.classList.toggle(
    "quantizer-target-mode",
    usesHistogramTargetControls(),
  );
  histogramsSection?.classList.toggle(
    "voronoi-color-bars",
    activeProcessingMode === "voronoi",
  );
  controlsPane?.classList.toggle(
    "target-assignment",
    activeProcessingMode === "voronoi" && activeVoronoiEditMode === "target",
  );
  quantizerList?.classList.toggle(
    "palette-grid",
    activeProcessingMode === "voronoi",
  );
  if (quantizerPanel) {
    quantizerPanel.hidden = activeProcessingMode === "threshold";
  }
  if (quantizerTitle) {
    quantizerTitle.textContent =
      activeProcessingMode === "bezier" ? "Bézier Volumes" : "Voronoi";
  }
  if (voronoiActions) {
    voronoiActions.hidden = activeProcessingMode !== "voronoi";
  }
  if (bezierActions) {
    bezierActions.hidden = activeProcessingMode !== "bezier";
  }
  voronoiSourceModeBtn?.classList.toggle(
    "active",
    activeVoronoiEditMode === "source",
  );
  voronoiTargetModeBtn?.classList.toggle(
    "active",
    activeVoronoiEditMode === "target",
  );
  voronoiSourceModeBtn?.setAttribute(
    "aria-pressed",
    activeVoronoiEditMode === "source" ? "true" : "false",
  );
  voronoiTargetModeBtn?.setAttribute(
    "aria-pressed",
    activeVoronoiEditMode === "target" ? "true" : "false",
  );
  updateToolbarModeState();
  updateModeLabels();
  if (activeProcessingMode === "threshold") {
    renderQuantizerOverlay();
    return;
  }
  if (activeProcessingMode === "voronoi") {
    renderVoronoiControls();
  } else {
    renderBezierControls();
  }
  renderQuantizerStats(null);
  renderQuantizerOverlay();
}

function renderQuantizerOverlay() {
  if (!quantizerOverlayCanvas || !quantizerOverlayCtx) return;
  if (!originalImageData || activeProcessingMode !== "voronoi") {
    quantizerOverlayCtx.clearRect(
      0,
      0,
      quantizerOverlayCanvas.width,
      quantizerOverlayCanvas.height,
    );
    return;
  }

  if (quantizerOverlayCanvas.width !== originalImageData.width) {
    quantizerOverlayCanvas.width = originalImageData.width;
  }
  if (quantizerOverlayCanvas.height !== originalImageData.height) {
    quantizerOverlayCanvas.height = originalImageData.height;
  }

  const ctx = quantizerOverlayCtx;
  const { width, height } = quantizerOverlayCanvas;
  ctx.clearRect(0, 0, width, height);

  const overlaySites = completedSites(quantizerGroups, activeColorSpace).filter(
    (site) =>
      activeVoronoiEditMode !== "target" ||
      site.groupId === activeQuantizerGroupId,
  );

  for (const site of overlaySites) {
    if (!site.samplePoint) continue;
    const group = quantizerGroups.find(
      (candidate) => candidate.id === site.groupId,
    );
    const x = site.samplePoint.x * (width - 1);
    const y = site.samplePoint.y * (height - 1);
    const active = site.id === activeQuantizerSiteId;

    if (active) {
      ctx.save();
      ctx.strokeStyle = "rgba(255, 255, 255, 0.38)";
      ctx.setLineDash([4, 4]);
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(x, y, voronoiSampleRadius, 0, Math.PI * 2);
      ctx.stroke();
      ctx.restore();
    }

    ctx.save();
    ctx.globalAlpha = activeVoronoiEditMode === "target" ? 0.74 : 1;
    ctx.shadowColor = "rgba(0, 0, 0, 0.7)";
    ctx.shadowBlur = 5;
    ctx.fillStyle = group ? rgbToHex(group.paletteColor) : "#ffffff";
    ctx.strokeStyle = active ? "#ffffff" : "rgba(255, 255, 255, 0.75)";
    ctx.lineWidth = active ? 3 : 2;
    ctx.beginPath();
    ctx.arc(x, y, active ? 7 : 5.5, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    ctx.restore();
  }
}

function renderVoronoiControls() {
  if (!quantizerEditor || !quantizerList) return;
  quantizerEditor.replaceChildren();
  quantizerList.replaceChildren();

  quantizerEditor.appendChild(createSampleRadiusRow());

  const group = activeQuantizerGroup();
  if (group) {
    const indexTitle = document.createElement("div");
    indexTitle.className = "control-section-title";
    indexTitle.textContent = activeIndexLabel(groupIndex(group.id));
    quantizerEditor.appendChild(indexTitle);

    quantizerEditor.appendChild(
      createColorRow("Target Color", group.paletteColor, (value) => {
        group.paletteColor = value;
        group.outputCoords[activeColorSpace] = outputCoordFromRgb(
          activeColorSpace,
          value,
        );
        group.outputFollowsSite[activeColorSpace] = false;
        touchQuantizerState();
        updateAllHistograms();
        renderPosterized();
        renderQuantizerControls();
      }),
    );
    const site = group.sites.find(
      (candidate) => candidate.id === activeQuantizerSiteId,
    );
    if (site) {
      quantizerEditor.appendChild(
        createNumberRow("Source Weight", site.mass, -10, 10, 0.1, (value) => {
          site.mass = Math.max(-10, Math.min(10, value));
          touchQuantizerState();
          renderPosterized();
        }),
      );
    }
  }

  quantizerGroups.forEach((candidate) => {
    const item = document.createElement("button");
    item.type = "button";
    item.className = `quantizer-item ${
      candidate.id === activeQuantizerGroupId ? "active" : ""
    }`;
    item.title = `${activeIndexLabel(groupIndex(candidate.id))}: ${
      candidate.sites.filter((site) => site.colorSpace === activeColorSpace)
        .length > 0
        ? "source set"
        : "no source"
    }`;
    item.setAttribute("aria-label", item.title);
    const swatch = document.createElement("span");
    swatch.className = "quantizer-swatch";
    const sourceColor = sourceColorForGroup(candidate);
    const sourceHex = sourceColor ? rgbToHex(sourceColor) : "#111418";
    const targetHex = rgbToHex(candidate.paletteColor);
    swatch.style.background = `linear-gradient(to right, ${sourceHex} 0 50%, ${targetHex} 50% 100%)`;
    const name = document.createElement("span");
    name.className = "quantizer-name";
    name.textContent = activeIndexLabel(groupIndex(candidate.id));
    const meta = document.createElement("span");
    meta.className = "quantizer-meta";
    meta.textContent =
      candidate.sites.filter((site) => site.colorSpace === activeColorSpace)
        .length > 0
        ? "source set"
        : "no source";
    item.append(swatch, name, meta);
    item.addEventListener("click", () => {
      activeQuantizerGroupId = candidate.id;
      activeQuantizerSiteId =
        candidate.sites.find((site) => site.colorSpace === activeColorSpace)
          ?.id ?? null;
      updateAllHistograms();
      renderQuantizerControls();
    });
    quantizerList.appendChild(item);
  });
}

function sanitizeVoronoiSampleRadius(value: unknown): number {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return 1;
  return Math.max(1, Math.min(10, Math.round(numeric)));
}

function createSampleRadiusRow(): HTMLElement {
  const label = document.createElement("label");
  label.className = "field-block";

  const span = document.createElement("span");
  span.textContent = "Sample Radius";

  const controls = document.createElement("div");
  controls.className = "range-with-value";

  const slider = document.createElement("input");
  slider.type = "range";
  slider.min = "1";
  slider.max = "10";
  slider.step = "1";
  slider.value = sanitizeVoronoiSampleRadius(voronoiSampleRadius).toString();

  const number = document.createElement("input");
  number.type = "number";
  number.min = "1";
  number.max = "10";
  number.step = "1";
  number.value = slider.value;
  number.setAttribute("aria-label", "Voronoi sample radius");

  const apply = (value: unknown) => {
    voronoiSampleRadius = sanitizeVoronoiSampleRadius(value);
    const next = voronoiSampleRadius.toString();
    slider.value = next;
    number.value = next;
    updateModeLabels();
  };

  slider.addEventListener("input", () => apply(slider.value));
  number.addEventListener("change", () => apply(number.value));
  commitNumberInputOnEnter(number, () => apply(number.value));

  controls.append(slider, number);
  label.append(span, controls);
  return label;
}

function renderBezierControls() {
  if (!quantizerEditor || !quantizerList) return;
  quantizerEditor.replaceChildren();
  quantizerList.replaceChildren();

  const region = activeBezierRegion();
  if (region) {
    const indexTitle = document.createElement("div");
    indexTitle.className = "control-section-title";
    indexTitle.textContent = activeIndexLabel(regionIndex(region.id));
    quantizerEditor.appendChild(indexTitle);

    quantizerEditor.appendChild(
      createColorRow("Target Color", region.outputColor, (value) => {
        region.outputColor = value;
        touchQuantizerState();
        updateAllHistograms();
        renderPosterized();
        renderQuantizerControls();
      }),
    );
    quantizerEditor.appendChild(
      createNumberRow("Source Weight", region.mass, -10, 10, 0.1, (value) => {
        region.mass = Math.max(-10, Math.min(10, value));
        touchQuantizerState();
        renderPosterized();
      }),
    );
    quantizerEditor.appendChild(
      createNumberRow(
        "Index Priority",
        region.priority,
        -100,
        100,
        1,
        (value) => {
          region.priority = Math.round(value);
          touchQuantizerState();
          renderPosterized();
        },
      ),
    );
  }

  bezierRegions
    .filter((candidate) => candidate.colorSpace === activeColorSpace)
    .forEach((candidate) => {
      const item = document.createElement("button");
      item.type = "button";
      item.className = `quantizer-item ${
        candidate.id === activeBezierRegionId ? "active" : ""
      }`;
      const swatch = document.createElement("span");
      swatch.className = "quantizer-swatch";
      swatch.style.background = rgbToHex(candidate.outputColor);
      const name = document.createElement("span");
      name.className = "quantizer-name";
      name.textContent = activeIndexLabel(regionIndex(candidate.id));
      const meta = document.createElement("span");
      meta.className = "quantizer-meta";
      const points =
        candidate.projections[projectionPanelsForSpace(activeColorSpace)[0]]
          ?.nodes.length ?? 0;
      meta.textContent = points > 0 ? `${points} source points` : "no source";
      item.append(swatch, name, meta);
      item.addEventListener("click", () => {
        activeBezierRegionId = candidate.id;
        updateAllHistograms();
        renderQuantizerControls();
      });
      quantizerList.appendChild(item);
    });
}

function renderQuantizerStats(image: AnalyzedImage | null) {
  if (!quantizerStats || activeProcessingMode === "threshold") return;
  const activeSites = completedSites(quantizerGroups, activeColorSpace);
  const activeRegions = bezierRegions.filter(
    (region) => region.colorSpace === activeColorSpace,
  );
  const validRegions = activeRegions.filter(
    (region) => validRegionProjections(region).length > 0,
  );
  quantizerStats.textContent =
    activeProcessingMode === "voronoi"
      ? `${activeColorSpace.toUpperCase()} Voronoi: ${activeSites.length} source sample${activeSites.length === 1 ? "" : "s"} across ${quantizerGroups.length} index${quantizerGroups.length === 1 ? "" : "es"}${image ? `, ${image.sourceColors.length} unique source colors` : ""}.`
      : `${activeColorSpace.toUpperCase()} Bézier: ${validRegions.length}/${activeRegions.length} closed source region${activeRegions.length === 1 ? "" : "s"}${image ? `, ${image.sourceColors.length} unique source colors` : ""}.`;
}

function createTextRow(
  labelText: string,
  value: string,
  onChange: (value: string) => void,
): HTMLElement {
  const label = document.createElement("label");
  label.className = "field-block";
  const span = document.createElement("span");
  span.textContent = labelText;
  const input = document.createElement("input");
  input.type = "text";
  input.value = value;
  input.addEventListener("change", () => onChange(input.value.trim()));
  label.append(span, input);
  return label;
}

function createColorRow(
  labelText: string,
  value: RGB,
  onChange: (value: RGB) => void,
): HTMLElement {
  const label = document.createElement("label");
  label.className = "quantizer-row";
  const span = document.createElement("span");
  span.className = "control-label";
  span.textContent = labelText;
  const input = document.createElement("input");
  input.type = "color";
  input.value = rgbToHex(value);
  input.addEventListener("input", () => onChange(hexToRgb(input.value)));
  label.append(span, input);
  return label;
}

function createNumberRow(
  labelText: string,
  value: number,
  min: number,
  max: number,
  step: number,
  onChange: (value: number) => void,
): HTMLElement {
  const label = document.createElement("label");
  label.className = "quantizer-row";
  const span = document.createElement("span");
  span.className = "control-label";
  span.textContent = labelText;
  const input = document.createElement("input");
  input.type = "number";
  input.min = min.toString();
  input.max = max.toString();
  input.step = step.toString();
  input.value = value.toString();
  input.addEventListener("change", () => {
    const next = Number(input.value);
    if (Number.isFinite(next)) onChange(next);
  });
  label.append(span, input);
  return label;
}

function randomReadableColor(): RGB {
  const hue =
    (quantizerRevision * 47 +
      quantizerGroups.length * 83 +
      bezierRegions.length * 61) %
    360;
  const [r, g, b] = hsv255ToRgb((hue / 360) * 255, 180, 230);
  return { r, g, b };
}

function sampledCoordDistance(a: unknown, b: unknown): number {
  if (activeColorSpace === "rgb") {
    const left = a as { r: number; g: number; b: number };
    const right = b as { r: number; g: number; b: number };
    const dr = (left.r - right.r) / 255;
    const dg = (left.g - right.g) / 255;
    const db = (left.b - right.b) / 255;
    return dr * dr + dg * dg + db * db;
  }

  const left = a as { h: number; s: number; v: number };
  const right = b as { h: number; s: number; v: number };
  const dh =
    Math.min(Math.abs(left.h - right.h), 360 - Math.abs(left.h - right.h)) /
    360;
  const ds = (left.s - right.s) / 255;
  const dv = (left.v - right.v) / 255;
  return dh * dh + ds * ds + dv * dv;
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

function exportQuantizerGroups(): QuantizationGroup[] {
  if (activeProcessingMode !== "voronoi") {
    return quantizerGroups;
  }
  const exportSiteForColorSpace = (
    groupId: string,
    sourceSite: QuantizationGroup["sites"][number],
    colorSpace: ColorSpace,
  ) => {
    const rgb =
      sourceSite.colorSpace === "hsv"
        ? hsvCoordToRgb(sourceSite.coord as HsvSiteCoord)
        : (sourceSite.coord as RGB);
    return {
      id:
        sourceSite.colorSpace === colorSpace
          ? sourceSite.id
          : `${sourceSite.id}-${colorSpace}`,
      groupId,
      colorSpace,
      coord:
        colorSpace === "hsv"
          ? siteCoordFromRgb("hsv", rgb)
          : siteCoordFromRgb("rgb", rgb),
      mass: sourceSite.mass,
      order: sourceSite.order,
    };
  };

  return quantizerGroups.map((group) => ({
    ...group,
    outputCoords: {
      ...group.outputCoords,
      rgb: outputCoordFromRgb("rgb", group.paletteColor),
      hsv: outputCoordFromRgb("hsv", group.paletteColor),
    },
    outputFollowsSite: { ...group.outputFollowsSite },
    sites: (() => {
      const sourceSite =
        group.sites.find((site) => site.colorSpace === activeColorSpace) ??
        group.sites[0];
      if (!sourceSite) return [];
      return [
        exportSiteForColorSpace(group.id, sourceSite, "rgb"),
        exportSiteForColorSpace(group.id, sourceSite, "hsv"),
      ];
    })(),
  }));
}

function buildSettingsPayload(): ExportSettings {
  const channels: Record<Channel, ChannelSettings> = {
    red: collectChannelSettings("red"),
    green: collectChannelSettings("green"),
    blue: collectChannelSettings("blue"),
  };
  const payload: ExportSettings = {
    version: 5,
    colorSpace: activeColorSpace,
    processingMode: activeProcessingMode,
    smoothing: smoothingAmount,
    lockChannels: lockChannelsEnabled,
    posterizeAlpha: posterizeAlphaEnabled,
    channels,
    quantizer: {
      editMode: activeVoronoiEditMode,
      sampleRadius: voronoiSampleRadius,
      groups: exportQuantizerGroups(),
      activeGroupId: activeQuantizerGroupId,
      activeSiteId:
        activeProcessingMode === "voronoi" ? null : activeQuantizerSiteId,
      regions: bezierRegions,
      activeRegionId: activeBezierRegionId,
    },
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

function extractSettingsMetadataFromPng(pngData: Uint8Array): string | null {
  return (
    extractMetadataFromPng(pngData, METADATA_KEYWORD) ??
    extractMetadataFromPng(pngData, LEGACY_METADATA_KEYWORD)
  );
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
  if (!fullResolutionImageData) {
    alert("No image to save!");
    return;
  }

  const fileName = prompt(
    "Enter a filename for the image:",
    "threshiator_image",
  );
  if (fileName === null) return;

  const sanitizedFileName = fileName.trim() || "threshiator_image";

  try {
    const settings = buildSettingsPayload();
    const settingsJson = JSON.stringify(settings);
    const saveSource =
      smoothingAmount > 0
        ? applyGaussianBlur(fullResolutionImageData, smoothingAmount)
        : fullResolutionImageData;
    const saveImageData = posterizeImageData(saveSource);
    const saveCanvas = document.createElement("canvas");
    saveCanvas.width = saveImageData.width;
    saveCanvas.height = saveImageData.height;
    const saveCtx = saveCanvas.getContext("2d");
    if (!saveCtx) {
      throw new Error("Failed to create export canvas");
    }
    saveCtx.putImageData(saveImageData, 0, 0);

    // Get PNG data from canvas
    const originalBlob = await new Promise<Blob>((resolve, reject) => {
      saveCanvas.toBlob(
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
      METADATA_KEYWORD,
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

      const metadata = extractSettingsMetadataFromPng(uint8Array);
      if (!metadata) {
        throw new Error("No Threshiator settings found in this PNG file");
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
  link.download = "ThreshiatorSettings.json";
  document.body.appendChild(link);
  link.click();
  document.body.removeChild(link);
  URL.revokeObjectURL(url);
}

function applySettings(settings: ExportSettings) {
  if (settings.colorSpace === "hsv" || settings.colorSpace === "rgb") {
    setColorSpace(settings.colorSpace, {
      saveToStorage: false,
      rerender: false,
    });
  } else {
    setColorSpace("rgb", { saveToStorage: false, rerender: false });
  }
  setSmoothingAmount(sanitizeSmoothing(settings.smoothing ?? 0), {
    saveToStorage: false,
    rerender: false,
  });

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
      Math.min(
        MAX_LEVELS,
        Math.floor(alphaConfig?.levels ?? channelLevels.alpha),
      ),
    );
    channelLevels.alpha = level;
    channelThresholds.alpha = sanitizeThresholds(
      alphaConfig?.thresholds,
      level,
    );
    channelOutputs.alpha = sanitizeOutputs(alphaConfig?.outputs, level);
    syncChannelLabel("alpha", level);
  }

  if (settings.quantizer) {
    activeVoronoiEditMode =
      settings.quantizer.editMode === "target" ? "target" : "source";
    voronoiSampleRadius = sanitizeVoronoiSampleRadius(
      settings.quantizer.sampleRadius ?? voronoiSampleRadius,
    );
    if (
      Array.isArray(settings.quantizer.groups) &&
      settings.quantizer.groups.length > 0
    ) {
      quantizerGroups = settings.quantizer.groups;
      activeQuantizerGroupId =
        quantizerGroups.find(
          (group) => group.id === settings.quantizer?.activeGroupId,
        )?.id ?? quantizerGroups[0].id;
      activeQuantizerSiteId = settings.quantizer.activeSiteId ?? null;
    }
    if (Array.isArray(settings.quantizer.regions)) {
      bezierRegions = settings.quantizer.regions;
      activeBezierRegionId =
        bezierRegions.find(
          (region) => region.id === settings.quantizer?.activeRegionId,
        )?.id ?? null;
    }
    touchQuantizerState();
  }

  activeProcessingMode =
    settings.processingMode === "voronoi"
      ? settings.processingMode
      : "threshold";
  if (activeProcessingMode === "voronoi") {
    rehydrateVoronoiSitesForActiveColorSpace();
    activeQuantizerSiteId =
      activeQuantizerGroup()?.sites.find(
        (site) => site.colorSpace === activeColorSpace,
      )?.id ?? null;
  }
  if (processingModeSelect) processingModeSelect.value = activeProcessingMode;

  togglePosterizeAlpha(alphaEnabled, { saveToStorage: false, rerender: false });
  toggleLockChannels(
    activeProcessingMode === "voronoi" ? false : (settings.lockChannels ?? false),
  );
  renderQuantizerControls();
}

type DragState = {
  channel: UiChannel;
  index: number;
  pointerId: number;
  type: "threshold" | "level" | "target";
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
  if (!outputs || index < 0 || index >= outputs.length) return null;
  const clamped = Math.max(0, Math.min(255, Math.round(value)));
  outputs[index] = clamped;
  if (lockChannelsEnabled && isSyncableChannel(channel)) {
    syncOtherChannelsFrom(channel as Channel);
    updateAllHistograms();
  } else {
    updateChannelHistogram(channel);
  }
  renderPosterized();
  return clamped;
}

/**
 * Render explicit threshold and output controls for a channel. The histogram
 * canvas remains draggable, but these rows make every editable value visible.
 */
function renderBandControls(channel: UiChannel) {
  const container = bandControlContainers[channel];
  if (!container) return;

  ensureChannelOutputs(channel);

  const thresholds = channelThresholds[channel];
  const outputs = channelOutputs[channel];

  container.innerHTML = "";

  const createTitle = (text: string) => {
    const title = document.createElement("div");
    title.className = "control-section-title";
    title.textContent = text;
    return title;
  };

  const createValueRow = (
    labelText: string,
    value: number,
    min: number,
    max: number,
    applyValue: (next: number, rerenderControls: boolean) => number | null,
  ) => {
    const row = document.createElement("div");
    row.className = "control-row";

    const label = document.createElement("span");
    label.className = "control-label";
    label.textContent = labelText;

    const slider = document.createElement("input");
    slider.type = "range";
    const displayValue = rawToControlValue(channel, value);
    const displayMin = rawToControlValue(channel, min);
    const displayMax = rawToControlValue(channel, max);

    slider.min = displayMin.toString();
    slider.max = displayMax.toString();
    slider.value = displayValue.toString();
    slider.step = "1";

    const number = document.createElement("input");
    number.type = "number";
    number.min = displayMin.toString();
    number.max = displayMax.toString();
    number.value = displayValue.toString();
    number.step = "1";

    const syncInputs = (
      next: number,
      options: { syncNumber?: boolean } = {},
    ) => {
      slider.value = next.toString();
      if (options.syncNumber ?? true) {
        number.value = next.toString();
      }
    };

    const clampDisplayValue = (next: number) =>
      Math.max(displayMin, Math.min(displayMax, Math.round(next)));

    const applyDisplayValue = (
      next: number,
      options: { rerenderControls: boolean; syncNumber: boolean },
    ) => {
      const displayNext = clampDisplayValue(next);
      const applied = applyValue(
        controlToRawValue(channel, displayNext),
        options.rerenderControls,
      );
      if (applied === null) return;

      const nextDisplay = usesHueDegreeControls(channel)
        ? displayNext
        : rawToControlValue(channel, applied);
      syncInputs(nextDisplay, { syncNumber: options.syncNumber });
    };

    const applyFrom = (
      source: HTMLInputElement,
      options: { rerenderControls: boolean; syncNumber: boolean },
    ) => {
      if (source.value === "") return;
      const raw = Number(source.value);
      if (!Number.isFinite(raw)) return;
      applyDisplayValue(raw, options);
    };

    slider.addEventListener("input", () =>
      applyFrom(slider, { rerenderControls: false, syncNumber: true }),
    );
    slider.addEventListener("change", () =>
      applyFrom(slider, { rerenderControls: true, syncNumber: true }),
    );
    number.addEventListener("change", () =>
      applyFrom(number, { rerenderControls: false, syncNumber: true }),
    );
    commitNumberInputOnEnter(number, () =>
      applyFrom(number, { rerenderControls: false, syncNumber: true }),
    );

    row.appendChild(label);
    row.appendChild(slider);
    row.appendChild(number);
    return row;
  };

  if (thresholds.length > 0) {
    const thresholdPanel = document.createElement("div");
    thresholdPanel.className = "threshold-panel";
    thresholdPanel.appendChild(
      createTitle(
        usesHueDegreeControls(channel) ? "Thresholds (deg)" : "Thresholds",
      ),
    );

    thresholds.forEach((value, index) => {
      const minValue = index === 0 ? 1 : thresholds[index - 1] + 1;
      const maxValue =
        index === thresholds.length - 1 ? 254 : thresholds[index + 1] - 1;

      thresholdPanel.appendChild(
        createValueRow(
          `T${index + 1}`,
          value,
          minValue,
          maxValue,
          (next, rerenderControls) =>
            setChannelThreshold(channel, index, next, { rerenderControls }),
        ),
      );
    });

    container.appendChild(thresholdPanel);
  }

  const outputPanel = document.createElement("div");
  outputPanel.className = "output-panel";
  outputPanel.appendChild(
    createTitle(usesHueDegreeControls(channel) ? "Outputs (deg)" : "Outputs"),
  );

  outputs.forEach((value, index) => {
    outputPanel.appendChild(
      createValueRow(`B${index + 1}`, value, 0, 255, (next) =>
        setBandOutput(channel, index, next),
      ),
    );
  });

  container.appendChild(outputPanel);
}

function renderAllBandControls() {
  if (activeProcessingMode !== "threshold") {
    (Object.keys(bandControlContainers) as UiChannel[]).forEach((channel) => {
      const container = bandControlContainers[channel];
      if (container) container.innerHTML = "";
    });
    return;
  }

  if (lockChannelsEnabled) {
    if (activeColorSpace === "hsv") {
      renderBandControls("red");
      renderBandControls("green");
    } else {
      renderBandControls("red");
    }
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

function drawAdditiveRgbHistogram(
  ctx: CanvasRenderingContext2D,
  canvas: HTMLCanvasElement,
  perChannel: Record<Channel, Uint32Array>,
) {
  const channelColors: Record<Channel, string> = {
    red: "#ff0000",
    green: "#00ff00",
    blue: "#0000ff",
  };
  const canvasWidth = canvas.width;
  const canvasHeight = canvas.height;
  let max = 0;
  COLOR_CHANNELS.forEach((channel) => {
    perChannel[channel].forEach((value) => {
      if (value > max) max = value;
    });
  });

  if (max === 0) return;

  const barWidth = canvasWidth / 256;
  const maxLog = Math.log1p(max);

  ctx.save();
  ctx.globalCompositeOperation = "lighter";
  ctx.globalAlpha = 0.82;

  COLOR_CHANNELS.forEach((channel) => {
    const counts = perChannel[channel];
    ctx.fillStyle = channelColors[channel];

    for (let i = 0; i < 256; i += 1) {
      const value = counts[i];
      if (value === 0) continue;
      const normalized = Math.log1p(value) / maxLog;
      const barHeight = normalized * canvasHeight;
      const x = i * barWidth;
      const y = canvasHeight - barHeight;
      ctx.fillRect(x, y, Math.max(1, barWidth), barHeight);
    }
  });

  ctx.restore();
}

function drawTargetChannelControl(
  ctx: CanvasRenderingContext2D,
  canvas: HTMLCanvasElement,
  channel: UiChannel,
) {
  if (channel === "alpha" || !usesHistogramTargetControls()) return;
  const value = targetChannelValue(channel);
  if (value === null) return;

  const x = (value / 255) * canvas.width;
  const displayValue = rawToControlValue(channel, value);
  const label =
    activeColorSpace === "hsv" && channel === "red"
      ? `Target ${displayValue}°`
      : `Target ${displayValue}`;

  ctx.save();
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.shadowColor = "rgba(0, 0, 0, 0.65)";
  ctx.shadowBlur = 4;
  ctx.strokeStyle = "#ffffff";
  ctx.fillStyle = "#ffffff";
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.moveTo(x, 0);
  ctx.lineTo(x, canvas.height);
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(x, 0);
  ctx.lineTo(x - 7, 11);
  ctx.lineTo(x + 7, 11);
  ctx.closePath();
  ctx.fill();

  ctx.font = "600 11px system-ui, sans-serif";
  ctx.textBaseline = "top";
  const metrics = ctx.measureText(label);
  const labelX = Math.max(4, Math.min(canvas.width - metrics.width - 8, x + 8));
  ctx.fillText(label, labelX, 8);
  ctx.restore();
}

function formatTargetReadout(channel: Channel, value: number): string {
  if (activeColorSpace === "hsv") {
    const prefix = channel === "red" ? "H" : channel === "green" ? "S" : "V";
    const displayValue =
      channel === "red" ? `${rawToControlValue(channel, value)}°` : value;
    return `${prefix} ${displayValue}`;
  }
  const prefix = channel === "red" ? "R" : channel === "green" ? "G" : "B";
  return `${prefix} ${value}`;
}

function syncChannelTargetReadout(channel: UiChannel) {
  const title = channelTitles[channel];
  if (!title) return;
  let readout = title.querySelector<HTMLElement>(".channel-value-readout");
  if (activeProcessingMode !== "voronoi" || channel === "alpha") {
    readout?.remove();
    return;
  }

  const value = targetChannelValue(channel);
  if (value === null) {
    readout?.remove();
    return;
  }

  if (!readout) {
    readout = document.createElement("span");
    readout.className = "channel-value-readout";
    title.appendChild(readout);
  }
  readout.textContent = formatTargetReadout(channel, Math.round(value));
}

function activeVoronoiSourceChannelValue(channel: Channel): number | null {
  const group = activeQuantizerGroup();
  if (!group) return null;
  const site =
    group.sites.find(
      (candidate) =>
        candidate.id === activeQuantizerSiteId &&
        candidate.colorSpace === activeColorSpace,
    ) ??
    group.sites.find((candidate) => candidate.colorSpace === activeColorSpace);
  if (!site) return null;

  if (activeColorSpace === "rgb") {
    const coord = site.coord as RGB;
    return (
      {
        red: coord.r,
        green: coord.g,
        blue: coord.b,
      } as const
    )[channel];
  }

  const coord = site.coord as HsvSiteCoord;
  return channel === "red"
    ? Math.round((coord.h / 360) * 255)
    : channel === "green"
      ? coord.s
      : coord.v;
}

function activeVoronoiTargetHsv(): HsvSiteCoord {
  const group = activeQuantizerGroup();
  if (!group) return { h: 0, s: 0, v: 0 };
  return (
    (group.outputCoords.hsv as HsvSiteCoord | undefined) ??
    (siteCoordFromRgb("hsv", group.paletteColor) as HsvSiteCoord)
  );
}

function activeVoronoiTargetRgb(): RGB {
  return activeQuantizerGroup()?.paletteColor ?? { r: 0, g: 150, b: 255 };
}

function sourceColorForGroup(group: QuantizationGroup): RGB | null {
  const site =
    group.sites.find((candidate) => candidate.colorSpace === activeColorSpace) ??
    null;
  if (!site) return null;
  return activeColorSpace === "hsv"
    ? hsvCoordToRgb(site.coord as HsvSiteCoord)
    : (site.coord as RGB);
}

function colorForVoronoiBarValue(channel: Channel, value: number): RGB {
  const clamped = Math.max(0, Math.min(255, Math.round(value)));
  if (activeColorSpace === "hsv") {
    const target = activeVoronoiTargetHsv();
    if (channel === "red") {
      return hsvCoordToRgb({
        h: Math.min(359, Math.round((clamped / 255) * 360)),
        s: 255,
        v: 255,
      });
    }
    if (channel === "green") {
      return hsvCoordToRgb({ h: target.h, s: clamped, v: 255 });
    }
    return hsvCoordToRgb({ h: target.h, s: target.s, v: clamped });
  }

  const target = activeVoronoiTargetRgb();
  return {
    r: channel === "red" ? clamped : target.r,
    g: channel === "green" ? clamped : target.g,
    b: channel === "blue" ? clamped : target.b,
  };
}

function drawVoronoiColorBar(
  ctx: CanvasRenderingContext2D,
  canvas: HTMLCanvasElement,
  channel: Channel,
) {
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  for (let x = 0; x < canvas.width; x += 1) {
    const value =
      canvas.width <= 1 ? 0 : Math.round((x / (canvas.width - 1)) * 255);
    ctx.fillStyle = rgbToHex(colorForVoronoiBarValue(channel, value));
    ctx.fillRect(x, 0, 1, canvas.height);
  }
}

function drawVoronoiColorBarControls(
  ctx: CanvasRenderingContext2D,
  canvas: HTMLCanvasElement,
  channel: Channel,
) {
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  const sourceValue = activeVoronoiSourceChannelValue(channel);
  if (sourceValue !== null) {
    const x = Math.max(
      0.5,
      Math.min(canvas.width - 0.5, (sourceValue / 255) * canvas.width),
    );
    ctx.save();
    ctx.strokeStyle = "#ffffff";
    ctx.shadowColor = "rgba(0, 0, 0, 0.72)";
    ctx.shadowBlur = 3;
    ctx.lineWidth = 1.25;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, canvas.height);
    ctx.stroke();
    ctx.restore();
  }

  const targetValue = targetChannelValue(channel);
  if (targetValue === null) return;

  const dotRadius = 9.5;
  const x = Math.max(
    dotRadius + 2,
    Math.min(canvas.width - dotRadius - 2, (targetValue / 255) * canvas.width),
  );
  const y = canvas.height / 2;
  const targetColor = rgbToHex(activeVoronoiTargetRgb());
  ctx.save();
  ctx.shadowColor = "rgba(0, 0, 0, 0.72)";
  ctx.shadowBlur = 4;
  ctx.fillStyle = targetColor;
  ctx.strokeStyle = "#ffffff";
  ctx.lineWidth = 2.25;
  ctx.beginPath();
  ctx.arc(x, y, dotRadius, 0, Math.PI * 2);
  ctx.fill();
  ctx.stroke();
  ctx.fillStyle = "#ffffff";
  ctx.beginPath();
  ctx.moveTo(x, 2);
  ctx.lineTo(x - 5, 9);
  ctx.lineTo(x + 5, 9);
  ctx.closePath();
  ctx.fill();
  ctx.beginPath();
  ctx.moveTo(x, canvas.height - 2);
  ctx.lineTo(x - 5, canvas.height - 9);
  ctx.lineTo(x + 5, canvas.height - 9);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function updateChannelHistogram(channel: UiChannel) {
  syncChannelTargetReadout(channel);
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

  const { bars, controls } = targets;
  const barsCtx = bars.ctx;
  const barsCanvas = bars.canvas;
  const controlsCtx = controls.ctx;
  const controlsCanvas = controls.canvas;

  if (!barsCtx || !barsCanvas || !controlsCtx || !controlsCanvas) return;

  if (activeProcessingMode === "voronoi" && channel !== "alpha") {
    drawVoronoiColorBar(barsCtx, barsCanvas, channel);
    drawVoronoiColorBarControls(controlsCtx, controlsCanvas, channel);
    return;
  }

  const cache = getHistogramCache();
  if (!cache) return;

  const drawRgbAdditiveHistogram =
    activeProcessingMode === "threshold" &&
    lockChannelsEnabled &&
    activeColorSpace === "rgb" &&
    channel === "red";
  const thresholds = channelThresholds[channel];
  const counts =
    activeProcessingMode !== "threshold" && channel !== "alpha"
      ? cache.perChannel[channel as Channel]
      : channel === "alpha"
        ? cache.alpha
        : lockChannelsEnabled &&
            isSyncableChannel(channel) &&
            channel === syncBaseChannelForColorSpace(activeColorSpace)
          ? cache.combined
          : lockChannelsEnabled && isSyncableChannel(channel)
            ? null
            : cache.perChannel[channel as Channel];

  if (!counts && !drawRgbAdditiveHistogram) return;

  // Clear and draw histogram bars
  const canvasWidth = barsCanvas.width;
  const canvasHeight = barsCanvas.height;

  barsCtx.clearRect(0, 0, canvasWidth, canvasHeight);
  barsCtx.fillStyle = "#0f0f0f";
  barsCtx.fillRect(0, 0, canvasWidth, canvasHeight);

  if (drawRgbAdditiveHistogram) {
    drawAdditiveRgbHistogram(barsCtx, barsCanvas, cache.perChannel);
  } else if (counts) {
    const max = counts.reduce((acc, value) => (value > acc ? value : acc), 0);
    const barWidth = canvasWidth / 256;
    barsCtx.fillStyle =
      lockChannelsEnabled &&
      isSyncableChannel(channel) &&
      channel === syncBaseChannelForColorSpace(activeColorSpace)
        ? "#ffffff"
        : targets.color;
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

  if (activeProcessingMode !== "threshold") {
    drawTargetChannelControl(controlsCtx, controlsCanvas, channel);
    return;
  }

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
  (["red", "green", "blue", "alpha"] as UiChannel[]).forEach((channel) => {
    if (isVisibleWhenSynced(channel)) {
      updateChannelHistogram(channel);
    }
  });
}

function posterizeImageData(source: ImageData): ImageData {
  ensureChannelOutputs("red");
  ensureChannelOutputs("green");
  ensureChannelOutputs("blue");
  if (posterizeAlphaEnabled) {
    ensureChannelOutputs("alpha");
  }

  const working = new ImageData(
    new Uint8ClampedArray(source.data),
    source.width,
    source.height,
  );

  let posterized: ImageData;

  if (activeProcessingMode === "voronoi") {
    posterized = applyVoronoiQuantization(working);
  } else if (activeProcessingMode === "bezier") {
    posterized = applyBezierQuantization(working);
  } else {
    posterized =
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
  }

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
  return posterized;
}

function applyVoronoiQuantization(source: ImageData): ImageData {
  const image = analyzeImageData(source);
  const stats = computeVoronoiAssignments(
    image,
    quantizerGroups,
    activeColorSpace,
  );
  renderQuantizerStats(image);
  return renderVoronoiImage(image, quantizerGroups, stats);
}

function applyBezierQuantization(source: ImageData): ImageData {
  const image = analyzeImageData(source);
  const stats = computeBezierAssignments(
    image,
    bezierRegions,
    activeColorSpace,
  );
  renderQuantizerStats(image);
  return renderBezierImage(image, bezierRegions, stats);
}

/**
 * Render the processed canvas using the current preview image data. The full
 * resolution source is processed only during export.
 */
function renderPosterized() {
  if (!originalImageData || !outputCanvas || !outputCtx) return;
  const posterizeSource = getPreprocessedImageData();
  if (!posterizeSource) return;

  if (outputCanvas.width !== posterizeSource.width) {
    outputCanvas.width = posterizeSource.width;
  }
  if (outputCanvas.height !== posterizeSource.height) {
    outputCanvas.height = posterizeSource.height;
  }
  if (
    quantizerOverlayCanvas &&
    quantizerOverlayCanvas.width !== posterizeSource.width
  ) {
    quantizerOverlayCanvas.width = posterizeSource.width;
  }
  if (
    quantizerOverlayCanvas &&
    quantizerOverlayCanvas.height !== posterizeSource.height
  ) {
    quantizerOverlayCanvas.height = posterizeSource.height;
  }

  const posterized = posterizeImageData(posterizeSource);
  outputCtx.putImageData(posterized, 0, 0);
  renderQuantizerOverlay();
}

function getPreviewDimensions(width: number, height: number) {
  if (!comparisonStage || width <= 0 || height <= 0) {
    return { width, height, scale: 1 };
  }
  const rect = comparisonStage.getBoundingClientRect();
  const maxWidth = Math.max(1, Math.floor(rect.width || width));
  const maxHeight = Math.max(1, Math.floor(rect.height || height));
  const scale = Math.min(maxWidth / width, maxHeight / height);
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
    scale,
  };
}

function buildPreviewFromFullResolution() {
  if (!fullResolutionImageData || !sourceCanvas || !sourceCtx) return;

  const { width, height, scale } = getPreviewDimensions(
    fullResolutionImageData.width,
    fullResolutionImageData.height,
  );

  if (
    originalImageData &&
    originalImageData.width === width &&
    originalImageData.height === height &&
    previewScale === scale
  ) {
    return;
  }
  previewScale = scale;

  const fullCanvas = document.createElement("canvas");
  fullCanvas.width = fullResolutionImageData.width;
  fullCanvas.height = fullResolutionImageData.height;
  const fullCtx = fullCanvas.getContext("2d");
  if (!fullCtx) return;
  fullCtx.putImageData(fullResolutionImageData, 0, 0);

  sourceCanvas.width = width;
  sourceCanvas.height = height;
  sourceCtx.clearRect(0, 0, width, height);
  sourceCtx.drawImage(fullCanvas, 0, 0, width, height);
  originalImageData = sourceCtx.getImageData(0, 0, width, height);
  originalImageRevision += 1;
  preprocessedImageCache = null;
  histogramCache = null;
  if (activeProcessingMode === "voronoi") {
    rehydrateVoronoiSitesForActiveColorSpace();
  }

  if (outputCanvas) {
    outputCanvas.width = width;
    outputCanvas.height = height;
  }
  if (quantizerOverlayCanvas) {
    quantizerOverlayCanvas.width = width;
    quantizerOverlayCanvas.height = height;
  }

  renderPosterized();
  updateAllHistograms();
}

function schedulePreviewResize() {
  if (!fullResolutionImageData) return;
  if (previewResizeFrame !== null) {
    cancelAnimationFrame(previewResizeFrame);
  }
  previewResizeFrame = requestAnimationFrame(() => {
    previewResizeFrame = null;
    buildPreviewFromFullResolution();
  });
}

function drawFromSource(
  src: string,
  cleanup?: () => void,
  saveToStorage = false,
) {
  if (!sourceCanvas || !sourceCtx) return;

  const img = new Image();
  img.onload = () => {
    const fullCanvas = document.createElement("canvas");
    const fullCtx = fullCanvas.getContext("2d");
    if (!fullCtx) return;
    drawImageToCanvas(img, fullCanvas, fullCtx);
    fullResolutionImageData = fullCtx.getImageData(
      0,
      0,
      fullCanvas.width,
      fullCanvas.height,
    );
    originalImageData = null;

    // Save image data to localStorage if requested
    if (saveToStorage && src !== DEFAULT_IMAGE) {
      try {
        const dataUrl = fullCanvas.toDataURL("image/png");
        localStorage.setItem("threshiator-image", dataUrl);
        localStorage.setItem("threshiator-has-image", "true");
      } catch (e) {
        console.warn("Failed to save image to localStorage:", e);
      }
    }

    buildPreviewFromFullResolution();
    cleanup?.();
  };
  img.src = src;
}

function syncChannelLabel(channel: UiChannel, value: number) {
  const label = levelLabels[channel];
  if (label) {
    label.value = value.toString();
  }
}

function setChannelLevel(channel: UiChannel, value: number) {
  const clamped = Math.max(MIN_LEVELS, Math.min(MAX_LEVELS, Math.floor(value)));
  if (channelLevels[channel] === clamped) {
    syncChannelLabel(channel, clamped);
    return clamped;
  }
  channelLevels[channel] = clamped;
  channelThresholds[channel] = createEvenThresholds(clamped);
  channelOutputs[channel] = createLinearOutputs(clamped);
  syncChannelLabel(channel, clamped);
  if (lockChannelsEnabled && isSyncableChannel(channel)) {
    syncOtherChannelsFrom(channel as Channel);
    renderAllBandControls();
    updateAllHistograms();
  } else {
    updateChannelHistogram(channel);
    renderBandControls(channel);
  }
  renderPosterized();
  return clamped;
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

function setChannelThreshold(
  channel: UiChannel,
  index: number,
  value: number,
  options: { rerenderControls?: boolean } = {},
) {
  const thresholds = channelThresholds[channel];
  if (!thresholds || index < 0 || index >= thresholds.length) return null;

  const minValue = index === 0 ? 1 : thresholds[index - 1] + 1;
  const maxValue =
    index === thresholds.length - 1 ? 254 : thresholds[index + 1] - 1;
  const clamped = Math.max(minValue, Math.min(maxValue, Math.round(value)));
  if (thresholds[index] === clamped) return clamped;

  thresholds[index] = clamped;
  const rerenderControls = options.rerenderControls ?? true;
  if (lockChannelsEnabled && isSyncableChannel(channel)) {
    syncOtherChannelsFrom(channel as Channel);
    if (rerenderControls) renderAllBandControls();
    updateAllHistograms();
  } else {
    updateChannelHistogram(channel);
    if (rerenderControls) renderBandControls(channel);
  }
  renderPosterized();
  return clamped;
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
    if (usesHistogramTargetControls() && channel !== "alpha") {
      canvas.style.cursor = "col-resize";
      return;
    }
    if (activeProcessingMode !== "threshold") {
      canvas.style.cursor = "default";
      return;
    }

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
        setChannelThreshold(channel, dragState.index, value, {
          rerenderControls: false,
        });

        // If channels are synchronized, apply only within the current sync group.
        if (lockChannelsEnabled && isSyncableChannel(channel)) {
          synchronizeChannels(channel as Channel);
        }
      } else if (dragState.type === "level") {
        const normalizedY = 255 - (y / rect.height) * 255;
        const value = Math.max(0, Math.min(255, Math.round(normalizedY)));
        const outputs = channelOutputs[channel];
        if (outputs[dragState.index] !== undefined) {
          outputs[dragState.index] = value;

          // If channels are synchronized, apply only within the current sync group.
          if (lockChannelsEnabled && isSyncableChannel(channel)) {
            const currentIndex = dragState.index; // Store index before forEach
            syncGroupForColorSpace(activeColorSpace).forEach((ch) => {
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
      } else if (
        dragState.type === "target" &&
        channel !== "alpha" &&
        usesHistogramTargetControls()
      ) {
        const scaleX = canvas.width / rect.width;
        const canvasX = x * scaleX;
        const value = (canvasX / canvas.width) * 255;
        setTargetChannelValue(channel, value);
      }
      event.preventDefault();
      return;
    }

    if (isVisibleWhenSynced(channel)) {
      updateCursor(x, y);
    } else {
      canvas.style.cursor = "default";
    }
  };

  const handlePointerDown = (event: PointerEvent) => {
    if (dragState) return;

    if (!isVisibleWhenSynced(channel)) return;

    const rect = canvas.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;

    const thresholdIndex = isOverThreshold(x);
    const levelIndex = isOverLevelHandle(x, y);

    if (usesHistogramTargetControls() && channel !== "alpha") {
      dragState = {
        channel,
        index: 0,
        pointerId: event.pointerId,
        type: "target",
      };
      canvas.setPointerCapture(event.pointerId);
      const scaleX = canvas.width / rect.width;
      const canvasX = x * scaleX;
      setTargetChannelValue(channel, (canvasX / canvas.width) * 255);
      event.preventDefault();
    } else if (activeProcessingMode !== "threshold") {
      event.preventDefault();
    } else if (thresholdIndex >= 0) {
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

    if (lockChannelsEnabled && isSyncableChannel(channel)) {
      renderAllBandControls();
    } else {
      renderBandControls(channel);
    }

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

document.addEventListener("keydown", (event) => {
  if (
    event.key.toLowerCase() !== "n" ||
    event.altKey ||
    event.ctrlKey ||
    event.metaKey ||
    isTextEntryTarget(event.target)
  ) {
    return;
  }
  event.preventDefault();
  toggleFloatingToolbar();
});

lockToggle?.addEventListener("change", () => {
  toggleLockChannels(lockToggle.checked);
});

colorSpaceSelect?.addEventListener("change", () => {
  const raw = colorSpaceSelect.value;
  if (raw === "rgb" || raw === "hsv") {
    setColorSpace(raw);
  }
});

processingModeSelect?.addEventListener("change", () => {
  const raw = processingModeSelect.value;
  if (raw === "threshold" || raw === "voronoi") {
    setProcessingMode(raw);
  }
});

addQuantizerGroupBtn?.addEventListener("click", () => {
  addQuantizerGroup();
});

deleteQuantizerGroupBtn?.addEventListener("click", () => {
  deleteActiveQuantizerGroup();
});

voronoiSourceModeBtn?.addEventListener("click", () => {
  setVoronoiEditMode("source");
});

voronoiTargetModeBtn?.addEventListener("click", () => {
  setVoronoiEditMode("target");
});

addBezierRegionBtn?.addEventListener("click", () => {
  addBezierRegion();
});

closeBezierRegionBtn?.addEventListener("click", () => {
  closeActiveBezierRegion();
});

deleteBezierRegionBtn?.addEventListener("click", () => {
  deleteActiveBezierRegion();
});

quantizerResetBtn?.addEventListener("click", () => {
  resetQuantizerState();
});

smoothingInput?.addEventListener("input", () => {
  setSmoothingAmount(Number(smoothingInput.value));
});

smoothingValue?.addEventListener("change", () => {
  setSmoothingAmount(Number(smoothingValue.value));
});
if (smoothingValue) {
  commitNumberInputOnEnter(smoothingValue, () => {
    setSmoothingAmount(Number(smoothingValue.value));
  });
}

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

document.querySelectorAll<HTMLInputElement>(".band-value").forEach((input) => {
  const channel = input.closest<HTMLElement>("[data-channel]")?.dataset
    .channel as UiChannel | undefined;
  if (!channel) return;

  const commit = () => {
    if (input.value === "") {
      syncChannelLabel(channel, channelLevels[channel]);
      return;
    }
    const next = Number(input.value);
    if (!Number.isFinite(next)) {
      syncChannelLabel(channel, channelLevels[channel]);
      return;
    }
    const applied = setChannelLevel(channel, next);
    input.value = applied.toString();
  };

  input.addEventListener("change", commit);
  commitNumberInputOnEnter(input, commit);
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
  localStorage.removeItem("threshiator-image");
  localStorage.removeItem("threshiator-has-image");
  localStorage.removeItem(ALPHA_STORAGE_KEY);
  localStorage.removeItem(SMOOTHING_STORAGE_KEY);
  setSmoothingAmount(0, { saveToStorage: false, rerender: false });
  activeProcessingMode = "voronoi";
  activeVoronoiEditMode = "source";
  if (processingModeSelect) processingModeSelect.value = activeProcessingMode;
  voronoiSampleRadius = 1;
  resetQuantizerState();
  togglePosterizeAlpha(false);
  resetChannelLevels();
  if (!fullResolutionImageData) {
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
  if (processingModeSelect) {
    processingModeSelect.value = activeProcessingMode;
  }

  const savedAlpha = localStorage.getItem(ALPHA_STORAGE_KEY);
  if (savedAlpha === "true" || savedAlpha === "false") {
    posterizeAlphaEnabled = savedAlpha === "true";
  }
  if (posterizeAlphaToggle) {
    posterizeAlphaToggle.checked = posterizeAlphaEnabled;
  }

  const savedSmoothing = localStorage.getItem(SMOOTHING_STORAGE_KEY);
  setSmoothingAmount(sanitizeSmoothing(savedSmoothing ?? 0), {
    saveToStorage: false,
    rerender: false,
  });

  resetChannelLevels();

  // Try to restore saved image, otherwise use default
  const savedImage = localStorage.getItem("threshiator-image");
  const hasImage = localStorage.getItem("threshiator-has-image");

  if (savedImage && hasImage === "true") {
    drawFromSource(savedImage);
  } else {
    drawFromSource(DEFAULT_IMAGE);
  }

  applyLockStateToUI();
  renderQuantizerControls();
}

// Start the application
initializeApp();
