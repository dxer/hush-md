#!/usr/bin/env python3
"""Regenerate the 官方图库 covers under assets/covers/.

    python3 tools/covers/gen_covers.py

Each series is drawn rather than photographed: no attribution, no external
files, and the whole set stays under ~1 MB in the binary. Output is 2000×400
(the cover is 27vh tall and full width), with 236×128 thumbnails for the
picker panel.

The look: a quiet base wash, then two or three layers of light or form with
soft edges, plus a faint grain so large flat areas do not band on a 10-bit
display or in JPEG. Everything is seeded, so a rerun reproduces the set.
"""

import math
import os
import random

import numpy as np
from PIL import Image, ImageFilter

W, H = 2000, 400
THUMB = (236, 128)
OUT = os.path.join(os.path.dirname(__file__), "..", "..", "assets", "covers")


def grid(w=W, h=H):
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    return x / w, y / h


def smoothstep(a, b, t):
    t = np.clip((t - a) / (b - a), 0.0, 1.0)
    return t * t * (3 - 2 * t)


def lerp(c0, c1, t):
    c0 = np.array(c0, np.float32)
    c1 = np.array(c1, np.float32)
    return c0 + (c1 - c0) * t[..., None]


def vignette(rgb, strength=0.18):
    x, y = grid()
    d = np.sqrt((x - 0.5) ** 2 * 0.7 + (y - 0.5) ** 2)
    return rgb * (1.0 - strength * smoothstep(0.25, 0.85, d))[..., None]


def grain(rgb, amount=2.2, seed=0):
    rng = np.random.default_rng(seed)
    n = rng.normal(0.0, amount, rgb.shape[:2]).astype(np.float32)
    return rgb + n[..., None]


def soften(rgb, radius):
    """A slight blur: inverse-distance weights leave creases where two
    anchors are equally near, and a couple of pixels of softness hides them."""
    im = Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8), "RGB").filter(ImageFilter.GaussianBlur(radius))
    return np.asarray(im, np.float32)


def finish(rgb, seed=0):
    rgb = np.clip(rgb, 0, 255)
    rgb = grain(rgb, 1.1, seed)
    return Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8), "RGB")


# ---- 渐变: a mesh of colour blobs, the soft-diffusion look, not a ramp

def gradient(i, seed):
    rng = random.Random(seed)
    palettes = [
        [(255, 154, 158), (250, 208, 196), (161, 196, 253), (255, 236, 210)],  # sunset mesh
        [(168, 237, 234), (254, 214, 227), (196, 229, 246), (255, 255, 255)],  # pastel water
        [(255, 236, 210), (252, 182, 159), (214, 198, 255), (255, 255, 255)],  # peach → lilac
        [(161, 140, 209), (251, 194, 235), (155, 246, 255), (255, 255, 255)],  # candy
        [(255, 214, 165), (255, 170, 165), (200, 182, 255), (240, 248, 255)],  # apricot → periwinkle
        [(162, 210, 255), (255, 255, 255), (255, 200, 221), (190, 240, 230)],  # airy blue
        [(255, 236, 179), (255, 183, 178), (179, 229, 252), (245, 245, 255)],  # honey → sky
        [(199, 236, 216), (255, 240, 200), (255, 205, 210), (232, 236, 255)],  # mint → blush
    ]
    cols = palettes[i % len(palettes)]
    x, y = grid()
    # A true mesh gradient: each colour pulls with a weight that rises sharply
    # near its own point, and the pixel is the weighted average. Blending one
    # blob over another in sequence just paints over the earlier ones; the
    # weighted sum keeps all of them visible where they meet. Inverse distance
    # (Shepard) keeps each colour strong around its anchor — a wide gaussian
    # averages everything into grey.
    anchors = [(rng.uniform(-0.12, 1.12), rng.uniform(-0.05, 1.05)) for _ in cols]
    rgb = np.zeros((H, W, 3), np.float32)
    total = np.zeros((H, W), np.float32)
    weights = []
    for (bx, by) in anchors:
        # Stretch horizontally: the frame is 5:1, round blobs look like dots.
        d2 = ((x - bx) / 2.1) ** 2 + (y - by) ** 2
        w = 1.0 / (d2 ** 1.05 + 1e-3)
        weights.append(w)
        total += w
    for w, c in zip(weights, cols):
        rgb += (w / total)[..., None] * np.array(c, np.float32)
    # A thin diagonal sheen: one soft band of light, barely there.
    ang = rng.uniform(0.3, 0.7) * math.pi
    t = x * math.cos(ang) + y * math.sin(ang)
    sheen = np.exp(-((t - rng.uniform(0.4, 0.75)) / 0.20) ** 2) * 0.16
    rgb = rgb + (255 - rgb) * sheen[..., None]
    return finish(vignette(soften(rgb, 3.0), 0.04), seed)


# ---- 极光: vertical curtains of light over a night sky

