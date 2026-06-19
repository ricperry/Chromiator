import type { ExportSettings } from "./main";

export const PRESETS: Record<string, ExportSettings> = {
  comicBook: {
    version: 1,
    presetName: "Comic Book",
    description: "High contrast with limited colors for a classic comic book look",
    lockChannels: true,
    channels: {
      red: {
        levels: 3,
        thresholds: [85, 170],
        outputs: [0, 128, 255],
      },
      green: {
        levels: 3,
        thresholds: [85, 170],
        outputs: [0, 128, 255],
      },
      blue: {
        levels: 3,
        thresholds: [85, 170],
        outputs: [0, 128, 255],
      },
    },
  },
  duotoneBlue: {
    version: 1,
    presetName: "Duotone Blue",
    description: "Dramatic two-tone effect with deep blacks and blue highlights",
    lockChannels: false,
    channels: {
      red: {
        levels: 2,
        thresholds: [128],
        outputs: [0, 0],
      },
      green: {
        levels: 2,
        thresholds: [128],
        outputs: [0, 0],
      },
      blue: {
        levels: 2,
        thresholds: [128],
        outputs: [0, 255],
      },
    },
  },
  vintagePhoto: {
    version: 1,
    presetName: "Vintage Photo",
    description: "Warm, sepia-toned effect reminiscent of old photographs",
    lockChannels: false,
    channels: {
      red: {
        levels: 4,
        thresholds: [64, 128, 192],
        outputs: [112, 176, 240, 255],
      },
      green: {
        levels: 4,
        thresholds: [64, 128, 192],
        outputs: [80, 144, 208, 224],
      },
      blue: {
        levels: 4,
        thresholds: [64, 128, 192],
        outputs: [48, 112, 176, 192],
      },
    },
  },
  noir: {
    version: 1,
    presetName: "Noir",
    description: "Dramatic black and white with deep shadows",
    lockChannels: true,
    channels: {
      red: {
        levels: 4,
        thresholds: [64, 128, 192],
        outputs: [0, 85, 170, 255],
      },
      green: {
        levels: 4,
        thresholds: [64, 128, 192],
        outputs: [0, 85, 170, 255],
      },
      blue: {
        levels: 4,
        thresholds: [64, 128, 192],
        outputs: [0, 85, 170, 255],
      },
    },
  },
  popArt: {
    version: 1,
    presetName: "Pop Art",
    description: "Vibrant, high-contrast colors in pop art style",
    lockChannels: false,
    channels: {
      red: {
        levels: 2,
        thresholds: [128],
        outputs: [255, 0],
      },
      green: {
        levels: 2,
        thresholds: [128],
        outputs: [0, 255],
      },
      blue: {
        levels: 2,
        thresholds: [128],
        outputs: [0, 255],
      },
    },
  },
};
