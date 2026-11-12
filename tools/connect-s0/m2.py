# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M2: the baseline. A build WITHOUT connect-probe (the same version's
release build), 30 min of GIF play then 30 min of a raced Formula; its
summary.json is the reference M3, M4, M6 and M10 compare with.

    uv run tools/connect-s0/m2.py 192.168.0.178 --play formula=@formula:<a raced one>
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import s0lib  # noqa: E402

ap = s0lib.parser(__doc__)
s0lib.add_phases(ap)
args = ap.parse_args()
run = s0lib.Run("m2", args)
st = run.sample("start")
if isinstance(st.get("connect"), dict):
    run.note("this build has connect-probe: a baseline should be the build without it")
for phase in args.phases.split(","):
    s0lib.start_phase(run, phase, args)
    run.sample_for(args.minutes * 60, phase)
    run.phases[phase] = stats = run.phase_stats(phase)
    print(f"  {phase}: {stats.get('deltas')}", flush=True)
    run.gate_content(phase)
    missing = [k for k in s0lib.LATE if k not in stats.get("deltas", {})]
    run.gate(f"{phase}: telemetry complete", not missing, missing or "all late-frame counters", "every counter reported")
run.gate("baseline build (no connect)", not isinstance(st.get("connect"), dict), "connect-probe" if isinstance(st.get("connect"), dict) else "none", "a build without connect-probe")
run.gate_no_restart()
sys.exit(run.finish())
