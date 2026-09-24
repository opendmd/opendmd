"""Shared pieces of the tweening experiment: GIF loading with the player's
delay rules, the per-pair motion classifier, and the shift tweener.

Frames are compared as colour codes (r<<16 | g<<8 | b, int32), which is
equality on the indexed image without caring which palette slot a colour
sits in. Everything is 128x32: other sizes are centre-cropped (or padded
with black), as playback does."""
from pathlib import Path
import numpy as np
from PIL import Image
from scipy import ndimage

W, H = 128, 32
R = 8  # shift search radius, px
HERE = Path(__file__).resolve().parent
LED_DIR = HERE.parent / "art" / "process"

# Offsets ordered by magnitude, so argmax on ties picks the smallest shift.
SHIFTS = sorted(
    [(dx, dy) for dy in range(-R, R + 1) for dx in range(-R, R + 1) if (dx, dy) != (0, 0)],
    key=lambda s: (max(abs(s[0]), abs(s[1])), abs(s[0]) + abs(s[1])),
)
SDX = np.array([s[0] for s in SHIFTS])
SDY = np.array([s[1] for s in SHIFTS])

STATIC_MAX = 0.005  # share of pixels
TRANS_MIN = 0.90
REGION_MIN = 0.80
CUT_MIN = 0.60
# The tweener is stricter than the classifier: a pair whose shift leaves more
# than 3% of its changed pixels unexplained is usually motion *plus* an
# animation change (a sprite turning, a morph), and in-betweens of that mix
# the two poses. Measured on the sheets; see docs/tweening-experiment.md.
TWEEN_FIT = 0.97
# ...and the two ends must agree on what passes through each pixel at every
# integer step (see `inconsistency`): a pure scroll scores 0; a text wipe, a
# laser growing along its own direction or a sprite that changes pose while
# it jumps score 0.5-6. Measured on the candidates and controls.
INCONS_MAX = 0.25


def player_delay(d):
    """GifSource's rules: 0 -> 100 ms, anything else floored at 20 ms."""
    d = int(d or 0)
    return 100 if d == 0 else max(20, d)


