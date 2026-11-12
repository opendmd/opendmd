# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M9: one Ed25519 verification timed on the device (a fixed vector, 300
bytes, verify_strict, checked 100 times on the network worker, where a
command's would be). Gates (CONNECT.md 10): <= 50 ms each; >= 1 KB of the
worker's stack left; every one verified.

    uv run tools/connect-s0/m9.py 192.168.0.178 [--n 100]
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import s0lib  # noqa: E402

ap = s0lib.parser(__doc__)
ap.add_argument("--n", type=int, default=100)
ap.add_argument("--max-ms", type=float, default=50)
args = ap.parse_args()
if args.sample_s == 5.0:
    args.sample_s = 1.0
run = s0lib.Run("m9", args)
s0lib.need_probe(run.sample("start"))
run.panel.post("ed25519", n=args.n)
st = run.sample_for(args.n * 2 + 60, "m9", until=lambda s: s0lib.get(s, "connect.m9.done") is True)
m = s0lib.get(st or {}, "connect.m9") or {}
run.phases["m9"] = {**run.phase_stats("m9"), "m9": m}
print(f"  {m}", flush=True)
run.gate("all verified", m.get("n") == args.n and m.get("failed") == 0, f"{m.get('n')} done, {m.get('failed')} failed", f"{args.n}, 0")
run.gate(f"each <= {args.max_ms:g} ms", m.get("max_us") is not None and m["max_us"] <= args.max_ms * 1000,
         f"min {m.get('min_us')} mean {m.get('mean_us')} max {m.get('max_us')} us", f"max <= {int(args.max_ms * 1000)} us")
run.gate(">= 1 KB of the worker's stack left", (m.get("stack_unused") or 0) >= 1024, m.get("stack_unused"), ">= 1024 B")
run.gate_no_restart()
sys.exit(run.finish())
