import type { ExportSettings } from "./main";

export type PresetEntry = {
  name: string;
  description: string;
  settings: ExportSettings;
};

export const PRESETS: Record<string, PresetEntry> = {
  comicBook: {
    name: "Comic Book",
    description:
      "High contrast with limited colors for a classic comic book look",
    settings: {
      version: 1,
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
  },
  duotoneBlue: {
    name: "Duotone Blue",
    description:
      "Dramatic two-tone effect with deep blacks and blue highlights",
    settings: {
      version: 1,
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
  },
  vintagePhoto: {
    name: "Vintage Photo",
    description: "Warm, sepia-toned effect reminiscent of old photographs",
    settings: {
      version: 1,
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
  },
  noir: {
    name: "Noir",
    description: "Dramatic black and white with deep shadows",
    settings: {
      version: 1,
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
  },
  popArt: {
    name: "Pop Art",
    description: "Vibrant, high-contrast colors in pop art style",
    settings: {
      version: 1,
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
  },
};
