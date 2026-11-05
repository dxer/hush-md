#!/usr/bin/env python3
"""Draw the hush.md app icon and every file that carries it.

    python3 tools/branding/gen_icon.py

Writes `assets/branding/app-icon.png` (1024), `assets/branding/app-icon.ico`
(Windows resource, 16–256) and `packaging/windows/wizard-logo.bmp` (the
installer's small logo). All three come from the same drawing, so the Dock,
the taskbar and the installer agree.

The mark: a lowercase `h` drawn as one continuous stroke — the ascender, a
semicircular shoulder, and a leg that stops just above the baseline — with a
blue dot finishing the word `h.`. Paper-coloured squircle, no gloss, no
shadow: it has to read at 16px in a menu bar as well as at 1024px in the
Dock.
"""

import os
import struct

import numpy as np
from PIL import Image

# Draw at 2048 and downsample once, so the curve and the squircle edge are
# both smooth without per-pixel supersampling.
S = 2048
SCALE = S / 1024.0
ROOT = os.path.join(os.path.dirname(__file__), "..", "..")

BODY = 832.0                 # squircle side, in 1024-grid units (Apple's grid)
INK = np.array([0x1d, 0x2a, 0x3c], np.float32)
ACCENT = np.array([0x3a, 0x85, 0xf7], np.float32)
PAPER = (np.array([253, 251, 247], np.float32), np.array([236, 228, 215], np.float32))


def u(v):
    return v * SCALE


def paper():
    """Warm paper, lit from the upper left — the same wash the app uses."""
    y, x = np.mgrid[0:S, 0:S].astype(np.float32)
    t = (y / S)[..., None]
    top, bottom = PAPER
    rgb = top * (1 - t) + bottom * t
    d = np.sqrt((x / S - 0.30) ** 2 + (y / S - 0.15) ** 2)
    glow = np.exp(-(d / 0.62) ** 2) * 0.30
    return rgb + (255 - rgb) * glow[..., None]


def squircle(body=BODY, exp=5.0):
    """Apple's rounded square: |x|^5 + |y|^5 ≤ 1, not a rounded rectangle."""
    y, x = np.mgrid[0:S, 0:S].astype(np.float32)
    c, half = S / 2.0, u(body) / 2.0
    d = np.abs((x - c) / half) ** exp + np.abs((y - c) / half) ** exp
    return np.clip((1.0 - d) * (half / 2.0), 0.0, 1.0)


def segment(X, Y, ax, ay, bx, by):
    ax, ay, bx, by = u(ax), u(ay), u(bx), u(by)
    vx, vy = bx - ax, by - ay
    wx, wy = X - ax, Y - ay
    t = np.clip((wx * vx + wy * vy) / max(vx * vx + vy * vy, 1e-6), 0.0, 1.0)
    return np.hypot(wx - t * vx, wy - t * vy)


def arc(X, Y, cx, cy, r, a0, a1):
    """Distance to a circular arc, capped by the distance to its two ends."""
    cx, cy, r = u(cx), u(cy), u(r)
    dx, dy = X - cx, Y - cy
    angle = np.arctan2(dy, dx) % (2 * np.pi)
    inside = (angle >= a0) & (angle <= a1)
    radial = np.abs(np.hypot(dx, dy) - r)
    ends = [np.hypot(dx - r * np.cos(a), dy - r * np.sin(a)) for a in (a0, a1)]
    return np.where(inside, radial, np.minimum(*ends))


def stroke(distance, width):
    """Coverage of a round-capped stroke of `width` at signed distance."""
    return np.clip((u(width) / 2.0 - distance) + 0.5, 0.0, 1.0)


def draw():
    X, Y = np.meshgrid(np.arange(S, dtype=np.float32), np.arange(S, dtype=np.float32))
    stem, top, base = 368.0, 252.0, 788.0
    right, xh = 664.0, 430.0
    r = (right - stem) / 2.0
    # One continuous line: ascender, half-round shoulder, leg to the baseline.
    d = np.minimum(segment(X, Y, stem, top, stem, base), segment(X, Y, right, xh + r, right, base))
    d = np.minimum(d, arc(X, Y, stem + r, xh + r, r, np.pi, 2 * np.pi))
    letter = stroke(d, 96)
    # The stop of the word: a round dot, optically clear of the leg.
    dot_r = 52.0
    dot_x, dot_y = right + r * 0.55 + dot_r * 2, base - dot_r
    dot = stroke(np.hypot(X - u(dot_x), Y - u(dot_y)), dot_r * 2)

    rgb = paper()
    rgb = rgb * (1 - letter[..., None]) + INK * letter[..., None]
    rgb = rgb * (1 - dot[..., None]) + ACCENT * dot[..., None]
    alpha = squircle() * 255.0
    return Image.fromarray(np.dstack([np.clip(rgb, 0, 255), alpha]).astype(np.uint8), "RGBA").resize((1024, 1024), Image.LANCZOS)


def write_bmp(path, image):
    """A 32-bit BMP with an alpha channel.

    PIL's BMP writer only emits the 3.x header, which has no room for an alpha
    mask, so the installer's logo would come out as a grey square over its
    background. Inno Setup reads BITMAPV4HEADER (the Windows 98 header), so
    write one by hand: bottom-up rows, BGRA, all-zero masks meaning "default".
    """
    w, h = image.size
    rows = []
    for y in range(h - 1, -1, -1):
        row = bytearray()
        for x in range(w):
            r, g, b, a = image.getpixel((x, y))
            row += bytes((b, g, r, a))
        rows.append(bytes(row))
    pixels = b"".join(rows)
    header = struct.pack("<IiiHHIIiiII", 108, w, h, 1, 32, 3, len(pixels), 3780, 3780, 0, 0)
    masks = struct.pack("<IIII", 0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000)
    space = bytes(4 * 8)  # colour space type + the 8 endpoints
    tail = struct.pack("<III", 0, 0, 0)  # gamma red / green / blue
    dib = header + masks + space + tail
    body = b"BM" + struct.pack("<IHHI", 14 + len(dib) + len(pixels), 0, 0, 14 + len(dib)) + dib + pixels
    with open(path, "wb") as f:
        f.write(body)


def main():
    icon = draw()
    png = os.path.join(ROOT, "assets", "branding", "app-icon.png")
    icon.save(png, optimize=True)
    ico = os.path.join(ROOT, "assets", "branding", "app-icon.ico")
    icon.save(ico, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    bmp = os.path.join(ROOT, "packaging", "windows", "wizard-logo.bmp")
    write_bmp(bmp, icon.resize((64, 64), Image.LANCZOS))
    for p in (png, ico, bmp):
        print(f"{os.path.relpath(p, ROOT)}  {os.path.getsize(p) / 1024:.0f} KB")


if __name__ == "__main__":
    main()
