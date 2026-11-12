# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""M8 and the probe's correctness, in the emulator (`make emu-connect`).

Correctness only, never timing (the emulator's cycles mean nothing; every
millisecond figure here is the emulator's). Boots a connect-probe image
built for it (`make emu-image-connect`: `emu-census` + `connect-probe`, the
host `connect-s0.test` at 10.0.2.2:8443, a local test root pinned beside
the real ones, token `emu-s0`) with WiFi credentials seeded, on esp32sim's
virtual access point, inside a private network namespace (`unshare -rn`:
nothing reaches a real network or device). In the namespace, 10.0.2.2 is
put on the loopback, where tools/connect-s0/echo_server.py listens: the
emulator's NAT carries the panel's connection to it.

M8 proper: `/api/status?pools=1` (the heap's use by SRAM pool) before the
first connection, during a handshake, with the session idle, and after the
session is closed again; the emulator's heap census (`census:` lines) and
mbedTLS's own byte count (`connect.heap.tls_in_use`). Gate: once closed,
nothing of the session is left in any pool (blocks and bytes back within
the noise of play).

Then the probe's behaviour, end to end: the clock floor (no SNTP in the
namespace), the pinned test root found by the CA callback, full and resumed
handshakes (and the server agreeing which), M9's vector, M10's 512 KB
stream with its checksum, the three ping kinds, a pong timeout, a server
close, a wrong token, an untrusted chain, no restart and no panic; and
m9.py and m10.py run against it, so the scripts are exercised too.

    uv run tools/connect-s0/m8_emu.py --image <full.bin> --certs <dir> [--card sd.img] [--wall 7200]
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "tools" / "emu"))
sys.path.insert(0, str(HERE))
import api_lane  # noqa: E402
import echo_server  # noqa: E402

SCRATCH = os.environ.get("SCRATCH")
EMU_DIR = Path(os.environ.get("EMU_DIR") or (Path(SCRATCH) / "opendmd-emu-sim" if SCRATCH else REPO / "target" / "emu-sim"))
TOKEN = "emu-s0"
PORT = 8080
FAIL = [
    (re.compile(r"Guru Meditation"), "Guru Meditation"),
    (re.compile(r"panicked at|\bpanic\b.*\bat\b|abort\(\) was called|\bassert failed\b", re.I), "panic/abort"),
    (re.compile(r"stack overflow", re.I), "stack overflow"),
    (re.compile(r"^Backtrace:"), "backtrace"),
    (re.compile(r"CORRUPT HEAP|heap_caps_.*(alloc|free) failed.*abort", re.I), "heap corruption"),
]
CENSUS = re.compile(r"census: (\d+) blocks in use, (\d+) B allocated")


class Fail(Exception):
    pass


