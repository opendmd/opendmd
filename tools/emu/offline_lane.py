"""The offline lane (`make emu-offline`, run by `run.py offline`): a display with WiFi off
(`"network": "off"`), booted in esp32sim inside the API lane's private network namespace.

Boot A: the smoke image with `network=off` seeded into NVS (as the installer's offline
manifest writes it), a card with no `network` key (so NVS decides), short dwells, the card's
own Formulas in /formulas (tools/emu/offline/: one with no compiled file, one whose
compiled file is another version's, one that does not compile, two nested to the
compiler's cap, one that reads the clock, a look that reads the clock), a playlist of them
and four GIFs, and the clock Formulas as the start-up show and the look. It must:
  - say the network is off, start no WiFi driver, no httpd, no setup network, no mDNS or
    SNTP, and never put "Join WiFi" on the panel;
  - hand the card over first, then prepare the card's Formulas in the background (each
    the right reason, the broken one marked), report the counts, the heap and the compile
    task's stack; the playlist plays a Formula once it is ready, and the look is built again
    once its file is written;
  - play GIFs and the Formula it compiled, skip the clock Formula with the reason, refuse
    the clock look and start-up show (the wordmark plays);
  - run on past the setup network's old reboot timer (`AP_TIMEOUT`, 600 s) with no restart.

Boot A's card is written back and its compiled files compared byte for byte with the
host's `psi build` of the same sources (`--host-psb`): the compiler's optimize(size) on
the ESP32 must not change what it writes. Every Formula nested to the cap must leave
8 KB of the compile task's stack.

Boot C: no card, NVS off: no WiFi and no httpd either.

Boot B: the image as released (NVS empty: WiFi on by default), a card whose /opendmd.json
is the installer's one-line file (`{"network":"off"}`): the card turns it off. Then the
browser installer's Connect to Wi-Fi, an Improv WiFi-settings frame on UART0 (esp32sim's
`--script`), for the virtual access point: the radio starts for it, the display joins,
saves, answers and restarts onto the network (`network_on`), and comes back online (the
httpd, the station's address) because the card's file says `on` now. Before that, a
wrong password: refused, nothing saved, still offline; then the radio stopped after its
idle time, and started again for the right one (a second driver init in the same boot).
"""
import re
import time
from pathlib import Path
import shutil
import subprocess

import api_lane
from api_lane import Emulator, LaneFail

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / "offline"
SSID = "demo"
# Seconds of emulated time boot A runs for: past the setup network's 600 s reboot timer.
SECONDS_A = 700
GIF_LINE = re.compile(r"\] /gifs/\S+\.gif <- (prepared|original)")
# Never in an offline boot (before any Connect to Wi-Fi).
FORBIDDEN = [
    (re.compile(r"httpd started on port"), "the httpd"),
    (re.compile(r"\bwifi:|WiFi IRAM OP|esp_wifi_init"), "the WiFi driver"),
    (re.compile(r"\[net\] worker started"), "the online network worker"),
    (re.compile(r"starting AP mode|OpenDMD-Setup"), "the setup network"),
    (re.compile(r"Join WiFi"), "Join WiFi on the panel"),
    (re.compile(r"mDNS responding|sntp", re.I), "mDNS or SNTP"),
    (re.compile(r"www: pack '.*' verified"), "the web app mapped"),
]
NETWORK_OFF = re.compile(r'network off \("network": "off" in the settings\)')
WORKER_OFF = re.compile(r"\[net\] network off: no WiFi this boot")
PREPARED = re.compile(r"formulas: /formulas/(\S+)\.psi prepared for the panel \(([^)]*)\): (\d+) B in (\d+) ms; heap (\d+) B free before, (\d+) B at the lowest; compile stack (\d+) B left")
SUMMARY = re.compile(r"formulas: (\d+) on the card: (\d+) ready, (\d+) prepared now, (\d+) do not compile, (\d+) short of memory")
HEARTBEAT = re.compile(r"heartbeat: heap (\d+) B free \(floor (\d+) B.*wifi off")


