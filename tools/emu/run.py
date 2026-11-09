#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# ///
"""Boot a full flash image in esp32sim with the SD-SPI card, capture the console, judge it.

    uv run tools/emu/run.py smoke|probe|nocard|rig|leak|heapmap --image FULL.bin [--card SD.img] [--log LOG] [--seconds N] [--wall N]
        [--built-after EPOCH]
    uv run tools/emu/run.py api --image FULL.bin --www-pack www.pack --bundle B.odfw --psb F.b64 [--formula F.psi] [--card SD.img]

The image must be the one that boots: its app descriptor's ELF SHA-256 is checked against
the boot log's `ELF file SHA256:` line, and with --built-after (the Makefile passes the time it
started the build) an image file older than that is refused before anything runs (a failed
build must never leave an earlier image to be booted in its place).

smoke: pass when the microSD mount, panel up, httpd, www pack verified and a GIF start
       from the playlist are all seen; prints EMU_SMOKE_OK.
probe: wait for `psi-probe: done`, then fail on any `N differ` with N > 0, raw card reads that
       do not match FatFs's (or none), or a missing/
       mismatching golden line, or a bundled Formula that does not open / a short heap
       that does not refuse Endless road, or a Formula by reference (a bundled one by
       name, the card's /formulas/ci-test.psi) that does not play, or a formula sent over
       Time gate with the heap short that is not played once Time gate goes (or, shorter
       still, not refused with the playlist going on), or a card Formula sent over a card
       Formula (Neon city, Wormhole run) that is not played; prints EMU_PROBE_OK
       and the correctness lines. A paint-probe image (`make emu-image-paint`) needs only its own
       `paint: N cases, 0 differ`.
rig:   (with the full-scale card, `mkcard.py --rig`) wait for the FAT index's build to finish
       (`fat index: built N records ...`, its heap floor and stack), with the panel playing: N > 0,
       stale false, no transition cut during it, no `fat index:` warning; prints EMU_RIG_OK, the
       build line and the lowest heartbeat heap floor. With an `emu-ballast` image (make
       emu-rig-tight) the heap is a test board's (~45 KB free while playing).
leak:  (an `emu-census` image, make emu-leak) the heap's census every 20 s: after 10 GIFs to warm
       up and 60 more (8 samples at least), the least blocks in use and bytes allocated of the
       second half of the samples against the first half's must not have grown past 24 blocks /
       2 KB (while they have, it plays on, to 140 after the warm-up at most); prints EMU_LEAK_OK.
heapmap: (a `heapmap` image, src/heapmap.rs) run the whole --seconds (default 300) playing, then
       pass if the heap map's first report ended and GIFs started; prints EMU_HEAPMAP_OK and the
       `heapmap:` lines (tools/heapmap-report.py makes the tables from the log).
api:   (an `emu-census` image, make emu-api) WiFi credentials seeded into a copy of the image's NVS,
       the virtual AP, the guest's port 80 on 127.0.0.1, all inside a private network namespace
       (`unshare -rn`: nothing reaches a real network); the HTTP API driven from the host, two boots
       (tools/emu/api_lane.py says what each covers); prints EMU_API_OK.
Each fails at once on a panic, Guru Meditation, stack overflow, abort() or a second reset.
Correctness only: emulated cycles are not trustworthy, never read timings from this.
"""
import argparse, os, re, shutil, signal, subprocess, sys, time
from pathlib import Path

EMU_DIR = Path(os.environ.get("EMU_DIR") or (Path(os.environ["SCRATCH"]) / "opendmd-emu-sim" if os.environ.get("SCRATCH") else Path(__file__).resolve().parents[2] / "target" / "emu-sim"))
ANSI = re.compile(r"\x1b\[[0-9;]*m")
BOOT_SHA = re.compile(r"ELF file SHA256:\s+([0-9a-f]+)")
APP_DESC_MAGIC = (0xABCD5432).to_bytes(4, "little")


def image_elf_sha(image: Path) -> str | None:
    """The app descriptor's ELF SHA-256 (hex) of the first app in a full flash image: the
    descriptor follows the app image's 24 B header and first 8 B segment header, its hash at +0x90."""
    data = image.read_bytes()
    for off in range(0x10000, len(data) - 0x100, 0x10000):
        d = off + 0x20
        if data[off] == 0xE9 and data[d:d + 4] == APP_DESC_MAGIC:
            return data[d + 0x90:d + 0xB0].hex()
    return None

