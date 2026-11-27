"""The resume lane (`run.py resume`, `make emu-resume`): an `emu-crash-test` image (a GIF whose
name holds CRASH_ON_OPEN panics the board when the player opens it, firmware src/sd.rs) on the
virtual access point, the guest's port 80 on 127.0.0.1 (inside the private network namespace
run.py sets up, as the API lane's), reboots allowed; a copy of the card written through. The
in-order place (`opendmd_core::resume`) across resets, which RTC memory carries in the emulator:

  restart   shuffle off, a playlist entry up 3.5 s, POST /api/reboot: the next boot says
            `resume: in-order places kept` and `resume: in order at entry N (after the last`,
            and the entry after it plays
  crash     a CRASH_ON_OPEN GIF put right after the one playing: the rebuild opens it, the
            board panics; the boot after goes back to it and panics again; the second crash
            boot steps over it (`2 crash boots in a row`, `one past the last`): the GIF after it
            plays, two panics in all, /api/status and the web app answer
  drop      two crash files in a row after the one playing: three panics, the third crash boot
            drops the places (`3 crash boots in a row: in-order places dropped`), the
            playlist's first entry plays, the API answers

Prints EMU_RESUME_OK. Correctness only, like every emulator lane.
"""
import gzip
import os
import re
import shutil
import time
from pathlib import Path

from api_lane import PORT, WIFI, Api, Emulator, LaneFail, machine, seeded_image

KEPT = re.compile(r"resume: in-order places kept")
RESUMED = re.compile(r"resume: in order at entry (\d+) \((.+), (\d+) read in (\d+) us\)")
STEPS = re.compile(r"resume: .*2 crash boots in a row: the next steps one further on")
DROPPED = re.compile(r"resume: 3 crash boots in a row: in-order places dropped")
CRASH = re.compile(r"emu-crash-test: (\S+) opened")
PANICKED = re.compile(r"panicked at|Guru Meditation")
GIF = re.compile(r"\] /gifs/\S+\.gif <- ")