def frame_wifi(ssid: str, psk: str) -> bytes:
    """An Improv RPC frame: WiFi settings (command 1)."""
    payload = bytes([len(ssid)]) + ssid.encode() + bytes([len(psk)]) + psk.encode()
    data = bytes([1, len(payload)]) + payload
    f = b"IMPROV" + bytes([1, 3, len(data)]) + data
    return f + bytes([sum(f) & 0xFF])


def scriptable(f: bytes) -> bool:
    """A frame esp32sim's script can carry as a line of text: ASCII, no line ends or NUL,
    and not ending in whitespace (the script trims a line)."""
    return all(b < 0x80 and b not in (0, 0x0A, 0x0D) for b in f) and 0x21 <= f[-1] <= 0x7E


def psk_and_frame(prefix="offline-pw") -> tuple[str, bytes]:
    """A password whose frame the script can carry."""
    import itertools, string
    for a, b in itertools.product(string.ascii_letters, repeat=2):
        psk = f"{prefix}{a}{b}"
        f = frame_wifi(SSID, psk)
        if scriptable(f):
            return psk, f
    raise LaneFail("no password makes a scriptable Improv frame")


# Formulas nested to the compiler's cap (tools/emu/offline/): each compile must leave
# this much of its task's stack untouched.
DEEP = ["deep", "deepcall", "fncall", "drawif", "tern", "ternelse", "spriteread"]
STACK_LEFT_MIN = 8 * 1024
# Generated Formulas on boot A's card besides the fixtures.
FILLERS = 100
# Compiled at mount, then compared byte for byte with the host's `psi build` (the
# compiler's optimize(size) on the ESP32 must not change what it writes).
SAME_AS_HOST = ["ramp", "stale", "deepcall", "fncall", "drawif", "spriteread"]
BOOT_DONE = re.compile(r"boot task done in \d+ ms \(stack (\d+) B unused\)|boot task done \(stack (\d+) B unused\)")


def mtools(card: Path, *args):
    subprocess.run(["mcopy", "-o", "-i", str(card), *args], check=True, stdout=subprocess.DEVNULL,
                   env={"MTOOLS_SKIP_CHECK": "1", "PATH": "/usr/bin:/bin"})


def make_card(base: Path, out: Path, files: dict[str, bytes]) -> Path:
    """A copy of `base` with `files` (card path -> bytes) written into it."""
    if not shutil.which("mcopy"):
        raise LaneFail("no mcopy (mtools)")
    out.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(base, out)
    tmp = out.parent / "offline-files"
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir()
    for path, data in files.items():
        local = tmp / path.lstrip("/").replace("/", "__")
        local.write_bytes(data)
        mtools(out, str(local), "::" + path)
    shutil.rmtree(tmp, ignore_errors=True)
    return out


