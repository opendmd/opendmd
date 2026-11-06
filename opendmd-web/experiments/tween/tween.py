# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow", "numpy", "scipy"]
# ///
"""Step 2: the shift tweener on the candidates picked from measure.py's
out/files.jsonl: the top 20 by tweenable motion time, plus 10 random
controls whose pairs are mostly `cut`/`other`.

    uv run tween.py [--top 20] [--controls 10] [--seed 1]

Writes out/gifs/<name>.tween.gif, out/tl/<stem>.{orig,tween}.npz (frames +
delays, what sheets.py and rife.py read), out/candidates.json and
out/tween_table.md. The source card is never written to."""
import argparse, hashlib, json, random, re, sys
from collections import Counter
from pathlib import Path
import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import (HERE, CARD_GIFS, load_gif, codes, decode, classify, n_inbetweens,  # noqa: E402
                    split_delay, inbetween, region_mask, tweenable)
from measure import hold_limit  # noqa: E402

ROOT = CARD_GIFS
OUT = HERE / "out"


def stem_of(path):
    return str(Path(path).relative_to(ROOT).with_suffix("")).replace("/", "__")


def pick(top, ncontrols, seed):
    rows = [json.loads(l) for l in open(OUT / "files.jsonl")]
    ok = [r for r in rows if "error" not in r]
    # The pack holds byte-identical copies under two categories (Other/ and
    # The_80s_90s/) and numbered series of the same scroll (Marvels01..05):
    # keep one copy and at most two per series, so the 20 are 20 kinds.
    cands, seen, series = [], set(), Counter()
    for r in sorted(ok, key=lambda r: (-r["tween_ms"], -r["tween_new"])):
        h = hashlib.md5(Path(r["path"]).read_bytes()).hexdigest()
        ser = re.sub(r"[\W_]*\d+$", "", Path(r["path"]).stem).lower()
        ser = re.sub(r"[^a-z]", "", re.sub(r"^(other|the_80s_90s)_(scroll_)?", "", ser))
        if h in seen or series[ser] >= 2:
            continue
        seen.add(h)
        series[ser] += 1
        cands.append(r)
        if len(cands) == top:
            break
    chosen = {r["path"] for r in cands}

    def messy(r):
        p = r["pairs"]
        tot = sum(p.values())
        return tot >= 10 and (p.get("cut", 0) + p.get("other", 0)) >= 0.6 * tot

    pool = sorted(r["path"] for r in ok if messy(r) and r["path"] not in chosen)
    controls = random.Random(seed).sample(pool, ncontrols)
    return [("candidate", r["path"]) for r in cands] + [("control", p) for p in controls]


def tween_file(path):
    frames, delays, _ = load_gif(path)
    c = codes(frames)
    n = len(c)
    hl = hold_limit(delays)
    out_c, out_d, synth = [], [], []
    pairs = []  # per pair: class, shift, k
    for i in range(n):
        out_c.append(c[i])
        synth.append(False)
        d = delays[i]
        if n < 2:
            out_d.append(d)
            continue
        a, b = c[i], c[(i + 1) % n]
        k = classify(a, b)
        kk = n_inbetweens(k["shift"], d, hl) if k["shift"] else 0
        if kk and not tweenable(k, a, b):
            kk = 0
        pairs.append(dict(i=i, cls=k["cls"], shift=k["shift"], k=kk, d=d, bbox=k["bbox"], fit=k.get("fit")))
        if not kk:
            out_d.append(d)
            continue
        region = region_mask(k["bbox"], k["shift"]) if k["cls"] == "regional" else None
        parts = split_delay(d, kk)
        out_d.append(parts[0])
        for t in range(1, kk + 1):
            out_c.append(inbetween(a, b, k["shift"], t / (kk + 1), region))
            out_d.append(parts[t])
            synth.append(True)
    return frames, delays, np.stack(out_c), out_d, np.array(synth), pairs