def run(a, sim, rom, fail_rx) -> int:
    t0 = time.time()
    logs = a.log.parent if a.log else a.emu_dir / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    log = a.log or logs / f"resume-{stamp}.log"
    card = logs / f"resume-{stamp}-card.img"
    passes = []

    def ok(name, detail):
        line = f"pass  {name}: {detail}"
        passes.append(line)
        print(line, flush=True)

    # The deliberate panics trip run.py's panic and backtrace patterns: judged here instead.
    fails = [(rx, name) for rx, name in fail_rx if name not in ("panic/abort", "backtrace", "Guru Meditation")]
    emu = None
    try:
        image = seeded_image(a.image, a.emu_dir / "resume")
        shutil.copyfile(a.card, card)
        cmd = [*machine(sim, "esp32", image, rom, ["--sd-rw", str(card)]), "--max-seconds", str(a.seconds or 3600),
               "--console", "uart0", "--wifi", WIFI, "--net", "nat", "--hostfwd", f"tcp:127.0.0.1:{PORT}-:80", "--no-dump"]
        emu = Emulator(cmd, dict(os.environ), log, fails, "resume")
        emu.start()
        api = Api(emu)
        api.wait_up(400)
        sha = emu.find(re.compile(r"ELF file SHA256:\s+([0-9a-f]+)"))
        if not (sha and a.want_sha.startswith(sha[0][1][1])):
            raise LaneFail(f"the booted app is not {a.image}")
        emu.wait_for(GIF, 300, what="a GIF started")
        _, r = api.json("PUT", "/api/config", {"shuffle": False, "dwell_min": 60, "dwell_max": 60}, timeout=120)
        if r.get("ok") is not True:
            raise LaneFail(f"PUT shuffle off answered {r}")
        listed = [l for l in api.call("GET", "/filelist.txt")[1].decode().split("\n") if l]
        on = lambda s: (s.get("now_playing") or {}).get("path")

        def wait_status(cond, timeout, what):
            end, st = time.time() + timeout, None
            while time.time() < end:
                try:
                    st = api.status()
                except LaneFail:
                    st = None
                if st and cond(st):
                    return st
                emu.check()
                time.sleep(0.5)
            raise LaneFail(f"{what} not seen in /api/status within {timeout} s wall (now playing {on(st or {})!r})")

        def settled(lst):
            # Up past resume::BRIEF_MS, so a rebuild moves on rather than replaying it.
            st = wait_status(lambda s: s.get("index_state") == "ready" and on(s) in lst
                             and s["now_playing"].get("elapsed_ms", 0) >= 3500, 120, "a playlist GIF up 3.5 s")
            return lst.index(on(st))

        def skip(lst):
            before = api.status()["now_playing"]["ordinal"]
            api.json("POST", "/api/skip")
            wait_status(lambda s: s["now_playing"]["ordinal"] != before and on(s) in lst, 60, "a skip")

        while settled(listed) < 2:
            skip(listed)

        # -- restart ----------------------------------------------------------------
        at = settled(listed)
        mark = emu.mark()
        emu.resets_allowed += 1
        _, r = api.json("POST", "/api/reboot")
        if r.get("rebooting") is not True:
            raise LaneFail(f"POST /api/reboot answered {r}")
        emu.wait_for(KEPT, 300, mark, "`resume: in-order places kept` after the restart")
        i, m = emu.wait_for(RESUMED, 300, mark, "the `resume:` line after the restart")
        if m[2] != "after the last" or int(m[1]) != at + 1:
            raise LaneFail(f"after the restart: {emu.lines[i][:200]} (was at entry {at})")
        api.wait_up(300)
        st = wait_status(lambda s: on(s) in listed, 300, "a playlist GIF after the restart")
        if on(st) != listed[at + 1]:
            raise LaneFail(f"after the restart playing {on(st)}, wanted {listed[at + 1]} (the one after {listed[at]}); the first is {listed[0]}")
        ok("restart", f"at entry {at} ({listed[at]}), POST /api/reboot: places kept, {emu.lines[i][emu.lines[i].index('resume:'):][:100]}; "
                      f"{listed[at + 1]} plays")

        gif = (a.emu_dir / "card" / "root" / "gifs").glob("Other/*.gif")
        data = next(iter(sorted(gif))).read_bytes()

        def crash_after(names, lst):
            """Put crash files right after the GIF playing; the panics and boots that follow."""
            for n in names:
                api.call("PUT", f"/api/files/gifs/EmuCrash/{n}", data, timeout=120)
            b = settled(lst)
            out = lst[:b + 1] + [f"/gifs/EmuCrash/{n}" for n in names] + lst[b + 1:]
            mark = emu.mark()
            resets = emu.resets
            emu.resets_allowed += len(names) + 1
            try:
                api.call("PUT", "/api/files/filelist.txt", "".join(p + "\n" for p in out).encode(), timeout=60)
            except LaneFail:
                pass  # (the reply may be cut by the panic)
            return b, out, mark, resets

        # -- crash: one file -------------------------------------------------------------
        b, list2, mark, resets = crash_after(["CRASH_ON_OPEN_1.gif"], listed)
        i, _ = emu.wait_for(STEPS, 600, mark, "the second crash boot's `2 crash boots in a row`")
        j, m = emu.wait_for(RESUMED, 300, i, "the `resume:` line of the second crash boot")
        if m[2] != "one past the last, after 2 crash boots" or int(m[1]) != b + 2:
            raise LaneFail(f"the second crash boot: {emu.lines[j][:200]} (the one before the crash file at entry {b})")
        api.wait_up(300)
        st = wait_status(lambda s: on(s) in list2, 300, "a playlist GIF after the crash boots")
        crashes = emu.find(CRASH, mark)
        if on(st) != list2[b + 2] or len(crashes) != 2 or emu.resets - resets != 2:
            raise LaneFail(f"after the crash boots playing {on(st)} (wanted {list2[b + 2]}), {len(crashes)} panics, "
                           f"{emu.resets - resets} resets (wanted 2 and 2)")
        panics = [l for l in emu.lines if PANICKED.search(l)]
        code, _, page = api.raw("GET", "/", timeout=60)
        if page[:2] == b"\x1f\x8b":
            page = gzip.decompress(page)
        if code != 200 or b"<html" not in page.lower():
            raise LaneFail(f"the web app after the crash boots: HTTP {code}, {page[:80]!r}")
        ok("crash", f"CRASH_ON_OPEN_1.gif after entry {b}: 2 panics opening it, 2 resets; the second crash boot stepped over it "
                    f"({emu.lines[j][emu.lines[j].index('resume:'):][:90]}): {list2[b + 2]} plays; status and the web app answer")
        if len(panics) != len(emu.find(CRASH)):
            raise LaneFail(f"{len(panics)} panics, {len(emu.find(CRASH))} of them the test's: {panics[:3]}")

        # Two playlist starts clear the count.
        skip(list2)
        # -- drop: two files in a row -----------------------------------------------------
        c, list3, mark, resets = crash_after(["CRASH_ON_OPEN_2.gif", "CRASH_ON_OPEN_3.gif"], list2)
        i, _ = emu.wait_for(DROPPED, 900, mark, "the third crash boot's `in-order places dropped`")
        api.wait_up(300)
        st = wait_status(lambda s: on(s) in list3, 300, "a playlist GIF after the places were dropped")
        crashes = [m[1] for _, m in emu.find(CRASH, mark)]
        want = ["/gifs/EmuCrash/CRASH_ON_OPEN_2.gif", "/gifs/EmuCrash/CRASH_ON_OPEN_3.gif"]
        if on(st) != list3[0] or len(crashes) != 3 or not set(crashes) <= set(want) or emu.resets - resets != 3:
            raise LaneFail(f"after the third crash boot playing {on(st)} (wanted {list3[0]}), panics {crashes}, {emu.resets - resets} resets")
        api.status()
        panics = [l for l in emu.lines if PANICKED.search(l)]
        if len(panics) != len(emu.find(CRASH)):
            raise LaneFail(f"{len(panics)} panics, {len(emu.find(CRASH))} of them the test's: {panics[:3]}")
        ok("drop", f"two crash files after entry {c}: 3 panics ({', '.join(Path(x).name for x in crashes)}), 3 resets; "
                   f"places dropped, {list3[0]} (the top) plays; the API answers")
    except LaneFail as e:
        print(f"EMU_RESUME_FAIL: {e}")
        if emu:
            print(f"--- last console lines of {emu.log}:")
            print(emu.tail())
        return 1
    finally:
        if emu and not emu.done:
            emu.kill()
    card.unlink(missing_ok=True)
    print(f"emu: log {log}")
    print(f"EMU_RESUME_OK ({time.time() - t0:.0f} s wall)")
    return 0
