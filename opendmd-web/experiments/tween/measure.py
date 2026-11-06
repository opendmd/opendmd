# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow", "numpy", "scipy"]
# ///
"""Step 1: measure the archive. For every GIF, the frame delays (after the
player's rules) and a motion class per consecutive frame pair, including the
loop-wrap pair (last -> first). Parallel over files.

    uv run measure.py [<card copy>/gifs] [--jobs 14] [--limit N]

Writes out/files.jsonl (one summary per file) and out/summary.json, and
prints the tables used in docs/tweening-experiment.md."""
import argparse, json, re, sys, time
from collections import Counter
from multiprocessing import Pool
from pathlib import Path
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import HERE, CARD_GIFS, load_gif, codes, classify, n_inbetweens, tweenable  # noqa: E402

CLASSES = ["static", "translation", "regional", "cut", "other"]
# Other experiments write variants beside the pack's GIFs (<name>.cap12.gif,
# <name>.cap10d.gif, ...): only the originals are measured.
DERIVED = re.compile(r"\.(cap\w+|tween)\.gif$")
DBUCKETS = [("<=29", 0, 29), ("30-49", 30, 49), ("50-79", 50, 79), (">=80", 80, 10**9)]


def dbucket(d):
    for name, lo, hi in DBUCKETS:
        if lo <= d <= hi:
            return name


def hold_limit(delays):
    return max(2 * int(np.median(delays)), 200)