class Lane:
    def __init__(self, a, sim, rom, fail_rx, ansi, boot_sha):
        self.a, self.sim, self.rom, self.fail_rx = a, sim, rom, fail_rx
        self.boot_sha = boot_sha
        api_lane.ANSI = ansi
        self.dir = Path(a.emu_dir) / "offline"
        self.dir.mkdir(parents=True, exist_ok=True)
        self.logs = Path(a.emu_dir) / "logs"
        self.logs.mkdir(parents=True, exist_ok=True)
        self.stamp = time.strftime("%Y%m%d-%H%M%S")
        self.passes = []

    def ok(self, name, detail):
        line = f"pass  {name}: {detail}"
        self.passes.append(line)
        print(line, flush=True)

    def emu(self, label, image: Path, card: Path, seconds: int, extra=(), card_out: Path | None = None):
        env = {"PATH": "/usr/bin:/bin", "HOME": str(Path.home()), "ESP32SIM_SD_IMG": str(card)}
        if card_out:
            env["ESP32SIM_SD_OUT"] = str(card_out)
        cmd = [str(self.sim), "--chip", "esp32", "--board", "esp32dev-sdspi", "--boot", "rom", "--rom", str(self.rom),
               "--flash-image", str(image), "--flash-mb", str(max(4, image.stat().st_size >> 20)),
               "--max-seconds", str(seconds), "--console", "uart0", "--no-dump", *extra]
        e = Emulator(cmd, env, self.logs / f"offline-{self.stamp}-{label}.log", self.fail_rx, label)
        e.start()
        return e

    def booted_ours(self, e: Emulator):
        i, m = e.wait_for(self.boot_sha, 120, what="the ELF SHA256 line")
        if not self.a.want_sha.startswith(m[1]):
            raise LaneFail(f"boot {e.label}: the booted app ({m[1]}) is not {self.a.image}")

    # -- boot A: NVS says off ------------------------------------------------------

    def run_a(self):
        stale = b"PSB\x02" + bytes(40)  # another version's compiled file
        files = {f"/formulas/{p.name}": p.read_bytes() for p in sorted(FIXTURES.glob("*.psi"))}
        files["/formulas/stale.psb"] = stale
        # A compiled file cut short (a power cut mid-write) with a good
        # header and its source's date: current by the check, refused where
        # it plays, so prepared again at the next start.
        files["/formulas/cut.psi"] = (FIXTURES / "ramp.psi").read_bytes()
        files["/formulas/cut.psb"] = (Path(self.a.host_psb) / "ramp.psb").read_bytes()[:20]
        # A card with many Formulas (a test board had 79), so the pass is
        # still going when the playlist starts ...
        for k in range(FILLERS):
            files[f"/formulas/fill{k:03d}.psi"] = f'name "Fill {k}"\nrgb(frac(x + t / {k + 2}), {k % 7} / 7, y)\n'.encode()
        # ... and one copied last, so compiled last, and first in the
        # playlist with no start-up show: skipped until it is ready, then
        # played.
        files["/formulas/zlate.psi"] = (FIXTURES / "ramp.psi").read_bytes()
        files["/filelist.txt"] = (
            "/formulas/zlate.psi\n/formulas/cut.psi\n/gifs/Spectrum_Slide/SPECTRUM_SLIDE_01.gif\n/formulas/ramp.psi\n/gifs/Pixel_Art/PIXEL_ART_01.gif\n"
            "/formulas/tick.psi\n/formulas/stale.psi\n/gifs/Spectrum_Slide/SPECTRUM_SLIDE_02.gif\n"
            "/formulas/broken.psi\n/formulas/deep.psi\n/gifs/Pixel_Art/PIXEL_ART_02.gif\n"
        ).encode()
        # A patch with no `network` key: NVS's off stands.
        files["/opendmd.json"] = (
            b'{"dwell_min":4,"dwell_max":6,"shuffle":false,"boot_show":"off",'
            b'"boot_formula":"/formulas/tick.psi","look_formula":"/formulas/ticklook.psi"}\n'
        )
        card = make_card(self.a.card, self.dir / "card-a.img", files)
        out = self.dir / "card-a-out.img"
        out.unlink(missing_ok=True)
        image = api_lane.seeded_image(self.a.image, self.dir, ssid=None, extra={"network": "off"})
        e = self.emu("A", image, card, SECONDS_A, ("--net", "none"), card_out=out)
        try:
            self.booted_ours(e)
            e.wait_for(NETWORK_OFF, 300, what="the network-off line")
            self.ok("network off", "NVS's network=off read at boot")
            e.wait_for(WORKER_OFF, 300, what="the offline worker")
            _, m = e.wait_for(SUMMARY, 300, what="the card Formulas' summary")
            self.ok("card formulas", m[0])
            want = {"ramp": "not prepared", "stale": "prepared for another version", "tick": "not prepared",
                    "ticklook": "not prepared", "zlate": "not prepared", **{d: "not prepared" for d in DEEP}}
            got = {m[1]: m for _, m in e.find(PREPARED)}
            for stem, why in want.items():
                if stem not in got:
                    raise LaneFail(f"/formulas/{stem}.psi was not prepared at mount")
                if got[stem][2] != why:
                    raise LaneFail(f"/formulas/{stem}.psi prepared for '{got[stem][2]}', not '{why}'")
            lows = sorted((int(m[6]), int(m[7]), s) for s, m in got.items())
            for s in DEEP:
                m = got[s]
                if int(m[7]) < STACK_LEFT_MIN:
                    raise LaneFail(f"/formulas/{s}.psi left only {m[7]} B of the compile task's stack (want {STACK_LEFT_MIN})")
                self.ok(f"{s} compiled", f"{m[3]} B in {m[4]} ms, heap {m[5]} B free before, {m[6]} B at the lowest, compile stack {m[7]} B left")
            self.ok("heap", f"lowest during a compile {lows[0][0]} B ({lows[0][2]}); least stack left {min(int(m[7]) for m in got.values())} B")
            if not e.find(re.compile(r"formulas: /formulas/broken\.psi does not compile")):
                raise LaneFail("broken.psi did not fail to compile")
            if int(m_sum(e)[4]) < 1:
                raise LaneFail("the summary counts no Formula that does not compile")
            self.ok("broken", "does not compile: marked, skipped where it plays")
            # The card is handed over before its Formulas are prepared.
            handed = e.find(re.compile(r"boot: card handed over (\d+) ms after the boot task started"))
            first_prep = min(i for i, _ in e.find(PREPARED))
            if not handed or handed[0][0] > first_prep:
                raise LaneFail("a card Formula was compiled before the card was handed over")
            self.ok("in the background", f"handed over in {handed[0][1][1]} ms, the compiles after it")
            # The look is built again once its file is written, and
            # refused for the clock.
            _, m = e.wait_for(re.compile(r"formula look: the saved look does not build, none drawn \(this Formula shows the time.*"), 300,
                              what="the clock look rebuilt once prepared, and refused")
            self.ok("look", "rebuilt once its file was written; the clock look refused, none drawn")
            # The playlist's first line, not ready at first: skipped, then
            # played once prepared.
            s_i, _ = e.wait_for(re.compile(r"skipping /formulas/zlate\.psi: (.*)"), 300, what="the not-yet-prepared Formula skipped")
            p_i, _ = e.wait_for(re.compile(r"formulas: /formulas/zlate\.psi prepared for the panel"), 300, what="zlate prepared")
            z_i, _ = e.wait_for(re.compile(r"\] /formulas/zlate\.psi <- "), 400, start=p_i, what="zlate played once ready")
            if not s_i < p_i < z_i:
                raise LaneFail("zlate.psi was not skipped, then prepared, then played")
            self.ok("played once ready", "/formulas/zlate.psi: skipped while not prepared, prepared in the background, then played")
            # The cut .psb: refused where it plays (the engine logs a pass's
            # first failure only) and noted on the card; prepared again by a
            # retry this start or at the next one (decided from the whole
            # log at the end of this boot).
            e.wait_for(re.compile(r"formulas: a compiled file the panel refused is prepared again"), 400,
                       what="the refused cut.psb noted")
            self.ok("refused, noted", "cut.psb refused where it played, noted on the card")
            j, _ = e.wait_for(re.compile(r"\] /formulas/ramp\.psi <- "), 400, what="the compiled card Formula playing")
            skipped = [i for i, _ in e.find(re.compile(r"skipping /formulas/ramp\.psi"))]
            self.ok("compiled formula plays", f"/formulas/ramp.psi, compiled after the playlist started, played"
                    + (f" (skipped {len(skipped)} time(s) before it was ready)" if skipped else ""))
            e.wait_for(re.compile(r"\] /formulas/stale\.psi <- "), 400, what="the re-prepared Formula playing")
            self.ok("re-prepared formula plays", "/formulas/stale.psi")
            _, m = e.wait_for(re.compile(r"skipping /formulas/tick\.psi: (.*)"), 400, what="the clock Formula skipped")
            if "shows the time" not in m[1]:
                raise LaneFail(f"tick.psi skipped for another reason: {m[0]}")
            self.ok("clock formula skipped", m[0][:160])
            n = 0
            while n < 3:
                i, _ = e.wait_for(GIF_LINE, 300, start=(i + 1) if n else 0, what="GIF starts")
                n += 1
            self.ok("GIFs play", f"{n}+ GIF starts")
            # On past the old setup network's reboot timer, to the end of the run.
            if not e.wait_exit(SECONDS_A * 4):
                raise LaneFail(f"boot A did not end within {SECONDS_A * 4} s wall")
            e.check()
            beats = e.find(HEARTBEAT)
            if not beats:
                raise LaneFail("no heartbeat says wifi off")
            last = beats[-1][1]
            t_last = last_uptime(e)
            if t_last < 650:
                raise LaneFail(f"boot A ended at {t_last} s of uptime, before the setup network's old 600 s timer had passed")
            if e.resets != 1:
                raise LaneFail(f"{e.resets} resets in boot A (only the power-on expected)")
            self.ok("no restart", f"one boot to {t_last} s of uptime (the setup network rebooted at 600 s); last heartbeat: {last[1]} B free, floor {last[2]} B")
            for rx, what in FORBIDDEN:
                hit = e.find(rx)
                if hit:
                    raise LaneFail(f"boot A: {what}: {hit[0][1].string[:160]}")
            self.ok("quiet", "no WiFi driver, httpd, setup network, mDNS/SNTP, web app or Join WiFi")
            # Healed this start (a retry) or left for the next: the whole log.
            healed = e.find(re.compile(r"formulas: /formulas/cut\.psi prepared for the panel \(refused where it played\)"))
            self.cut_healed = bool(healed)
            if healed and not e.find(re.compile(r"\] /formulas/cut\.psi <- "), healed[0][0]):
                raise LaneFail("cut.psi was prepared again but never played after")
            self.ok("refused, then", "prepared again by a retry this start, then played" if healed else "left for the next start")
            # The emu-census image takes one Formula as short at first: a
            # retry compiles it on the network worker, whose stack is read.
            if e.find(re.compile(r"census: \d+ blocks in use")):
                # `heavy.psi` is short even after its compile, every time
                # (the image's hook): the retries skip it, and `zlate.psi`,
                # behind it, short at the first pass, is still prepared.
                if not e.find(re.compile(r"formulas: /formulas/heavy\.psi taken as short of memory after its compile")):
                    raise LaneFail("heavy.psi was not taken as short after its compile")
                if e.find(re.compile(r"formulas: /formulas/heavy\.psi prepared for the panel")):
                    raise LaneFail("heavy.psi was prepared (the hook should keep it short)")
                zs = e.find(re.compile(r"formulas: /formulas/zlate\.psi taken as short of memory \(a test image\), for a retry"))
                zp = e.find(re.compile(r"formulas: /formulas/zlate\.psi prepared for the panel"))
                if not zs or not zp or zp[0][0] < zs[0][0]:
                    raise LaneFail("zlate.psi (behind the always-short heavy.psi) was not prepared by a retry")
                self.ok("past a heavy one", "heavy.psi short after its compile, skipped by the retries; zlate.psi behind it prepared by a retry")
                r = e.find(re.compile(r"formulas: a retry compiled on the network worker \((\d+) B of its stack unused at its deepest\)"))
                if not r:
                    raise LaneFail("no retry compiled on the network worker (an emu-census image takes one Formula as short at first)")
                unused = min(int(m[1]) for _, m in r)
                if unused < 1024:
                    raise LaneFail(f"the network worker's stack had only {unused} B unused after a retry compile")
                self.ok("retry on the worker", f"a retry compiled on the network worker: {unused} B of its stack unused at its deepest")
            done = e.find(BOOT_DONE)
            if done:
                self.ok("boot task stack", done[0][1][0][:160])
            self.ok("gifs", f"{len(e.find(GIF_LINE))} GIF starts")
        finally:
            e.kill()
        self.same_as_host(out)
        # The next start from that card: everything current, nothing compiled again.
        again = self.dir / "card-a2.img"
        shutil.copyfile(out, again)
        tmp = self.dir / "a2.json"
        tmp.write_bytes(b'{"boot_show":"short","boot_formula":"/formulas/tick.psi"}\n')
        mtools(again, str(tmp), "::/opendmd.json")
        e = self.emu("A2", image, again, 30, ("--net", "none"))
        try:
            self.booted_ours(e)
            _, m = e.wait_for(SUMMARY, 300, what="the second start's summary")
            want = 0 if self.cut_healed else 1
            if int(m[3]) != want or int(m[2]) + int(m[3]) + int(m[4]) + int(m[5]) != int(m[1]):
                raise LaneFail(f"the second start should compile {want} (the refused one, if not healed already): {m[0]}")
            if want and not e.find(re.compile(r"formulas: /formulas/cut\.psi prepared for the panel \(refused where it played\)")):
                raise LaneFail("cut.psi was not prepared again at the next start")
            self.ok("next start", m[0] + " (the marks and times kept)")
            _, m = e.wait_for(re.compile(r"boot show: formula /formulas/tick\.psi does not build \((.*)\): the built-in show"), 120,
                              what="the clock start-up show refused")
            if "shows the time" not in m[1]:
                raise LaneFail(f"the start-up show was refused for another reason: {m[0]}")
            self.ok("start-up show, prepared", "the clock Formula refused, the wordmark")
        finally:
            e.kill()

    def same_as_host(self, out: Path):
        """The card's compiled files, written by the display, against the host's."""
        if not out.exists():
            raise LaneFail(f"boot A wrote no card image back ({out})")
        host = Path(self.a.host_psb)
        got_dir = self.dir / "from-card"
        shutil.rmtree(got_dir, ignore_errors=True)
        got_dir.mkdir()
        for stem in SAME_AS_HOST + ["broken"]:
            subprocess.run(["mcopy", "-n", "-i", str(out), f"::/formulas/{stem}.psb", str(got_dir / f"{stem}.psb")],
                           check=True, env={"MTOOLS_SKIP_CHECK": "1", "PATH": "/usr/bin:/bin"})
        for stem in SAME_AS_HOST:
            a, b = (got_dir / f"{stem}.psb").read_bytes(), (host / f"{stem}.psb").read_bytes()
            if a != b:
                raise LaneFail(f"/formulas/{stem}.psb compiled on the display ({len(a)} B) differs from the host's psi build ({len(b)} B)")
        mark = (got_dir / "broken.psb").read_bytes()
        if len(mark) != 12 or mark[:4] != b"PSB\0":
            raise LaneFail(f"broken.psb is not the mark ({mark[:16]!r})")
        self.ok("same as the host", f"{', '.join(SAME_AS_HOST)}: byte for byte the host's psi build; broken.psb the mark")

    # -- boot C: no card, NVS says off -----------------------------------------------

    def run_c(self):
        empty = self.dir / "empty.img"
        empty.write_bytes(b"")
        image = api_lane.seeded_image(self.a.image, self.dir, ssid=None, extra={"network": "off"})
        e = self.emu("C", image, empty, 40, ("--net", "none"))
        try:
            self.booted_ours(e)
            e.wait_for(NETWORK_OFF, 300, what="the network-off line with no card")
            e.wait_for(WORKER_OFF, 300, what="the offline worker with no card")
            e.wait_for(re.compile(r"microSD: no card yet; retrying every"), 300, what="the no-card line")
            if not e.wait_exit(600):
                raise LaneFail("boot C did not end")
            e.check()
            for rx, what in FORBIDDEN:
                hit = e.find(rx)
                if hit:
                    raise LaneFail(f"boot C: {what}: {hit[0][1].string[:160]}")
            self.ok("no card, NVS off", "network off from NVS, no WiFi, no httpd, no web app")
        finally:
            e.kill()

    # -- boot B: the card says off; Connect to Wi-Fi over USB --------------------------

    def run_b(self):
        psk, frame = psk_and_frame()
        _, wrong = psk_and_frame("not-the-pw")
        card = make_card(self.a.card, self.dir / "card-b.img", {"/opendmd.json": b'{"network":"off"}\n'})
        script = self.dir / "improv.txt"
        # (Text lines on UART0; the frame after a line end, as the installer's are.)
        script.write_bytes(
            b"0 waituart0 600 [net] network off: no WiFi this boot\n"
            b"0 waituart0 600 <- \n"
            b"0 uart0 hello\n"
            b"0 uart0 " + wrong + b"\n"
            b"0 waituart0 600 [net] network off: radio stopped\n"
            b"0 uart0 hello\n"
            b"0 uart0 " + frame + b"\n"
        )
        wifi = f"ssid={SSID},chan=6,psk={psk}"
        e = self.emu("B", self.a.image, card, 600, ("--wifi", wifi, "--net", "nat", "--script", str(script)))
        e.resets_allowed = 2
        try:
            self.booted_ours(e)
            i, _ = e.wait_for(NETWORK_OFF, 300, what="the card's network=off")
            booted = i
            self.ok("card off", "the card's one-line opendmd.json turns WiFi off")
            i, m = e.wait_for(re.compile(rf"\[net\] improv: WiFi details for '{SSID}' received over USB \(the network is off\)"), 600, start=i,
                              what="the WiFi details over USB")
            i, _ = e.wait_for(re.compile(r"\[net\] radio up for the installer"), 120, start=i, what="the radio started (first)")
            i, _ = e.wait_for(re.compile(rf"\[net\] improv: could not join '{SSID}'; the network stays off"), 300, start=i,
                              what="the wrong password refused")
            self.ok("failed join", "the wrong password: unable to connect, nothing saved, still offline")
            i, m = e.wait_for(re.compile(r"\[net\] network off: radio stopped \(no installer job for (\d+) s\); (\d+) B free"), 600, start=i,
                              what="the idle radio stopped")
            self.ok("radio stopped", f"after {m[1]} s idle, {m[2]} B free")
            if any(re.search(r"^rst:0x|httpd started", l) for l in e.lines[booted:i]):
                raise LaneFail("boot B restarted or went online after the failed join")
            i, m = e.wait_for(re.compile(rf"\[net\] improv: WiFi details for '{SSID}' received over USB \(the network is off\)"), 600, start=i + 1,
                              what="the second WiFi details over USB")
            j, m = e.wait_for(re.compile(r"\[net\] radio up for the installer: (\d+) B free \(took (\d+) B\)"), 120, start=i, what="the radio started")
            self.ok("radio on demand again", f"a second start after the stop: {m[1]} B free with the radio up (it took {m[2]} B)")
            j, m = e.wait_for(re.compile(rf"\[net\] improv: joined '{SSID}' at http://(\S+)/; WiFi on, restarting"), 120, start=j, what="the join")
            self.ok("joined", m[0][:120])
            before = e.lines[:j]
            for rx, what in FORBIDDEN[:1]:
                if any(rx.search(l) for l in before):
                    raise LaneFail(f"boot B: {what} before the join")
            j, _ = e.wait_for(re.compile(r"restarting \(panel off\): network_on"), 60, start=j, what="the restart")
            k, _ = e.wait_for(re.compile(r"^rst:0x"), 60, start=j, what="the reset")
            k, _ = e.wait_for(re.compile(r"httpd started on port"), 300, start=k, what="the httpd after the restart")
            k, m = e.wait_for(re.compile(r"\[net\] ready: http://(\S+)/"), 300, start=k, what="the station's address")
            if any(NETWORK_OFF.search(l) for l in e.lines[j:k]):
                raise LaneFail("the second boot read network off again (the card's file was not updated)")
            self.ok("online after the restart", f"httpd up, {m[0][:80]}; the card's file says on")
            e.check()
        finally:
            e.kill()


def m_sum(e: Emulator):
    return e.find(SUMMARY)[-1][1]


UPTIME = re.compile(r"^I \((\d+)\)")


def last_uptime(e: Emulator) -> int:
    """The last log line's uptime stamp (s)."""
    for l in reversed(e.lines):
        m = UPTIME.match(l)
        if m:
            return int(m[1]) // 1000
    return 0


def run(a, sim, rom, fail_rx, ansi, boot_sha) -> int:
    lane = Lane(a, sim, rom, fail_rx, ansi, boot_sha)
    t0 = time.time()
    try:
        # EMU_OFFLINE_ONLY=a|b|c: one boot alone (a diagnostic shortcut).
        only = __import__("os").environ.get("EMU_OFFLINE_ONLY", "")
        if only in ("", "a"):
            lane.run_a()
        if only in ("", "c"):
            lane.run_c()
        if only in ("", "b"):
            lane.run_b()
    except LaneFail as e:
        print(f"EMU_OFFLINE_FAIL: {e}")
        return 1
    print(f"EMU_OFFLINE_OK ({len(lane.passes)} checks, {time.time() - t0:.0f} s wall)")
    return 0
