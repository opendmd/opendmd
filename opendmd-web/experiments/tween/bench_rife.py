# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow", "numpy", "scipy"]
# ///
"""RIFE timing on this CPU (lavapipe): one directory-mode batch of real
pairs (A1 B1 A2 B2 ..., one in-between per gap at 0.5), native and 4x, and a
2-frame batch for the fixed start-up cost. Prints ms per inference with the
start-up subtracted, and the load average.

    uv run bench_rife.py [stem]"""
import os, sys, tempfile
from pathlib import Path
import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import rife  # noqa: E402

stem = sys.argv[1] if len(sys.argv) > 1 else "Other__OTHER_SCROLL_Xmen01"
fr = np.load(HERE / "out" / "tl" / f"{stem}.orig.npz")["frames"]
tmp = tempfile.mkdtemp(prefix="rife-bench-", dir=os.environ.get("TMPDIR", "/tmp"))
print("load", os.getloadavg())
for scale, npairs in ((1, 60), (4, 20)):
    _, t_one = rife.run_batch([(fr[0], fr[1])], 2, scale, tmp)
    _, t_all = rife.run_batch([(fr[i], fr[i + 1]) for i in range(npairs)], 2, scale, tmp)
    inf = 2 * npairs - 1  # gaps computed, B->A included
    print(f"scale {scale}: start-up + 1 inference {t_one:.2f} s; {inf} inferences {t_all:.2f} s "
          f"-> {1000 * (t_all - t_one) / (inf - 1):.0f} ms per inference")
print("load", os.getloadavg())
