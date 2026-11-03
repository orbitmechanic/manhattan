#!/usr/bin/env python3
"""Extract one human-scale floor from each facade texture.

For every `Textures/day/facades/facade_*.jpg` this finds the vertical window-row
period and crops exactly one period, writing a one-floor day texture and the
matching band from the night texture.

  Textures/day/floors/<name>_floor.jpg
  Textures/night/floors/<name>_floor_night.jpg
"""
import os
import numpy as np
from PIL import Image

ROOT = "/home/orbit9/Work/games/Manhattan/Textures"
DAY = os.path.join(ROOT, "day", "facades")
NIGHT = os.path.join(ROOT, "night", "facades")
OUT_DAY = os.path.join(ROOT, "day", "floors")
OUT_NIGHT = os.path.join(ROOT, "night", "floors")
os.makedirs(OUT_DAY, exist_ok=True)
os.makedirs(OUT_NIGHT, exist_ok=True)


def smooth(a, k):
    if k <= 1:
        return a
    return np.convolve(a, np.ones(k, dtype=np.float32) / k, mode="same")


def row_period(gray):
    h, w = gray.shape
    # horizontal edge strength per row
    edge = np.abs(np.diff(gray, axis=0)).mean(axis=1)
    edge = smooth(edge.astype(np.float32), 3)
    edge = edge - smooth(edge, max(9, h // 8))
    edge = edge - edge.min()
    n = len(edge)
    best_lag, best = None, -1e9
    for lag in range(max(18, n // 30), min(n // 2, n // 3) + 1):
        a, b = edge[:-lag], edge[lag:]
        na = np.sqrt((a * a).sum()) + 1e-9
        nb = np.sqrt((b * b).sum()) + 1e-9
        score = float((a * b).sum() / (na * nb))
        if score > best:
            best_lag, best = lag, score
    if best_lag is None or best < 0.05:
        best_lag = max(24, h // 8)
    # phase: strongest row-edge sum within one period
    phase = int(np.argmax([edge[o::best_lag].sum() for o in range(best_lag)]))
    period = int(best_lag)
    # keep the crop inside the image
    period = min(period, h)
    y0 = phase % max(1, h - period + 1)
    if y0 + period > h:
        y0 = h - period
    return y0, period


for f in sorted(os.listdir(DAY)):
    if not f.lower().endswith((".jpg", ".jpeg", ".png")):
        continue
    day = Image.open(os.path.join(DAY, f)).convert("RGB")
    night_path = os.path.join(NIGHT, f.rsplit(".", 1)[0] + "_night.jpg")
    if not os.path.exists(night_path):
        print("skip no night", f)
        continue
    night = Image.open(night_path).convert("RGB")
    if night.size != day.size:
        night = night.resize(day.size)
    gray = np.asarray(day.convert("L")).astype(np.float32) / 255.0
    y0, period = row_period(gray)
    base = os.path.splitext(f)[0]
    d = day.crop((0, y0, day.width, y0 + period))
    n = night.crop((0, y0, night.width, y0 + period))
    d.save(os.path.join(OUT_DAY, base + "_floor.jpg"), quality=92, optimize=True)
    n.save(os.path.join(OUT_NIGHT, base + "_floor_night.jpg"), quality=92, optimize=True)
    print(f"{f}: crop y {y0}..{y0+period} of {day.height}  -> {d.width}x{d.height}")

print("done")