def write_gif(path, rgb_frames, delays):
    flat = rgb_frames.reshape(-1, 3)
    cols, inv = np.unique(flat[:, 0].astype(np.int32) << 16 | flat[:, 1].astype(np.int32) << 8 | flat[:, 2],
                          return_inverse=True)
    if len(cols) <= 256:
        pal = decode(cols).reshape(-1, 3)
        pal_bytes = pal.tobytes() + bytes(3 * (256 - len(pal)))
        idx = inv.reshape(rgb_frames.shape[:3]).astype(np.uint8)
        ims = []
        for f in idx:
            im = Image.fromarray(f, "P")
            im.putpalette(pal_bytes)
            ims.append(im)
    else:  # > 256 colours overall: let Pillow pick per frame (not hit by the tweener)
        ims = [Image.fromarray(f).quantize(256, dither=Image.Dither.NONE) for f in rgb_frames]
    ims[0].save(path, save_all=True, append_images=ims[1:], duration=list(delays), loop=0,
                optimize=False, disposal=1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--top", type=int, default=20)
    ap.add_argument("--controls", type=int, default=10)
    ap.add_argument("--seed", type=int, default=1)
    a = ap.parse_args()
    (OUT / "gifs").mkdir(parents=True, exist_ok=True)
    (OUT / "tl").mkdir(parents=True, exist_ok=True)
    picks = pick(a.top, a.controls, a.seed)
    meta, rows = [], []
    for role, path in picks:
        stem = stem_of(path)
        frames, delays, oc, od, synth, pairs = tween_file(path)
        tf = decode(oc)
        write_gif(OUT / "gifs" / f"{Path(path).stem}.tween.gif", tf, od)
        np.savez_compressed(OUT / "tl" / f"{stem}.orig.npz", frames=frames, delays=np.array(delays))
        np.savez_compressed(OUT / "tl" / f"{stem}.tween.npz", frames=tf, delays=np.array(od),
                            synth=synth)
        assert sum(od) == sum(delays) and min(od) >= 20
        cls = Counter(p["cls"] for p in pairs)
        tw = Counter(p["cls"] for p in pairs if p["k"])
        mags = Counter(max(abs(p["shift"][0]), abs(p["shift"][1])) for p in pairs if p["k"])
        m = dict(role=role, path=path, stem=stem, name=Path(path).stem, frames=len(frames),
                 frames_after=len(oc), ms=sum(delays), delays=Counter(delays).most_common(3),
                 delays_after=Counter(od).most_common(3), synth_share=float(synth.mean()),
                 pairs=dict(cls), tweened=dict(tw), mags=dict(sorted(mags.items())),
                 tween_pairs=[(p["i"], p["cls"], p["shift"], p["k"], p["d"]) for p in pairs if p["k"]])
        meta.append(m)
        print(f"{role:9} {m['name'][:48]:48} {m['frames']:5} -> {m['frames_after']:5} "
              f"synth {100 * m['synth_share']:4.1f}% tweened {dict(tw)} mags {m['mags']}")
    (OUT / "candidates.json").write_text(json.dumps(meta, indent=1))
    fmt = lambda c: ", ".join(f"{d}x{n}" for d, n in c)  # noqa: E731
    lines = ["| role | file | frames | after | delays (ms x count) | after | synthesised | pairs tweened (T/R) | shift sizes |",
             "|---|---|---:|---:|---|---|---:|---|---|"]
    for m in meta:
        lines.append(f"| {m['role']} | `{m['stem'].replace('__', '/')}` | {m['frames']} | {m['frames_after']} | "
                     f"{fmt(m['delays'])} | {fmt(m['delays_after'])} | {100 * m['synth_share']:.0f}% | "
                     f"{m['tweened'].get('translation', 0)}/{m['tweened'].get('regional', 0)} | "
                     f"{', '.join(f'{k}px x{v}' for k, v in m['mags'].items())} |")
    (OUT / "tween_table.md").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
