# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow", "numpy", "scipy"]
# ///
"""Step 3: RIFE (rife-ncnn-vulkan 20221029, model rife-v4.6) on the same 30
files, on the CPU through Mesa's lavapipe Vulkan driver (the iGPU needs the
`render` group, which this user isn't in).

Which pairs get in-betweens:
  - the pairs the shift tweener tweened get the same k, at the same
    fractions t/(k+1), so the two can be compared frame for frame;
  - every other non-static pair whose delay splits into two >= 20 ms halves
    (and isn't a hold) gets one in-between at 0.5: the "double the frame
    rate" policy a model-only pipeline would use, cuts and all.
To bound the run, per file: all such pairs in the sheet window, plus up to
--extra more spread over the file (for the colour/timing statistics).
Frames outside those pairs stay original.

Each timestep batch goes through the binary's directory mode: inputs laid
out A1 B1 A2 B2 ..., -n m*len gives timesteps 0, 1/m, ..., (m-1)/m on every
gap; the B->A gaps are wasted work, and timing counts only the wanted
frames' share (reported both ways).

    RIFE=/path/to/rife-ncnn-vulkan-dir uv run rife.py [--scale 1|4] [--extra 30] [--window-only]

Writes out/tl/<stem>.rife[4x].npz (raw) and .rife[4x]snap.npz (snapped to
the source palette), out/rife[4x].json (stats)."""
import argparse, json, os, shutil, subprocess, sys, tempfile, time
from pathlib import Path
import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from common import classify, codes, split_delay  # noqa: E402
from measure import hold_limit  # noqa: E402
import sheets  # noqa: E402

OUT = HERE / "out"
RIFE = Path(os.environ.get("RIFE", "/tmp/scratch/rife/rife-ncnn-vulkan-20221029-ubuntu"))
ENV = dict(os.environ, VK_ICD_FILENAMES="/usr/share/vulkan/icd.d/lvp_icd.json")


