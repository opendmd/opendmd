"""The low-heap lane (`run.py oom`, `make emu-oom`): an `emu-oom-test` image (firmware
src/oom_test.rs) on the virtual access point, the guest's port 80 on 127.0.0.1 (inside the private
network namespace run.py sets up, as the API lane's). The firmware drives the steps itself and logs
each as `oom-test: …`; this judges the console and the API:

  boot 1    WiFi up, GIFs playing
  drop 1    the association dropped: the reconnect goes straight back (`no scan`), no scan logged
  drop 2    the learned channel spoiled: the pinned connect fails, a scan for `demo` finds it, the
            connect stages join (`reconnected (after a scan)`)
  scan      GET /api/wifi/scan lists `demo` (the scan read a record at a time, no list of ours)
  squeeze   playback paused, the heap in 1 KB pieces with ~12 KB free: the heartbeat in it is the
            short one (`heartbeat (short #N, the heap is low)`, `during: wifi up`), no whole one,
            no abort
  drop 3    squeezed harder (pieces of 1 KB): the reconnect held back for the heap (`reconnect held
            back`) until the release, then straight back
  status    net.wifi_drops, wifi_reconnects, wifi_reconnect_tries, wifi_last_drop_s, wifi_activity
  abort     1 MB asked of the heap (Rust's out-of-memory abort), one reset
  boot 2    `reset reason: panic (unrecorded_crash)` naming the failed allocation and its task;
            /api/status restarts[0] where/activity; GET /api/log/last the crashed run's last lines
            (the abort's own `oom-test:` line among them) under its `#` head (the allocation's
            size, caps, task and backtrace, named with addr2line when the ELF is found)

Prints EMU_OOM_OK. Correctness only, like every emulator lane: the drops are the station's own
disconnects (the emulator models no access point going away), the heap squeeze is the test's own.
"""
import glob
import os
import re
import subprocess
import time
from pathlib import Path

from api_lane import PORT, WIFI, Api, Emulator, LaneFail, machine, seeded_image

CONNECTED = re.compile(r"WiFi connected, IP ")
GIF = re.compile(r"\] /gifs/\S+\.gif <- ")
STEP = {k: re.compile(rf"oom-test: {k}\b") for k in ("drop 1", "drop 2", "squeeze", "drop 3", "release", "playback resumed", "abort")}
DROPPED = re.compile(r"\[net\] WiFi dropped; reconnecting")
DIRECT = re.compile(r"reconnecting to 'demo' at ([0-9a-f:]{17}) ch (\d+) .*no scan")
SCANNING = re.compile(r"scanning for 'demo'")
FOUND = re.compile(r"'demo' found: rssi=")
BACK_DIRECT = re.compile(r"\[net\] reconnected \(straight back\)")
BACK_SCAN = re.compile(r"\[net\] reconnected \(after a scan\)")
DIRECT_FAILED = re.compile(r"\[net\] reconnect failed \(straight back\)")
STACKS = re.compile(r"oom-test: stacks: tiT (\d+) B, wifi (\d+) B, sys_evt (\d+) B, Tmr Svc (\d+) B unused")
HELD = re.compile(r"\[net\] reconnect held back: heap (\d+) B free, largest block (\d+) B")
SQUEEZED = re.compile(r"heap (\d+) B free, largest block (\d+) B")
SHORT_BEAT = re.compile(r"heartbeat \(short #(\d+), the heap is low\): heap (\d+) B free, largest block (\d+) B.*during: (wifi \S+.*)$")
FULL_BEAT = re.compile(r"opendmd_firmware: heartbeat: heap")
OOM = re.compile(r"memory allocation of 1048576 bytes failed")
BOOT2 = re.compile(r"reset reason: (\S+) \((\w+)\)(.*)")
# The deliberate abort's own lines: allowed only after `oom-test: abort`.
ABORTISH = re.compile(r"panicked at|\bpanic\b.*\bat\b|abort\(\) was called|^Backtrace:|Guru Meditation", re.I)


