#!/usr/bin/env python3
"""Bake the shipped photographic tiles from the CC0 sources.

Inputs (committed, ambientCG, CC0 1.0):
  Bricks075A_1K-JPG_Color.jpg      masonry wall photograph
  WoodSiding008_1K-JPG_Color.jpg   timber siding photograph
  RoofingTiles006_1K-JPG_Color.jpg roof photograph

Outputs (committed):
  brick_wall.png / siding_wall.png / roof_wall.png   1024^2 plain wall art
  brick_windows.png / siding_windows.png             1024^2 wall + punched
                                                     window row (one 3 m floor)

Each tile is one floor high so the proxy UVs (v = local y / 3.0) land a window
band in every floor. Rows are horizontal by construction: the punched band is
the same rows in every column, which is what the Up-Vector Doctrine and the
night-stripe gate require.

The KTX2 outputs are UASTC (transcodes to BC7 on this GPU) with zstd
supercompression, so the runtime decodes to ~1 MiB per tile instead of 4 MiB.

Usage:  python3 scripts/bake_photo_tiles.py [--no-ktx2]
Requires: Pillow, and the `ktx` CLI (KTX-Software 4.x) on PATH for --ktx2.
"""

from __future__ import annotations

import pathlib
import shutil
import subprocess
import sys

from PIL import Image

SIZE = 1024
HERE = pathlib.Path(__file__).resolve().parent.parent
TILES = HERE / "assets" / "textures" / "photographic"

# Window band as a fraction of one floor tile (0 = slab, 1 = next slab).
BAND_TOP = 0.30
BAND_BOTTOM = 0.72
GLASS = (19, 24, 29)
FRAME = (14, 12, 10)


def wall(name: str) -> Image.Image:
    source = Image.open(TILES / f"{name}.jpg").convert("RGB")
    return source.resize((SIZE, SIZE), Image.LANCZOS)


def punched(base: Image.Image, columns: int) -> Image.Image:
    """Composite one floor's window row over the wall photograph."""
    tile = base.copy()
    pixels = tile.load()
    cell = SIZE // columns
    top = int(SIZE * BAND_TOP)
    bottom = int(SIZE * BAND_BOTTOM)
    for column in range(columns):
        x0 = column * cell + cell // 4
        x1 = x0 + cell // 2
        for y in range(max(0, top - 4), min(SIZE, bottom + 4)):
            for x in range(max(0, x0 - 4), min(SIZE, x1 + 4)):
                frame = x < x0 or x >= x1 or y < top or y >= bottom
                pixels[x, y] = FRAME if frame else GLASS
    return tile


def encode(png: pathlib.Path, ktx: str) -> None:
    out = png.with_suffix(".ktx2")
    subprocess.run(
        [
            ktx, "create",
            "--format", "R8G8B8A8_SRGB",
            "--encode", "uastc",
            "--uastc-quality", "3",
            "--generate-mipmap",
            "--assign-tf", "srgb",
            "--zstd", "18",
            str(png), str(out),
        ],
        check=True,
    )
    print(f"{out.name}: {out.stat().st_size} bytes")


def main() -> int:
    do_ktx = "--no-ktx2" not in sys.argv[1:]
    ktx = shutil.which("ktx")
    if do_ktx and ktx is None:
        print("ktx CLI not found on PATH; pass --no-ktx2 to skip encoding", file=sys.stderr)
        return 1

    brick = wall("Bricks075A_1K-JPG_Color")
    siding = wall("WoodSiding008_1K-JPG_Color")
    roof = wall("RoofingTiles006_1K-JPG_Color")

    tiles = {
        "brick_wall": brick,
        "siding_wall": siding,
        "roof_wall": roof,
        # 18 m commercial module / 6 bays = 3 m; 12 m residential / 4 = 3 m.
        "brick_windows": punched(brick, 6),
        "siding_windows": punched(siding, 4),
    }
    for name, image in tiles.items():
        png = TILES / f"{name}.png"
        image.save(png, optimize=True)
        print(f"{png.name}: {png.stat().st_size} bytes")
        if do_ktx:
            encode(png, ktx)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
