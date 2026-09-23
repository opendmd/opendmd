"""Processing primitives for 128x32 panel art (prototype; the real thing
would be wasm/JS). Arrays are (h, w, 3) uint8 RGB, or (h, w, 4) RGBA."""
import os, subprocess
from pathlib import Path
import numpy as np
from PIL import Image

W, H = 128, 32
HERE = Path(__file__).resolve().parent
SCRATCH = Path(os.environ.get("ART_TMP", "/tmp/opendmd-art"))


# --- Recovering native pixels ----------------------------------------------

def _axis_scale(img, axis, kmax=8, thr=60):
    """Integer upscale factor + phase along one axis, from where colour
    edges fall. Returns (k, phase, share of edges on the grid)."""
    a = img[..., :3].astype(int)
    d = np.abs(np.diff(a, axis=axis)).sum(2)
    # Only strong edges: JPEG ringing and bilinear ramps make weak ones
    # everywhere, off the grid.
    thr = max(thr, np.percentile(d[d > 0], 75) if (d > 0).any() else thr)
    e = (d > thr).sum(1 - axis)  # edge count per boundary position
    pos = np.nonzero(e)[0] + 1
    w = e[pos - 1]
    if len(pos) < 8:
        return 1, 0, 1.0
    for k in range(kmax, 1, -1):
        hist = np.bincount(pos % k, weights=w, minlength=k)
        ph = int(hist.argmax())
        share = hist[ph] / w.sum()
        if share > 0.8:
            return k, ph, share
    return 1, 0, 1.0