def aurora(i, seed):
    rng = random.Random(seed)
    skies = [
        ((8, 20, 44), (18, 64, 96), (54, 222, 190)),
        ((12, 16, 40), (40, 30, 86), (150, 120, 245)),
        ((6, 24, 34), (12, 70, 84), (120, 240, 190)),
        ((16, 12, 38), (64, 26, 74), (240, 140, 210)),
        ((8, 20, 38), (16, 52, 92), (120, 200, 255)),
        ((20, 14, 30), (34, 48, 90), (190, 160, 255)),
    ]
    base, mid, glow = skies[i % len(skies)]
    x, y = grid()
    # Sky: night at the top fading to the horizon colour.
    sky = lerp(base, mid, smoothstep(0.0, 1.0, y * 0.95 + 0.05))
    rgb = sky.copy()
    # Curtains: wavy vertical bands whose light fades in slowly and out
    # slowly — a hard cutoff reads as a painted rectangle, not air.
    glow_c = np.array(glow, np.float32)
    for band in range(3):
        amp = rng.uniform(0.05, 0.13)
        freq = rng.uniform(1.2, 3.0)
        phase = rng.uniform(0, math.tau)
        centre = rng.uniform(0.08, 0.92)
        width = rng.uniform(0.13, 0.30)
        wobble = amp * np.sin(x * freq * math.tau + phase)
        wobble += 0.45 * amp * np.sin(x * freq * 2.7 * math.tau + phase * 1.7)
        d = np.abs(x - (centre + wobble))
        curtain = np.exp(-(d / width) ** 2)
        # Vertical: bright in the middle of the sky, fading both ways.
        vert = smoothstep(0.02, 0.62, y) * (1.0 - smoothstep(0.72, 0.85, y))
        # Fine rays down the curtain, the way aurora light is striated.
        rays = 0.80 + 0.20 * np.sin(x * 140.0 + phase * 3.0 + y * 3.0)
        a = np.clip(curtain * vert * rays * rng.uniform(0.35, 0.62), 0, 1)
        rgb = rgb + (glow_c - rgb) * a[..., None]
    # Blur the light a little: sky has no edges.
    rgb = soften(rgb, 5.0)
    # Stars in the upper third, thinning downward.
    img = finish(vignette(rgb, 0.22), seed)
    px = img.load()
    rng2 = random.Random(seed + 991)
    for _ in range(260):
        sx = rng2.randrange(W)
        sy = int((rng2.random() ** 1.6) * H * 0.62)
        b = rng2.uniform(90, 235)
        w = 1 if rng2.random() < 0.88 else 2
        for dx in range(w):
            for dy in range(w):
                if sx + dx < W and sy + dy < H:
                    r, g, bl = px[sx + dx, sy + dy]
                    px[sx + dx, sy + dy] = (min(255, r + int(b * 0.35)), min(255, g + int(b * 0.35)), min(255, bl + int(b * 0.4)))
    return img


# ---- 山峦: layered ridges with atmospheric haze

def landscape(i, seed):
    rng = random.Random(seed)
    skies = [
        ((236, 246, 255), (168, 210, 245)),
        ((255, 238, 214), (250, 190, 160)),
        ((238, 242, 248), (186, 200, 220)),
        ((255, 244, 226), (214, 226, 240)),
        ((232, 244, 240), (166, 200, 196)),
        ((250, 236, 244), (206, 190, 226)),
    ]
    top, bottom = skies[i % len(skies)]
    x, y = grid()
    rgb = lerp(top, bottom, smoothstep(0.0, 1.0, y * 1.15))
    # The sun: a soft disc above the ridge line, offset from centre.
    sx, sy = rng.uniform(0.18, 0.82), rng.uniform(0.18, 0.38)
    sun_r = rng.uniform(0.045, 0.075)
    sd = np.sqrt(((x - sx) * W / H) ** 2 + (y - sy) ** 2)
    sun = np.exp(-(sd / sun_r) ** 2) * 0.85
    rgb = rgb + (255 - rgb) * sun[..., None]
    # Ridges: far to near, each darker and lower, with haze between them.
    layers = 5
    for k in range(layers):
        depth = k / (layers - 1)
        base_y = 0.42 + depth * 0.42
        amp = 0.10 * (1.0 - depth * 0.55)
        freq = rng.uniform(0.8, 1.8)
        phase = rng.uniform(0, math.tau)
        ridge = base_y
        ridge = ridge + amp * np.sin(x * freq * math.tau + phase)
        ridge = ridge + amp * 0.45 * np.sin(x * freq * 2.7 * math.tau + phase * 1.9)
        ridge = ridge + amp * 0.22 * np.sin(x * freq * 5.3 * math.tau + phase * 2.7)
        mask = smoothstep(0.0, 0.018, y - ridge)
        # Nearer ridges are darker and a little warmer; the haze lifts the far ones.
        shade = np.array(bottom, np.float32) * (1.0 - depth * 0.72)
        shade = shade + np.array([24, 12, 4], np.float32) * depth
        haze = 0.55 * (1.0 - depth)     # far ridges sit further back in the air
        colour = shade + (np.array(bottom, np.float32) - shade) * haze
        rgb = rgb * (1 - mask[..., None]) + colour * mask[..., None]
    return finish(vignette(rgb, 0.10), seed)