FAIL = [
    (re.compile(r"Guru Meditation"), "Guru Meditation"),
    (re.compile(r"panicked at|\bpanic\b.*\bat\b|abort\(\) was called|\bassert failed\b", re.I), "panic/abort"),
    (re.compile(r"stack overflow", re.I), "stack overflow"),
    (re.compile(r"HTTP server failed to start"), "HTTP server failed to start"),
    (re.compile(r"^Backtrace:"), "backtrace"),
    (re.compile(r"CORRUPT HEAP|heap_caps_.*(alloc|free) failed.*abort", re.I), "heap corruption"),
]
SMOKE = [
    ("microSD mounted", re.compile(r"microSD mounted at /sdcard")),
    ("panel up", re.compile(r"opendmd_firmware: panel up: 128x32")),
    ("httpd started", re.compile(r"httpd started on port")),
    ("www pack verified", re.compile(r"www: pack '.*' verified")),
    ("GIFs from the playlist", re.compile(r"\] /gifs/\S+\.gif <- (prepared|original)")),
    # The boot task's playlist (opened before the card was the player's,
    # through FatFs) opened again raw once it is.
    ("playlist read raw", re.compile(r"playlist: opened again after the handover in \d+ ms, read raw")),
]
# No card: the firmware boots, its panel runs, the settings are kept in NVS,
# and a change made then survives a restart (src/nocard_test.rs).
NOCARD = [
    ("panel up", re.compile(r"opendmd_firmware: panel up: 128x32")),
    ("settings in NVS", re.compile(r'nocard-test: store,"settings_store":"nvs \(no card\)"')),
    ("change applied", re.compile(r"nocard-test: applied brightness 37: ok")),
    ("kept after the restart", re.compile(r"nocard-test: after the restart: brightness 37, kept")),
    # NVS names a card Formula as the start-up show (seeded): with no card, the wordmark.
    ("start-up formula: the wordmark", re.compile(r"boot show: formula /formulas/x\.psi does not build \((the card was not ready in time|no card)\): the built-in show")),
]
CENSUS = re.compile(r"census: (\d+) blocks in use, (\d+) B allocated")
# 60 after the warm-up (was 40): a sample's level depends on where in a GIF it
# falls (~194, 199, 209 or 219 KB allocated in the census image), and with 40
# GIFs a half could miss the lowest phase by chance alone (heap-trim 84b2527:
# 193,880 B in the first half, the second's least 199,312 B, both levels seen
# in either half of the passing run before it): ~12 samples, 6 a half.
LEAK_WARM, LEAK_GIFS, LEAK_BLOCKS, LEAK_BYTES = 10, 60, 24, 2048
# The emulator is deterministic, so a half that misses the lowest phase misses
# it on every run of that image (net-wedge e973481: 194,224 B once in the first
# half, the second's least 199,428 B, a level the first half had three times).
# So after the 60, while the halves do not compare flat, it plays on, to 140 at
# most: a leak lifts every level for good and fails there; a flat heap meets
# its lowest phase again.
LEAK_GIFS_MAX = 140


def leak_halves(census):
    """The leak verdict's figures: the least blocks and bytes of the first and second half of
    the census samples after the warm-up, the GIFs they span, and whether that is flat."""
    after = [c for c in census if c[0] >= LEAK_WARM]
    half = len(after) // 2
    warm, late = after[:half], after[half:]
    b0, b1 = min(c[1] for c in warm), min(c[1] for c in late)
    y0, y1 = min(c[2] for c in warm), min(c[2] for c in late)
    flat = b1 - b0 <= LEAK_BLOCKS and y1 - y0 <= LEAK_BYTES
    return b0, b1, y0, y1, after[-1][0] - after[0][0], flat
