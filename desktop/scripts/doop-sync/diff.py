#!/usr/bin/env python3
"""Pixel diff of two screenshots: python diff.py a.png b.png [diff.png] [--tol N]

Prints the share of pixels that differ by more than the tolerance (default 8 per channel),
the bounding box of the differences and the rows/columns where they cluster.
Writes an amplified difference image (white = same, red = different) when a third path is given.
"""
import sys

import numpy as np
from PIL import Image

args = [a for a in sys.argv[1:] if not a.startswith("--")]
tol = int(sys.argv[sys.argv.index("--tol") + 1]) if "--tol" in sys.argv else 8
if "--tol" in sys.argv:
    args.remove(str(tol))
a = np.asarray(Image.open(args[0]).convert("RGB")).astype(int)
b = np.asarray(Image.open(args[1]).convert("RGB")).astype(int)
if a.shape != b.shape:
    print(f"size differs: {a.shape[1]}x{a.shape[0]} vs {b.shape[1]}x{b.shape[0]}")
    h, w = min(a.shape[0], b.shape[0]), min(a.shape[1], b.shape[1])
    a, b = a[:h, :w], b[:h, :w]
d = np.abs(a - b).max(axis=2)
mask = d > tol
n = int(mask.sum())
print(f"differing pixels: {n} of {mask.size} ({100 * n / mask.size:.3f}%), tolerance {tol}")
if n:
    ys, xs = np.where(mask)
    print(f"bounding box: x {xs.min()}-{xs.max()}, y {ys.min()}-{ys.max()}")
    cols = np.bincount(xs, minlength=mask.shape[1])
    rows = np.bincount(ys, minlength=mask.shape[0])
    top_cols = np.argsort(cols)[::-1][:5]
    top_rows = np.argsort(rows)[::-1][:5]
    print("busiest columns:", [(int(c), int(cols[c])) for c in top_cols if cols[c]])
    print("busiest rows:", [(int(r), int(rows[r])) for r in top_rows if rows[r]])
if len(args) > 2:
    out = np.full(a.shape, 255, dtype=np.uint8)
    out[mask] = [220, 40, 40]
    Image.fromarray(out).save(args[2])
    print(f"wrote {args[2]}")
