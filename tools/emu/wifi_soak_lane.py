"""The WiFi soak lane (`run.py wifi-soak`, `make emu-wifi-soak`): an `emu-wifi-soak` image
(firmware src/wifi_soak.rs) on the virtual access point, the guest's port 80 on 127.0.0.1 (inside the
private network namespace run.py sets up, as the API lane's). The firmware keeps the network away from
the station and logs each step as `wifi-soak: …`; this judges the console and the API:

  boot      WiFi up, GIFs playing
  direct    40 connects pinned to an access point and channel that are not there, playback paused:
            the heap after each ten; fails if the tries keep heap (a least-squares slope of more than
            SLOPE_MAX bytes a try lost, or more than DRIFT_MAX lost in all, from the first checkpoint: the
            first ten take the left network's buffers back)
  away      the reconnect looks for a name nobody answers to for 20 tries: every one a scan that does
            not find it; the driver restarted at rounds 6, 12 and 18 (`restarting the WiFi driver`),
            each round's `reconnect failed` line with the heap after it: the same slope rule
  refused   the name given back with a wrong password for 3 rounds: each scan finds `demo` and the
            connect ladder's ten attempts fail their handshake; the heap after each round, held flat
            (DRIFT_MAX over the step)
  back      the password given back: `reconnected (after a scan)`
  tight     ~12 KB free in pieces just big enough for the reconnect's scan, the learned access point
            spoiled, the association dropped, for 150 s: what the reconnect does there is reported
            (the finding: can it rejoin in such a heap?); then the heap given back: it must rejoin
  status    net.wifi_driver_resets 3 or more, wifi_last_reason 0 (the last try joined), wifi_activity up;
            the reasons logged: 201 for every scan that did not find the network, never 8 (our own
            leave) for a refused round
  stuck 1   the name taken away with a short hour (150 s): the `wifi_stuck` restart; boot 2, told to
            look for the name too, does not find it: no setup network, the reconnect carries on, and
            joins once the name is given back; restarts[0] `wifi_stuck`, /api/log/last kept
  stuck 2   a wrong password with a short hour: the `wifi_stuck` restart; boot 3, told to join with
            it, finds the network and is refused: the setup network, as at any boot

A test panel was offline for hours on 2026-11-21 with its network up: 1356 tries, the heap at 11.7 KB
free in pieces of 2.3 KB at the end, from ~45 KB. Its loss, ~25 B a try, would be ~1 KB over the
direct step's 40. Prints EMU_WIFI_SOAK_OK. Correctness only, like every emulator lane: the access point
never goes away here, the station's side of it does (a spoiled pin, a name nobody answers to).
"""
import os
import re
import time

from api_lane import PORT, WIFI, Api, Emulator, LaneFail, machine, seeded_image

