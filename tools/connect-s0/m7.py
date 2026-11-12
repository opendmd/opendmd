# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M7: the session left alone. 24 h with pings every 45 s, then steps of 90,
150 and 300 s (each --step-hours, default 2): reconnects and their causes,
each session's length, and the idle limit Cloudflare applies (the first
ping interval whose sessions end by themselves). Gate (CONNECT.md 10): <= 10
reconnects a day at 45 s. The script sets the interval itself
(POST /api/debug/connect/config?ping_s=N) and puts 45 s back at the end.

    uv run tools/connect-s0/m7.py 192.168.0.178 [--hours 24] [--steps 90,150,300] [--step-hours 2] [--ping ws|text|echo]
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import s0lib  # noqa: E402

ap = s0lib.parser(__doc__)
ap.add_argument("--hours", type=float, default=24)
ap.add_argument("--steps", default="90,150,300")
ap.add_argument("--step-hours", type=float, default=2)
ap.add_argument("--ping", default="ws", choices=["ws", "text", "echo"])
ap.add_argument("--per-day", type=float, default=10)
args = ap.parse_args()
if args.sample_s == 5.0:
    args.sample_s = 30.0
run = s0lib.Run("m7", args)
s0lib.need_probe(run.sample("start"))
s0lib.reset_probe(run)
plan = [(45, args.hours)] + [(int(s), args.step_hours) for s in args.steps.split(",") if s]
results = {}
try:
    for ping_s, hours in plan:
        name = f"ping-{ping_s}s"
        print(f"\n== {name} for {hours:g} h ==", flush=True)
        run.panel.post("config", ping_s=ping_s, ping=args.ping)
        opened = s0lib.wait_open(run, phase=f"{name}:open")
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
                         "pings": (s0lib.get(c1, "ping.sent") or 0) - (s0lib.get(c0, "ping.sent") or 0),
                         "missed": (s0lib.get(c1, "ping.missed") or 0) - (s0lib.get(c0, "ping.missed") or 0),
                         "rtt_max_ms": s0lib.get(c1, "ping.rtt_max_ms")}
        print(f"  {name}: {rec} sessions ended, {attempts} attempts ({per_day:.1f} a day), open {open_share:.2%} of samples, causes {results[name]['causes']}", flush=True)
        run.gate(f"{name}: session open in >= 99 % of samples", open_share >= 0.99, f"{open_share:.2%} of {len(rows)} (failed reads count as not open)", ">= 99 %",
                 required=(ping_s == 45))
        run.gate(f"{name}: status reads answered", failed_reads / max(1, len(rows)) <= 0.01, f"{failed_reads} of {len(rows)} failed", "<= 1 %")
        results[name]["failed_reads"] = failed_reads
finally:
    try:
        run.panel.post("config", ping_s=45, ping="ws")
    except Exception as e:  # noqa: BLE001
        run.note(f"could not put ping_s=45 back: {e}")
run.phases = results
first = results.get("ping-45s", {})
run.gate(f"<= {args.per_day:g} reconnects a day at 45 s", first.get("per_day", 1e9) <= args.per_day, f"{first.get('per_day')} a day ({first.get('causes')})", f"<= {args.per_day:g}")
limit = next((n for n, r in results.items() if r["per_day"] > max(args.per_day, 4 * first.get("per_day", 0) + 2)), None)
run.gate("idle limit", None, f"sessions start dropping at {limit}" if limit else f"none of the steps up to {plan[-1][0]} s dropped sessions: the limit is above it",
         "a figure", required=False)
run.gate_no_restart()
sys.exit(run.finish())