def fit(rgb):
    h, w = rgb.shape[:2]
    if (h, w) == (H, W):
        return rgb
    out = np.zeros((H, W, 3), np.uint8)
    sy, sx = max(0, (h - H) // 2), max(0, (w - W) // 2)
    dy, dx = max(0, (H - h) // 2), max(0, (W - w) // 2)
    ch, cw = min(h, H), min(w, W)
    out[dy : dy + ch, dx : dx + cw] = rgb[sy : sy + ch, sx : sx + cw]
    return out


def load_gif(path):
    """-> (frames (N,32,128,3) uint8, delays [ms, after player rules], native size)."""
    im = Image.open(path)
    size = im.size
    frames, delays = [], []
    i = 0
    while True:
        try:
            im.seek(i)
        except EOFError:
            break
        rgba = np.asarray(im.convert("RGBA"))
        rgb = np.where(rgba[..., 3:4] > 0, rgba[..., :3], 0).astype(np.uint8)
        frames.append(fit(rgb))
        delays.append(player_delay(im.info.get("duration", 0)))
        i += 1
    return np.stack(frames), delays, size


def codes(frames):
    f = frames.astype(np.int32)
    return (f[..., 0] << 16) | (f[..., 1] << 8) | f[..., 2]


SAMPLE = 512  # changed pixels used for the shift search (strided subsample)
WP = W + 2 * R
OFF = -SDY * WP - SDX  # flat offsets of p - s in the padded image


def _pad(img):
    out = np.full((img.shape[0] + 2 * R, img.shape[1] + 2 * R), -1, np.int32)
    out[R:-R, R:-R] = img
    return out.ravel()


def _best_shift(A, B, ys, xs):
    """Best shift s for the changed pixels (ys, xs). A changed pixel p is
    explained if it arrived (B[p] == A[p - s]) or left (A[p] == B[p + s]):
    the second covers the pixels a sprite vacates over a textured background.
    Returns (share explained among pixels with either end in frame, (dx,dy)).
    At most SAMPLE pixels (strided) are tested; frames are 128 wide."""
    if len(ys) > SAMPLE:
        step = len(ys) / SAMPLE
        sel = (np.arange(SAMPLE) * step).astype(int)
        ys, xs = ys[sel], xs[sel]
    Ap, Bp = _pad(A), _pad(B)
    idx = (ys + R) * WP + xs + R
    a_from = Ap[idx[None, :] + OFF[:, None]]
    b_to = Bp[idx[None, :] - OFF[:, None]]
    av, bv = Ap[idx], Bp[idx]
    ok1, ok2 = a_from >= 0, b_to >= 0
    match = ((a_from == bv[None, :]) & ok1) | ((b_to == av[None, :]) & ok2)
    nin = (ok1 | ok2).sum(1)
    share = match.sum(1) / np.maximum(nin, 1)
    # Must be substantially in frame, or a big shift "explains" a sliver.
    share = np.where(nin >= 0.5 * len(ys), share, 0)
    i = int(np.argmax(share))
    return float(share[i]), (int(SDX[i]), int(SDY[i]))


def _frame_fit(A, B, s):
    """Share of the whole frame (where A[p-s] is in frame) with B[p] == A[p-s].
    A sprite moving over a static textured background explains its changed
    pixels with its own shift, but the background does not follow it: that
    pair is regional, not a global translation."""
    SA = _shifted(A, *s)
    ok = SA >= 0
    return float(((SA == B) & ok).sum() / max(ok.sum(), 1))


def classify(A, B):
    """A, B: (32,128) int32 codes. Returns dict(cls, changed, shift, bbox)."""
    diff = A != B
    n = int(diff.sum())
    total = A.size
    if n <= STATIC_MAX * total:
        return dict(cls="static", changed=n, shift=None, bbox=None)
    ys, xs = np.nonzero(diff)
    share, s = _best_shift(A, B, ys, xs)
    if share >= TRANS_MIN and _frame_fit(A, B, s) >= TRANS_MIN:
        return dict(cls="translation", changed=n, shift=s, bbox=None, fit=share)
    # One connected changed region (8-connected, gaps of 2 px bridged).
    grown = ndimage.binary_dilation(diff, iterations=2)
    lab, nlab = ndimage.label(grown, structure=np.ones((3, 3), bool))
    if nlab >= 1:
        sizes = np.bincount(lab[ys, xs], minlength=nlab + 1)
        big = int(np.argmax(sizes))
        if sizes[big] >= 0.9 * n:
            sel = lab[ys, xs] == big
            ry, rx = ys[sel], xs[sel]
            share, s = _best_shift(A, B, ry, rx)
            if share >= REGION_MIN:
                bbox = (int(ry.min()), int(ry.max()) + 1, int(rx.min()), int(rx.max()) + 1)
                return dict(cls="regional", changed=n, shift=s, bbox=bbox, fit=share)
    if n > CUT_MIN * total:
        return dict(cls="cut", changed=n, shift=None, bbox=None)
    return dict(cls="other", changed=n, shift=None, bbox=None)


# --- The shift tweener -------------------------------------------------------

def split_delay(d, k):
    """Split d ms into k+1 parts on the GIF's 10 ms grid, each >= 20 ms.
    Returns None if it can't be done."""
    cs = d // 10
    if k < 1 or cs < 2 * (k + 1) or d % 10:
        return None
    base, rem = divmod(cs, k + 1)
    return [(base + (1 if i < rem else 0)) * 10 for i in range(k + 1)]


def inconsistency(A, B, k):
    """Worst, over the integer steps of the shift, of the number of pixels
    where the moved A and the moved-back B disagree (inside the region,
    where either the pair changes or the step changes the pixel; for a
    regional pair, where the moved object lands), as a share
    of the pair's changed pixels. Rigid motion scores 0."""
    dx, dy = k["shift"]
    n = max(abs(dx), abs(dy))
    region = region_mask(k["bbox"], k["shift"]) if k["cls"] == "regional" else None
    changed = A != B
    nch = max(1, int(changed.sum()))
    if region is not None:  # only the moving object, as inbetween() pastes it
        OA = (ndimage.binary_dilation(changed, iterations=n) & region
              & (A == _shifted(B, -dx, -dy))).astype(np.int32)
    worst = 0.0
    for t in range(1, n):
        ox, oy = round(t * dx / n), round(t * dy / n)
        SA = _shifted(A, ox, oy)
        SB = _shifted(B, ox - dx, oy - dy)
        bad = (SA >= 0) & (SB >= 0) & (SA != SB) & (changed | (SA != A))
        if region is not None:
            bad &= _shifted(OA, ox, oy, 0) > 0
        worst = max(worst, bad.sum() / nch)
    return worst


def tweenable(k, A, B):
    """The tweener's gate, stricter than the classifier (see TWEEN_FIT)."""
    return (k["cls"] in ("translation", "regional") and k.get("fit", 0) >= TWEEN_FIT
            and inconsistency(A, B, k) <= INCONS_MAX)


def n_inbetweens(shift, d, hold_limit):
    """How many integer-step in-betweens a pair gets: one per extra pixel of
    motion, as many as the delay can be split into >= 20 ms parts, none on a
    hold (a long delay is a pause, not slow motion)."""
    if shift is None or d > hold_limit:
        return 0
    n = max(abs(shift[0]), abs(shift[1]))
    k = n - 1
    while k >= 1 and split_delay(d, k) is None:
        k -= 1
    return max(k, 0)


def _shifted(img, dx, dy, fill=-1):
    """out[p] = img[p - (dx,dy)], fill where out of bounds."""
    h, w = img.shape
    out = np.full_like(img, fill)
    ys0, ys1 = max(0, dy), min(h, h + dy)
    xs0, xs1 = max(0, dx), min(w, w + dx)
    out[ys0:ys1, xs0:xs1] = img[ys0 - dy : ys1 - dy, xs0 - dx : xs1 - dx]
    return out


def inbetween(A, B, shift, f, region=None):
    """One in-between at fraction f (0<f<1) of an integer shift (dx,dy).
    A, B: (32,128) int32 codes. Integer steps only; every output pixel is a
    pixel of A or B from somewhere; nothing is blended. o = round(f*shift).

    Global translation (region None): the frame is A moved by o, with the
    strip entering the frame taken from B moved back by shift - o; where
    both ends are in frame and disagree (a static HUD, a flicker), a pixel
    that is the same in A and B keeps it, otherwise the nearer end shows.

    Regional (a sprite over a background): layers. The object is the part
    of the region that moves with the shift (A[q] == B[q+s]), taken where A
    changes or within n px of a change. The background is A where the
    object isn't in A, else B where it isn't in B, else (hidden at both
    ends) the nearer end. The object is pasted at o. Outside the region, A."""
    dx, dy = shift
    ox, oy = round(f * dx), round(f * dy)
    near = A if f < 0.5 else B
    if region is None:
        SA = _shifted(A, ox, oy)
        SB = _shifted(B, ox - dx, oy - dy)
        va, vb = SA >= 0, SB >= 0
        out = np.where(A == B, A, near)
        out = np.where(va & ~vb, SA, out)
        out = np.where(vb & ~va, SB, out)
        return np.where(va & vb & (SA == SB), SA, out)
    n = max(abs(dx), abs(dy))
    changed = A != B
    nearby = ndimage.binary_dilation(changed, iterations=n) & region
    OA = nearby & (A == _shifted(B, -dx, -dy)) & region
    OB = _shifted(OA.astype(np.int32), dx, dy, 0) > 0
    bg = np.where(~OA, A, np.where(~OB, B, near))
    OI = _shifted(OA.astype(np.int32), ox, oy, 0) > 0
    out = np.where(OI, _shifted(A, ox, oy), bg)
    return np.where(region, out, A)


def region_mask(bbox, shift):
    """The regional pair's bbox grown by the shift, which covers the path."""
    y0, y1, x0, x1 = bbox
    n = max(abs(shift[0]), abs(shift[1]))
    m = np.zeros((H, W), bool)
    m[max(0, y0 - n) : y1 + n, max(0, x0 - n) : x1 + n] = True
    return m


def decode(c):
    return np.stack([(c >> 16) & 255, (c >> 8) & 255, c & 255], -1).astype(np.uint8)
