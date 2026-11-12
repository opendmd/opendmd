# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M6: M4's full handshakes while `make api-load` (tools/api-load.py, 3
workers) and one upload at a time run on the LAN. Gates (CONNECT.md 10): no
restart; heap floor >= 20 KB; refusals allowed (net.heavy_refused), aborted
streams not (streams_aborted). The uploads go to /connect-s0/upload.bin on
the card and are deleted at the end.

    uv run tools/connect-s0/m6.py 192.168.0.178 [--n 200] [--interval-s 10]
"""
import os
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import s0lib  # noqa: E402
from handshake_job import handshake_gates, run_job  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
ap = s0lib.parser(__doc__)
ap.add_argument("--n", type=int, default=200)
ap.add_argument("--interval-s", type=float, default=10.0)
ap.add_argument("--floor-kb", type=float, default=20)
ap.add_argument("--workers", type=int, default=3)
ap.add_argument("--upload-kb", type=int, default=300)
ap.add_argument("--upload-path", default="/connect-s0/upload.bin")
args = ap.parse_args()
run = s0lib.Run("m6", args)
s0lib.need_probe(run.sample("start"))
s0lib.wait_open(run)
seconds = int(args.n * args.interval_s + 60)
load = subprocess.Popen(["uv", "run", "--quiet", str(REPO / "tools/api-load.py"), args.ip, str(seconds), "--workers", str(args.workers)],
                        stdout=open(run.dir / "api-load.log", "w"), stderr=subprocess.STDOUT)
stop = threading.Event()
up = {"ok": 0, "refused": 0, "failed": 0}


def uploads():
    body = os.urandom(args.upload_kb * 1024)
    url = f"{run.panel.base}/api/files{args.upload_path}"
    while not stop.is_set():
        req = urllib.request.Request(url, data=body, method="PUT", headers={"Content-Type": "application/octet-stream"})
        if args.key:
            req.add_header("Authorization", f"Bearer {args.key}")
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                up["ok" if r.status == 200 else "failed"] += 1
        except urllib.error.HTTPError as e:
            up["refused" if e.code == 503 else "failed"] += 1
            time.sleep(2)
        except Exception:  # noqa: BLE001
            up["failed"] += 1
            time.sleep(2)


t = threading.Thread(target=uploads, daemon=True)
t.start()
try:
    before, after = run_job(run, args, "load", False, args.n, args.interval_s, seconds * 1.5 + 120)
finally:
    stop.set()
    t.join(timeout=150)
    load.terminate()
    try:
        req = urllib.request.Request(f"{run.panel.base}/api/files{args.upload_path}", method="DELETE")
        if args.key:
            req.add_header("Authorization", f"Bearer {args.key}")
        urllib.request.urlopen(req, timeout=30).close()
    except Exception as e:  # noqa: BLE001
        run.note(f"could not delete {args.upload_path}: {e}")
run.phases["load"] = stats = run.phase_stats("load")
stats["uploads"] = up
print(f"  uploads: {up}", flush=True)
handshake_gates(run, "load", before, after, False, args.n, 4000)
run.gate_floor("load", stats, args.floor_kb)
run.gate("load: no aborted streams", stats["deltas"].get("streams_aborted", 0) == 0, stats["deltas"].get("streams_aborted"), "0")
run.gate("load: refusals (allowed)", True, f"heavy_refused +{stats['deltas'].get('net.heavy_refused')}, uploads {up}", "any")
run.gate_no_restart()
sys.exit(run.finish())