# ---- 几何: a loose arrangement of soft shapes, not stripes

def geometry(i, seed):
    rng = random.Random(seed)
    schemes = [
        ((250, 246, 236), [(238, 120, 76), (250, 190, 60), (60, 130, 160)]),
        ((18, 24, 32), [(90, 210, 200), (240, 200, 90), (150, 120, 240)]),
        ((244, 240, 234), [(60, 90, 140), (230, 150, 120), (90, 170, 150)]),
        ((24, 20, 28), [(250, 160, 190), (250, 210, 120), (140, 190, 250)]),
        ((248, 244, 238), [(40, 40, 48), (200, 90, 80), (230, 210, 180)]),
        ((16, 22, 26), [(120, 200, 170), (250, 180, 120), (200, 140, 220)]),
    ]
    bg, inks = schemes[i % len(schemes)]
    x, y = grid()
    aspect = W / H
    rgb = np.array(bg, np.float32) + np.zeros((H, W, 3), np.float32)
    dark = sum(bg) < 380
    # A colour field across one end, as a soft gradient ground.
    c0 = np.array(inks[0], np.float32)
    c1 = np.array(inks[1], np.float32)
    side = rng.choice([0.0, 1.0])
    t = smoothstep(0.0, 1.0, np.abs(x - side) * 1.4)
    rgb = rgb + (c0 - rgb) * (0.22 * t)[..., None]
    # The motif: crisp geometric forms with hard edges — the contrast with
    # the soft ground is what makes it read as design rather than blur.
    def paint(colour, alpha, mask):
        nonlocal rgb
        c = np.array(colour, np.float32)
        rgb = rgb + (c - rgb) * (mask * alpha)[..., None]

    motif = i % 3
    if motif == 0:
        # Concentric rings, off-centre, one breaking the edge of the frame.
        cx, cy = rng.uniform(0.55, 0.95), 0.5
        for k in range(rng.randint(5, 7)):
            rad = 0.06 + k * rng.uniform(0.055, 0.085)
            d = np.sqrt(((x - cx) * aspect) ** 2 + (y - cy) ** 2)
            ring = (np.abs(d - rad) < 0.012 * rng.uniform(0.6, 1.4)).astype(np.float32)
            paint(inks[k % len(inks)], rng.uniform(0.5, 0.9), ring)
    elif motif == 1:
        # Overlapping circles with a multiply feel: edges stay visible.
        for k in range(rng.randint(5, 7)):
            ox, oy = rng.uniform(0.1, 0.9), rng.uniform(0.1, 0.9)
            rad = rng.uniform(0.10, 0.30)
            d = np.sqrt(((x - ox) * aspect) ** 2 + (y - oy) ** 2)
            disc = (d < rad).astype(np.float32)
            paint(inks[k % len(inks)], rng.uniform(0.35, 0.6), disc)
    else:
        # A rising stack of bars, lengths varying: a quiet bar chart.
        n = rng.randint(7, 11)
        for k in range(n):
            bx = 0.06 + k * (0.88 / n)
            bw = (0.88 / n) * rng.uniform(0.45, 0.78)
            top = rng.uniform(0.15, 0.75)
            bar = ((x > bx) & (x < bx + bw) & (y > top)).astype(np.float32)
            paint(inks[k % len(inks)], rng.uniform(0.45, 0.8), bar)
    # A faint grain on the flat areas; the crisp shapes keep their edges.
    return finish(vignette(rgb, 0.10), seed)


SERIES = [
    ("gradient", 8, gradient),
    ("aurora", 6, aurora),
    ("landscape", 6, landscape),
    ("geometry", 6, geometry),
]


def main():
    out = os.path.abspath(OUT)
    thumbs = os.path.join(out, "thumb")
    os.makedirs(thumbs, exist_ok=True)
    total = 0
    for si, (prefix, count, draw) in enumerate(SERIES):
        for i in range(count):
            img = draw(i, seed=si * 100 + i)
            path = os.path.join(out, f"{prefix}-{i + 1}.jpg")
            img.save(path, "JPEG", quality=86, optimize=True, progressive=True)
            t = img.resize(THUMB, Image.LANCZOS)
            t.save(os.path.join(thumbs, f"{prefix}-{i + 1}.jpg"), "JPEG", quality=86, optimize=True)
            size = os.path.getsize(path)
            total += size
            print(f"{prefix}-{i + 1}.jpg  {size / 1024:.0f} KB")
    print(f"\n{sum(c for _, c, _ in SERIES)} covers, {total / 1024:.0f} KB total")


if __name__ == "__main__":
    main()