class Lane:
    def __init__(self, emu, srv, other, logs: Path):
        self.emu, self.srv, self.other, self.logs = emu, srv, other, logs
        self.base = f"http://127.0.0.1:{PORT}"
        self.passes: list[str] = []
        self.report: dict = {}

    def http(self, method, path, timeout=30.0):
        req = urllib.request.Request(self.base + path, method=method, data=b"" if method != "GET" else None)
        for k in range(8):
            try:
                with urllib.request.urlopen(req, timeout=timeout) as r:
                    return json.loads(r.read())
            except urllib.error.HTTPError as e:
                if e.code == 503 and k < 7:
                    time.sleep(2)
                    continue
                raise
            except (urllib.error.URLError, TimeoutError, ConnectionError):
                self.emu.check()
                if k == 7:
                    raise
                time.sleep(2)
        return None

    def status(self):
        return self.http("GET", "/api/status")

    def c(self):
        return self.status().get("connect") or {}

    def post(self, what, **q):
        qs = "&".join(f"{k}={v}" for k, v in q.items())
        return self.http("POST", f"/api/debug/connect/{what}" + (f"?{qs}" if qs else ""))

    def ok(self, name, detail):
        line = f"pass  {name}: {detail}"
        self.passes.append(line)
        print(line, flush=True)

    def expect(self, cond, name, detail):
        if not cond:
            raise Fail(f"{name}: {detail}")
        self.ok(name, detail)

    def wait(self, what, cond, timeout, every=2.0):
        """Poll the status until cond(status) or the timeout (wall s)."""
        end = time.time() + timeout
        last = None
        while time.time() < end:
            self.emu.check()
            try:
                last = self.status()
            except Exception:  # noqa: BLE001
                last = None
            if last is not None and cond(last):
                return last
            time.sleep(every)
        c = (last or {}).get("connect") or {}
        raise Fail(f"{what}: not within {timeout:.0f} s wall (state {c.get('state')}, last_error {c.get('last_error')})")

    def renewed(self, what, ask, timeout=900):
        """Do `ask` (a config change, a reconnect), then wait for a session
        opened after it (the old one may still read as open for a turn)."""
        before = self.c().get("opened", 0)
        ask()
        return self.wait(what, lambda s: s["connect"].get("state") == "open" and s["connect"].get("opened", 0) > before, timeout)

    def pools(self):
        return self.http("GET", "/api/status?pools=1").get("pools")

    def census(self, start=0):
        return [(int(m[1]), int(m[2])) for _, m in self.emu.find(CENSUS, start)]

    # --- the steps ---------------------------------------------------------------------
    def run(self):
        st = self.wait("the API answers", lambda s: True, 900, every=5)
        c = st.get("connect") or {}
        self.expect("connect-probe" in (st.get("features") or []), "feature", f"features {st.get('features')}")
        self.expect(isinstance(c, dict) and c.get("state"), "telemetry", f"connect.state {c.get('state')!r}")
        self.expect(c.get("tls_in") > 0 and c.get("tls_out") > 0, "record sizes", f"in {c.get('tls_in')} out {c.get('tls_out')}")

        # M8 1: before the first connection (the clock wait, 60 emulated s).
        before = None
        if c.get("state") in ("waiting for clock", "waiting for network", "starting"):
            before = self.pools()
            self.report["pools_before"] = before
            self.ok("pools before the first connection", json.dumps(before))
        # M8 2: during a handshake.
        mid = None
        end = time.time() + 900
        while time.time() < end:
            self.emu.check()
            s = self.status()
            state = (s.get("connect") or {}).get("state")
            if state == "handshake" and mid is None:
                mid = self.pools()
                self.report["pools_handshake"] = mid
                self.ok("pools during a handshake", json.dumps(mid))
            if state == "open":
                break
            time.sleep(0.3)
        c = self.c()
        self.expect(c.get("state") == "open", "session open", f"after {c.get('attempts')} attempt(s); last_error {c.get('last_error')}")
        self.expect(c.get("clock") == "build date", "clock floor (no SNTP in the namespace)", f"clock {c.get('clock')!r}; last_error {c.get('last_error')!r}")
        self.expect(c.get("root") == "emulator_test_ca", "the pinned test root found by the CA callback", f"root {c.get('root')}")
        self.expect((c.get("full") or {}).get("n", 0) >= 1, "a full handshake", f"{c.get('full')}")
        self.expect(self.srv.handshakes["full"] >= 1 and self.srv.upgrades >= 1, "the server saw it", f"{self.srv.handshakes}, {self.srv.upgrades} upgrades")
        heap = c.get("heap") or {}
        self.ok("heap (emulated)", json.dumps(heap))
        self.report["heap_first"] = heap

        # M8 3: idle.
        time.sleep(30)
        idle = self.pools()
        self.report["pools_idle"] = idle
        self.ok("pools with the session idle", json.dumps(idle))
        census0 = self.census()
        tls0 = (self.c().get("heap") or {}).get("tls_in_use")

        # The ping kinds.
        for kind in ("ws", "text", "echo"):
            p0 = (self.c().get("ping") or {}).get("pongs", 0)
            self.post("config", ping_s=5, ping=kind)
            s = self.wait(f"pongs to {kind} pings", lambda s, p0=p0: s["connect"].get("state") == "open" and (s["connect"].get("ping") or {}).get("pongs", 0) >= p0 + 2, 600)
            self.expect(s["connect"].get("ping_mode") == kind, f"{kind} pings answered", f"ping {s['connect']['ping']} (pongs were {p0})")
        self.renewed("open again", lambda: self.post("config", ping_s=45, ping="ws"))

        # M9.
        self.post("ed25519", n=20)
        s = self.wait("M9 done", lambda s: (s["connect"].get("m9") or {}).get("done") is True, 900)
        m9 = s["connect"]["m9"]
        self.expect(m9.get("n") == 20 and m9.get("failed") == 0, "M9: the vector verifies (20 of 20)", json.dumps(m9))
        self.expect((m9.get("stack_unused") or 0) >= 1024, "M9: the worker's stack has >= 1 KB left (emulated, but the depth is real)", f"{m9.get('stack_unused')} B")

        # M10.
        self.post("stream", bytes=524288, chunk=1000, window=4, seed=3)
        s = self.wait("M10 done", lambda s: (s["connect"].get("m10") or {}).get("state") not in (None, "running", "waiting for the session"), 1800, every=3)
        m10 = s["connect"]["m10"]
        self.expect(m10.get("state") == "done" and m10.get("checksum_ok") is True and m10.get("bytes") == 524288 and m10.get("mismatches") == 0,
                    "M10: 512 KB received, every byte and the FNV right", json.dumps(m10))

        # M4 and M5 in small.
        f0, r0, sf, sr = (self.c().get("full") or {}).get("n", 0), (self.c().get("resumed") or {}).get("n", 0), self.srv.handshakes["full"], self.srv.handshakes["resumed"]
        self.post("handshakes", n=3, interval_s=5, resume=0)
        self.wait("3 full handshakes", lambda s: s["connect"]["job"]["kind"] == "none" and (s["connect"].get("full") or {}).get("n", 0) >= f0 + 3, 900)
        self.expect(self.srv.handshakes["full"] >= sf + 3, "M4: full handshakes, tickets off (the server agrees)", f"{self.srv.handshakes}")
        self.post("handshakes", n=6, interval_s=3, resume=1)
        s = self.wait("6 resumed", lambda s: s["connect"]["job"]["kind"] == "none" and (s["connect"].get("resumed") or {}).get("n", 0) >= r0 + 5, 900)
        self.expect(self.srv.handshakes["resumed"] >= sr + 5, "M5: resumed handshakes from the ticket (the server agrees)",
                    f"{self.srv.handshakes}; device resumed {s['connect']['resumed']}, resume_missed {s['connect'].get('resume_missed')}")

        # A pong timeout.
        self.wait("open", lambda s: s["connect"].get("state") == "open", 300)
        pt = self.c()["causes"]["pong_timeout"]
        self.srv.mute_pings = True
        self.post("config", ping_s=5, ping="ws")
        self.wait("a pong timeout", lambda s: s["connect"]["causes"]["pong_timeout"] > pt, 600)
        self.srv.mute_pings = False
        self.renewed("open after the pong timeout", lambda: self.post("config", ping_s=45, ping="ws"))
        self.ok("pong timeout", "two pings unanswered ended the session; it came back")

        # The server closes.
        sc = self.c()["causes"]["server_close"]
        self.srv.close_all(1001)
        s = self.wait("a server close", lambda s: s["connect"]["causes"]["server_close"] > sc, 300)
        self.expect(any(x[2] == 1001 for x in s["connect"]["sessions"]), "server close counted with its code", f"sessions {s['connect']['sessions'][:3]}")
        self.renewed("open after the server close", lambda: self.post("reconnect"))

        # A wrong token.
        up = self.c()["causes"]["upgrade"]  # (counted once the new settings' attempt fails)
        refused = self.srv.refused
        self.post("config", token="wrong")
        s = self.wait("the upgrade refused", lambda s: s["connect"]["causes"]["upgrade"] > up, 900)
        self.expect(self.srv.refused > refused and "403" in (s["connect"].get("last_error") or ""), "a wrong token: upgrade refused", s["connect"].get("last_error"))
        self.renewed("open with the right token", lambda: (self.post("config", token=TOKEN), self.post("reconnect")))

        # An untrusted chain (another root's, on :8444).
        hs = self.c()["causes"]["handshake"]
        self.post("config", addr="10.0.2.2:8444")
        s = self.wait("the untrusted chain refused", lambda s: s["connect"]["causes"]["handshake"] > hs, 900)
        self.expect(self.other.upgrades == 0, "an untrusted chain: the handshake refused, nothing sent", s["connect"].get("last_error"))
        self.renewed("open again", lambda: (self.post("config", addr="10.0.2.2:8443"), self.post("reconnect")))

        # The scripts, against this panel (their timing gates mean nothing here).
        for script, extra in (("m9.py", ["--n", "10"]), ("m10.py", ["--runs", "1", "--bytes", "65536"])):
            r = subprocess.run([sys.executable, str(HERE / script), f"127.0.0.1:{PORT}", "--out", str(self.logs / "connect-s0-results"), *extra],
                               capture_output=True, text=True, timeout=3600)
            out = r.stdout + r.stderr
            (self.logs / f"m8-{script}.log").write_text(out)
            # Every gate the emulator can judge passes (timing ones too: the
            # emulator's are generous); the baseline's are not judged.
            self.expect(r.returncode == 0 and "Traceback" not in out,
                        f"{script} against the emulator", out.strip().splitlines()[-1] if out.strip() else "(no output)")
            self.wait("open", lambda s: s["connect"].get("state") == "open", 600)

        # M8 4: the session closed: nothing of it left.
        time.sleep(20)
        tls1 = (self.c().get("heap") or {}).get("tls_in_use")
        self.post("config", enabled=0)
        self.wait("switched off", lambda s: s["connect"].get("state") == "off", 300)
        time.sleep(30)
        after = self.pools()
        self.report["pools_after_close"] = after
        self.ok("pools with the session closed", json.dumps(after))
        census1 = self.census()
        tls_off = (self.c().get("heap") or {}).get("tls_in_use")
        if before:
            for name in before:
                db = after[name]["blocks"] - before[name]["blocks"]
                du = after[name]["used"] - before[name]["used"]
                self.report.setdefault("pool_delta_closed_vs_before", {})[name] = {"blocks": db, "used": du}
            print("  pools, closed minus before the first connection:", json.dumps(self.report["pool_delta_closed_vs_before"]), flush=True)
        if idle and after:
            self.report["pool_session_cost"] = {n: {"blocks": idle[n]["blocks"] - after[n]["blocks"], "used": idle[n]["used"] - after[n]["used"]} for n in idle}
            self.ok("the idle session's blocks by pool (idle minus closed)", json.dumps(self.report["pool_session_cost"]))
        self.expect(tls0 is not None and tls1 is not None and tls1 - tls0 <= 512,
                    "mbedTLS's bytes held, idle, after all the reconnects", f"{tls0} -> {tls1} B (switched off: {tls_off} B)")
        if census0 and census1:
            b0 = min(b for b, _ in census0[-3:])
            b1 = min(b for b, _ in census1[-3:])
            y0 = min(y for _, y in census0[-3:])
            y1 = min(y for _, y in census1[-3:])
            self.report["census"] = {"idle_open": [b0, y0], "closed_at_end": [b1, y1]}
            self.expect(b1 - b0 <= 24 and y1 - y0 <= 2048, "census: closed at the end against idle at the start (no growth)",
                        f"blocks {b0} -> {b1}, bytes {y0} -> {y1} (the leak lane's limits: 24 blocks, 2 KB)")
        st = self.status()
        self.expect(st.get("uptime_secs", 0) > 0 and not self.emu.failure and self.emu.resets <= 1, "no restart, no panic", f"uptime {st.get('uptime_secs')} s, resets {self.emu.resets}")
        self.post("config", enabled=1)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--image", type=Path, required=True)
    ap.add_argument("--certs", type=Path, required=True, help="echo_server.py certs' directory (the image pins its root.der)")
    ap.add_argument("--card", type=Path, default=EMU_DIR / "card" / "sd.img")
    ap.add_argument("--rom", type=Path)
    ap.add_argument("--seconds", type=int, default=6000, help="emulated seconds at most")
    ap.add_argument("--wall", type=int, default=10800, help="wall-clock limit (s)")
    a = ap.parse_args()
    sim = EMU_DIR / "src" / "target" / "release" / "esp32sim"
    rom = a.rom or next(iter(sorted(Path.home().glob(".espressif/tools/esp-rom-elfs/*/esp32_rev300_rom.elf"))[::-1]), None)
    for what, p in (("emulator (make emu-build)", sim), ("image", a.image), ("card (make emu-card)", a.card), ("rom", rom), ("certs", a.certs / "chain.pem")):
        if not p or not Path(p).exists():
            sys.exit(f"EMU_CONNECT_FAIL: missing {what}: {p}")
    seeded = api_lane.seeded_image(a.image, EMU_DIR / "connect" / "seed")
    if os.environ.get("S0_NETNS") != "1":
        if not shutil.which("unshare"):
            sys.exit("EMU_CONNECT_FAIL: no `unshare`: the lane runs in its own network namespace")
        os.execvpe("unshare", ["unshare", "--user", "--map-root-user", "--net", "--", sys.executable, __file__, *sys.argv[1:]], {**os.environ, "S0_NETNS": "1"})
    ip = shutil.which("ip") or next((p for p in ("/usr/sbin/ip", "/sbin/ip") if os.path.exists(p)), None)
    if not ip or subprocess.run([ip, "link", "set", "lo", "up"]).returncode != 0 or subprocess.run([ip, "addr", "add", "10.0.2.2/32", "dev", "lo"]).returncode != 0:
        sys.exit("EMU_CONNECT_FAIL: cannot set up the namespace's loopback (lo up, 10.0.2.2)")
    srv = echo_server.Server("10.0.2.2:8443", a.certs, TOKEN).start()
    other = echo_server.Server("10.0.2.2:8444", a.certs, TOKEN, chain="other-chain.pem", key="other-leaf.key").start()
    logs = EMU_DIR / "logs" / f"connect-{time.strftime('%Y%m%d-%H%M%S')}"
    logs.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "ESP32SIM_SD_IMG": str(a.card)}
    env.pop("ESP32SIM_SD_OUT", None)
    cmd = [str(sim), "--chip", "esp32", "--board", "esp32dev-sdspi", "--boot", "rom", "--rom", str(rom),
           "--flash-image", str(seeded), "--flash-mb", str(max(4, seeded.stat().st_size >> 20)),
           "--max-seconds", str(a.seconds), "--console", "uart0",
           "--wifi", api_lane.WIFI, "--net", "nat", "--hostfwd", f"tcp:127.0.0.1:{PORT}-:80", "--no-dump"]
    emu = api_lane.Emulator(cmd, env, logs / "console.log", FAIL, "m8")
    emu.start()
    lane = Lane(emu, srv, other, logs)
    t0 = time.time()
    code = 0
    try:
        import threading
        timer = threading.Timer(a.wall, emu.kill)
        timer.daemon = True
        timer.start()
        lane.run()
        print(f"EMU_CONNECT_OK ({len(lane.passes)} checks, {time.time() - t0:.0f} s wall)", flush=True)
    except (Fail, api_lane.LaneFail) as e:
        print(f"EMU_CONNECT_FAIL: {e}\n--- console tail:\n{emu.tail(25)}", flush=True)
        code = 1
    finally:
        (logs / "m8-report.json").write_text(json.dumps({"passes": lane.passes, **lane.report}, indent=1) + "\n")
        print(f"report {logs / 'm8-report.json'}; console {logs / 'console.log'}", flush=True)
        emu.kill()
    sys.exit(code)


if __name__ == "__main__":
    main()
