#!/usr/bin/env python3
"""Draws art/title.png, the example's CC0 title painting, with the standard library only.

A winter dusk: a violet-to-amber sky with a low sun, dark spruce silhouettes on the
horizon, a snowfield crossed by a road, and the words NO GLORY in a blocky font. The
output is deterministic: run it again and the PNG is byte-identical.

    python3 tools/make_title.py art/title.png
"""

import math
import struct
import sys
import zlib

W, H = 320, 200
HORIZON = 118

# 5 x 7 capitals, one string per row, '#' = ink.
FONT = {
    "N": ["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"],
    "O": [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "G": [".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".####"],
    "L": ["#....", "#....", "#....", "#....", "#....", "#....", "#####"],
    "R": ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"],
    "Y": ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."],
    " ": ["....."] * 7,
}


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def sky(y):
    t = y / HORIZON
    if t < 0.55:
        return lerp((24, 20, 64), (110, 60, 120), t / 0.55)
    return lerp((110, 60, 120), (236, 150, 80), (t - 0.55) / 0.45)


def snow(x, y):
    t = (y - HORIZON) / (H - HORIZON)
    base = lerp((150, 150, 190), (232, 236, 246), t)
    # A road curving down to the left, darker and warmer.
    centre = 200 - 140 * t * t
    half = 3 + 30 * t
    if abs(x - centre) < half:
        return lerp(base, (120, 96, 88), 0.55)
    return base


def tree_height(x):
    # A ragged spruce line: overlapping triangles, deterministic.
    h = 0.0
    for i in range(0, W + 24, 11):
        peak = 14 + (i * 37 % 23)
        width = 6 + (i * 13 % 5)
        h = max(h, peak - abs(x - i) * peak / width)
    return h


def pixel(x, y):
    if y < HORIZON:
        c = sky(y)
        # The sun, low over the trees.
        d = math.hypot(x - 96, y - (HORIZON - 14))
        if d < 13:
            c = (252, 224, 150)
        elif d < 22:
            c = lerp((252, 224, 150), c, (d - 13) / 9)
        if HORIZON - y < tree_height(x):
            c = (18, 28, 30)
        return c
    return snow(x, y)


def draw():
    px = [[pixel(x, y) for x in range(W)] for y in range(H)]
    text, scale, top = "NO GLORY", 4, 22
    width = len(text) * 6 * scale - scale
    left = (W - width) // 2
    ink = [
        (left + (k * 6 + c) * scale, top + r * scale)
        for k, ch in enumerate(text)
        for r, row in enumerate(FONT[ch])
        for c, bit in enumerate(row)
        if bit == "#"
    ]
    # A one-pixel black outline, then the red letters over it.
    for pad, colour in ((1, (8, 8, 12)), (0, (196, 28, 36))):
        for x0, y0 in ink:
            for y in range(y0 - pad, y0 + scale + pad):
                for x in range(x0 - pad, x0 + scale + pad):
                    px[y][x] = colour
    return px


def png(px):
    raw = b"".join(b"\x00" + bytes(v for p in row for v in p) for row in px)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


if __name__ == "__main__":
    with open(sys.argv[1] if len(sys.argv) > 1 else "art/title.png", "wb") as f:
        f.write(png(draw()))