def addr2line(elf: Path, pcs: list[str]) -> list[str]:
    tools = sorted(glob.glob(os.path.expanduser("~/.rustup/toolchains/esp/xtensa-esp-elf/*/xtensa-esp-elf/bin/xtensa-esp32-elf-addr2line")))
    if not tools or not elf.exists() or not pcs:
        return []
    r = subprocess.run([tools[-1], "-pfiC", "-e", str(elf), *pcs], capture_output=True, text=True)
    return [l.strip() for l in r.stdout.splitlines() if l.strip()]


def run(a, sim, rom, fail_rx) -> int:
    t0 = time.time()
    logs = a.log.parent if a.log else a.emu_dir / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    log = a.log or logs / f"oom-{time.strftime('%Y%m%d-%H%M%S')}.log"
    passes = []

    def ok(name, detail):
        line = f"pass  {name}: {detail}"
        passes.append(line)
        print(line, flush=True)

    # The deliberate abort trips run.py's panic/abort and backtrace patterns: judged here instead.
    fails = [(rx, name) for rx, name in fail_rx if name not in ("panic/abort", "backtrace", "Guru Meditation")]
    emu = None
    try:
        image = seeded_image(a.image, a.emu_dir / "oom")
        cmd = [*machine(sim, "esp32", image, rom, ["--sd", str(a.card)]), "--max-seconds", str(a.seconds or 900),
               "--console", "uart0", "--wifi", WIFI, "--net", "nat", "--hostfwd", f"tcp:127.0.0.1:{PORT}-:80", "--no-dump"]
        emu = Emulator(cmd, dict(os.environ), log, fails, "oom")
        emu.resets_allowed = 2  # the power-on, and the abort's
        emu.start()
        wall = a.wall or 3600
        api = Api(emu)

        i, _ = emu.wait_for(CONNECTED, 600, what="WiFi connected at boot")
        sha = emu.find(re.compile(r"ELF file SHA256:\s+([0-9a-f]+)"))
        if not (sha and a.want_sha.startswith(sha[0][1][1])):
            raise LaneFail(f"the booted app is not {a.image}")
        g, _ = emu.wait_for(GIF, 600, what="a GIF started")
        ok("boot 1", f"WiFi up, {emu.lines[g][emu.lines[g].index('/gifs/'):][:70]}")

        # Drop 1: straight back.
        d, _ = emu.wait_for(STEP["drop 1"], wall, i, what="oom-test: drop 1")
        emu.wait_for(DROPPED, 60, d, what="the drop seen")
        p, pm = emu.wait_for(DIRECT, 60, d, what="the pinned connect (no scan)")
        b, _ = emu.wait_for(BACK_DIRECT, 120, d, what="reconnected straight back")
        if emu.find(SCANNING, d, b):
            raise LaneFail("drop 1: a scan before the reconnect straight back")
        ok("drop 1", f"straight back to {pm[1]} ch {pm[2]}, no scan: {emu.lines[b][emu.lines[b].index('[net]'):][:60]}")

        # Drop 2: the pinned connect fails, a scan, the ladder.
        d, _ = emu.wait_for(STEP["drop 2"], 120, b, what="oom-test: drop 2")
        f, _ = emu.wait_for(DIRECT_FAILED, 120, d, what="the pinned connect failing on the spoiled channel")
        s, _ = emu.wait_for(SCANNING, 60, f, what="a scan after it")
        emu.wait_for(FOUND, 60, s, what="the scan finding 'demo'")
        b, _ = emu.wait_for(BACK_SCAN, 180, s, what="reconnected after a scan")
        ok("drop 2", f"the pinned connect failed, a scan for 'demo' found it, joined: {emu.lines[b][emu.lines[b].index('[net]'):][:60]}")

        # The setup page's scan, through the API.
        code, _, body = api.raw("GET", "/api/wifi/scan", timeout=30)
        import json
        aps = json.loads(body) if code == 200 else None
        if not (isinstance(aps, list) and any(x.get("ssid") == "demo" for x in aps)):
            raise LaneFail(f"GET /api/wifi/scan: HTTP {code}, {body[:200]!r}")
        ok("scan", f"GET /api/wifi/scan: {len(aps)} network(s), {[x.get('ssid') for x in aps][:4]}")

        # The squeeze: the short heartbeat, no whole one, no abort.
        q, qm = emu.wait_for(STEP["squeeze"], 120, b, what="oom-test: squeeze")
        sq = SQUEEZED.search(emu.lines[q])
        h, hm = emu.wait_for(SHORT_BEAT, 60, q, what="a short heartbeat in the squeeze")
        d3, _ = emu.wait_for(STEP["drop 3"], 60, h, what="oom-test: drop 3")
        if emu.find(FULL_BEAT, q, d3):
            raise LaneFail("a whole heartbeat while the heap was squeezed: " + emu.find(FULL_BEAT, q, d3)[0][1].string[:160])
        if not hm[4].startswith("wifi up"):
            raise LaneFail(f"the short heartbeat's activity is {hm[4]!r}, not wifi up")
        ok("squeeze", f"heap {sq[1]} B free, largest {sq[2]} B: short heartbeat #{hm[1]} ({hm[2]} B free, largest {hm[3]} B, during: {hm[4][:60]}), no whole one")

        # Drop 3 in a harder squeeze: held back, then straight back after the release.
        sq3 = SQUEEZED.search(emu.lines[d3])
        r, _ = emu.wait_for(STEP["release"], 60, d3, what="oom-test: release")
        held = emu.find(HELD, d3, r)
        # A pinned connect wants 10 KB free and a 2 KB block (opendmd_core::wifi_reconnect).
        if int(sq3[1]) < 10 * 1024 or int(sq3[2]) < 2048:
            if not held:
                raise LaneFail(f"drop 3 ({sq3[1]} B free, largest block {sq3[2]} B): the reconnect was not held back for the heap")
            if emu.find(DIRECT, d3, r) or emu.find(SCANNING, d3, r):
                raise LaneFail("drop 3: a reconnect step tried in the squeezed heap")
            said = f"held back ({emu.lines[held[0][0]][emu.lines[held[0][0]].index('heap'):][:60]})"
        else:
            said = f"not held back (the squeeze left {sq3[1]} B free and a {sq3[2]} B block, room for a pinned connect)"
        st_i, stm = emu.wait_for(STACKS, 60, d3, what="the stacks' high-water marks after the squeeze")
        stacks = dict(zip(("tiT", "wifi", "sys_evt", "Tmr Svc"), map(int, stm.groups())))
        low = [k for k, v in stacks.items() if v < 256]
        if low:
            raise LaneFail(f"a stack all but used up after the squeeze: {stacks}")
        ok("stacks", f"unused after the squeeze (bytes): {stacks}")
        b, _ = emu.wait_for(BACK_DIRECT, 120, r, what="reconnected straight back after the release")
        ok("drop 3", f"heap {sq3[1]} B free, largest {sq3[2]} B: {said}; after the release straight back")
        emu.wait_for(STEP["playback resumed"], 60, r, what="oom-test: playback resumed")

        st = api.wait_up(120)
        net = st.get("net") or {}
        drops, back, tries = net.get("wifi_drops"), net.get("wifi_reconnects"), net.get("wifi_reconnect_tries")
        if not (drops == 3 and back == 3 and tries and tries >= 4 and net.get("wifi_last_drop_s") is not None and net.get("wifi_activity") == "up"):
            raise LaneFail(f"/api/status net: wifi_drops {drops}, wifi_reconnects {back}, wifi_reconnect_tries {tries}, "
                           f"wifi_last_drop_s {net.get('wifi_last_drop_s')}, wifi_activity {net.get('wifi_activity')!r}")
        code, _, body = api.raw("GET", "/api/log/last", timeout=20)
        if code != 404:
            raise LaneFail(f"GET /api/log/last after a power-on: HTTP {code}, wanted 404: {body[:120]!r}")
        ok("status", f"net.wifi_drops 3, wifi_reconnects 3, wifi_reconnect_tries {tries}, wifi_last_drop_s {net['wifi_last_drop_s']}, wifi_activity up; /api/log/last 404 (a power-on)")

        # The abort, and the boot after it.
        ab, _ = emu.wait_for(STEP["abort"], 120, b, what="oom-test: abort")
        bad = [l for l in emu.lines[:ab] if ABORTISH.search(l)]
        if bad:
            raise LaneFail(f"an abort or panic before the deliberate one: {bad[0][:160]}")
        o, _ = emu.wait_for(OOM, 60, ab, what="Rust's out-of-memory message")
        r2, rm = emu.wait_for(BOOT2, 240, o, what="boot 2's reset reason")
        where = rm[3]
        if rm[2] != "unrecorded_crash" or "last failed alloc: 1048576 B in net-worker" not in where \
                or "(the abort)" not in where or "during: wifi" not in where:
            raise LaneFail(f"boot 2: {emu.lines[r2][:240]}")
        ok("abort", f"one reset; {emu.lines[r2][emu.lines[r2].index('reset reason'):][:200]}")
        st = api.wait_up(400)
        last = (st.get("restarts") or [{}])[0]
        if not (last.get("reason") == "unrecorded_crash" and (last.get("where") or "").startswith("last failed alloc: 1048576 B in net-worker")
                and (last.get("where") or "").endswith("(the abort)")
                and (last.get("activity") or "").startswith("wifi ")):
            raise LaneFail(f"boot 2: /api/status restarts[0] {last}")
        code, headers, body = api.raw("GET", "/api/log/last", timeout=20)
        text = body.decode(errors="replace")
        if headers.get("X-Content-Type-Options") != "nosniff":
            raise LaneFail(f"GET /api/log/last: X-Content-Type-Options {headers.get('X-Content-Type-Options')!r}, not nosniff")
        head = [l for l in text.splitlines() if l.startswith("#")]
        if code != 200 or not text.startswith("# crash log: restart") or "# last failed alloc: 1048576 B, caps" not in text \
                or "task net-worker" not in text or "oom-test: abort" not in text or not headers.get("X-Restart"):
            raise LaneFail(f"boot 2: GET /api/log/last HTTP {code}, X-Restart {headers.get('X-Restart')}: {text[:400]!r}")
        pcs = re.findall(r"0x[0-9a-f]{8}", next((l for l in head if l.startswith("# backtrace:")), ""))
        lines = [l for l in text.splitlines() if not l.startswith("#")]
        ok("crash log", f"/api/log/last {len(body)} B (restart {headers['X-Restart']}): {len(lines)} lines, the last {lines[-1][:80]!r}; {len(pcs)} return addresses")
        ok("crash record", f"restarts[0] {last['reason']}, where {last['where']!r}, activity {last['activity']!r}")
        print("emu: GET /api/log/last:")
        for l in text.splitlines():
            print(f"  | {l[:300]}")
        elf = Path(a.image).parent.parent / "target" / "xtensa-esp32-espidf" / "release" / "opendmd-firmware"
        named = addr2line(elf, pcs)
        if not named:
            raise LaneFail(f"the backtrace could not be named (no addr2line, or no ELF at {elf}): {pcs}")
        print("emu: the failed allocation's return addresses:")
        for n in named:
            print(f"  {n}")
        if not any("oom_test::tick" in n or "net_worker::run" in n for n in named):
            raise LaneFail("the backtrace stopped short of the caller (oom_test::tick / net_worker::run not among its frames)")
        ok("backtrace", f"{len(pcs)} return addresses, the caller among them (oom_test::tick / net_worker::run)")
    except LaneFail as e:
        print(f"EMU_OOM_FAIL: {e}")
        if emu:
            print(f"--- last console lines of {emu.log}:")
            print(emu.tail())
        return 1
    finally:
        if emu and not emu.done:
            emu.kill()
    print(f"emu: log {log}")
    print(f"EMU_OOM_OK ({time.time() - t0:.0f} s wall)")
    return 0