def run_batch(pairs, m, scale, tmp):
    """pairs: [(A_rgb, B_rgb)] -> ([[frame at t/m for t in 1..m-1]], seconds)."""
    ind, outd = Path(tmp) / "in", Path(tmp) / "out"
    for d in (ind, outd):
        shutil.rmtree(d, ignore_errors=True)
        d.mkdir()
    seq = [f for ab in pairs for f in ab]
    for i, f in enumerate(seq):
        im = Image.fromarray(f)
        if scale > 1:
            im = im.resize((im.width * scale, im.height * scale), Image.NEAREST)
        im.save(ind / f"{i + 1:08d}.png")
    t0 = time.time()
    subprocess.run(["./rife-ncnn-vulkan", "-i", str(ind), "-o", str(outd), "-m", "rife-v4.6",
                    "-n", str(m * len(seq))], cwd=RIFE, env=ENV, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    dt = time.time() - t0
    res = []
    for j in range(len(pairs)):
        fr = []
        for t in range(1, m):
            im = Image.open(outd / f"{2 * j * m + t + 1:08d}.png").convert("RGB")
            if scale > 1:  # nearest back down: the centre pixel of each block
                a = np.asarray(im)[scale // 2 :: scale, scale // 2 :: scale]
            else:
                a = np.asarray(im)
            fr.append(np.ascontiguousarray(a))
        res.append(fr)
    return res, dt


def snap(frame, pal):
    """Nearest source colour (RGB distance)."""
    f = frame.reshape(-1, 1, 3).astype(np.int32)
    d = ((f - pal[None].astype(np.int32)) ** 2).sum(-1)
    return pal[d.argmin(1)].reshape(frame.shape)


def plan(meta, frames, delays, extra, window_only, nslices=12):
    c = codes(frames)
    n = len(c)
    hl = hold_limit(delays)
    tk = {i: k for i, _, _, k, _ in meta["tween_pairs"]}
    want = {}
    for i in range(n if n > 1 else 0):
        if i in tk:
            want[i] = tk[i]
            continue
        d = delays[i]
        if d > hl or split_delay(d, 1) is None:
            continue
        if classify(c[i], c[(i + 1) % n])["cls"] == "static":
            continue
        want[i] = 1
    t0 = sheets.window(meta, delays, nslices, frames)
    if t0 is None:
        t0 = sheets.busiest(frames, delays, nslices)
    starts = np.concatenate([[0], np.cumsum(delays)[:-1]])
    t1 = t0 + nslices * sheets.STEP
    win = [i for i in want if starts[i] < t1 and starts[i] + delays[i] > t0 - 200]
    rest = [i for i in want if i not in win]
    if window_only or not rest:
        pick = win
    else:
        pick = win + [rest[j] for j in np.linspace(0, len(rest) - 1, min(extra, len(rest))).astype(int)]
    return {i: want[i] for i in sorted(set(pick))}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--scale", type=int, default=1)
    ap.add_argument("--extra", type=int, default=30)
    ap.add_argument("--window-only", action="store_true")
    ap.add_argument("stems", nargs="*")
    a = ap.parse_args()
    tag = "rife" if a.scale == 1 else f"rife{a.scale}x"
    meta_all = json.loads((OUT / "candidates.json").read_text())
    stats = []
    tmp = tempfile.mkdtemp(prefix="rife-", dir=os.environ.get("TMPDIR", "/tmp"))
    for meta in meta_all:
        if a.stems and meta["stem"] not in a.stems:
            continue
        z = np.load(OUT / "tl" / f"{meta['stem']}.orig.npz")
        frames, delays = z["frames"], [int(d) for d in z["delays"]]
        n = len(frames)
        todo = plan(meta, frames, delays, a.extra, a.window_only)
        pal = np.unique(frames.reshape(-1, 3), axis=0)
        palset = set(map(tuple, pal))
        got, secs, wanted, computed = {}, 0.0, 0, 0
        for m in sorted({k + 1 for k in todo.values()}):
            idx = [i for i, k in todo.items() if k + 1 == m]
            res, dt = run_batch([(frames[i], frames[(i + 1) % n]) for i in idx], m, a.scale, tmp)
            secs += dt
            wanted += len(idx) * (m - 1)
            computed += (2 * len(idx) - 1) * (m - 1)
            got.update(zip(idx, res))
        raw, snp, syn, dl = [], [], [], []
        newc, newpx = [], []
        for i in range(n):
            raw.append(frames[i]); snp.append(frames[i]); syn.append(False)
            if i not in got:
                dl.append(delays[i])
                continue
            k = len(got[i])
            parts = split_delay(delays[i], k)
            dl.append(parts[0])
            for t, f in enumerate(got[i]):
                cols = {tuple(x) for x in np.unique(f.reshape(-1, 3), axis=0)}
                nc = cols - palset
                newc.append(len(nc))
                newpx.append(float(np.mean([tuple(x) not in palset for x in f.reshape(-1, 3)])))
                raw.append(f); snp.append(snap(f, pal)); syn.append(True); dl.append(parts[t + 1])
        assert sum(dl) == sum(delays)
        np.savez_compressed(OUT / "tl" / f"{meta['stem']}.{tag}.npz", frames=np.stack(raw), delays=np.array(dl), synth=np.array(syn))
        np.savez_compressed(OUT / "tl" / f"{meta['stem']}.{tag}snap.npz", frames=np.stack(snp), delays=np.array(dl), synth=np.array(syn))
        s = dict(stem=meta["stem"], role=meta["role"], pairs=len(todo), frames=wanted,
                 secs=round(secs, 2), ms_per_frame=round(1000 * secs / max(wanted, 1)),
                 ms_per_inference=round(1000 * secs / max(computed, 1)),
                 new_colours_mean=round(float(np.mean(newc)), 1) if newc else 0,
                 new_colours_max=int(max(newc)) if newc else 0,
                 new_px_share=round(float(np.mean(newpx)), 3) if newpx else 0,
                 source_colours=len(pal))
        stats.append(s)
        print(s, flush=True)
    shutil.rmtree(tmp, ignore_errors=True)
    (OUT / f"{tag}.json").write_text(json.dumps(stats, indent=1))


if __name__ == "__main__":
    main()
