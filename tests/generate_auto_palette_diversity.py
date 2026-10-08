#!/usr/bin/env python3
"""Generate a small visual fixture for automatic palette diversity checks."""

from __future__ import annotations

import hashlib
import json
import struct
import zlib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target/validation/auto-palette-diversity"
WIDTH = HEIGHT = 100


def png_chunk(kind: bytes, payload: bytes) -> bytes:
    checksum = zlib.crc32(kind + payload) & 0xFFFFFFFF
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", checksum)


def main() -> None:
    shades = [(132, 94, 67), (137, 98, 70), (142, 102, 73)]
    pixels: list[tuple[int, int, int, int] | None] = [None] * (WIDTH * HEIGHT)

    def fill_patch(x: int, y: int, width: int, height: int, color: tuple[int, int, int, int]) -> None:
        for yy in range(y, y + height):
            for xx in range(x, x + width):
                pixels[yy * WIDTH + xx] = color

    fill_patch(18, 18, 8, 5, (255, 255, 255, 255))
    fill_patch(62, 58, 8, 5, (0, 205, 255, 255))
    pixels[50 * WIDTH + 50] = (255, 0, 255, 255)
    pixels[82 * WIDTH + 22] = (255, 20, 20, 0)
    remaining = (index for index, pixel in enumerate(pixels) if pixel is None)
    for shade in shades:
        for _ in range(3306):
            pixels[next(remaining)] = (*shade, 255)
    assert all(pixel is not None for pixel in pixels)
    output_pixels = [pixel for pixel in pixels if pixel is not None]

    scanlines = b"".join(
        b"\x00"
        + b"".join(bytes(pixel) for pixel in output_pixels[y * WIDTH : (y + 1) * WIDTH])
        for y in range(HEIGHT)
    )
    png = (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8, 6, 0, 0, 0))
        + png_chunk(b"IDAT", zlib.compress(scanlines, level=9))
        + png_chunk(b"IEND", b"")
    )

    OUT.mkdir(parents=True, exist_ok=True)
    image_path = OUT / "source.png"
    image_path.write_bytes(png)
    manifest = {
        "image": image_path.name,
        "sha256": hashlib.sha256(png).hexdigest(),
        "dimensions": [WIDTH, HEIGHT],
        "pixels": len(output_pixels),
        "expected": {
            "nearby_dominant_shades": {"sRGB8": shades, "each_pixels": 3306},
            "distinct_white_highlight": {"sRGB8": [255, 255, 255], "pixels": 40},
            "cyan_accent": {"sRGB8": [0, 205, 255], "pixels": 40},
            "isolated_noise": {"sRGB8": [255, 0, 255], "pixels": 1},
            "transparent_rgb": {"sRGB8": [255, 20, 20], "pixels": 1},
            "after": "One family representative plus white and cyan; no noise site; three sites total.",
        },
        "artifacts": {
            "initial_identity_render": "after-identity.png",
            "artificial_target_recolor_diagnostic": "diagnostic-recolored.png",
            "generator_test": "generate_auto_palette_diversity_processed_artifact",
        },
    }
    (OUT / "expected.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"{image_path} sha256={manifest['sha256']}")


if __name__ == "__main__":
    main()
