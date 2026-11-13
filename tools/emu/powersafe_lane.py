"""The power-safe lane (`run.py power-safe --chip s3`, `make emu-s3-power-safe`): the ESP32-S3's
brownout-loop guard (firmware/src/s3board.rs) in the emulator.

The S3 no-card test image (`nocard-test`) with NVS `ps_test` = 3 and WiFi credentials seeded,
one emulator run, on the virtual access point with the guest's port 80 on 127.0.0.1 (inside the
private network namespace run.py sets up, as the API lane's). The firmware fakes the brownouts
itself (`s3board::test`): after the power-on it restarts ~3 s into each boot, and the next boot
counts as a brownout reset, three times.

  boot 1   the power-on: the fakes armed
  boot 2   one brownout: logged, and the panel starts as ever (driver up, the scan modelled)
  boot 3   two in a row: the same
  boot 4   three in a row: power-safe: "power-safe: panel not started", no driver, no scan;
           WiFi and the httpd up; GET /api/status answers with reset_note the power-safe note
           and scan null
  boot 5   POST /api/reboot (a software restart breaks the run): the panel and the scan back,
           /api/status scan non-null (with beam.late_frames), reset_note not power-safe

Prints EMU_POWER_SAFE_OK. Correctness only, like every emulator lane.
"""
import os
import re
import time

from api_lane import PORT, WIFI, Api, Emulator, LaneFail, machine, seeded_image

NOTE_PREFIX = "power-safe:"
GUARD = re.compile(r"brownout guard: (\d+) brownout resets? in a row \(faked: nocard-test\)")
POWER_SAFE = re.compile(r"power-safe: panel not started: (\d+) brownout resets in a row \(faked: nocard-test\)")
ARMED = re.compile(r"ps-test: power-on; 3 brownout resets to fake")
DRIVER_UP = re.compile(r"hub75_shim: panel 128x32 up")
SCAN = re.compile(r"hub75_shim: scan: \d+ descriptors, \d+ per row pair, refresh \d+ us")
RESET = re.compile(r"reset reason: (\w+)")
HTTPD = re.compile(r"httpd started on port")


def run(a, sim, rom, fail_rx) -> int:
    t0 = time.time()
    logs = a.log.parent if a.log else a.emu_dir / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    log = a.log or logs / f"power-safe-{time.strftime('%Y%m%d-%H%M%S')}.log"
    passes = []

    def ok(name, detail):
        line = f"pass  {name}: {detail}"
        passes.append(line)
        print(line, flush=True)

    emu = None
    try:
        image = seeded_image(a.image, a.emu_dir / "power-safe", extra={"ps_test": "3"})
        cmd = [*machine(sim, "s3", image, rom, []), "--max-seconds", str(a.seconds or 900), "--console", "uart0",
               "--wifi", WIFI, "--net", "nat", "--hostfwd", f"tcp:127.0.0.1:{PORT}-:80", "--no-dump"]
        emu = Emulator(cmd, dict(os.environ), log, fail_rx, "power-safe")
        # The power-on, the three fakes and the reboot asked for.
        emu.resets_allowed = 5
        emu.start()
        wall = a.wall or 1800
        api = Api(emu)

        i, _ = emu.wait_for(ARMED, wall, what="the test armed at the power-on")
        sha = emu.find(re.compile(r"ELF file SHA256:\s+([0-9a-f]+)"))
        if not (sha and a.want_sha.startswith(sha[0][1][1])):
            raise LaneFail(f"the booted app is not {a.image}")
        ok("boot 1", "the power-on: three brownout resets to fake")

        # Boots 2 and 3: counted, and the panel starts as ever.
        for n in (1, 2):
            i, m = emu.wait_for(GUARD, wall, i, what=f"the guard's count {n}")
            if int(m[1]) != n:
                raise LaneFail(f"boot {n + 1}: the guard counted {m[1]}, not {n}: {emu.lines[i][:200]}")
            j, _ = emu.wait_for(DRIVER_UP, 240, i, what=f"boot {n + 1}'s panel driver")
            k, _ = emu.wait_for(SCAN, 240, i, what=f"boot {n + 1}'s scan line")
            if emu.find(POWER_SAFE, i, max(j, k)):
                raise LaneFail(f"boot {n + 1}: power-safe after {n} brownout(s)")
            ok(f"boot {n + 1}", f"{n} brownout{'s' if n > 1 else ''} in a row: the panel started, {emu.lines[k][emu.lines[k].index('scan:'):][:90]}")
            i = max(j, k)

        # Boot 4: power-safe.
        i, m = emu.wait_for(POWER_SAFE, wall, i, what="power-safe at the third brownout")
        if int(m[1]) != 3:
            raise LaneFail(f"power-safe at {m[1]} brownouts, not 3: {emu.lines[i][:200]}")
        ok("boot 4", emu.lines[i][emu.lines[i].index("power-safe:"):][:150])
        emu.wait_for(HTTPD, 240, i, what="boot 4's httpd")
        st = api.wait_up(400)
        note = st.get("reset_note") or ""
        if not note.startswith(NOTE_PREFIX):
            raise LaneFail(f"boot 4: /api/status reset_note is {note!r}, not the power-safe note")
        if st.get("scan") is not None:
            raise LaneFail(f"boot 4: /api/status scan is {st.get('scan')!r} with no panel started")
        later = emu.mark()
        for rx, what in ((DRIVER_UP, "the panel driver"), (SCAN, "a scan line")):
            if emu.find(rx, i, later):
                raise LaneFail(f"boot 4 (power-safe) started {what}")
        ok("power-safe status", f"/api/status answers on 127.0.0.1:{PORT}: reset_note {note!r}, scan null, no driver started")

        # Boot 5: a software restart ends the run of brownouts.
        mark = emu.mark()
        api.call("POST", "/api/reboot", want=(200, 202, 204))
        r, m = emu.wait_for(RESET, 240, mark, what="boot 5's reset line")
        j, _ = emu.wait_for(DRIVER_UP, 240, r, what="boot 5's panel driver")
        k, _ = emu.wait_for(SCAN, 240, r, what="boot 5's scan line")
        if emu.find(GUARD, r, k) or emu.find(POWER_SAFE, r, k):
            raise LaneFail("boot 5: the guard still counted after a software restart")
        end = time.time() + 400
        while True:
            st = api.wait_up(400)
            note = st.get("reset_note") or ""
            if not note.startswith(NOTE_PREFIX) and st.get("scan") is not None:
                break
            if time.time() > end:
                raise LaneFail(f"boot 5: /api/status reset_note {note!r}, scan {st.get('scan')!r}")
            time.sleep(2)
        scan = st["scan"]
        if not (scan.get("row_us", 0) > 0 and "late_frames" in (scan.get("beam") or {})):
            raise LaneFail(f"boot 5: /api/status scan is incomplete: {scan}")
        ok("boot 5", f"reset {m[1]}: the panel started again; /api/status scan row_us {scan['row_us']}, beam.late_frames {scan['beam']['late_frames']}, reset_note {note or None!r}")
    except LaneFail as e:
        print(f"EMU_POWER_SAFE_FAIL: {e}")
        if emu:
            print(f"--- last console lines of {emu.log}:")
            print(emu.tail())
        return 1
    finally:
        if emu and not emu.done:
            emu.kill()
    print(f"emu: log {log}")
    print(f"EMU_POWER_SAFE_OK ({time.time() - t0:.0f} s wall)")
    return 0
