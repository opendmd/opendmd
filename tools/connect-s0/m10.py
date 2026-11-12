# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M10 (owner, 2026-11-12: remote GIF sends are in the MVP): a 512 KB binary
stream from the echo Worker to the panel in credited chunks (1,000 B, a
window of 4) while play goes on; the panel checks every byte against the
seed's stream and its FNV-1a against the server's, then drops them (no card
write). Repeated --runs times (20). Gates: 0 added late frames against M2's
baseline over the transfers; heap floor >= 26 KB; no restart; checksum ok;
throughput reported.

    uv run tools/connect-s0/m10.py 192.168.0.178 --baseline .../m2-.../summary.json [--phase gif] [--runs 3]
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import s0lib  # noqa: E402

ap = s0lib.parser(__doc__)
ap.add_argument("--phase", default="gif", help="the baseline's phase to compare with (what is playing): gif or formula")
ap.add_argument("--play", default=None, help="POST /api/play this first (e.g. @formula:<one>)")
ap.add_argument("--runs", type=int, default=20)
ap.add_argument("--bytes", type=int, default=524288)
ap.add_argument("--chunk", type=int, default=1000)
ap.add_argument("--window", type=int, default=4)
ap.add_argument("--floor-kb", type=float, default=26)
s0lib.add_baseline(ap)
args = ap.parse_args()
if args.sample_s == 5.0:
    args.sample_s = 1.0
base = s0lib.load_baseline(args.baseline)
run = s0lib.Run("m10", args)
s0lib.need_probe(run.sample("start"))
if args.phase == "formula" and not args.play:
    sys.exit("--phase formula needs --play @formula:<a raced one>")
s0lib.reset_probe(run)
if args.play:
    run.panel.play(args.play)
run.gate("the session opened", s0lib.wait_open(run), "open", "open within 180 s")
done = []
for k in range(args.runs):
    s0lib.wait_open(run, phase=f"{args.phase}:open")
    seq = run.panel.post("stream", bytes=args.bytes, chunk=args.chunk, window=args.window, seed=k + 1).get("ask")
    st = run.sample_for(240, args.phase, until=lambda s, seq=seq: s0lib.get(s, "connect.m10.seq") == seq
                        and s0lib.get(s, "connect.m10.state") not in ("running", "waiting for the session", None))
    m = s0lib.get(st or {}, "connect.m10") or {}
    if m.get("seq") != seq:
        # Not this request's: it never started, or the read failed.
        m = {"seq": seq, "state": f"no result for request {seq}"}
    done.append(m)
    print(f"  run {k + 1}: {m.get('state')} {m.get('bytes')} B in {m.get('ms')} ms ({m.get('kb_s')} KB/s), checksum {m.get('checksum_ok')}, heap low {m.get('heap_low')}", flush=True)
stats = run.phase_stats(args.phase)
stats["transfers"] = done
run.phases[args.phase] = stats
ok = [m for m in done if m.get("checksum_ok") is True and m.get("bytes") == args.bytes]
run.gate("every transfer complete, checksum ok", len(ok) == args.runs, f"{len(ok)} of {args.runs}", f"{args.runs}")
rates = [m.get("kb_s") for m in done if m.get("kb_s")]
run.gate("throughput (reported)", None, f"{rates} KB/s", "a figure", required=False)
run.gate_floor(args.phase, stats, args.floor_kb)
run.gate_content(args.phase)
lows = [m.get("heap_low") for m in done if isinstance(m.get("heap_low"), int)]
run.gate(f"heap low during the transfers >= {args.floor_kb:g} KB", bool(lows) and min(lows) >= args.floor_kb * 1024, min(lows) if lows else None, f">= {int(args.floor_kb * 1024)} B (sampled at each record)")
run.gate_no_added_late(args.phase, stats, base, args.tolerance)
run.gate_no_restart()
sys.exit(run.finish())
