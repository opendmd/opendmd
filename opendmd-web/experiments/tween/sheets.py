# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow", "numpy", "scipy"]
# ///
"""Side-by-side sheets through the LED renderer (art/process/led.py, the
numpy port of opendmd-render). One PNG per file: time runs left to right in
20 ms slices (what the panel shows at that instant), one row per method:

  original        the GIF as it plays
  persistence     original through a phosphor decay, per 10 ms refresh:
                  out = max(new, prev * 0.6) in linear light
  shift tween     tween.py's output
  RIFE raw/snap   rife.py's output (native and 4x-nearest input), if
                  out/tl/<stem>.rife*.npz exist

Synthesised frames get a mark above the cell (orange: shift tween, blue:
RIFE). The window is the stretch with the most tweening (the file's busiest
motion for controls), cropped to the moving area.

    uv run sheets.py [--slices 12] [--scale 5] [--width 48] [stem ...]"""
import argparse, json, sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from common import LED_DIR, W, H  # noqa: E402
sys.path.insert(0, str(LED_DIR))
from led import Led  # noqa: E402

OUT = HERE / "out"
STEP = 20  # ms per slice
TICK = 10  # panel refresh, ms


def load_tl(p):
    z = np.load(p)
    return z["frames"], z["delays"].astype(int), (z["synth"] if "synth" in z else np.zeros(len(z["frames"]), bool))


def at(delays, t):
    """Index of the frame showing at time t (ms), looping."""
    ends = np.cumsum(delays)
    t = t % ends[-1]
    return int(np.searchsorted(ends, t, side="right"))


def persistence(frames, delays, times, decay=0.6, gamma=2.2, warm=300):
    """Frames seen at `times` with per-refresh decay toward the new value."""
    out = []
    t = times[0] - warm
    lin_prev = None
    want = list(times)
    while want:
        cur = (frames[at(delays, t)].astype(np.float32) / 255) ** gamma
        lin_prev = cur if lin_prev is None else np.maximum(cur, lin_prev * decay)
        if t >= want[0]:
            out.append(np.clip(lin_prev ** (1 / gamma) * 255 + 0.5, 0, 255).astype(np.uint8))
            want.pop(0)
        t += TICK
    return out


def window(meta, delays, nslices, frames=None):
    """Start of the 4 consecutive pairs with the most tweening, weighted by
    how many pixels change (so a sprite's first 2 px entering the frame
    doesn't win over the sprite crossing it)."""
    starts = np.concatenate([[0], np.cumsum(delays)[:-1]])
    tp = meta.get("tween_pairs") or []
    if tp:
        n = len(delays)
        ks = {i: k * (1 if frames is None else
                      1 + int((frames[i] != frames[(i + 1) % n]).any(-1).sum()))
              for i, _, _, k, _ in tp}
        score = [sum(ks.get(j, 0) for j in range(i, i + 4)) for i in range(n)]
        return int(starts[int(np.argmax(score))])
    return None


def busiest(frames, delays, nslices):
    starts = np.concatenate([[0], np.cumsum(delays)[:-1]])
    ch = np.array([(frames[i] != frames[(i + 1) % len(frames)]).any(-1).sum() for i in range(len(frames))])
    k = max(1, nslices * STEP // max(1, int(np.median(delays))))
    score = np.convolve(ch, np.ones(k), "valid") if len(ch) >= k else ch
    return int(starts[int(np.argmax(score))])


def crop_x(seq, width):
    ch = np.zeros(W, bool)
    for a, b in zip(seq, seq[1:]):
        ch |= (a != b).any(-1).any(0)
    xs = np.nonzero(ch)[0]
    cx = (xs.min() + xs.max()) // 2 if len(xs) else W // 2
    return int(np.clip(cx - width // 2, 0, W - width))


def sheet(stem, meta, nslices, scale, width):
    rows = [("original", OUT / "tl" / f"{stem}.orig.npz", None),
            ("shift tween", OUT / "tl" / f"{stem}.tween.npz", (255, 140, 0)),
            ("RIFE raw", OUT / "tl" / f"{stem}.rife.npz", (60, 140, 255)),
            ("RIFE snapped", OUT / "tl" / f"{stem}.rifesnap.npz", (60, 140, 255)),
            ("RIFE 4x raw", OUT / "tl" / f"{stem}.rife4x.npz", (60, 140, 255)),
            ("RIFE 4x snapped", OUT / "tl" / f"{stem}.rife4xsnap.npz", (60, 140, 255))]
    tls = [(name, *load_tl(p), mark) for name, p, mark in rows if p.exists()]
    of, od, _ = tls[0][1:4]
    t0 = window(meta, od, nslices, of)
    if t0 is None:
        t0 = busiest(of, od, nslices)
    times = [t0 + j * STEP for j in range(nslices)]
    cells = []  # (name, [frames], [synth flags], mark)
    for name, fr, de, sy, mark in tls:
        idx = [at(de, t) for t in times]
        cells.append((name, [fr[i] for i in idx], [bool(sy[i]) for i in idx], mark))
    cells.insert(1, ("persistence 0.6", persistence(of, od, times), [False] * nslices, None))
    x0 = crop_x(cells[0][1] + cells[2][1], width) if width < W else 0
    led = Led(scale=scale)
    cw, chh = width * scale, H * scale
    lab, top, gap = 120, 22, 6
    img = Image.new("RGB", (lab + nslices * (cw + gap), top + len(cells) * (chh + gap + 4)), (24, 24, 24))
    dr = ImageDraw.Draw(img)
    for j, t in enumerate(times):
        dr.text((lab + j * (cw + gap) + 2, 4), f"+{t - t0} ms", fill=(200, 200, 200))
    for r, (name, frs, sy, mark) in enumerate(cells):
        y = top + r * (chh + gap + 4)
        dr.text((4, y + chh // 2 - 6), name, fill=(230, 230, 230))
        for j, f in enumerate(frs):
            x = lab + j * (cw + gap)
            big = led.render(f)[:, x0 * scale : (x0 + width) * scale]
            img.paste(Image.fromarray(big), (x, y + 4))
            if sy[j] and mark:
                dr.rectangle([x, y, x + cw - 1, y + 2], fill=mark)
    (OUT / "sheets").mkdir(exist_ok=True)
    p = OUT / "sheets" / f"{stem}.png"
    img.save(p, optimize=True)
    return p


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--slices", type=int, default=12)
    ap.add_argument("--scale", type=int, default=5)
    ap.add_argument("--width", type=int, default=48)
    ap.add_argument("stems", nargs="*")
    a = ap.parse_args()
    meta = json.loads((OUT / "candidates.json").read_text())
    for m in meta:
        if a.stems and m["stem"] not in a.stems:
            continue
        print(sheet(m["stem"], m, a.slices, a.scale, a.width))


if __name__ == "__main__":
    main()
