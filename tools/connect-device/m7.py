# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M7 (D1's second half): the session left alone against the real server.
24 h at the build's own ping interval (P1: 90 s, WebSocket pings; read from
GET /api/debug/connect), then steps of 150 and 300 s (each --step-hours,
default 2): reconnects and their causes, each session's length, and the
idle limit Cloudflare applies (the first ping interval whose sessions end
by themselves). Gate (CONNECT.md 10): <= 10 reconnects a day at the build's
interval. The script sets the interval itself
(POST /api/debug/connect/config?ping_s=N) and puts the build's back at the
end. (Fixed for P1: S0's echo server and its text and echo pings are gone.)

    uv run tools/connect-device/m7.py 192.0.2.178 [--hours 24] [--steps 150,300] [--step-hours 2]
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import devlib  # noqa: E402

ap = devlib.parser(__doc__)
ap.add_argument("--hours", type=float, default=24)
ap.add_argument("--steps", default="150,300")
ap.add_argument("--step-hours", type=float, default=2)
ap.add_argument("--per-day", type=float, default=10)
args = ap.parse_args()
if args.sample_s == 5.0:
    args.sample_s = 30.0
run = devlib.Run("m7", args)
devlib.need_probe(run.sample("start"))
devlib.reset_probe(run)
own = int(devlib.get(run.panel.debug(), "cfg.ping_s") or 90)
plan = [(own, args.hours)] + [(int(s), args.step_hours) for s in args.steps.split(",") if s]
results = {}
try:
    for ping_s, hours in plan:
        name = f"ping-{ping_s}s"
        print(f"\n== {name} for {hours:g} h ==", flush=True)
        run.panel.post("config", ping_s=ping_s)
        opened = devlib.wait_open(run, phase=f"{name}:open")
        run.gate(f"{name}: the session opened", opened, "open" if opened else "not open", "open within 180 s")
        st0 = run.sample(name)
        run.sample_for(hours * 3600, name)
        st1 = run.sample(name)
        c0, c1 = st0.get("connect") or {}, st1.get("connect") or {}
        rows = [r for r in run.rows if r["phase"] == name]
        # A failed status read counts as not open.
        failed_reads = sum(1 for r in rows if r.get("read_failed"))
        open_share = sum(1 for r in rows if r.get("connect.state") == "open") / max(1, len(rows))
        rec = (c1.get("reconnects") or 0) - (c0.get("reconnects") or 0)
        attempts = (c1.get("attempts") or 0) - (c0.get("attempts") or 0)
        causes = {k: (c1.get("causes") or {}).get(k, 0) - (c0.get("causes") or {}).get(k, 0) for k in (c1.get("causes") or {})}
        # Attempts, not only sessions that ended: a failed attempt is a
        # reconnect too (the first sample's session is already counted).
        per_day = max(rec, attempts) * 24 / max(hours, 1e-9)
        results[name] = {"hours": hours, "reconnects": rec, "attempts": attempts, "open_share": round(open_share, 4),
                         "per_day": round(per_day, 2), "causes": {k: v for k, v in causes.items() if v},
                         "sessions_tail": c1.get("sessions"), "session_max_s": c1.get("session_max_s"),
                         "pings": (devlib.get(c1, "ping.sent") or 0) - (devlib.get(c0, "ping.sent") or 0),
                         "missed": (devlib.get(c1, "ping.missed") or 0) - (devlib.get(c0, "ping.missed") or 0),
                         "rtt_max_ms": devlib.get(c1, "ping.rtt_max_ms")}
        print(f"  {name}: {rec} sessions ended, {attempts} attempts ({per_day:.1f} a day), open {open_share:.2%} of samples, causes {results[name]['causes']}", flush=True)
        run.gate(f"{name}: session open in >= 99 % of samples", open_share >= 0.99, f"{open_share:.2%} of {len(rows)} (failed reads count as not open)", ">= 99 %",
                 required=(ping_s == own))
        run.gate(f"{name}: status reads answered", failed_reads / max(1, len(rows)) <= 0.01, f"{failed_reads} of {len(rows)} failed", "<= 1 %")
        results[name]["failed_reads"] = failed_reads
finally:
    try:
        run.panel.post("config", ping_s=own)
    except Exception as e:  # noqa: BLE001
        run.note(f"could not put ping_s={own} back: {e}")
run.phases = results
first = results.get(f"ping-{own}s", {})
run.gate(f"<= {args.per_day:g} reconnects a day at {own} s", first.get("per_day", 1e9) <= args.per_day, f"{first.get('per_day')} a day ({first.get('causes')})", f"<= {args.per_day:g}")
limit = next((n for n, r in results.items() if r["per_day"] > max(args.per_day, 4 * first.get("per_day", 0) + 2)), None)
run.gate("idle limit", None, f"sessions start dropping at {limit}" if limit else f"none of the steps up to {plan[-1][0]} s dropped sessions: the limit is above it",
         "a figure", required=False)
run.gate_no_restart()
sys.exit(run.finish())