HEARTBEAT_FLOOR = re.compile(r"heartbeat: heap \d+ B free \(floor (\d+) B")
RIG_BUILT = re.compile(r"fat index: built (\d+) records .*transitions cut during it (\d+);.* stale (true|false)")
RIG_FAILED = re.compile(r"^W \(\d+\) \S+: fat index: (?!.*no raw reads)")
DIFFER = re.compile(r"(\d+)(?: of \d+ \w+)? differ")
GOLDEN = re.compile(r"psi-probe: golden frames: (\d+) match the host/wasm hashes, (\d+) differ")
BUNDLED = re.compile(r"psi-probe: bundled formulas: (\d+) opened, (\d+) failed; short heap: (.*) far short, (.*) just short")
REFS = re.compile(r"psi-probe: formulas by reference: by name (\w+), card file (\w+)")
SWAP = re.compile(r"psi-probe: formula over formula: short beside Time gate: (.*); short without it: (.*); heap")
SETTINGS = re.compile(r"psi-probe: settings file: a save cut between its renames (loads the new settings|DID NOT load the new settings)")
NAMES = re.compile(r"psi-probe: names file: (\d+) folders match their listings, (\d+) differ")
READAHEAD = re.compile(r"psi-probe: read-ahead: (\d+) B read a block a frame, (the same as FatFs's|DIFFERS from FatFs's); (\d+) served from the ring|psi-probe: read-ahead: NOT CHECKED")
RAW = re.compile(r"psi-probe: raw reads: (\d+) files match FatFs's reads, (\d+) differ, (\d+) of them through the index")
PAINT = re.compile(r"psi-probe: paint: (\d+) cases, (\d+) differ")
SWAP_CARD = re.compile(r"psi-probe: card formula over card formula: (all played|NOT all played.*); heap")
PROBE_KEYS = re.compile(r"psi-probe: (paint: |settings file|raw reads|names file|read-ahead|bundled |short heap|formulas by reference|(card )?formula over (card )?formula|@formula:|/formulas/|jit: .*(differ|returned|the same)|speed2? frames|golden frames:|done)")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", choices=["smoke", "probe", "nocard", "rig", "leak", "heapmap", "api", "offline"])
    ap.add_argument("--image", type=Path, required=True)
    ap.add_argument("--card", type=Path, default=EMU_DIR / "card" / "sd.img")
    ap.add_argument("--log", type=Path)
    ap.add_argument("--seconds", type=int, help="emulated seconds (default smoke 60, probe 400)")
    ap.add_argument("--wall", type=int, help="wall-clock limit in s (default smoke 900, probe 3600)")
    ap.add_argument("--rom", type=Path)
    ap.add_argument("--min-gifs", type=int, default=3)
    ap.add_argument("--built-after", type=float, help="refuse an image file older than this (epoch s)")
    ap.add_argument("--www-pack", type=Path, help="api: the pack PUT to /api/www")
    ap.add_argument("--bundle", type=Path, help="api: the signed bundle PUT to /api/firmware")
    ap.add_argument("--old-updater", type=Path, help="api: an updater from before the panel and table v4 (a release's): boot C, where it refuses the bundle")
    ap.add_argument("--psb", type=Path, help="api: a compiled Formula, base64url (`psi build --base64`)")
    ap.add_argument("--long-psb", type=Path, help="api: a long Formula compiled (`psi build -o`), past any path: shown with POST /api/play/formula")
    ap.add_argument("--look", type=Path, help="api: a Formula that can be a look (its source), saved to the card and set as look_formula")
    ap.add_argument("--look-psb", type=Path, help="api: that look compiled (`psi build -o`)")
    ap.add_argument("--connections", type=int, choices=[1, 2, 5], default=5,
                    help="api: the load's connections at once: 5 (the default: display-stress's five loads each on its own, and the simultaneous-upload bursts), 2 (status polls beside the rest) or 1")
    ap.add_argument("--host-psb", type=Path, help="offline: a directory of the lane's Formulas compiled by the host's psi build")
    ap.add_argument("--formula", dest="formula_src", type=Path, help="api: that Formula's source (saved beside its .psb on the card)")
    a = ap.parse_args()

    sim = EMU_DIR / "src" / "target" / "release" / "esp32sim"
    rom = a.rom or next(iter(sorted(Path.home().glob(".espressif/tools/esp-rom-elfs/*/esp32_rev300_rom.elf"))[::-1]), None)
    for what, p in (("emulator (run make emu-build)", sim), ("flash image", a.image), ("card image (run mkcard.py)", a.card), ("esp32_rev300_rom.elf under ~/.espressif/tools/esp-rom-elfs/", rom)):
        if not p or not Path(p).exists():
            sys.exit(f"run: missing {what}: {p}")
    if a.mode == "nocard" and a.card == EMU_DIR / "card" / "sd.img":
        # No card: an empty image (a card the firmware cannot mount).
        a.card = EMU_DIR / "card" / "empty.img"
        a.card.parent.mkdir(parents=True, exist_ok=True)
        a.card.write_bytes(b"")
    if a.built_after is not None and a.image.stat().st_mtime < a.built_after:
        sys.exit(f"EMU_{a.mode.upper()}_FAIL: {a.image} is older than this build (a failed build left it): not booting it")
    want_sha = image_elf_sha(a.image)
    if not want_sha:
        sys.exit(f"EMU_{a.mode.upper()}_FAIL: no app descriptor found in {a.image}")
    if a.mode == "api":
        return api(a, sim, rom, want_sha)
    if a.mode == "offline":
        return offline(a, sim, rom, want_sha)
    if a.mode == "nocard":
        # The start-up show's Formula is on the card (`boot_fx` seeded, no WiFi).
        import api_lane
        a.image = api_lane.seeded_image(a.image, EMU_DIR / "nocard-seed", ssid=None, extra={"boot_fx": "/formulas/x.psi"})
    if a.mode == "rig" and a.card == EMU_DIR / "card" / "sd.img":
        a.card = EMU_DIR / "card" / "rig.img"
    seconds = a.seconds or {"smoke": 60, "nocard": 40, "rig": 1800, "leak": 1500, "heapmap": 300}.get(a.mode, 400)
    wall = a.wall or {"smoke": 900, "rig": 5400, "leak": 5400}.get(a.mode, 3600)
    log = a.log or EMU_DIR / "logs" / f"{a.mode}-{time.strftime('%Y%m%d-%H%M%S')}.log"
    log.parent.mkdir(parents=True, exist_ok=True)

    # the card is copy-on-write in RAM; the output is discarded (ESP32SIM_SD_OUT unset)
    env = {**os.environ, "ESP32SIM_SD_IMG": str(a.card)}
    env.pop("ESP32SIM_SD_OUT", None)
    cmd = [str(sim), "--chip", "esp32", "--board", "esp32dev-sdspi", "--boot", "rom", "--rom", str(rom),
           "--flash-image", str(a.image), "--flash-mb", str(max(4, a.image.stat().st_size >> 20)),
           "--max-seconds", str(seconds), "--console", "uart0", "--net", "none", "--no-dump"]
    print("emu:", " ".join(cmd), flush=True)
    print(f"emu: log {log}  (card {a.card}; {seconds} emulated s, {wall} s wall limit)", flush=True)

    t0 = time.time()
    seen, gifs, resets, failure, done_at = {}, 0, 0, None, None
    booted_sha = None
    golden = None
    bundled = None
    refs = None
    swap = None
    swap_card = None
    raw_reads = None
    readahead = None
    names_file = None
    settings_file = None
    paint = None
    differ_bad = []
    lines = []
    built = None
    floors = []
    census = []  # (GIF starts so far, blocks, bytes)
    p = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", start_new_session=True)
    with open(log, "w") as lf:
        import threading
        stop = threading.Event()
        def watchdog():
            if not stop.wait(wall):
                os.killpg(p.pid, signal.SIGKILL)
        threading.Thread(target=watchdog, daemon=True).start()
        for raw in p.stdout:
            lf.write(raw)
            line = ANSI.sub("", raw.rstrip("\n"))
            lines.append(line)
            m = BOOT_SHA.search(line)
            if m and booted_sha is None:
                booted_sha = m[1]
                if not want_sha.startswith(booted_sha):
                    failure = f"the booted app (ELF SHA256 {booted_sha}...) is not {a.image} ({want_sha[:16]}...)"
            if line.startswith("rst:0x"):
                resets += 1
                # (nocard: the test restarts once on purpose.)
                if resets > (2 if a.mode == "nocard" else 1):
                    failure = f"unexpected reset: {line}"
            for rx, name in FAIL:
                if rx.search(line):
                    failure = f"{name}: {line[:200]}"
            if failure:
                # The register dump and backtrace that follow, for the log.
                for _ in range(40):
                    more = p.stdout.readline()
                    if not more:
                        break
                    lf.write(more)
                    lines.append(ANSI.sub("", more.rstrip("\n")))
                break
            if a.mode == "heapmap":
                if SMOKE[4][1].search(line):
                    gifs += 1
                if re.search(r"heapmap: report \d+ end", line):
                    seen["report"] = line
                continue
            if a.mode == "leak":
                if SMOKE[4][1].search(line):
                    gifs += 1
                m = CENSUS.search(line)
                if m:
                    census.append((gifs, int(m[1]), int(m[2])))
                # 10 GIFs to warm up, then 60 more, with census lines after;
                # on to LEAK_GIFS_MAX while the halves do not compare flat.
                if m and len([c for c in census if c[0] >= LEAK_WARM + LEAK_GIFS]) >= 3 and len([c for c in census if c[0] >= LEAK_WARM]) >= 8:
                    if leak_halves(census)[5] or gifs >= LEAK_WARM + LEAK_GIFS_MAX:
                        break
            elif a.mode == "rig":
                m = HEARTBEAT_FLOOR.search(line)
                if m:
                    floors.append(int(m[1]))
                if SMOKE[4][1].search(line):
                    gifs += 1
                m = RIG_BUILT.search(line)
                if m:
                    built = (line, int(m[1]), int(m[2]), m[3])
                if RIG_FAILED.search(line) and not built:
                    failure = f"the build failed: {line[:200]}"
                    break
                if built and gifs >= a.min_gifs:
                    break
            elif a.mode == "nocard":
                for name, rx in NOCARD:
                    if name not in seen and rx.search(line):
                        seen[name] = line
                if len(seen) == len(NOCARD):
                    break
            elif a.mode == "smoke":
                for name, rx in SMOKE:
                    if name not in seen and rx.search(line) and name != "GIFs from the playlist":
                        seen[name] = line
                if SMOKE[4][1].search(line):
                    gifs += 1
                    seen.setdefault("GIFs from the playlist", line)
                if len(seen) == len(SMOKE) and gifs >= a.min_gifs:
                    break
            else:
                m = GOLDEN.search(line)
                if m:
                    golden = (line, int(m[1]), int(m[2]))
                m = BUNDLED.search(line)
                if m:
                    bundled = (line, int(m[1]), int(m[2]), m[3], m[4])
                m = REFS.search(line)
                if m:
                    refs = (line, m[1], m[2])
                m = SWAP.search(line)
                if m:
                    swap = (line, m[1], m[2])
                m = SWAP_CARD.search(line)
                if m:
                    swap_card = (line, m[1])
                m = SETTINGS.search(line)
                if m:
                    settings_file = (line, m[1])
                m = RAW.search(line)
                if m:
                    raw_reads = (line, int(m[1]), int(m[2]), int(m[3]))
                m = NAMES.search(line)
                if m:
                    names_file = (line, int(m[1]), int(m[2]))
                m = READAHEAD.search(line)
                if m:
                    readahead = (line, m[2], int(m[3]) if m[3] else 0)
                m = PAINT.search(line)
                if m:
                    paint = (line, int(m[1]), int(m[2]))
                if "psi-probe:" in line:
                    for d in DIFFER.finditer(line):
                        if int(d[1]) > 0:
                            differ_bad.append(line)
                if "psi-probe: done" in line:
                    done_at = line
                    break
        stop.set()
    try:
        os.killpg(p.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    p.wait()
    took = time.time() - t0
    print(f"emu: {len(lines)} console lines, {took:.0f} s wall; log {log}")

    def die(msg):
        print(f"EMU_{a.mode.upper()}_FAIL: {msg}")
        print(f"--- last console lines of {log}:")
        print("\n".join(l[:200] for l in lines[-15:]))
        sys.exit(1)

    if failure:
        die(failure)
    if booted_sha is None:
        die("no `ELF file SHA256:` line at boot: cannot tell which app ran")
    if a.mode == "heapmap":
        if "report" not in seen or gifs == 0:
            die(f"no heap map report or no GIF within {seconds} emulated s / {wall} s wall (GIF starts {gifs})")
        n = sum(1 for l in lines if "heapmap:" in l)
        print(f"pass  {n} heapmap lines, GIF starts {gifs}")
        print(f"EMU_HEAPMAP_OK ({took:.0f} s wall)")
        return
    if a.mode == "leak":
        # The census swings with what is held at the sample (a decoder's
        # pieces, the ring or not): the least of the first half of the
        # samples after the warm-up against the least of the second half.
        after = [c for c in census if c[0] >= LEAK_WARM]
        if len(after) < 6 or after[-1][0] < LEAK_WARM + LEAK_GIFS:
            die(f"not enough census lines ({len(after)} after the warm-up) within {seconds} emulated s (GIF starts {gifs})")
        b0, b1, y0, y1, span, flat = leak_halves(census)
        print(f"census: {len(census)} lines; least blocks {b0} -> {b1}, least allocated {y0} -> {y1} B, first half against second, over {span} GIFs")
        # A std thread's leftovers were ~4 blocks and ~190 B a GIF: 40 GIFs
        # were ~160 blocks and ~7.6 KB. Flat means within one GIF's own
        # churn (a decoder's pieces, the ring held or not).
        if not flat:
            die(f"the heap grew across {span} GIFs: blocks {b0} -> {b1}, allocated {y0} -> {y1} B")
        print(f"EMU_LEAK_OK ({took:.0f} s wall)")
        return
    if a.mode == "rig":
        if floors:
            print(f"heartbeat heap floors: lowest {min(floors)} B over {len(floors)} heartbeats")
        if not built:
            die(f"no `fat index: built` within {seconds} emulated s / {wall} s wall (GIF starts {gifs})")
        if built[1] == 0 or built[3] != "false":
            die(f"the build did not leave a current index: {built[0][:230]}")
        if built[2] != 0:
            die(f"transitions were cut during the build: {built[0][:230]}")
        print(f"pass  build: {built[0][:300]}")
        print(f"pass  GIF starts: {gifs}")
        print(f"EMU_RIG_OK ({took:.0f} s wall)")
        return
    if a.mode == "nocard":
        missing = [n for n, _ in NOCARD if n not in seen]
        if missing:
            die(f"not seen within {seconds} emulated s / {wall} s wall: {missing}")
        for n, _ in NOCARD:
            print(f"pass  {n}: {seen[n][:170]}")
        print(f"EMU_NOCARD_OK ({took:.0f} s wall)")
        return
    if a.mode == "smoke":
        missing = [n for n, _ in SMOKE if n not in seen]
        if missing or gifs < a.min_gifs:
            die(f"not seen within {seconds} emulated s / {wall} s wall: {missing or ''} (GIF starts {gifs}/{a.min_gifs})")
        for n, _ in SMOKE:
            print(f"pass  {n}: {seen[n][:170]}")
        print(f"pass  GIF starts: {gifs}")
        print(f"EMU_SMOKE_OK ({took:.0f} s wall)")
    else:
        if not done_at:
            die(f"no `psi-probe: done` within {seconds} emulated s / {wall} s wall")
        if differ_bad:
            die("differ with a non-zero count:\n" + "\n".join(differ_bad))
        # The paint probe's image (`make emu-image-paint`): its own summary
        # line (each case's kernel frames against the VM's), nothing else.
        if "psi-probe: done: paint" in done_at:
            if not paint:
                die("the paint rows' summary line is missing")
            if paint[1] == 0 or paint[2] != 0:
                die(f"paint rows: {paint[0]}")
            print("--- correctness lines:")
            for l in lines:
                if PROBE_KEYS.search(l):
                    print(l[:300])
            print(f"paint: {paint[1]} cases, {paint[2]} differ")
            print(f"EMU_PROBE_OK ({took:.0f} s wall)")
            return
        if not golden:
            die("the golden-frames result line is missing")
        if golden[1] == 0 or golden[2] != 0:
            die(f"golden frames: {golden[1]} match, {golden[2]} differ")
        # Every bundled Formula opens through the real path, and a short heap
        # refuses Endless road (far short, and just short of what its open
        # takes) instead of aborting.
        if not bundled:
            die("the bundled-formulas line is missing")
        if bundled[1] == 0 or bundled[2] != 0 or bundled[3] != "refused" or bundled[4] != "refused":
            die(f"bundled formulas: {bundled[0]}")
        # Formulas by reference: a bundled one by name, the card's .psi file.
        if not refs:
            die("the formulas-by-reference line is missing")
        if refs[1] != "played" or refs[2] != "played":
            die(f"formulas by reference: {refs[0]}")
        # A formula over a formula with the heap short (Engine::retune): played
        # once the one on screen is let go; refused when even that is short.
        if not swap:
            die("the formula-over-formula line is missing")
        # (The second case cannot be made when the Formula on screen holds
        # more than Endless road's open takes: the player loads compiled
        # Formulas now, and Time gate holds more.)
        if swap[1] != "played" or not (swap[2] == "refused, the playlist on" or swap[2].startswith("not constructible")):
            die(f"formula over formula: {swap[0]}")
        # A card Formula over a card Formula (the same file written anew, the
        # same within a skip's transition, another): each played.
        if not swap_card:
            die("the card-formula-over-card-formula line is missing")
        if swap_card[1] != "all played":
            die(f"card formula over card formula: {swap_card[0]}")
        # The player's raw card reads (fatidx): every file read raw matches
        # FatFs's bytes, and some were read raw.
        # The settings file on the card: a save cut between its renames
        # (a power cut there) loads the new settings.
        if not settings_file:
            die("the settings-file line is missing")
        if settings_file[1] != "loads the new settings":
            die(f"settings file: {settings_file[0]}")
        if not raw_reads:
            die("the raw-reads line is missing")
        if raw_reads[1] == 0 or raw_reads[2] != 0 or raw_reads[3] == 0:
            die(f"raw reads: {raw_reads[0]}")
        # The names file: every folder's names as its listing has them.
        if not names_file:
            die("the names-file line is missing")
        if names_file[1] == 0 or names_file[2] != 0:
            die(f"names file: {names_file[0]}")
        # The read-ahead: the same bytes as FatFs, and some served from it.
        if not readahead:
            die("the read-ahead line is missing")
        if readahead[1] != "the same as FatFs's" or readahead[2] == 0:
            die(f"read-ahead: {readahead[0]}")
        print("--- correctness lines:")
        for l in lines:
            if PROBE_KEYS.search(l):
                print(l[:230])
        print(f"golden: {golden[1]} match, {golden[2]} differ")
        print(f"EMU_PROBE_OK ({took:.0f} s wall)")


def offline(a, sim, rom, want_sha):
    """The offline lane (offline_lane.py), in its own network namespace like the API lane's:
    its second boot joins the virtual access point."""
    import offline_lane
    if os.environ.get("EMU_API_NETNS") != "1":
        if not shutil.which("unshare"):
            sys.exit("EMU_OFFLINE_FAIL: no `unshare` (util-linux): the lane runs in its own network namespace")
        env = {**os.environ, "EMU_API_NETNS": "1"}
        os.execvpe("unshare", ["unshare", "--user", "--map-root-user", "--net", "--", sys.executable, __file__, *sys.argv[1:]], env)
    ip = shutil.which("ip") or next((p for p in ("/usr/sbin/ip", "/sbin/ip") if os.path.exists(p)), None)
    if not ip or subprocess.run([ip, "link", "set", "lo", "up"]).returncode != 0:
        sys.exit("EMU_OFFLINE_FAIL: cannot bring up the loopback in the lane's network namespace")
    a.emu_dir, a.want_sha = EMU_DIR, want_sha
    sys.exit(offline_lane.run(a, sim, rom, FAIL, ANSI, BOOT_SHA))


def api(a, sim, rom, want_sha):
    """The API lane (api_lane.py), inside its own network namespace: run.py runs itself again
    under `unshare -rn` (an unprivileged user and network namespace with only a loopback)."""
    import api_lane
    for what, p in (("--www-pack", a.www_pack), ("--bundle", a.bundle), ("--psb", a.psb)):
        if not p or not p.exists():
            sys.exit(f"EMU_API_FAIL: {what} missing: {p}")
    try:
        a.seeded = api_lane.seeded_image(a.image, EMU_DIR / "api", extra={"boot_fx": api_lane.BOOT_GOOD})
        a.seeded_bad = api_lane.seeded_image(a.image, EMU_DIR / "api", extra={"boot_fx": api_lane.BOOT_BAD})
    except api_lane.LaneFail as e:
        sys.exit(f"EMU_API_FAIL: {e}")
    if os.environ.get("EMU_API_NETNS") != "1":
        if not shutil.which("unshare"):
            sys.exit("EMU_API_FAIL: no `unshare` (util-linux): the lane runs in its own network namespace")
        env = {**os.environ, "EMU_API_NETNS": "1"}
        os.execvpe("unshare", ["unshare", "--user", "--map-root-user", "--net", "--", sys.executable, __file__, *sys.argv[1:]], env)
    ip = shutil.which("ip") or next((p for p in ("/usr/sbin/ip", "/sbin/ip") if os.path.exists(p)), None)
    if not ip or subprocess.run([ip, "link", "set", "lo", "up"]).returncode != 0:
        sys.exit("EMU_API_FAIL: cannot bring up the loopback in the lane's network namespace")
    a.emu_dir, a.want_sha = EMU_DIR, want_sha
    sys.exit(api_lane.run(a, sim, rom, FAIL))


main()
