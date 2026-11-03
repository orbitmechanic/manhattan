"""Generate the soft fog-bank sprite used by the viewer.

    python3 scripts/make_fog_texture.py

Writes viewer/assets/textures/fog_bank.png (512x256, RGBA).
"""
import os

import numpy as np
from PIL import Image, ImageFilter

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT = os.path.join(ROOT, "viewer", "assets", "textures", "fog_bank.png")

w, h = 512, 256
rng = np.random.default_rng(20261102)
# Value-noise clouds at several scales.
acc = np.zeros((h, w), dtype=np.float32)
for octave, weight in [(8, 0.45), (16, 0.28), (32, 0.17), (64, 0.10)]:
    small = rng.random((max(2, h // (256 // octave)), max(2, w // (256 // octave))))
    layer = np.asarray(
        Image.fromarray((small * 255).astype(np.uint8)).resize((w, h), Image.BICUBIC),
        dtype=np.float32,
    ) / 255.0
    acc += layer * weight

acc = (acc - acc.min()) / (acc.max() - acc.min() + 1e-6)
# Keep the sheet inside an elliptical soft mask so the quad has no hard edge.
yy, xx = np.mgrid[0:h, 0:w]
cx, cy = w / 2, h / 2
r = np.sqrt(((xx - cx) / (w * 0.52)) ** 2 + ((yy - cy) / (h * 0.55)) ** 2)
mask = np.clip(1.0 - r, 0.0, 1.0) ** 1.35
alpha = np.clip((acc * 0.75 + 0.25) * mask, 0.0, 1.0)
alpha = np.asarray(
    Image.fromarray((alpha * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(3)),
    dtype=np.float32,
) / 255.0

rgb = np.empty((h, w, 3), dtype=np.uint8)
# A cool, slightly blue-grey mist.
rgb[..., 0] = 196
rgb[..., 1] = 208
rgb[..., 2] = 222
rgba = np.dstack([rgb, (alpha * 255).astype(np.uint8)])
os.makedirs(os.path.dirname(OUT), exist_ok=True)
Image.fromarray(rgba, "RGBA").save(OUT, optimize=True)
print("wrote", OUT, os.path.getsize(OUT), "alpha max", float(alpha.max()))
