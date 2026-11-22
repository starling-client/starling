#!/usr/bin/env python3
"""Render Sunrise's app icon (docs/media/brand/icon.svg, the website's 16-unit pixel-art sunrise) into every icon
file the launcher, the game and the packages use, with no dependency beyond Python.

The master is drawn on whole units of a 16 grid with crispEdges, so each output pixel takes the colour of the shape
under its centre (nearest neighbour): every size stays as sharp as the 16 px one. Only `<rect>` (with `rx`) and
`<path>` made of `M`, `h`, `v`, `H`, `V` and `z` are read, which is all the master uses; anything else is refused.

    python3 tools/render_brand_icons.py
"""
import re
import shutil
import struct
import xml.etree.ElementTree as ET
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BRAND = ROOT / 'docs/media/brand'
MASTER = BRAND / 'icon.svg'
ICONS = ROOT / 'packaging/icons'
SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
NS = '{http://www.w3.org/2000/svg}'


def colour(text):
    text = text.lstrip('#')
    return tuple(int(text[i:i + 2], 16) for i in (0, 2, 4)) + (255,)


def path_rects(d):
    """The axis-aligned rectangles of a path written as `M x y h w v h h -w z` runs."""
    tokens = re.findall(r'[MmHhVvZz]|-?\d*\.?\d+', d)
    rects, points, i, x, y = [], [], 0, 0.0, 0.0
    while i < len(tokens):
        op = tokens[i]
        if op in 'Mm':
            x, y = float(tokens[i + 1]), float(tokens[i + 2])
            points, i = [(x, y)], i + 3
        elif op in 'HhVv':
            value = float(tokens[i + 1])
            if op == 'h':
                x += value
            elif op == 'H':
                x = value
            elif op == 'v':
                y += value
            else:
                y = value
            points.append((x, y))
            i += 2
        elif op in 'Zz':
            xs, ys = [p[0] for p in points], [p[1] for p in points]
            rects.append((min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys), 0.0))
            i += 1
        else:
            raise SystemExit(f'{MASTER}: unsupported path data {op!r}')
    return rects


def shapes(svg):
    """(rects, fill) in paint order, and the view box side."""
    root = ET.fromstring(svg)
    side = float(root.get('viewBox').split()[2])
    out = []
    for element in root.iter():
        tag = element.tag.removeprefix(NS)
        if tag == 'rect':
            rect = (float(element.get('x', 0)), float(element.get('y', 0)), float(element.get('width')),
                    float(element.get('height')), float(element.get('rx', 0)))
            out.append(([rect], colour(element.get('fill'))))
        elif tag == 'path':
            out.append((path_rects(element.get('d')), colour(element.get('fill'))))
        elif tag not in ('svg', 'title'):
            raise SystemExit(f'{MASTER}: unsupported element <{tag}>')
    return out, side


def inside(rect, px, py):
    x, y, w, h, r = rect
    if not (x <= px < x + w and y <= py < y + h):
        return False
    cx = min(max(px, x + r), x + w - r)
    cy = min(max(py, y + r), y + h - r)
    return (px - cx) ** 2 + (py - cy) ** 2 <= r * r


def render(svg, size, tile=None):
    """RGBA rows of the icon at `size` px; `tile` draws it on a smaller square centred in a transparent one."""
    painted, side = shapes(svg)
    tile = tile or size
    margin = (size - tile) / 2
    pixels = []
    for row in range(size):
        line = []
        for column in range(size):
            px = (column + 0.5 - margin) * side / tile
            py = (row + 0.5 - margin) * side / tile
            rgba = (0, 0, 0, 0)
            for rects, fill in painted:
                if any(inside(rect, px, py) for rect in rects):
                    rgba = fill
            line.append(rgba)
        pixels.append(line)
    return pixels


def png(pixels):
    raw = b''.join(b'\0' + b''.join(bytes(p) for p in line) for line in pixels)

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    size = len(pixels)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))


def ico(images):
    """A Windows .ico of PNG entries: (size, bytes) pairs."""
    head = struct.pack('<HHH', 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, data = b'', b''
    for size, image in images:
        entries += struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(image), offset + len(data))
        data += image
    return head + entries + data


def icns(images):
    """A macOS .icns of PNG entries: (OSType, bytes) pairs."""
    body = b''.join(kind + struct.pack('>I', len(image) + 8) + image for kind, image in images)
    return b'icns' + struct.pack('>I', len(body) + 8) + body


def main():
    svg = MASTER.read_text()
    at = {size: png(render(svg, size)) for size in (*SIZES, 1024)}
    # Apple's grid: the tile is 13/16 of the canvas (832 of 1024), centred, so it sits with other Mac icons.
    mac = {size: png(render(svg, size, tile=size * 13 // 16)) for size in (128, 256, 512, 1024)}
    for size in SIZES:
        (ICONS / f'hicolor/{size}x{size}/apps/sunrise.png').write_bytes(at[size])
    shutil.copyfile(MASTER, ICONS / 'hicolor/scalable/apps/sunrise.svg')
    windows = ico([(size, at[size]) for size in (16, 24, 32, 48, 64, 256)])
    mac_icns = icns([(b'ic07', mac[128]), (b'ic08', mac[256]), (b'ic09', mac[512]), (b'ic10', mac[1024])])
    (ICONS / 'sunrise.ico').write_bytes(windows)
    (ICONS / 'sunrise.icns').write_bytes(mac_icns)
    (BRAND / 'icon-1024.png').write_bytes(at[1024])
    (BRAND / 'icon-macos-1024.png').write_bytes(mac[1024])
    (BRAND / 'web/favicon-32.png').write_bytes(at[32])
    (BRAND / 'web/apple-touch-icon.png').write_bytes(png(render(svg, 180)))
    shutil.copyfile(MASTER, BRAND / 'web/favicon.svg')
    (ROOT / 'docs/media/icon-256.png').write_bytes(at[256])
    (ROOT / 'crates/sunrise-theme/icons/sunrise-64.png').write_bytes(at[64])
    # The game's inline text sprite, on its 8 px text grid.
    (ROOT / 'crates/sunrise-client/assets/sunrise/textures/gui/mark.png').write_bytes(png(render(svg, 8)))


if __name__ == '__main__':
    main()
