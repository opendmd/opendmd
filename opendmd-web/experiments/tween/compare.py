# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow", "numpy", "scipy"]
# ///
"""Numbers behind the RIFE assessment, from the out/tl/*.npz timelines.

Per file and RIFE variant (native, 4x):
  visibly_new  share of pixels in RIFE's raw in-betweens further than 24
               (RGB distance) from every source colour: palette breakage you
               would see, not the +-1 drift a snap undoes
  vs_exact     on pairs the shift tweener also did (rigid motion, where its
               integer-step frame is the right answer): share of the pair's
               changed pixels where RIFE-snapped differs from it
  neither      on pairs the shift tweener left alone (cut / other /
               animation): share of changed pixels where RIFE-snapped is
               neither A's nor B's pixel, i.e. a dissolve or a warp

    uv run compare.py    -> out/compare.md"""
import json, sys
from pathlib import Path
import numpy as np

HERE = Path(__file__).resolve().parent
OUT = HERE / "out"


def load(p):
    z = np.load(p)
    return z["frames"], z["delays"], z["synth"] if "synth" in z else np.zeros(len(z["frames"]), bool)


def groups(synth):
    """-> {original index: [timeline indices of its in-betweens]}"""
    g, orig = {}, -1
    for j, s in enumerate(synth):
        if not s:
            orig += 1
        else:
            g.setdefault(orig, []).append(j)
    return g


def main():
    meta = json.loads((OUT / "candidates.json").read_text())
    rows = []
    for m in meta:
        stem = m["stem"]
        of, _, _ = load(OUT / "tl" / f"{stem}.orig.npz")
        n = len(of)
        pal = np.unique(of.reshape(-1, 3), axis=0).astype(np.int32)
        tf, _, ts = load(OUT / "tl" / f"{stem}.tween.npz")
        tg = groups(ts)
        for tag in ("rife", "rife4x"):
            p = OUT / "tl" / f"{stem}.{tag}.npz"
            if not p.exists():
                continue
            rf, _, rs = load(p)
            sf, _, _ = load(OUT / "tl" / f"{stem}.{tag}snap.npz")
            rg = groups(rs)
            vis, exact, neither = [], [], []
            for i, idx in rg.items():
                a, b = of[i].astype(np.int32), of[(i + 1) % n].astype(np.int32)
                ch = (a != b).any(-1)
                nch = max(1, int(ch.sum()))
                for j in idx:
                    f = rf[j].reshape(-1, 1, 3).astype(np.int32)
                    d = np.sqrt(((f - pal[None]) ** 2).sum(-1).min(1))
                    vis.append(float((d > 24).mean()))
                if i in tg and len(tg[i]) == len(idx):
                    for jt, jr in zip(tg[i], idx):
                        exact.append(float(((sf[jr] != tf[jt]).any(-1) & (ch | (tf[jt] != a).any(-1))).sum() / nch))
                elif i not in tg:
                    for jr in idx:
                        s = sf[jr].astype(np.int32)
                        bad = ~(s == a).all(-1) & ~(s == b).all(-1)
                        neither.append(float((bad & ch).sum() / nch))
            mean = lambda v: round(float(np.mean(v)), 3) if v else None  # noqa: E731
            rows.append(dict(stem=stem, role=m["role"], tag=tag, frames=int(rs.sum()),
                             visibly_new=mean(vis), vs_exact=mean(exact), n_exact=len(exact),
                             neither=mean(neither), n_neither=len(neither)))
    (OUT / "compare.json").write_text(json.dumps(rows, indent=1))
    lines = ["| role | file | RIFE | in-betweens | visibly new colour px | differs from exact shift (rigid pairs, n) | neither A nor B (other pairs, n) |",
             "|---|---|---|---:|---:|---:|---:|"]
    f = lambda v: "-" if v is None else f"{100 * v:.0f}%"  # noqa: E731
    for r in rows:
        lines.append(f"| {r['role']} | `{r['stem'].split('__')[-1]}` | {'native' if r['tag'] == 'rife' else '4x'} | {r['frames']} | "
                     f"{f(r['visibly_new'])} | {f(r['vs_exact'])} ({r['n_exact']}) | {f(r['neither'])} ({r['n_neither']}) |")
    (OUT / "compare.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    for tag in ("rife", "rife4x"):
        rr = [r for r in rows if r["tag"] == tag]
        for key in ("visibly_new", "vs_exact", "neither"):
            v = [r[key] for r in rr if r[key] is not None]
            if v:
                print(tag, key, "median", round(float(np.median(v)), 3), "mean", round(float(np.mean(v)), 3))


if __name__ == "__main__":
    main()