CONNECTED = re.compile(r"WiFi connected, IP ")
GIF = re.compile(r"\] /gifs/\S+\.gif <- ")
START = re.compile(r"wifi-soak: start: playback paused, heap (\d+) B free, largest block (\d+) B")
DIRECT_AT = re.compile(r"wifi-soak: direct (\d+)/(\d+): heap (\d+) B free, largest block (\d+) B, least (\d+) B, last disconnect reason (\d+)")
DIRECT_DONE = re.compile(r"wifi-soak: direct done: (\d+) tries, heap (\d+) B free \(from (\d+) B\), largest block (\d+) B")
JOINED_SPOILED = re.compile(r"wifi-soak: direct \d+: joined a spoiled pin")
AWAY = re.compile(r"wifi-soak: away: the reconnect looks for '([^']+)'")
FAILED = re.compile(r"\[net\] reconnect failed \((straight back|scan), reason (\d+), heap (\d+) B free, largest block (\d+) B\)")
RESET = re.compile(r"\[net\] (\d+) reconnect rounds failed \(last disconnect reason (\d+)\): restarting the WiFi driver")
RESET_ERR = re.compile(r"\[net\] WiFi driver restart: ")
FOUND_AWAY = re.compile(r"'demo-away' found")
REFUSED = re.compile(r"wifi-soak: refused: .*heap (\d+) B free, largest block (\d+) B")
FOUND_DEMO = re.compile(r"'demo' found: rssi=")
BACK = re.compile(r"wifi-soak: back: ")
BACK_SCAN = re.compile(r"\[net\] reconnected \(after a scan\)")
REJOINED = re.compile(r"wifi-soak: rejoined: heap (\d+) B free")
TIGHT = re.compile(r"wifi-soak: tight: (\d+) blocks held, heap (\d+) B free, largest block (\d+) B")
RELEASE = re.compile(r"wifi-soak: release: (joined|not joined) in the squeezed heap \((\d+) B free, largest block (\d+) B at the end\)")
HELD = re.compile(r"\[net\] reconnect held back: heap (\d+) B free, largest block (\d+) B")
SCANNING = re.compile(r"scanning for 'demo'$")
NOT_FOUND = re.compile(r"'demo' not found \(a scan for it answered by (\d+) APs\)")
BACK_ANY = re.compile(r"\[net\] reconnected \((straight back|after a scan)\)")
REC_ERR = re.compile(r"\[net\] reconnect error: (.*)")
DONE = re.compile(r"wifi-soak: done: playback resumed")
STUCK1 = re.compile(r"wifi-soak: stuck 1: ")
STUCK2 = re.compile(r"wifi-soak: stuck 2: ")
HOUR = re.compile(r"\[net\] WiFi down for an hour \(.*\): restarting to rejoin")
REASON = re.compile(r"reset reason: (\S+) \((\w+)\)")
BOOT2 = re.compile(r"wifi-soak: boot 2: joining 'demo-away'")
BOOT2_BACK = re.compile(r"wifi-soak: boot 2 back: joined")
BOOT3 = re.compile(r"wifi-soak: boot 3: joining 'demo' with a wrong password")
QUIET = re.compile(r"not joined after a restart to rejoin, the network not found; reconnecting, no setup network")
AP_MODE = re.compile(r"starting AP mode: OpenDMD-Setup")
RESTART = re.compile(r"restarting to rejoin")

# Bytes a try may lose on a least-squares slope, and in all over a step: well under the test panel's
# ~25 B a try, well over the heap's own jitter with playback paused.
SLOPE_MAX = 8.0
DRIFT_MAX = 512
TIGHT_MS_S = 150  # wifi_soak::TIGHT_MS


def slope(points):
    """Least-squares bytes per try of (try, free) points (negative: lost)."""
    n = len(points)
    mx = sum(x for x, _ in points) / n
    my = sum(y for _, y in points) / n
    den = sum((x - mx) ** 2 for x, _ in points)
    return sum((x - mx) * (y - my) for x, y in points) / den if den else 0.0


