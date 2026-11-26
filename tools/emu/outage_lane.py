"""The WiFi outage test's lane (`run.py outage`, `make emu-outage`): a `wifi-outage-test` image
(firmware src/outage_test.rs, the hardware test's hook) on the virtual access point, the guest's port
80 on 127.0.0.1 (inside the private network namespace run.py sets up, as the API lane's). Two outages
asked for over the API, judged on the console:

  A   PUT {"wifi_outage_s": 300}: `wifi-outage: start`, driver restarts (`wifi-outage: reset`), GIFs
      playing throughout, no restart, `wifi-outage: over` after ~300 s, `wifi-outage: rejoined`
  B   PUT {"wifi_outage_s": 300, "wifi_outage_hour_s": 120}: the short hour's `wifi_stuck` restart
      (`wifi-outage: restart wifi_stuck`), the next boot `wifi-outage: boot, N s left` still away,
      then `over` and `rejoined`; the saved name never changed (the boot after joins 'demo')
  bad PUT {"wifi_outage_s": 0} and 7201 answered 400

Prints EMU_OUTAGE_OK. Correctness only, never timing.
"""
import os
import re
import time

from api_lane import PORT, WIFI, Api, Emulator, LaneFail, machine, seeded_image

CONNECTED = re.compile(r"WiFi connected, IP ")
GIF = re.compile(r"\] /gifs/\S+\.gif <- ")
START = re.compile(r"wifi-outage: start (\d+) s hour (\d+) s")
RESET = re.compile(r"wifi-outage: reset rd (\d+) try (\d+) miss (\d+) heap (\d+)/(\d+)")
MINUTE = re.compile(r"wifi-outage: (\d+) s left")
OVER = re.compile(r"wifi-outage: over, name back")
REJOINED = re.compile(r"wifi-outage: rejoined (\d+) s after")
RESTART = re.compile(r"wifi-outage: restart (wifi_\w+) try (\d+) miss (\d+) (\d+) s left")
BOOT = re.compile(r"wifi-outage: boot, (\d+) s left")
REASON = re.compile(r"reset reason: (\S+) \((\w+)\)")
SCAN_AWAY = re.compile(r"scanning for 'demo-outage'")
LONG = 110  # the console lines this hook writes stay under this


