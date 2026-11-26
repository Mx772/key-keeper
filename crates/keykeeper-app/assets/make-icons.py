"""Regenerates every icon file from icon.svg.

Requirements: the `resvg` CLI (`cargo install resvg --locked`) and Pillow (`pip install pillow`).
Run from anywhere: `python crates/keykeeper-app/assets/make-icons.py`.

Outputs (next to this script):
  icon-{32,128,256,512}.png  window icon (256) and Linux/packaging sizes
  icon.ico                   Windows: executable, installer and shortcuts (16-256 px)
  icon.icns                  macOS bundle, drawn on Apple's icon grid (824 px body in 1024)
"""

import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
SVG = HERE / "icon.svg"
PNG_SIZES = [32, 128, 256, 512]
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
# macOS draws icons on a 1024 grid with an 824 px rounded rect body, leaving room for the shadow.
MAC_BODY = 824 / 1024


def render(size: int, tmp: Path) -> Image.Image:
    out = tmp / f"r{size}.png"
    subprocess.run(["resvg", "-w", str(size), "-h", str(size), str(SVG), str(out)], check=True)
    with Image.open(out) as im:
        return im.convert("RGBA").copy()


def mac(size: int, tmp: Path) -> Image.Image:
    body = round(size * MAC_BODY)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    off = (size - body) // 2
    canvas.paste(render(body, tmp), (off, off))
    return canvas


def main() -> int:
    with tempfile.TemporaryDirectory() as t:
        tmp = Path(t)
        for s in PNG_SIZES:
            render(s, tmp).save(HERE / f"icon-{s}.png", optimize=True)

        ico = [render(s, tmp) for s in ICO_SIZES]
        ico[-1].save(
            HERE / "icon.ico",
            sizes=[(s, s) for s in ICO_SIZES],
            append_images=ico[:-1],
        )

        icns = [mac(s, tmp) for s in (16, 32, 64, 128, 256, 512, 1024)]
        icns[-1].save(HERE / "icon.icns", append_images=icns[:-1])
    print("icons written to", HERE)
    return 0


if __name__ == "__main__":
    sys.exit(main())