def run(a, sim, rom, fail_rx) -> int:
    t0 = time.time()
    logs = a.log.parent if a.log else a.emu_dir / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    log = a.log or logs / f"wifi-soak-{time.strftime('%Y%m%d-%H%M%S')}.log"
    emu = None

    def ok(name, detail):
        print(f"pass  {name}: {detail}", flush=True)

    try:
        image = seeded_image(a.image, a.emu_dir / "wifisoak")
        cmd = [*machine(sim, "esp32", image, rom, ["--sd", str(a.card)]), "--max-seconds", str(a.seconds or 3600),
               "--console", "uart0", "--wifi", WIFI, "--net", "nat", "--hostfwd", f"tcp:127.0.0.1:{PORT}-:80", "--no-dump"]
        emu = Emulator(cmd, dict(os.environ), log, fail_rx, "wifi-soak")
        emu.resets_allowed = 3  # the power-on, and the two wifi_stuck restarts
        emu.start()
        wall = a.wall or 7200
        api = Api(emu)

        i, _ = emu.wait_for(CONNECTED, 600, what="WiFi connected at boot")
        sha = emu.find(re.compile(r"ELF file SHA256:\s+([0-9a-f]+)"))
        if not (sha and a.want_sha.startswith(sha[0][1][1])):
            raise LaneFail(f"the booted app is not {a.image}")
        g, _ = emu.wait_for(GIF, 600, what="a GIF started")
        ok("boot", f"WiFi up, {emu.lines[g][emu.lines[g].index('/gifs/'):][:70]}")

        # The pinned connects.
        s, sm = emu.wait_for(START, wall, i, what="wifi-soak: start")
        d, dm = emu.wait_for(DIRECT_DONE, wall, s, what="wifi-soak: direct done")
        if emu.find(JOINED_SPOILED, s, d):
            raise LaneFail("a pinned connect to the spoiled access point joined")
        at = [(int(m[1]), int(m[3])) for _, m in emu.find(DIRECT_AT, s, d)]
        if len(at) != 4:
            raise LaneFail(f"{len(at)} heap checkpoints in the direct step, wanted 4")
        start_free, end_free = int(sm[1]), int(dm[2])
        # From the first checkpoint: the first tries take the left network's buffers back.
        pts = [*at, (int(dm[1]), end_free)]
        sl = slope(pts)
        least = min(int(m[5]) for _, m in emu.find(DIRECT_AT, s, d))
        reasons = sorted({int(m[6]) for _, m in emu.find(DIRECT_AT, s, d)})
        detail = (f"{dm[1]} tries: heap {start_free} -> {end_free} B ({end_free - start_free:+d} B, slope {sl:+.1f} B/try, "
                  f"least {least} B), largest {sm[2]} -> {dm[4]} B; checkpoints {at}; disconnect reasons {reasons}")
        if sl < -SLOPE_MAX or at[0][1] - end_free > DRIFT_MAX:
            raise LaneFail(f"direct: the tries keep heap: {detail}")
        ok("direct", detail)

        # Away: the worker's reconnect, scans that find nothing.
        w, wm = emu.wait_for(AWAY, 60, d, what="wifi-soak: away")
        b, _ = emu.wait_for(REFUSED, wall, w, what="wifi-soak: refused")
        fails = emu.find(FAILED, w, b)
        resets = emu.find(RESET, w, b)
        if emu.find(FOUND_AWAY, w, b):
            raise LaneFail("away: a scan found the name nobody answers to")
        if emu.find(RESET_ERR, w, b):
            raise LaneFail("away: a driver restart failed: " + emu.find(RESET_ERR, w, b)[0][1].string[:160])
        if emu.find(RESTART, w, b):
            raise LaneFail("away: a firmware restart within 20 tries")
        if len(fails) < 20 or any(m[1] != "scan" for _, m in fails):
            raise LaneFail(f"away: {len(fails)} failed tries ({sorted({m[1] for _, m in fails})}), wanted 20 scans")
        rounds = [int(m[1]) for _, m in resets]
        if rounds != [6, 12, 18]:
            raise LaneFail(f"away: driver restarts at rounds {rounds}, wanted [6, 12, 18]")
        pts = [(k, int(m[3])) for k, (_, m) in enumerate(fails, 1)]
        sl = slope(pts)
        lost = pts[0][1] - pts[-1][1]
        reasons = sorted({int(m[2]) for _, m in fails})
        if reasons != [201]:
            raise LaneFail(f"away: the failed scans' reasons {reasons}, wanted [201] (not found)")
        detail = (f"{len(fails)} scans for '{wm[1]}' that found nothing, driver restarts at rounds {rounds}; heap after each "
                  f"{pts[0][1]} -> {pts[-1][1]} B ({-lost:+d} B, slope {sl:+.1f} B/try, least {min(y for _, y in pts)} B), "
                  f"largest {fails[0][1][4]} -> {fails[-1][1][4]} B; disconnect reasons {reasons}")
        if sl < -SLOPE_MAX or lost > DRIFT_MAX:
            raise LaneFail(f"away: the tries keep heap: {detail}")
        ok("away", detail)

        # Refused: found, every rung's handshake failing.
        rf, rfm = b, emu.find(REFUSED, b, b + 1)[0][1]
        b, _ = emu.wait_for(BACK, wall, rf, what="wifi-soak: back")
        rounds = emu.find(FAILED, rf, b)
        found = emu.find(FOUND_DEMO, rf, b)
        if len(rounds) < 3 or len(found) < len(rounds) or any(m[1] != "scan" for _, m in rounds):
            raise LaneFail(f"refused: {len(rounds)} failed rounds, {len(found)} scans finding 'demo': wanted 3 and 3")
        if emu.find(BACK_SCAN, rf, b):
            raise LaneFail("refused: joined with a wrong password")
        frees = [int(rfm[1]), *(int(m[3]) for _, m in rounds)]
        lost = frees[1] - frees[-1]
        reasons = sorted({int(m[2]) for _, m in rounds})
        if 8 in reasons or 0 in reasons or 201 in reasons:
            raise LaneFail(f"refused: the failed rounds' reasons {reasons}: our own leave (8), none (0) or not found (201)")
        detail = (f"{len(rounds)} rounds of ten handshakes that failed (disconnect reasons {reasons}); heap {frees} B, "
                  f"{-lost:+d} B from the first round's end to the last's, largest {rounds[-1][1][4]} B")
        if lost > DRIFT_MAX:
            raise LaneFail(f"refused: the rounds keep heap: {detail}")
        ok("refused", detail)

        # Back.
        r, _ = emu.wait_for(BACK_SCAN, 300, b, what="reconnected after a scan")
        j, jm = emu.wait_for(REJOINED, 120, r, what="wifi-soak: rejoined")
        ok("back", f"{emu.lines[r][emu.lines[r].index('[net]'):][:70]}; heap {jm[1]} B free (from {start_free} B at the start)")

        # Tight: what the reconnect does in a heap like the test panel's, then the way back.
        t, tm = emu.wait_for(TIGHT, 120, j, what="wifi-soak: tight")
        rl, rlm = emu.wait_for(RELEASE, 600, t, what="wifi-soak: release")
        steps = {
            "pinned connects": len(emu.find(re.compile(r"reconnecting to 'demo' at .* no scan"), t, rl)),
            "scans": len(emu.find(SCANNING, t, rl)),
            "scans not finding it": len(emu.find(NOT_FOUND, t, rl)),
            "failed rounds": len(emu.find(FAILED, t, rl)),
            "held back": len(emu.find(HELD, t, rl)),
            "errors": len(emu.find(REC_ERR, t, rl)),
        }
        joined = emu.find(BACK_ANY, t, rl)
        fails = [(m[2], m[3], m[4]) for _, m in emu.find(FAILED, t, rl)]
        if (rlm[1] == "joined") != bool(joined):
            raise LaneFail(f"tight: the firmware says {rlm[1]!r}, the console has {len(joined)} reconnects in the squeeze")
        if joined:
            after = "still up after the release"
        else:
            b2, _ = emu.wait_for(BACK_ANY, 300, rl, what="rejoined after the release")
            after = f"after the release: {emu.lines[b2][emu.lines[b2].index('[net]'):][:60]}"
        emu.wait_for(DONE, 120, rl, what="wifi-soak: done")
        ok("tight", f"heap {tm[2]} B free, largest block {tm[3]} B ({tm[1]} blocks held): {rlm[1]} in {TIGHT_MS_S} s "
                    f"({'; '.join(f'{k} {v}' for k, v in steps.items())}; failed rounds' reason, free, largest {fails[:6]}"
                    f"{', ' + emu.lines[joined[0][0]][emu.lines[joined[0][0]].index('[net]'):][:50] if joined else ''}); {after}")

        st = api.wait_up(120)
        net = st.get("net") or {}
        # (3 while away; the tight step's pinned connects can add one after the release.)
        if not ((net.get("wifi_driver_resets") or 0) >= 3 and net.get("wifi_last_reason") == 0 and net.get("wifi_activity") == "up"):
            raise LaneFail(f"/api/status net: wifi_driver_resets {net.get('wifi_driver_resets')}, wifi_last_reason "
                           f"{net.get('wifi_last_reason')}, wifi_activity {net.get('wifi_activity')!r}")
        ok("status", f"net.wifi_driver_resets {net['wifi_driver_resets']}, wifi_last_reason 0, "
                     f"wifi_reconnect_tries {net.get('wifi_reconnect_tries')}, wifi_activity up")

        # Stuck 1: the short hour's restart, boot 2 quiet, then back.
        s1, _ = emu.wait_for(STUCK1, 300, rl, what="wifi-soak: stuck 1")
        if emu.find(RESTART, 0, s1):
            raise LaneFail("a firmware restart to rejoin before stuck 1: " + emu.find(RESTART, 0, s1)[0][1].string[:160])
        h1, _ = emu.wait_for(HOUR, 600, s1, what="the hour's restart (stuck 1)")
        r2, rm2 = emu.wait_for(REASON, 300, h1, what="boot 2's reset reason")
        if rm2[2] != "wifi_stuck":
            raise LaneFail(f"boot 2: {emu.lines[r2][:200]}")
        emu.wait_for(BOOT2, 120, r2, what="wifi-soak: boot 2")
        q, _ = emu.wait_for(QUIET, 300, r2, what="boot 2 reconnecting without the setup network")
        bb, _ = emu.wait_for(BOOT2_BACK, 600, q, what="wifi-soak: boot 2 back")
        if emu.find(AP_MODE, r2, bb):
            raise LaneFail("boot 2: the setup network for a network not found")
        if emu.find(HOUR, r2, bb):
            raise LaneFail("boot 2: a second hour's restart in the same outage")
        st = api.wait_up(120)
        last = (st.get("restarts") or [{}])[0]
        if last.get("reason") != "wifi_stuck":
            raise LaneFail(f"boot 2: /api/status restarts[0] {last}")
        code, _, body = api.raw("GET", "/api/log/last", timeout=20)
        text = body.decode(errors="replace")
        if code != 200 or "wifi_stuck" not in text.splitlines()[0]:
            raise LaneFail(f"boot 2: GET /api/log/last HTTP {code}: {text[:200]!r}")
        ok("stuck 1", f"{emu.lines[h1][emu.lines[h1].index('[net]'):][:150]}; boot 2 {emu.lines[r2][emu.lines[r2].index('reset reason'):][:60]}: "
                      f"not found, no setup network, joined once the name came back; restarts[0] wifi_stuck; /api/log/last {len(body)} B")

        # Stuck 2: refused; boot 3 raises the setup network.
        s2, _ = emu.wait_for(STUCK2, 300, bb, what="wifi-soak: stuck 2")
        h2, _ = emu.wait_for(HOUR, 900, s2, what="the hour's restart (stuck 2)")
        r3, rm3 = emu.wait_for(REASON, 300, h2, what="boot 3's reset reason")
        if rm3[2] != "wifi_stuck":
            raise LaneFail(f"boot 3: {emu.lines[r3][:200]}")
        emu.wait_for(BOOT3, 120, r3, what="wifi-soak: boot 3")
        ap, _ = emu.wait_for(AP_MODE, 900, r3, what="boot 3's setup network")
        if emu.find(QUIET, r3, ap):
            raise LaneFail("boot 3: said quiet for a network found and refused")
        if not emu.find(FOUND_DEMO, r3, ap):
            raise LaneFail("boot 3: the setup network without the scan finding 'demo'")
        ok("stuck 2", f"{emu.lines[h2][emu.lines[h2].index('[net]'):][:150]}; boot 3 {emu.lines[r3][emu.lines[r3].index('reset reason'):][:60]}: "
                      f"found, refused: {emu.lines[ap][emu.lines[ap].index('starting'):][:40]}")
    except LaneFail as e:
        print(f"EMU_WIFI_SOAK_FAIL: {e}")
        if emu:
            print(f"--- last console lines of {emu.log}:")
            print(emu.tail())
        return 1
    finally:
        if emu and not emu.done:
            emu.kill()
    print(f"emu: log {log}")
    print(f"EMU_WIFI_SOAK_OK ({time.time() - t0:.0f} s wall)")
    return 0