def run(a, sim, rom, fail_rx) -> int:
    t0 = time.time()
    logs = a.log.parent if a.log else a.emu_dir / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    log = a.log or logs / f"outage-{time.strftime('%Y%m%d-%H%M%S')}.log"
    emu = None

    def ok(name, detail):
        print(f"pass  {name}: {detail}", flush=True)

    def short(i0, i1):
        for l in emu.lines[i0:i1]:
            if "wifi-outage:" in l and len(l[l.index("wifi-outage:"):]) > LONG:
                raise LaneFail(f"a long wifi-outage line: {l[:200]}")

    try:
        image = seeded_image(a.image, a.emu_dir / "outage")
        cmd = [*machine(sim, "esp32", image, rom, ["--sd", str(a.card)]), "--max-seconds", str(a.seconds or 1800),
               "--console", "uart0", "--wifi", WIFI, "--net", "nat", "--hostfwd", f"tcp:127.0.0.1:{PORT}-:80", "--no-dump"]
        emu = Emulator(cmd, dict(os.environ), log, fail_rx, "outage")
        emu.resets_allowed = 2  # the power-on, and B's wifi_stuck restart
        emu.start()
        wall = a.wall or 3600
        api = Api(emu)
        i, _ = emu.wait_for(CONNECTED, 600, what="WiFi connected at boot")
        emu.wait_for(GIF, 600, i, what="a GIF started")
        api.wait_up(300)

        for body in ({"wifi_outage_s": 0}, {"wifi_outage_s": 7201}, {"wifi_outage_s": 60, "wifi_outage_hour_s": 10}):
            code, _, got = api.raw("PUT", "/api/config", body=body, timeout=30)
            if code != 400:
                raise LaneFail(f"PUT {body}: HTTP {code} {got[:120]!r}, wanted 400")
        ok("bad", "wifi_outage_s 0 and 7201, wifi_outage_hour_s 10: 400")

        # A: five minutes, the real hour.
        m = emu.mark()
        code, _, got = api.raw("PUT", "/api/config", body={"wifi_outage_s": 300}, timeout=30)
        if code != 200:
            raise LaneFail(f"PUT wifi_outage_s 300: HTTP {code} {got[:120]!r}")
        s, sm = emu.wait_for(START, 60, m, what="wifi-outage: start (A)")
        o, _ = emu.wait_for(OVER, wall, s, what="wifi-outage: over (A)")
        r, rm = emu.wait_for(REJOINED, 300, o, what="wifi-outage: rejoined (A)")
        resets = emu.find(RESET, s, o)
        gifs = emu.find(GIF, s, o)
        if not resets:
            raise LaneFail("A: no driver restart in a five-minute outage")
        if emu.find(RESTART, s, r) or emu.find(REASON, s, r):
            raise LaneFail("A: a restart in a five-minute outage with the real hour")
        if len(gifs) < 5:
            raise LaneFail(f"A: {len(gifs)} GIFs started during the outage: playback stopped?")
        if not emu.find(SCAN_AWAY, s, o):
            raise LaneFail("A: no scan for the absent name")
        short(s, r)
        ok("A", f"start {sm[1]} s (hour {sm[2]} s); {len(resets)} driver restarts at rounds "
                f"{[int(x[1]) for _, x in resets]} (misses {[int(x[3]) for _, x in resets]}); {len(gifs)} GIFs "
                f"started meanwhile; {len(emu.find(MINUTE, s, o))} minute lines; rejoined {rm[1]} s after the end")

        # B: five minutes, a two-minute hour: the restart, the boot after it still away.
        api.wait_up(120)
        m = emu.mark()
        code, _, got = api.raw("PUT", "/api/config", body={"wifi_outage_s": 300, "wifi_outage_hour_s": 120}, timeout=30)
        if code != 200:
            raise LaneFail(f"PUT wifi_outage_s 300 hour 120: HTTP {code} {got[:120]!r}")
        s, sm = emu.wait_for(START, 60, m, what="wifi-outage: start (B)")
        x, xm = emu.wait_for(RESTART, wall, s, what="wifi-outage: restart (B)")
        if xm[1] != "wifi_stuck":
            raise LaneFail(f"B: {emu.lines[x][:160]}")
        b, bm = emu.wait_for(REASON, 300, x, what="the boot after B's restart")
        if bm[2] != "wifi_stuck":
            raise LaneFail(f"B's boot: {emu.lines[b][:160]}")
        k, km = emu.wait_for(BOOT, 120, b, what="wifi-outage: boot (B)")
        if abs(int(km[1]) - int(xm[4])) > 30:
            raise LaneFail(f"B: {xm[4]} s left at the restart, {km[1]} s at the boot")
        o, _ = emu.wait_for(OVER, wall, k, what="wifi-outage: over (B)")
        if not emu.find(SCAN_AWAY, k, o):
            raise LaneFail("B's boot: no scan for the absent name")
        if emu.find(CONNECTED, k, o):
            raise LaneFail("B's boot: joined before the outage was over")
        r, rm = emu.wait_for(REJOINED, 300, o, what="wifi-outage: rejoined (B)")
        short(s, r)
        st = api.wait_up(120)
        if (st.get("restarts") or [{}])[0].get("reason") != "wifi_stuck":
            raise LaneFail(f"B: /api/status restarts[0] {(st.get('restarts') or [{}])[0]}")
        ok("B", f"{emu.lines[x][emu.lines[x].index('wifi-outage'):]}; the boot after: {km[1]} s left, "
                f"no join until over, rejoined {rm[1]} s after; restarts[0] wifi_stuck")
    except LaneFail as e:
        print(f"EMU_OUTAGE_FAIL: {e}")
        if emu:
            print(f"--- last console lines of {emu.log}:")
            print(emu.tail())
        return 1
    finally:
        if emu and not emu.done:
            emu.kill()
    print(f"emu: log {log}")
    print(f"EMU_OUTAGE_OK ({time.time() - t0:.0f} s wall)")
    return 0
