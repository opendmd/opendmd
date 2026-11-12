"""OpenDMD Connect spike S0: what the M scripts share (CONNECT.md section 10).

Each M script (m2.py ... m10.py, run with `uv run`) talks to one panel over
its LAN HTTP API only: `GET /api/status` for the telemetry (the console link
is unreliable), and, on a `connect-probe` build, the debug hooks under
`/api/debug/connect`. Nothing here contacts anything else.

Every run writes a directory (default `connect-s0-results/<m>-<UTC time>/`)
holding `samples.csv` (one row per status read, flattened), `samples.jsonl`
(the raw `connect` objects and a few others), and `summary.json` (the phase
statistics and each gate with its value and verdict), and prints the same
summary with PASS / FAIL lines. The exit status is 0 only if every gate
passed.

Counters the gates compare (deltas over a phase; "late frames" in the
CONNECT.md gates): `deadline_misses`, `misses.steady`, `misses.switch`,
`display.wake_late`, `scan.beam.late_frames`, `scan.beam.late_starts`,
`heap.transitions_cut`, `net.heavy_refused`, `streams_aborted`,
`sd.readahead.no_heap` (the read-ahead's yields to low heap). Glow
yields are not in `/api/status` (the console heartbeat only): not gated.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import json
import math
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

COUNTERS = [
    "deadline_misses",
    "misses.steady",
    "misses.switch",
    "display.wake_late",
    "scan.beam.late_frames",
    "scan.beam.late_starts",
    "heap.transitions_cut",
    "net.heavy_refused",
    "streams_aborted",
    "sd.readahead.no_heap",
]
# The late frames of "0 added late frames" (the rest are reported).
LATE = ["deadline_misses", "misses.steady", "display.wake_late", "scan.beam.late_frames"]

KB = 1024


def get(d, path, default=None):
    """`a.b.c` out of nested dicts."""
    for k in path.split("."):
        if not isinstance(d, dict) or k not in d:
            return default
        d = d[k]
    return d


class Panel:
    def __init__(self, host: str, key: str | None = None, timeout: float = 8.0):
        self.base = host if host.startswith("http") else f"http://{host}"
        self.key = key
        self.timeout = timeout

    def _req(self, method: str, path: str):
        req = urllib.request.Request(self.base + path, method=method, headers={"User-Agent": "opendmd-connect-s0"})
        if self.key:
            req.add_header("Authorization", f"Bearer {self.key}")
        if method != "GET":
            req.data = b""
        with urllib.request.urlopen(req, timeout=self.timeout) as r:
            return json.loads(r.read().decode("utf-8"))

    def status(self, tries: int = 3):
        for k in range(tries):
            try:
                return self._req("GET", "/api/status")
            except (urllib.error.URLError, TimeoutError, ConnectionError, json.JSONDecodeError) as e:
                if k == tries - 1:
                    raise
                time.sleep(1.0 + k)
                last = e  # noqa: F841
        return None

    def post(self, what: str, **params):
        q = "&".join(f"{k}={urllib.request.quote(str(v))}" for k, v in params.items() if v is not None)
        return self._req("POST", f"/api/debug/connect/{what}" + (f"?{q}" if q else ""))

    def debug(self):
        return self._req("GET", "/api/debug/connect")

    def play(self, path: str):
        req = urllib.request.Request(
            self.base + "/api/play",
            method="POST",
            data=json.dumps({"path": path}).encode(),
            headers={"Content-Type": "application/json", "User-Agent": "opendmd-connect-s0"},
        )
        if self.key:
            req.add_header("Authorization", f"Bearer {self.key}")
        with urllib.request.urlopen(req, timeout=self.timeout) as r:
            return r.status


def _window(st: dict, k: int):
    w = get(st, "net.heap_low_recent")
    if isinstance(w, list) and len(w) > k and isinstance(w[k], dict):
        return w[k].get("free")
    return None


def flatten(st: dict) -> dict:
    """The row a status read becomes."""
    row = {
        "uptime_s": st.get("uptime_secs"),
        "heap.free": get(st, "heap.free"),
        "heap.floor": get(st, "heap.floor"),
        "heap.idf_min": get(st, "heap.idf_min"),
        "heap_floor_at.largest": get(st, "heap_floor_at.largest"),
        "heap_floor_at.task0": get(st, "heap_floor_at.task0"),
        "refresh_us": st.get("refresh_us"),
        "display.wake_max_us": get(st, "display.wake_max_us"),
        "net.heap_low.free": get(st, "net.heap_low.free"),
        "net.heap_low.at": get(st, "net.heap_low.at"),
        "net.heap_low_recent.0": _window(st, 0),
        "net.heap_low_recent.1": _window(st, 1),
        "net.lwip.time_wait": get(st, "net.lwip.time_wait"),
        "net.lwip.pcbs": get(st, "net.lwip.pcbs"),
        "playing": get(st, "now_playing.path"),
        "formula_frame_us": get(st, "now_playing.formula.frame_us"),
        "restarts.0.reason": (st.get("restarts") or [{}])[0].get("reason") if isinstance(st.get("restarts"), list) else None,
    }
    for c in COUNTERS:
        row[c] = get(st, c)
    for cause, n in (get(st, "misses.by_cause") or {}).items():
        row[f"misses.by_cause.{cause}"] = n
    c = st.get("connect")
    if isinstance(c, dict):
        for k in ["state", "attempts", "opened", "reconnects", "resume_missed", "gate_waits", "stack_unused", "errors", "hs_cpu_ms", "connect_ms"]:
            row[f"connect.{k}"] = c.get(k)
        for h in ["full", "resumed"]:
            for k in ["n", "last", "max", "p50", "p95"]:
                row[f"connect.{h}.{k}"] = get(c, f"{h}.{k}")
        for k in ["at_connect", "hs_low", "low", "low_largest", "open", "tls_hs_peak", "tls_open"]:
            row[f"connect.heap.{k}"] = get(c, f"heap.{k}")
        for k in ["sent", "pongs", "missed", "rtt_ms"]:
            row[f"connect.ping.{k}"] = get(c, f"ping.{k}")
        row["connect.job.done"] = get(c, "job.done")
        row["connect.job.kind"] = get(c, "job.kind")
        row["connect.m10.state"] = get(c, "m10.state")
        row["connect.m10.bytes"] = get(c, "m10.bytes")
        row["connect.last_error"] = c.get("last_error")
    return row


class Run:
    """One measurement: its directory, its samples, its gates."""

    def __init__(self, name: str, args):
        stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        self.dir = Path(args.out) / f"{name}-{stamp}"
        self.dir.mkdir(parents=True, exist_ok=True)
        self.name = name
        self.panel = Panel(args.ip, args.key)
        self.interval = args.sample_s
        self.rows: list[dict] = []
        self.gates: list[dict] = []
        self.phases: dict[str, dict] = {}
        self.notes: list[str] = []
        self.args = vars(args)
        self._csv = None
        self._jsonl = open(self.dir / "samples.jsonl", "a", encoding="utf-8")
        self.first_uptime = None
        self.restarted = False

    # --- sampling ---------------------------------------------------------
    def sample(self, phase: str = "") -> dict:
        st = self.panel.status()
        row = {"t": round(time.time(), 3), "phase": phase, **flatten(st)}
        up = row.get("uptime_s")
        if up is not None:
            if self.rows and self.rows[-1].get("uptime_s") is not None and up + 5 < self.rows[-1]["uptime_s"]:
                self.restarted = True
                self.note(f"RESTART seen: uptime {self.rows[-1]['uptime_s']} -> {up} (restarts[0] {row.get('restarts.0.reason')})")
            if self.first_uptime is None:
                self.first_uptime = up
        self.rows.append(row)
        self._jsonl.write(json.dumps({"t": row["t"], "phase": phase, "connect": st.get("connect"), "net": st.get("net"), "heap": st.get("heap")}) + "\n")
        self._jsonl.flush()
        self._write_csv_row(row)
        return st

    def _write_csv_row(self, row):
        path = self.dir / "samples.csv"
        if self._csv is None:
            self._fields = list(row.keys())
            self._csv_file = open(path, "w", newline="", encoding="utf-8")
            self._csv = csv.DictWriter(self._csv_file, fieldnames=self._fields, extrasaction="ignore")
            self._csv.writeheader()
        new = [k for k in row if k not in self._fields]
        if new:
            # A key first seen later (a new miss cause): rewrite with it.
            self._fields += new
            self._csv_file.close()
            self._csv_file = open(path, "w", newline="", encoding="utf-8")
            self._csv = csv.DictWriter(self._csv_file, fieldnames=self._fields, extrasaction="ignore")
            self._csv.writeheader()
            for r in self.rows:
                self._csv.writerow(r)
        else:
            self._csv.writerow(row)
        self._csv_file.flush()

    def sample_for(self, seconds: float, phase: str, until=None, every=None):
        """Sample every `self.interval` s for `seconds` (or until `until(status)`)."""
        end = time.time() + seconds
        st = None
        while time.time() < end:
            t0 = time.time()
            try:
                st = self.sample(phase)
            except Exception as e:  # noqa: BLE001 - a missed read is data too
                self.note(f"status read failed at {time.strftime('%H:%M:%S')}: {e}")
                st = None
            if st is not None and every:
                every(st)
            if st is not None and until and until(st):
                break
            time.sleep(max(0.0, self.interval - (time.time() - t0)))
        return st

    def note(self, s: str):
        print(f"  note: {s}", flush=True)
        self.notes.append(s)

    # --- statistics ------------------------------------------------------------
    def phase_stats(self, phase: str) -> dict:
        rows = [r for r in self.rows if r["phase"] == phase]
        if len(rows) < 2:
            return {"samples": len(rows)}
        first, last = rows[0], rows[-1]
        dur = max(1e-9, last["t"] - first["t"])
        deltas, rates = {}, {}
        keys = set(COUNTERS) | {k for r in rows for k in r if k.startswith("misses.by_cause.")}
        for k in sorted(keys):
            a, b = first.get(k), last.get(k)
            if isinstance(a, (int, float)) and isinstance(b, (int, float)):
                deltas[k] = b - a
                rates[k] = (b - a) * 3600.0 / dur
        frees = [r["heap.free"] for r in rows if isinstance(r.get("heap.free"), int)]
        refresh = sorted(r["refresh_us"] for r in rows if isinstance(r.get("refresh_us"), (int, float)))
        net_recent = [r[k] for r in rows for k in ("net.heap_low_recent.0", "net.heap_low_recent.1") if isinstance(r.get(k), int)]
        probe_low = [r["connect.heap.low"] for r in rows if isinstance(r.get("connect.heap.low"), int) and r["connect.heap.low"] > 0]
        dropped = isinstance(first.get("heap.floor"), int) and isinstance(last.get("heap.floor"), int) and last["heap.floor"] < first["heap.floor"]
        # This phase's floor: the since-boot floor if it fell during the
        # phase (then it is a value reached in it); else the least of what
        # was sampled in the phase: the free heap, the network side's 30 s
        # windows, the probe's own low (reset as the phase began).
        sampled = frees + net_recent + probe_low[-1:]
        floor_phase = last["heap.floor"] if dropped else (min(sampled) if sampled else None)
        largest = last.get("heap_floor_at.largest") if dropped else last.get("connect.heap.low_largest")
        return {
            "samples": len(rows),
            "duration_s": round(dur, 1),
            "deltas": deltas,
            "rates_per_h": {k: round(v, 3) for k, v in rates.items()},
            "floor_start": first.get("heap.floor"),
            "floor_end": last.get("heap.floor"),
            "floor_dropped_in_phase": dropped,
            "floor_phase": floor_phase,
            "largest_at_floor": largest,
            "idf_min_end": last.get("heap.idf_min"),
            "free_min_sampled": min(frees) if frees else None,
            "net_low_min_30s_windows": min(net_recent) if net_recent else None,
            "refresh_us_median": refresh[len(refresh) // 2] if refresh else None,
            "refresh_us_min_max": [refresh[0], refresh[-1]] if refresh else None,
            "wake_max_us_end": last.get("display.wake_max_us"),
            "restarted": self.restarted,
            "playing": sorted({r.get("playing") or "" for r in rows})[:8],
        }

    # --- gates -------------------------------------------------------------------------
    def gate(self, name: str, ok: bool | None, value, want: str, required: bool = True):
        """`ok` None: not judged; a required gate not judged fails the run.
        `required` False: reported only (a figure, not a verdict)."""
        g = {"gate": name, "pass": ok, "value": value, "want": want, "required": required}
        self.gates.append(g)
        verdict = "PASS" if ok else (("SKIP" if not required else "MISSING") if ok is None else "FAIL")
        print(f"  {verdict:4}  {name}: {value} (want {want})", flush=True)

    def gate_floor(self, phase: str, stats: dict, kb: float, label: str = "heap floor"):
        f = stats.get("floor_phase")
        how = "heap.floor, fallen in the phase" if stats.get("floor_dropped_in_phase") else "the phase's least sampled free heap, net 30 s windows and probe low"
        self.gate(f"{phase}: {label} >= {kb:g} KB", None if f is None else f >= kb * KB, f, f">= {int(kb * KB)} B ({how})")

    def gate_content(self, phase: str):
        """What played: a `formula` phase must have a Formula on screen (its
        frame time reported) in >= 90 % of the samples, a `gif` phase not."""
        rows = [r for r in self.rows if r["phase"] == phase]
        if not rows or phase not in ("gif", "formula"):
            return
        share = sum(1 for r in rows if r.get("formula_frame_us") is not None) / len(rows)
        if phase == "formula":
            self.gate("formula: a Formula on screen", share >= 0.9, f"{share:.0%} of samples", ">= 90 % (now_playing.formula.frame_us)")
        else:
            self.gate("gif: GIFs on screen", share <= 0.1, f"{share:.0%} of samples with a Formula", "<= 10 %")

    def gate_no_added_late(self, phase: str, stats: dict, baseline: dict | None, tolerance: int):
        """Each late-frame counter's delta against what the baseline's rate
        predicts for this phase's length: no more than ceil(expected) +
        tolerance."""
        if not baseline:
            self.gate(f"{phase}: no added late frames", None, "no baseline given", "--baseline <m2 summary.json>")
            return
        bstats = baseline.get("phases", {}).get(phase)
        if not bstats or "rates_per_h" not in bstats:
            self.gate(f"{phase}: no added late frames", None, f"the baseline has no phase {phase!r}", "an M2 run with that phase")
            return
        for k in LATE:
            got = stats.get("deltas", {}).get(k)
            rate = bstats["rates_per_h"].get(k)
            if got is None or rate is None:
                self.gate(f"{phase}: {k} not above baseline", None, "not reported", "")
                continue
            expected = rate * stats["duration_s"] / 3600.0
            allowed = math.ceil(expected - 1e-9) + tolerance
            self.gate(f"{phase}: {k} not above baseline", got <= allowed, f"{got} (baseline predicts {expected:.2f})", f"<= {allowed}")

    def gate_refresh_same(self, phase: str, stats: dict, baseline: dict | None):
        b = get(baseline or {}, f"phases.{phase}.refresh_us_median")
        r = stats.get("refresh_us_median")
        if b is None or r is None:
            self.gate(f"{phase}: refresh_us unchanged", None, r, "a baseline")
            return
        self.gate(f"{phase}: refresh_us unchanged", abs(r - b) <= max(2, 0.001 * b), f"{r} vs {b}", "within 0.1 %")

    def gate_no_restart(self):
        self.gate("no restart", not self.restarted, "restarted" if self.restarted else "none", "no uptime reset")

    # --- the end ---------------------------------------------------------------------------
    def finish(self) -> int:
        summary = {
            "measurement": self.name,
            "args": {k: v for k, v in self.args.items() if k != "key"},
            "phases": self.phases,
            "gates": self.gates,
            "notes": self.notes,
            # Every gate passed, a required one not judged is a failure.
            "passed": all(g["pass"] is True or (g["pass"] is None and not g.get("required", True)) for g in self.gates)
            and any(g["pass"] for g in self.gates),
        }
        (self.dir / "summary.json").write_text(json.dumps(summary, indent=1) + "\n")
        fails = [g for g in self.gates if g["pass"] is False]
        missing = [g for g in self.gates if g["pass"] is None and g.get("required", True)]
        skips = [g for g in self.gates if g["pass"] is None and not g.get("required", True)]
        print(f"\n{self.name}: {'PASS' if summary['passed'] else 'FAIL'} "
              f"({len(self.gates) - len(fails) - len(skips) - len(missing)} passed, {len(fails)} failed, "
              f"{len(missing)} required but not judged, {len(skips)} reported only)  -> {self.dir}", flush=True)
        self._jsonl.close()
        if self._csv is not None:
            self._csv_file.close()
        return 0 if summary["passed"] else 1


def parser(desc: str) -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(description=desc)
    ap.add_argument("ip", help="the panel's address (e.g. 192.168.0.178)")
    ap.add_argument("--key", help="the panel's API key, if one is set")
    ap.add_argument("--out", default="connect-s0-results", help="results directory (default ./connect-s0-results)")
    ap.add_argument("--sample-s", type=float, default=5.0, help="seconds between status reads (default 5)")
    return ap


def add_phases(ap: argparse.ArgumentParser, minutes: float = 30.0):
    ap.add_argument("--phases", default="gif,formula", help="comma-separated phase names (default gif,formula)")
    ap.add_argument("--minutes", type=float, default=minutes, help=f"minutes per phase (default {minutes:g})")
    ap.add_argument("--play", action="append", default=[], metavar="PHASE=PATH",
                    help="POST /api/play PATH as PHASE starts (e.g. formula=@formula:starfield); else set the content up yourself")
    ap.add_argument("--interactive", action="store_true", help="wait for Enter before each phase")
    ap.add_argument("--settle-s", type=float, default=20.0, help="seconds after a phase starts before sampling counts (default 20)")


def add_baseline(ap: argparse.ArgumentParser):
    ap.add_argument("--baseline", help="an M2 run's summary.json (the late-frame and refresh gates need it)")
    ap.add_argument("--tolerance", type=int, default=0, help="late frames allowed over the baseline's prediction (default 0)")


def load_baseline(path: str | None):
    if not path:
        return None
    return json.loads(Path(path).read_text())


def start_phase(run: Run, phase: str, args, probe: bool = False):
    plays = dict(p.split("=", 1) for p in args.play if "=" in p)
    if phase == "formula" and phase not in plays and not args.interactive:
        sys.exit("the formula phase needs --play formula=@formula:<a raced one> (or --interactive, to set it up by hand)")
    print(f"\n== phase {phase} ==", flush=True)
    if probe:
        reset_probe(run)
    if args.interactive:
        input(f"  set the panel up for '{phase}' and press Enter... ")
    if phase in plays:
        code = run.panel.play(plays[phase])
        print(f"  played {plays[phase]} ({code})", flush=True)
    if args.settle_s > 0:
        run.sample_for(args.settle_s, phase=f"{phase}:settle")


def reset_probe(run: "Run"):
    """The probe's counters, histograms and lows from zero (POST reset),
    then a second for its next turn to take it."""
    run.panel.post("reset")
    time.sleep(1.0)


def need_probe(st: dict) -> dict:
    c = st.get("connect")
    if not isinstance(c, dict):
        sys.exit("this panel's /api/status has no `connect`: it is not a connect-probe build")
    return c


def wait_open(run: Run, timeout_s: float = 180.0, phase: str = "wait-open") -> bool:
    st = run.sample_for(timeout_s, phase, until=lambda s: get(s, "connect.state") == "open")
    ok = st is not None and get(st, "connect.state") == "open"
    if not ok:
        run.note(f"the session did not open in {timeout_s:g} s: state {get(st or {}, 'connect.state')}, last_error {get(st or {}, 'connect.last_error')}")
    return ok