def native(img):
    """Undo a nearest-neighbour integer upscale (2x/3x screenshots are
    common on the web). Samples block centres. Returns (native, (ky, kx))."""
    ky, py, _ = _axis_scale(img, 0)
    kx, px, _ = _axis_scale(img, 1)
    k = min(ky, kx) if ky > 1 and kx > 1 else 1  # square pixels only
    if k == 1:
        return img, (1, 1)
    return img[py % k + k // 2 :: k, px % k + k // 2 :: k], (k, k)


# --- Where the interest is --------------------------------------------------

def detail(img):
    """Per-pixel interest: local contrast (luma gradient) plus how far the
    pixel is from the image's dominant (background) colour."""
    a = img[..., :3].astype(np.float32)
    luma = a @ np.array([0.299, 0.587, 0.114], np.float32)
    gx = np.abs(np.diff(luma, axis=1, append=luma[:, -1:]))
    gy = np.abs(np.diff(luma, axis=0, append=luma[-1:]))
    q = (img[..., :3] >> 4).reshape(-1, 3).astype(int)
    keys = q[:, 0] * 256 + q[:, 1] * 16 + q[:, 2]
    bg = np.bincount(keys).argmax()
    notbg = (keys != bg).reshape(img.shape[:2]).astype(np.float32)
    d = np.minimum(gx + gy, 96) + 24 * notbg
    if img.shape[2] == 4:
        d *= img[..., 3] > 0
    return d


def best_window(img, w, h, centre_bias=0.15):
    """Top-left (x, y) of the w x h window with the most detail (integral
    image), mildly biased towards the centre."""
    d = detail(img)
    ih, iw = d.shape
    w, h = min(w, iw), min(h, ih)
    S = np.pad(d.cumsum(0).cumsum(1), ((1, 0), (1, 0)))
    tot = S[h:, w:] - S[:-h, w:] - S[h:, :-w] + S[:-h, :-w]
    ys, xs = np.mgrid[0 : tot.shape[0], 0 : tot.shape[1]]
    cy, cx = (ih - h) / 2, (iw - w) / 2
    dist = np.hypot((ys - cy) / max(ih, 1), (xs - cx) / max(iw, 1))
    score = tot * (1 - centre_bias * dist)
    y, x = np.unravel_index(score.argmax(), score.shape)
    return int(x), int(y)


def busy_bbox(img, frac=0.02):
    """Bounding box of rows/columns carrying more than `frac` of the peak
    detail — the "crop the border off" step."""
    d = detail(img)
    rows, cols = d.sum(1), d.sum(0)
    r = np.nonzero(rows > frac * rows.max())[0]
    c = np.nonzero(cols > frac * cols.max())[0]
    return int(c[0]), int(r[0]), int(c[-1] + 1), int(r[-1] + 1)


# --- Resampling ---------------------------------------------------------------

def down_nearest(img, f):
    return img[f // 2 :: f, f // 2 :: f]


def down_area(img, f):
    h, w = img.shape[0] // f * f, img.shape[1] // f * f
    a = img[:h, :w].astype(np.float32).reshape(h // f, f, w // f, f, -1)
    return (a.mean((1, 3)) + 0.5).astype(np.uint8)


def down_mode(img, f):
    """Pixel-art-aware: each output pixel is the most common source colour
    of its block (no new colours are invented). Ties go to the colour
    furthest from the block mean, which keeps 1-px outlines alive."""
    h, w = img.shape[0] // f, img.shape[1] // f
    out = np.zeros((h, w, img.shape[2]), np.uint8)
    for y in range(h):
        for x in range(w):
            blk = img[y * f : (y + 1) * f, x * f : (x + 1) * f].reshape(-1, img.shape[2])
            cols, cnt = np.unique(blk, axis=0, return_counts=True)
            best = np.nonzero(cnt == cnt.max())[0]
            if len(best) > 1:
                m = blk[:, :3].mean(0)
                best = [max(best, key=lambda i: np.abs(cols[i, :3].astype(int) - m).sum())]
            out[y, x] = cols[best[0]]
    return out


def resize_area(img, w, h):
    """Non-integer area-average (Pillow BOX) for smooth sources (logos)."""
    mode = "RGBA" if img.shape[2] == 4 else "RGB"
    return np.asarray(Image.fromarray(img, mode).resize((w, h), Image.Resampling.BOX))


# --- Palettes -----------------------------------------------------------------

def colours(img):
    return np.unique(img[..., :3].reshape(-1, 3), axis=0)


def q_mediancut(frames, n, dither=False):
    strip = np.concatenate(frames, 0)
    q = Image.fromarray(strip).quantize(n, method=Image.Quantize.MEDIANCUT,
                                        dither=Image.Dither.FLOYDSTEINBERG if dither else Image.Dither.NONE)
    out = np.asarray(q.convert("RGB"))
    return np.split(out, len(frames), 0)


def _nq_bin():
    exe = SCRATCH / "nq-target" / "release" / "nq"
    if not exe.exists():
        subprocess.run(["cargo", "build", "--release", "--offline", "-q"], cwd=HERE / "nq", check=True,
                       env={**os.environ, "CARGO_TARGET_DIR": str(SCRATCH / "nq-target"), "RUSTUP_TOOLCHAIN": "stable"})
    return exe


def q_neuquant(frames, n, samplefac=10):
    """color_quant 1.1.0's NeuQuant (what the core's ODM encoder uses past
    256 colours), via the nq/ helper."""
    strip = np.concatenate(frames, 0)
    rgba = np.concatenate([strip, np.full(strip.shape[:2] + (1,), 255, np.uint8)], 2)
    out = subprocess.run([str(_nq_bin()), str(n), str(samplefac)], input=rgba.tobytes(),
                         capture_output=True, check=True).stdout
    pal = np.frombuffer(out[: n * 3], np.uint8).reshape(n, 3)
    idx = np.frombuffer(out[n * 3 :], np.uint8).reshape(strip.shape[:2])
    return np.split(pal[idx], len(frames), 0)


def nearest(img, pal):
    a = img[..., :3].reshape(-1, 1, 3).astype(np.int32)
    d = ((a - pal[None].astype(np.int32)) ** 2).sum(2)
    return pal[d.argmin(1)].reshape(img.shape[:2] + (3,))


def q_native(frames, n, source):
    """Snap to the source's own palette, keeping accents: pixel art has few
    colours, and the ones that matter (a yellow coin, a blue gem) are rare,
    which is exactly what count-driven quantisers (median cut, NeuQuant)
    drop. Every output pixel is first snapped to its nearest source colour;
    then n of those are picked greedily — the most used first, then
    whichever is furthest from everything picked so far, weighted by
    log(1 + uses) — and each pixel maps to its nearest pick."""
    src = colours(source)
    snapped = [nearest(f, src) for f in frames]
    cols, cnt = np.unique(np.concatenate(snapped, 0).reshape(-1, 3), axis=0, return_counts=True)
    if len(cols) <= n:
        return snapped
    c = cols.astype(np.float32)
    w = np.log1p(cnt)
    picked = [int(cnt.argmax())]
    dmin = ((c - c[picked[0]]) ** 2).sum(1)
    while len(picked) < n:
        i = int((np.sqrt(dmin) * w).argmax())
        picked.append(i)
        dmin = np.minimum(dmin, ((c - c[i]) ** 2).sum(1))
    pal = cols[picked]
    return [nearest(f, pal) for f in snapped]


# --- Compositing ----------------------------------------------------------------

def on_black(rgba, bg=(0, 0, 0)):
    a = rgba[..., 3:4].astype(np.float32) / 255
    return (rgba[..., :3] * a + np.array(bg) * (1 - a) + 0.5).astype(np.uint8)


def place(img, w=W, h=H, x=None, y=None, bg=(0, 0, 0)):
    """Put img on a w x h canvas (centred by default), clipping."""
    out = np.zeros((h, w, 3), np.uint8)
    out[:] = bg
    ih, iw = img.shape[:2]
    x = (w - iw) // 2 if x is None else x
    y = (h - ih) // 2 if y is None else y
    sx0, sy0 = max(0, -x), max(0, -y)
    dx0, dy0 = max(0, x), max(0, y)
    cw, ch = min(iw - sx0, w - dx0), min(ih - sy0, h - dy0)
    if cw > 0 and ch > 0:
        src = img[sy0 : sy0 + ch, sx0 : sx0 + cw]
        if src.shape[2] == 4:
            m = src[..., 3:4] > 127
            out[dy0 : dy0 + ch, dx0 : dx0 + cw] = np.where(m, src[..., :3], out[dy0 : dy0 + ch, dx0 : dx0 + cw])
        else:
            out[dy0 : dy0 + ch, dx0 : dx0 + cw] = src
    return out


# --- Sprite segmentation --------------------------------------------------------

def components(rgba, gap=1):
    """Bounding boxes of opaque blobs (8-connected, blobs within `gap` px
    merged — detached hands and feet). Reading order."""
    from scipy import ndimage
    m = rgba[..., 3] > 0
    grown = ndimage.binary_dilation(m, iterations=gap) if gap else m
    lab, n = ndimage.label(grown, structure=np.ones((3, 3)))
    boxes = []
    for sl in ndimage.find_objects(lab):
        ys, xs = sl
        sub = m[sl]
        rr, cc = np.nonzero(sub.any(1))[0], np.nonzero(sub.any(0))[0]
        if len(rr) == 0:
            continue
        boxes.append((xs.start + cc[0], ys.start + rr[0], xs.start + cc[-1] + 1, ys.start + rr[-1] + 1))
    rowh = np.median([b[3] - b[1] for b in boxes]) if boxes else 1
    boxes.sort(key=lambda b: (round(b[1] / rowh), b[0]))
    return boxes


def signature(rgba):
    """Colour histogram (4 bits/channel) of the opaque pixels, normalised."""
    px = rgba[rgba[..., 3] > 0][:, :3] >> 5
    h = np.bincount(px[:, 0].astype(int) * 64 + px[:, 1] * 8 + px[:, 2], minlength=512).astype(np.float32)
    return h / max(h.sum(), 1)


def cycles(rgba, boxes, thr=0.5):
    """Group consecutive sprites that look alike (colour histogram overlap
    and similar size) into animation cycles."""
    groups, cur = [], []
    for b in boxes:
        spr = rgba[b[1] : b[3], b[0] : b[2]]
        if cur:
            p = cur[-1]
            ps = rgba[p[1] : p[3], p[0] : p[2]]
            overlap = np.minimum(signature(spr), signature(ps)).sum()
            size_ok = abs((b[2] - b[0]) - (p[2] - p[0])) <= 4 and abs((b[3] - b[1]) - (p[3] - p[1])) <= 4
            same_row = abs(b[1] - p[1]) < (b[3] - b[1])
            if overlap > thr and size_ok and same_row:
                cur.append(b)
                continue
            groups.append(cur)
        cur = [b]
    if cur:
        groups.append(cur)
    return groups


# --- Output ---------------------------------------------------------------------

def save_gif(path, frames, delays_ms, n_colours=None):
    """One global palette for the whole animation (so the ODM prepare step
    keeps the exact colours and never re-quantises), no dithering, loop
    forever."""
    strip = np.concatenate(frames, 0)
    cols = colours(strip)
    if n_colours is None:
        n_colours = min(256, len(cols))
    if len(cols) <= 256:
        pal = cols
    else:
        q = Image.fromarray(strip).quantize(n_colours, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
        pal = np.asarray(q.getpalette()[: 3 * n_colours], np.uint8).reshape(-1, 3)
    palimg = Image.new("P", (1, 1))
    flat = pal.flatten().tolist()
    palimg.putpalette(flat + [0] * (768 - len(flat)))
    ims = [Image.fromarray(f).quantize(palette=palimg, dither=Image.Dither.NONE) for f in frames]
    if isinstance(delays_ms, int):
        delays_ms = [delays_ms] * len(frames)
    ims[0].save(path, save_all=True, append_images=ims[1:], duration=delays_ms, loop=0,
                optimize=False, disposal=1)
    return len(pal)
