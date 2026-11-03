#!/usr/bin/env python3
"""Generate night variants of the Manhattan textures.

Facades: dark blue ambient + randomly illuminated windows aligned to the
detected window grid.
Ground satellite: darkened city with street-lamp glows and neon signage.
Street textures: street-lamp pools and lit neon signs.
"""
import os, sys, math, random
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageChops

random.seed(20261101)
np.random.seed(20261101)

ROOT = "/home/orbit9/Work/games/Manhattan/Textures"
DAY = os.path.join(ROOT, "day")
NIGHT = os.path.join(ROOT, "night")


def ensure(path):
    os.makedirs(os.path.dirname(path), exist_ok=True)


# --------------------------------------------------------------------------
# helpers
# --------------------------------------------------------------------------
def to_np(im):
    return np.asarray(im.convert("RGB")).astype(np.float32) / 255.0


def to_img(a):
    return Image.fromarray(np.clip(a * 255.0, 0, 255).astype(np.uint8), "RGB")


def smooth(a, k):
    if k <= 1:
        return a
    ker = np.ones(k, dtype=np.float32) / k
    return np.convolve(a, ker, mode="same")


def autocorr_period(proj, lo, hi):
    """Return (period, phase) for a 1-D projection, or (None, None)."""
    n = len(proj)
    if n < 16:
        return None, None
    p = proj - proj.mean()
    norm = np.sqrt((p * p).sum()) + 1e-9
    best_lag, best_score = None, -1e9
    for lag in range(max(lo, 6), min(hi, n // 2) + 1):
        a, b = p[:-lag], p[lag:]
        na = np.sqrt((a * a).sum()) + 1e-9
        nb = np.sqrt((b * b).sum()) + 1e-9
        score = float((a * b).sum() / (na * nb))
        # prefer plausible window spacing: penalise very large lags a little
        score -= 0.0002 * lag
        if score > best_score:
            best_score, best_lag = score, lag
    if best_lag is None or best_score < 0.12:
        return None, None
    lag = best_lag
    phase_scores = []
    for off in range(lag):
        s = p[off::lag]
        phase_scores.append(float((s * s).sum()))
    phase = int(np.argmax(phase_scores))
    return lag, phase


def grid_axis(gray, axis, min_cells=3, max_cells=22):
    """Detect a repeating window pitch along one axis."""
    g = gray
    if axis == 1:  # vertical mullions -> columns
        edge = np.abs(np.diff(g, axis=1)).mean(axis=0)
        n = g.shape[1]
    else:          # horizontal spandrels -> rows
        edge = np.abs(np.diff(g, axis=0)).mean(axis=1)
        n = g.shape[0]
    edge = smooth(edge.astype(np.float32), 3)
    # detrend: subtract a wide moving average
    edge = edge - smooth(edge, max(9, n // 12))
    edge = edge - edge.min()
    lo = max(8, n // max_cells)
    hi = max(lo + 2, n // min_cells)
    period, phase = autocorr_period(edge, lo, hi)
    if period is None:
        period = max(12, n // 8)
        phase = 0
    # clamp period so we get a sane number of cells
    period = max(period, max(8, n // max_cells))
    period = min(period, max(12, n // min_cells))
    bounds = list(range(phase % period, n + 1, period))
    if bounds[0] != 0:
        bounds.insert(0, 0)
    if bounds[-1] != n:
        bounds.append(n)
    # drop degenerate slices
    bounds = [b for i, b in enumerate(bounds) if i == 0 or b > bounds[i - 1] + 3]
    return bounds


WINDOW_PALETTE = [
    (255, 214, 148), (255, 231, 184), (255, 197, 118),
    (255, 245, 220), (206, 224, 255), (170, 205, 255),
    (255, 170, 96), (255, 120, 70),
]
WINDOW_WEIGHTS = [28, 22, 14, 15, 6, 5, 7, 3]


def connected_components(mask):
    """Simple 8-connected component labeller (scipy-free)."""
    h, w = mask.shape
    seen = np.zeros_like(mask, dtype=bool)
    out = []
    for y in range(h):
        for x in range(w):
            if not mask[y, x] or seen[y, x]:
                continue
            stack = [(y, x)]
            seen[y, x] = True
            pts = []
            while stack:
                cy, cx = stack.pop()
                pts.append((cy, cx))
                for dy in (-1, 0, 1):
                    for dx in (-1, 0, 1):
                        ny, nx = cy + dy, cx + dx
                        if 0 <= ny < h and 0 <= nx < w and mask[ny, nx] and not seen[ny, nx]:
                            seen[ny, nx] = True
                            stack.append((ny, nx))
            out.append(pts)
    return out


def detect_windows(gray_img):
    """Return window bounding boxes in image coordinates (may be empty).

    Windows are found as regions darker than a heavily blurred local wall
    estimate.  This avoids the pitfalls of a rigid detected grid and works for
    both punched-window masonry and glass curtain walls.
    """
    W, H = gray_img.size
    aw = min(360, W)
    ah = max(1, int(H * aw / W))
    small = gray_img.resize((aw, ah), Image.BOX)
    bg = small.filter(ImageFilter.GaussianBlur(max(4, aw / 14)))
    a = np.asarray(small).astype(np.float32) / 255.0
    b = np.asarray(bg).astype(np.float32) / 255.0
    diff = b - a
    # adaptive threshold: catch the darker third of the facade, with a floor
    thr = float(np.clip(np.percentile(diff, 68), 0.035, 0.14))
    mask = diff > thr
    mask_img = Image.fromarray((mask * 255).astype(np.uint8), "L")
    mask_img = (mask_img.filter(ImageFilter.MedianFilter(3))
                        .filter(ImageFilter.MinFilter(3))
                        .filter(ImageFilter.MaxFilter(3)))
    mask = np.asarray(mask_img) > 127
    comps = connected_components(mask)
    boxes = []
    sx, sy = W / aw, H / ah
    for pts in comps:
        if len(pts) < 12:
            continue
        ys = [p[0] for p in pts]
        xs = [p[1] for p in pts]
        x0, x1 = min(xs), max(xs)
        y0, y1 = min(ys), max(ys)
        bw, bh = x1 - x0 + 1, y1 - y0 + 1
        # reject spandrel bands, whole-facade blobs and tiny noise
        if bw > aw * 0.55 or bh > ah * 0.55:
            continue
        if bw < 3 or bh < 3:
            continue
        if not (0.18 < bw / bh < 4.5):
            continue
        boxes.append((int(x0 * sx), int(y0 * sy), int((x1 + 1) * sx), int((y1 + 1) * sy)))
    return boxes


def facade_night(path, out_path):
    im = Image.open(path).convert("RGB")
    W, H = im.size
    day = to_np(im)
    gray = day.mean(axis=2)
    gray_img = im.convert("L")

    # dark blue ambient, keep a little facade detail
    base = day * np.array([0.200, 0.220, 0.300], dtype=np.float32) + 0.012
    # faint cool sky bounce at the top
    yy = np.linspace(1.05, 0.62, H, dtype=np.float32)[:, None, None]
    base *= yy

    em_img = Image.new("RGB", (W, H), (0, 0, 0))
    em_draw = ImageDraw.Draw(em_img)
    boxes = detect_windows(gray_img)
    used_grid = False
    lit = 0

    name = os.path.basename(path).lower()
    use_grid = ("glass" in name) or (len(boxes) < 8)

    if use_grid:
        # glass curtain wall with no readable punched openings: light a random
        # share of the repeating glazing cells, subdivided into room-sized bays
        used_grid = True
        cols = grid_axis(gray, 1, min_cells=3, max_cells=20)
        rows = grid_axis(gray, 0, min_cells=3, max_cells=26)

        def subdivide(bounds, max_size):
            out = [bounds[0]]
            for a, b in zip(bounds[:-1], bounds[1:]):
                n = max(1, int(math.ceil((b - a) / max_size)))
                for k in range(1, n + 1):
                    out.append(a + (b - a) * k / n)
            return out

        cols = subdivide(cols, max(18.0, W / 16.0))
        rows = subdivide(rows, max(18.0, H / 20.0))
        cols = [int(round(c)) for c in cols]
        rows = [int(round(r)) for r in rows]
        boxes = []
        for i in range(len(rows) - 1):
            for j in range(len(cols) - 1):
                x0, y0, x1, y1 = cols[j], rows[i], cols[j + 1], rows[i + 1]
                if x1 - x0 < 4 or y1 - y0 < 4:
                    continue
                boxes.append((x0, y0, x1, y1))

    # filter to usable rectangles, then choose a fixed share so every facade
    # gets a believable number of lit windows
    usable = []
    for (bx0, by0, bx1, by1) in boxes:
        bw, bh = bx1 - bx0, by1 - by0
        if bw < 3 or bh < 3:
            continue
        if used_grid and (bw < 6 or bh < 6):
            continue
        usable.append((bx0, by0, bx1, by1))
    random.shuffle(usable)
    want = int(round(len(usable) * (0.44 if used_grid else 0.50)))
    want = max(want, min(12, len(usable)))

    for (bx0, by0, bx1, by1) in usable[:want]:
        bw, bh = bx1 - bx0, by1 - by0
        inset_x = max(1, int(bw * (0.26 if used_grid else 0.06)))
        inset_y = max(1, int(bh * (0.24 if used_grid else 0.08)))
        rx0, ry0 = bx0 + inset_x, by0 + inset_y
        rx1, ry1 = bx1 - inset_x, by1 - inset_y
        if rx1 - rx0 < 2 or ry1 - ry0 < 2:
            continue
        col = random.choices(WINDOW_PALETTE, weights=WINDOW_WEIGHTS)[0]
        b = random.uniform(0.55, 1.0)
        # very large glazing panels read better when dimmer
        b *= float(np.clip(42.0 / max(bw, bh), 0.40, 1.0))
        if random.random() < 0.10:      # dim, curtained or blinds
            b *= 0.30
        em_draw.rectangle([rx0, ry0, rx1 - 1, ry1 - 1],
                          fill=tuple(int(c * b) for c in col))
        lit += 1

    # glow pass
    glow = em_img.filter(ImageFilter.GaussianBlur(max(2.0, W / 190.0)))
    sharp = np.asarray(em_img).astype(np.float32) / 255.0
    soft = np.asarray(glow).astype(np.float32) / 255.0
    out = base + sharp * 0.90 + soft * (0.40 if used_grid else 0.55)
    # subtle warm spill on the surrounding wall
    out = np.clip(out, 0, 1) ** (1 / 1.02)
    ensure(out_path)
    to_img(out).save(out_path, quality=92, optimize=True, progressive=True)
    mode = "grid" if used_grid else "windows"
    print(f"  facade {os.path.basename(path)}: {len(usable)} {mode}, "
          f"{lit} lit -> {os.path.relpath(out_path, ROOT)}")


# --------------------------------------------------------------------------
# ground / satellite
# --------------------------------------------------------------------------
def radial_glow(arr, cx, cy, radius, color, strength):
    h, w = arr.shape[:2]
    x0, x1 = max(0, int(cx - radius)), min(w, int(cx + radius + 1))
    y0, y1 = max(0, int(cy - radius)), min(h, int(cy + radius + 1))
    if x0 >= x1 or y0 >= y1:
        return
    xs = np.arange(x0, x1, dtype=np.float32)[None, :] - cx
    ys = np.arange(y0, y1, dtype=np.float32)[:, None] - cy
    d = np.sqrt(xs * xs + ys * ys) / max(radius, 1e-6)
    fall = np.clip(1.0 - d, 0, 1) ** 2.2
    for k in range(3):
        arr[y0:y1, x0:x1, k] += fall * color[k] * strength


def satellite_night(path, out_path):
    im = Image.open(path).convert("RGB")
    W, H = im.size
    day = to_np(im)
    hsv = np.asarray(im.convert("HSV")).astype(np.float32) / 255.0
    hue, sat, val = hsv[..., 0], hsv[..., 1], hsv[..., 2]

    # water: dark, blue/green, fairly saturated.  Everything else is land.
    water = (val < 0.42) & (sat > 0.18) & (hue > 0.45) & (hue < 0.72)
    # roads: low-saturation mid-grey pixels on land
    roads = (~water) & (sat < 0.15) & (val > 0.30) & (val < 0.78)
    # remove tiny isolated specks by requiring a little local road support
    roads_img = Image.fromarray((roads * 255).astype(np.uint8), "L")
    roads = np.asarray(roads_img.filter(ImageFilter.MedianFilter(3))) > 127
    road_pts = np.argwhere(roads)
    if len(road_pts) == 0:
        road_pts = np.argwhere(~water)

    # base night city
    out = day * np.array([0.18, 0.20, 0.28], dtype=np.float32) + 0.010
    # water stays black-blue
    out[water] = day[water] * np.array([0.045, 0.055, 0.095], dtype=np.float32)
    # warm ambient glow over built-up land
    built = (~water) & (val > 0.25)
    out[built] += np.array([0.040, 0.030, 0.018], dtype=np.float32)

    arr = out.astype(np.float32)

    # overall lit-street glow: blur the road mask so avenues read as warm lines
    rm = Image.fromarray((roads * 255).astype(np.uint8), "L")
    rm = rm.filter(ImageFilter.GaussianBlur(2.0))
    ra = np.asarray(rm).astype(np.float32) / 255.0
    arr += ra[..., None] * np.array([0.20, 0.13, 0.055], dtype=np.float32)

    # street lamps along the road network
    n_lamps = 6200
    idx = np.random.choice(len(road_pts), size=min(n_lamps, len(road_pts)), replace=False)
    for k in idx:
        y, x = road_pts[k]
        r = random.uniform(3.0, 7.0)
        col = random.choice([[1.0, 0.82, 0.52], [1.0, 0.90, 0.68], [0.85, 0.90, 1.0]])
        radial_glow(arr, x, y, r, col, random.uniform(0.55, 1.25))
    # brighter major intersections / avenues
    for _ in range(420):
        y, x = road_pts[np.random.randint(len(road_pts))]
        radial_glow(arr, x, y, random.uniform(12, 26), [1.0, 0.78, 0.45], random.uniform(0.20, 0.42))

    # neon signage: short bright bars near roads, in saturated colours
    neon_cols = [
        [1.0, 0.15, 0.45], [0.15, 0.95, 1.0], [1.0, 0.55, 0.1],
        [0.45, 1.0, 0.3], [0.8, 0.3, 1.0], [1.0, 0.9, 0.25],
    ]
    for _ in range(300):
        y, x = road_pts[np.random.randint(len(road_pts))]
        col = random.choice(neon_cols)
        ln = random.randint(4, 14)
        ang = random.uniform(0, math.pi)
        dx, dy = math.cos(ang) * ln, math.sin(ang) * ln
        glow = Image.new("RGB", (W, H), (0, 0, 0))
        d = ImageDraw.Draw(glow)
        d.line([x - dx, y - dy, x + dx, y + dy],
               fill=tuple(int(c * 255) for c in col), width=random.randint(1, 3))
        glow = glow.filter(ImageFilter.GaussianBlur(random.uniform(6, 14)))
        arr += np.asarray(glow).astype(np.float32) / 255.0 * 1.15
        # crisp core
        a = math.atan2(dy, dx)
        for t in np.linspace(-1, 1, 8):
            px, py = int(x + dx * t), int(y + dy * t)
            if 0 <= px < W and 0 <= py < H:
                arr[py, px] = np.maximum(arr[py, px], np.array(col) * 0.95)

    # a soft general city glow so the island reads at night
    lum = day.mean(axis=2)
    city = Image.fromarray((np.clip((lum - 0.25) * 1.5, 0, 1) * 255).astype(np.uint8), "L")
    city = city.filter(ImageFilter.GaussianBlur(3))
    ca = np.asarray(city).astype(np.float32) / 255.0
    ca[water] = 0
    arr += ca[..., None] * np.array([0.20, 0.15, 0.09], dtype=np.float32)

    arr = np.clip(arr, 0, 1)
    ensure(out_path)
    to_img(arr).save(out_path, quality=92, optimize=True, progressive=True)
    print(f"  ground {os.path.basename(path)} -> {os.path.relpath(out_path, ROOT)}")


# --------------------------------------------------------------------------
# street textures
# --------------------------------------------------------------------------
def street_night(path, out_path, lamps=True, neon=True, lane_lines=False):
    im = Image.open(path).convert("RGB")
    W, H = im.size
    day = to_np(im)
    arr = day * np.array([0.20, 0.215, 0.265], dtype=np.float32) + 0.006

    if lane_lines:
        # keep the painted lines faintly visible
        g = day.mean(axis=2)
        mask = g > 0.55
        arr[mask] += np.array([0.22, 0.20, 0.10], dtype=np.float32)

    if lamps:
        for side in (0.10, 0.90):
            x = int(W * side)
            step = max(34, H // 5)
            for y in range(step // 2, H, step):
                jx = x + random.randint(-int(W * 0.03), int(W * 0.03))
                jy = y + random.randint(-step // 6, step // 6)
                radial_glow(arr, jx, jy, random.uniform(18, 34),
                            [1.0, 0.80, 0.48], random.uniform(0.55, 0.95))
                # lamp head
                arr[max(0, jy - 2):jy + 2, max(0, jx - 2):jx + 2] = [1.0, 0.95, 0.80]
                # pool on the pavement
                radial_glow(arr, jx, jy + 12, random.uniform(30, 60),
                            [1.0, 0.70, 0.40], random.uniform(0.10, 0.22))

    if neon:
        neon_cols = [
            [1.0, 0.12, 0.40], [0.10, 0.95, 1.0], [1.0, 0.50, 0.08],
            [0.45, 1.0, 0.25], [0.85, 0.25, 1.0], [1.0, 0.92, 0.20],
        ]
        for _ in range(max(8, W * H // 20000)):
            side = random.choice([0, 1])
            x = random.randint(int(W * 0.02), int(W * 0.18)) if side == 0 else \
                random.randint(int(W * 0.82), int(W * 0.98))
            y = random.randint(int(H * 0.08), int(H * 0.9))
            col = random.choice(neon_cols)
            w = random.randint(max(4, W // 40), max(6, W // 14))
            h = random.randint(max(2, H // 80), max(4, H // 30))
            glow = Image.new("RGB", (W, H), (0, 0, 0))
            d = ImageDraw.Draw(glow)
            d.rectangle([x, y, x + w, y + h], fill=tuple(int(c * 255) for c in col))
            glow = glow.filter(ImageFilter.GaussianBlur(random.uniform(6, 14)))
            arr += np.asarray(glow).astype(np.float32) / 255.0 * 1.0
            # crisp sign core, with a darker frame
            core = Image.new("RGB", (W, H), (0, 0, 0))
            dc = ImageDraw.Draw(core)
            dc.rectangle([x + 1, y + 1, x + w - 1, y + h - 1],
                         fill=tuple(int(c * 255) for c in col))
            ca = np.asarray(core).astype(np.float32) / 255.0
            arr = np.maximum(arr, ca * 0.9)
            # reflection smear below the sign
            refl = np.zeros_like(arr)
            y0, y1 = y + h, min(H, y + h + int(h * 2.2))
            if y1 > y0:
                blend = np.linspace(0.45, 0.0, y1 - y0, dtype=np.float32)[:, None, None]
                refl[y0:y1, x:x + w] = ca[y0:y1, x:x + w] * 0.55
                refl[y0:y1] *= blend
                arr += refl

    arr = np.clip(arr, 0, 1)
    ensure(out_path)
    to_img(arr).save(out_path, quality=92, optimize=True, progressive=True)
    print(f"  street {os.path.basename(path)} -> {os.path.relpath(out_path, ROOT)}")


def roof_night(path, out_path):
    im = Image.open(path).convert("RGB")
    day = to_np(im)
    arr = day * np.array([0.20, 0.215, 0.27], dtype=np.float32) + 0.008
    # a few dim warm penthouses / skylights
    H, W = arr.shape[:2]
    em = Image.new("RGB", (W, H), (0, 0, 0))
    d = ImageDraw.Draw(em)
    for _ in range(max(3, W * H // 22000)):
        x, y = random.randint(0, W - 1), random.randint(0, H - 1)
        w, h = random.randint(4, 12), random.randint(3, 8)
        d.rectangle([x, y, x + w, y + h], fill=(255, 200, 130))
    glow = em.filter(ImageFilter.GaussianBlur(5))
    arr += np.asarray(em).astype(np.float32) / 255.0 * 0.55
    arr += np.asarray(glow).astype(np.float32) / 255.0 * 0.30
    arr = np.clip(arr, 0, 1)
    ensure(out_path)
    to_img(arr).save(out_path, quality=92, optimize=True, progressive=True)
    print(f"  roof {os.path.basename(path)} -> {os.path.relpath(out_path, ROOT)}")


# --------------------------------------------------------------------------
if __name__ == "__main__":
    only = sys.argv[1] if len(sys.argv) > 1 else None

    facades = sorted(os.listdir(os.path.join(DAY, "facades")))
    for f in facades:
        if only and only not in f:
            continue
        facade_night(os.path.join(DAY, "facades", f),
                     os.path.join(NIGHT, "facades", f.replace(".jpg", "_night.jpg")))

    if not only or only in ("ground",):
        satellite_night(os.path.join(DAY, "ground", "satellite_manhattan_ground.jpg"),
                        os.path.join(NIGHT, "ground", "satellite_manhattan_ground_night.jpg"))

    if not only or only in ("roof",):
        roof_night(os.path.join(DAY, "roofs", "roof_concrete_gravel.jpg"),
                   os.path.join(NIGHT, "roofs", "roof_concrete_gravel_night.jpg"))

    if not only or only in ("street",):
        street_night(os.path.join(DAY, "streets", "road_asphalt.jpg"),
                     os.path.join(NIGHT, "streets", "road_asphalt_night.jpg"),
                     lamps=True, neon=True, lane_lines=True)
        street_night(os.path.join(DAY, "streets", "sidewalk_concrete.jpg"),
                     os.path.join(NIGHT, "streets", "sidewalk_concrete_night.jpg"),
                     lamps=True, neon=False)
        street_night(os.path.join(DAY, "streets", "pavement_weave_roughness.jpg"),
                     os.path.join(NIGHT, "streets", "pavement_weave_roughness_night.jpg"),
                     lamps=False, neon=False)