def analyse(path):
    try:
        frames, delays, size = load_gif(path)
    except Exception as e:
        return dict(path=str(path), error=f"{type(e).__name__}: {e}")
    c = codes(frames)
    n = len(c)
    r = dict(path=str(path), frames=n, size=list(size), delays=Counter(), pairs=Counter(),
             px=Counter(), ms=Counter(), shift_mag=Counter(), tween_pairs=0, tween_new=0,
             tween_ms=0, loose_pairs=0, loose_new=0, trans_ms=0, trans_pairs=0, colours=0)
    for d in delays:
        r["delays"][dbucket(d)] += 1
    r["colours"] = int(len(np.unique(c)))
    hl = hold_limit(delays)
    npairs = n if n > 1 else 0  # includes the loop wrap
    for i in range(npairs):
        a, b = c[i], c[(i + 1) % n]
        k = classify(a, b)
        cls, d = k["cls"], delays[i]
        r["pairs"][cls] += 1
        r["px"][cls] += k["changed"]
        r["ms"][cls] += d
        if cls in ("translation", "regional"):
            m = max(abs(k["shift"][0]), abs(k["shift"][1]))
            r["shift_mag"][f"{cls}:{min(m, 5)}:{dbucket(d)}"] += 1
            if cls == "translation":
                r["trans_pairs"] += 1
                r["trans_ms"] += d
            kk = n_inbetweens(k["shift"], d, hl)
            if kk:
                r["loose_pairs"] += 1
                r["loose_new"] += kk
            if kk and not tweenable(k, a, b):
                kk = 0
            if kk:
                r["tween_pairs"] += 1
                r["tween_new"] += kk
                r["tween_ms"] += d
    for key in ("delays", "pairs", "px", "ms", "shift_mag"):
        r[key] = dict(r[key])
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("root", nargs="?", default=CARD_GIFS)
    ap.add_argument("--jobs", type=int, default=14)
    ap.add_argument("--limit", type=int, default=0)
    a = ap.parse_args()
    files = sorted(p for p in Path(a.root).rglob("*.gif") if not DERIVED.search(p.name))
    if a.limit:
        files = files[:: max(1, len(files) // a.limit)][: a.limit]
    out = HERE / "out"
    out.mkdir(exist_ok=True)
    t0 = time.time()
    res = []
    with Pool(a.jobs) as pool, open(out / "files.jsonl", "w") as fh:
        for i, r in enumerate(pool.imap_unordered(analyse, files, chunksize=4)):
            fh.write(json.dumps(r) + "\n")
            res.append(r)
            if i % 500 == 0:
                print(f"{i}/{len(files)} {time.time() - t0:.0f}s", file=sys.stderr)
    print(f"{len(files)} files in {time.time() - t0:.0f}s", file=sys.stderr)
    summarise(res, a.root)


def summarise(res, root=CARD_GIFS):
    ok = [r for r in res if "error" not in r]
    bad = [r for r in res if "error" in r]
    S = dict(files=len(res), decoded=len(ok), errors=[(r["path"], r["error"]) for r in bad])
    tot = lambda key: sum((Counter(r[key]) for r in ok), Counter())  # noqa: E731
    S["frames"] = sum(r["frames"] for r in ok)
    S["not_128x32"] = sum(1 for r in ok if r["size"] != [128, 32])
    for key in ("delays", "pairs", "px", "ms", "shift_mag"):
        S[key] = dict(tot(key))
    for key in ("tween_pairs", "tween_new", "tween_ms", "loose_pairs", "loose_new", "trans_pairs", "trans_ms"):
        S[key] = sum(r[key] for r in ok)
    S["files_with_tweenable"] = sum(1 for r in ok if r["tween_pairs"])
    S["files_tweenable_10pct"] = sum(1 for r in ok if r["tween_pairs"] >= 0.1 * r["frames"])
    rel = lambda p: str(Path(p).relative_to(root))  # noqa: E731
    top_lit = sorted(ok, key=lambda r: -r["trans_ms"])[:20]
    S["top_literal"] = [dict(file=rel(r["path"]), frames=r["frames"], trans_pairs=r["trans_pairs"],
                             trans_ms=r["trans_ms"], tween_pairs=r["tween_pairs"]) for r in top_lit]
    top_tw = sorted(ok, key=lambda r: (-r["tween_ms"], -r["tween_new"]))[:20]
    S["top_tween"] = [dict(file=rel(r["path"]), frames=r["frames"], tween_pairs=r["tween_pairs"],
                           tween_new=r["tween_new"], tween_ms=r["tween_ms"], trans_pairs=r["trans_pairs"])
                      for r in top_tw]
    (HERE / "out" / "summary.json").write_text(json.dumps(S, indent=1))
    report(S)


def report(S):
    print(f"\nfiles {S['files']}, decoded {S['decoded']}, frames {S['frames']}, not 128x32 {S['not_128x32']}")
    nf = sum(S["delays"].values())
    print("\ndelay bucket | frames | share")
    for name, _, _ in DBUCKETS[::-1]:
        v = S["delays"].get(name, 0)
        print(f"{name} | {v} | {100 * v / nf:.1f}%")
    npair = sum(S["pairs"].values())
    npx = sum(S["px"].values())
    nms = sum(S["ms"].values())
    print("\nclass | pairs | share | changed px share | time share")
    for c in CLASSES:
        print(f"{c} | {S['pairs'].get(c, 0)} | {100 * S['pairs'].get(c, 0) / npair:.1f}% | "
              f"{100 * S['px'].get(c, 0) / npx:.1f}% | {100 * S['ms'].get(c, 0) / nms:.1f}%")
    print("\nshift magnitude (translation+regional) by delay bucket:")
    agg = Counter()
    for k, v in S["shift_mag"].items():
        cls, m, db = k.split(":")
        agg[(cls, int(m), db)] += v
    for cls in ("translation", "regional"):
        for m in range(1, 6):
            row = [agg.get((cls, m, db), 0) for db, _, _ in DBUCKETS]
            print(f"{cls} |{'>=5' if m == 5 else m}| " + " | ".join(map(str, row)))
    print(f"\nloose (class only) tweenable pairs {S['loose_pairs']}, new frames {S['loose_new']}")
    print(f"\ntweenable pairs {S['tween_pairs']} ({100 * S['tween_pairs'] / npair:.2f}% of pairs), "
          f"new frames {S['tween_new']}, files with any {S['files_with_tweenable']}, "
          f">=10% of pairs {S['files_tweenable_10pct']}")
    print("\ntop 20 literal (translation pairs x delay):")
    for r in S["top_literal"]:
        print(r)
    print("\ntop 20 tweenable:")
    for r in S["top_tween"]:
        print(r)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--report":
        report(json.loads((HERE / "out" / "summary.json").read_text()))
    else:
        main()
