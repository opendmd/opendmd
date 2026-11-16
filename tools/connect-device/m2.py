# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M2 (D1's first half): the baseline. The build under test with Remote
control OFF (Settings, or `PUT /api/connect {"enabled": false}`), 30 min
of GIF play then 30 min of a raced Formula (D1: --minutes 720 for 24 h);
its summary.json is the reference m3.py, m5.py and m6.py compare with.

    uv run tools/connect-device/m2.py 192.0.2.178 --play formula=@formula:<a raced one>
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import devlib  # noqa: E402

ap = devlib.parser(__doc__)
devlib.add_phases(ap)
args = ap.parse_args()
run = devlib.Run("m2", args)
st = run.sample("start")
on = isinstance(st.get("connect"), dict) and st["connect"].get("enabled") is True
if on:
    run.note("Remote control is on: a baseline wants it off")
for phase in args.phases.split(","):
    devlib.start_phase(run, phase, args)
    run.sample_for(args.minutes * 60, phase)
    run.phases[phase] = stats = run.phase_stats(phase)
    print(f"  {phase}: {stats.get('deltas')}", flush=True)
    run.gate_content(phase)
    missing = [k for k in devlib.LATE if k not in stats.get("deltas", {})]
    run.gate(f"{phase}: telemetry complete", not missing, missing or "all late-frame counters", "every counter reported")
run.gate("baseline: Remote control off", not on, "on" if on else "off", "off (or not in the build)")
run.gate_no_restart()
sys.exit(run.finish())
