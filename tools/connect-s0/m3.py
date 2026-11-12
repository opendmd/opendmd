# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M3: the probe's session held open and idle (pings only), the same two
half-hours as M2. Gates (CONNECT.md 10): heap floor >= 40 KB; late frames
and wake_late not above M2's; refresh_us the same; the session open
throughout; no restart.

    uv run tools/connect-s0/m3.py 192.168.0.178 --baseline connect-s0-results/m2-.../summary.json \\
        --play formula=@formula:<the same one>
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import s0lib  # noqa: E402

ap = s0lib.parser(__doc__)
s0lib.add_phases(ap)
s0lib.add_baseline(ap)
ap.add_argument("--floor-kb", type=float, default=40)
args = ap.parse_args()
base = s0lib.load_baseline(args.baseline)
run = s0lib.Run("m3", args)
s0lib.need_probe(run.sample("start"))
s0lib.wait_open(run)
for phase in args.phases.split(","):
    s0lib.start_phase(run, phase, args)
    st = run.sample_for(args.minutes * 60, phase)
    run.phases[phase] = stats = run.phase_stats(phase)
    rows = [r for r in run.rows if r["phase"] == phase]
    not_open = sum(1 for r in rows if r.get("connect.state") != "open")
    reconnects = (rows[-1].get("connect.reconnects") or 0) - (rows[0].get("connect.reconnects") or 0) if rows else None
    run.gate(f"{phase}: session open throughout", not_open == 0 and reconnects == 0, f"{not_open} samples not open, {reconnects} reconnects", "0 and 0")
    run.gate_floor(phase, stats, args.floor_kb)
    run.gate_no_added_late(phase, stats, base, args.tolerance)
    run.gate_refresh_same(phase, stats, base)
run.gate_no_restart()
sys.exit(run.finish())
