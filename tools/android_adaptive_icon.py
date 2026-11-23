r"""Android adaptive launcher icon from the murmuration SVG.

    python tools/android_adaptive_icon.py ICON.svg RES_DIR [NAME]

The SVG is the app icon as drawn (packaging/icons/*.svg): a rounded `<rect>` whose fill becomes the adaptive
background, and `<polygon>` birds, drawn here as the foreground, 4x supersampled. The birds are scaled so they sit in
the 66 dp safe zone of the 108 dp layer, so no launcher's mask cuts them. Writes `mipmap-<density>/NAME_foreground.png`,
`mipmap-anydpi-v26/NAME.xml` and `values/NAME_background.xml` under RES_DIR.
"""
import pathlib
import re
import sys

from PIL import Image, ImageDraw

src, res = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
name = sys.argv[3] if len(sys.argv) > 3 else "ic_launcher"
svg = src.read_text(encoding="utf-8")
background = re.search(r'<rect[^>]*fill="(#[0-9a-fA-F]{6})"', svg).group(1)
polygons = [
    ([tuple(map(float, p.split(","))) for p in points.split()], fill)
    for points, fill in re.findall(r'<polygon points="([^"]+)" fill="(#[0-9a-fA-F]{6})"', svg)
]
xs = [x for pts, _ in polygons for x, _ in pts]
ys = [y for pts, _ in polygons for _, y in pts]
box = (min(xs), min(ys), max(xs), max(ys))
SUPER = 4
DENSITIES = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}
for density, scale in DENSITIES.items():
    side = round(108 * scale)
    # The birds' longer side in 60 dp: inside the 66 dp circle every launcher shows.
    fit = 60 * scale / max(box[2] - box[0], box[3] - box[1])
    ox = side / 2 - (box[0] + box[2]) / 2 * fit
    oy = side / 2 - (box[1] + box[3]) / 2 * fit
    big = Image.new("RGBA", (side * SUPER, side * SUPER), (0, 0, 0, 0))
    draw = ImageDraw.Draw(big)
    for pts, fill in polygons:
        draw.polygon([((x * fit + ox) * SUPER, (y * fit + oy) * SUPER) for x, y in pts], fill=fill)
    folder = res / f"mipmap-{density}"
    folder.mkdir(parents=True, exist_ok=True)
    big.resize((side, side), Image.LANCZOS).save(folder / f"{name}_foreground.png", optimize=True)
(res / "mipmap-anydpi-v26").mkdir(parents=True, exist_ok=True)
(res / "mipmap-anydpi-v26" / f"{name}.xml").write_text(
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
    f'    <background android:drawable="@color/{name}_background"/>\n'
    f'    <foreground android:drawable="@mipmap/{name}_foreground"/>\n'
    "</adaptive-icon>\n",
    encoding="utf-8",
)
(res / "values").mkdir(parents=True, exist_ok=True)
(res / "values" / f"{name}_background.xml").write_text(
    '<?xml version="1.0" encoding="utf-8"?>\n<resources>\n'
    f'    <color name="{name}_background">{background.upper()}</color>\n</resources>\n',
    encoding="utf-8",
)
print(f"{src.name}: background {background}, birds {len(polygons)} in {tuple(round(v) for v in box)}")
