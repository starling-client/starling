r"""Android adaptive icon from the square app icon: python adaptive_icon.py ICON.png RES_DIR [NAME]

The icon's flat background colour becomes the adaptive background; what differs from it (the art) is the foreground,
scaled to sit in the 66 dp safe zone of the 108 dp layer, so a launcher's mask never cuts it."""
import pathlib
import sys

from PIL import Image

src, res = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
name = sys.argv[3] if len(sys.argv) > 3 else "ic_launcher"
im = Image.open(src).convert("RGBA")
w, h = im.size
bg = im.getpixel((w // 2, h // 12))[:3]
fg = Image.new("RGBA", im.size, (0, 0, 0, 0))
px, out = im.load(), fg.load()
for y in range(h):
    for x in range(w):
        r, g, b, a = px[x, y]
        d = max(abs(r - bg[0]), abs(g - bg[1]), abs(b - bg[2]))
        edge = min(x, y, w - 1 - x, h - 1 - y) < w // 20  # the square's own rounded border
        if a and d > 6 and not edge:
            k = min(1.0, d / 48)
            out[x, y] = (r, g, b, round(a * k))
# The art's box by its solid pixels: a stray faint one shouldn't shrink the rest.
box = fg.getchannel('A').point(lambda v: 255 if v > 160 else 0).getbbox()
art = fg.crop(box)
DENS = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}
for dens, scale in DENS.items():
    side = round(108 * scale)
    # The art's longer side in 60 dp: inside the 66 dp circle a launcher always shows.
    fit = 60 * scale / max(art.size)
    small = art.resize((max(1, round(art.size[0] * fit)), max(1, round(art.size[1] * fit))), Image.LANCZOS)
    layer = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    layer.paste(small, ((side - small.size[0]) // 2, (side - small.size[1]) // 2), small)
    d = res / f"mipmap-{dens}"
    d.mkdir(parents=True, exist_ok=True)
    layer.save(d / f"{name}_foreground.png", optimize=True)
(res / "mipmap-anydpi-v26").mkdir(parents=True, exist_ok=True)
(res / "mipmap-anydpi-v26" / f"{name}.xml").write_text(
    '<?xml version="1.0" encoding="utf-8"?>\n'
    '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
    f'    <background android:drawable="@color/{name}_background"/>\n'
    f'    <foreground android:drawable="@mipmap/{name}_foreground"/>\n'
    '</adaptive-icon>\n', encoding="utf-8")
(res / "values").mkdir(parents=True, exist_ok=True)
(res / "values" / f"{name}_background.xml").write_text(
    '<?xml version="1.0" encoding="utf-8"?>\n<resources>\n'
    f'    <color name="{name}_background">#{bg[0]:02X}{bg[1]:02X}{bg[2]:02X}</color>\n</resources>\n', encoding="utf-8")
print("background", bg, "art box", box)
