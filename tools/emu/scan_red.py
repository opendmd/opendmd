#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# ///
"""Red checks of the ESP32-S3's scan judge (`run.py smoke|nocard --chip s3`): replay a passing
lane's log as it is (it must pass) and doctored the ways an unmodelled scan would show (each must
fail): the scan line replaced by "scan: not modelled", by "scan: no refresh in 150 ms", by an
implausible refresh, or missing; and for nocard, the address-pin check's lines missing.

A double-buffered log (smoke: the S3's default, firmware/src/s3panel.rs) is doctored the ways a
broken flip would show instead: the start line missing or replaced by the single-buffered
fallback's, its refresh 0, and in every `s3panel:` line a write that found the DMA in the buffer
being written (`back hits`), a switch never seen (`timeouts`) or seen late, switches behind the
flips by more than the one pending, no row pairs copied across, flips that stop between the
lines, and no flips at all.

The address-pin check is judged only for being logged: its k/n is meaningless in the emulator
(esp32sim does not loop the LCD_CAM's output back to GPIO_IN: ~1/16 for every register). On the
bench the current register agrees ~97% (misses as "ahead"), one a descriptor early ~95% (~13
"behind"), one late ~80%.

    uv run tools/emu/scan_red.py --mode smoke|nocard --image FULL.bin --log PASSING.log

Prints SCAN_RED_OK when the judge passes the original and fails every doctored copy.
"""
import argparse, re, subprocess, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCAN = re.compile(r"(hub75_shim: )scan: \d+ descriptors, \d+ per row pair, refresh \d+ us[^\n]*")


DBUF = re.compile(r"(hub75_shim: )panel: double-buffered: [^\n]*")
REFRESH = re.compile(r"(panel: double-buffered: .*?, refresh )\d+( us)")
PANEL = re.compile(r"s3panel: buffers 2, flips (\d+), switched (\d+), (skipped \d+, waits \d+ \(\d+ us in all, max \d+\)), "
                   r"late (\d+ \(at the wrap \d+\)), timeouts (\d+), back hits (\d+), pairs copied (\d+)")


def panel_sub(text: str, f) -> str:
    """Every `s3panel:` line rewritten by f(flips, switched, rest, late, timeouts, hits, copied)
    (late: its number and the rest of its field, `N (at the wrap M)`, as a string)."""
    return PANEL.sub(lambda m: "s3panel: buffers 2, flips {}, switched {}, {}, late {}, timeouts {}, back hits {}, pairs copied {}".format(
        *f(int(m[1]), int(m[2]), m[3], m[4], int(m[5]), int(m[6]), int(m[7]))), text)


def doctored_dbuf(text: str):
    if not PANEL.search(text):
        sys.exit("scan_red: the double-buffered log has no `s3panel: buffers 2` line")
    yield "no double-buffered line", "\n".join(l for l in text.splitlines() if not DBUF.search(l)) + "\n"
    yield "fell back to single", DBUF.sub(r"\1panel: single-buffered (double buffering did not start): internal heap 200000 B free (largest 100000) before the driver, 160000 (90000) after", text)
    yield "refresh 0", REFRESH.sub(r"\g<1>0\2", text)
    yield "back hits", panel_sub(text, lambda f, s, r, l, t, h, c: (f, s, r, l, t, h + 3, c))
    yield "timeouts", panel_sub(text, lambda f, s, r, l, t, h, c: (f, s, r, l, t + 1, h, c))
    yield "late", panel_sub(text, lambda f, s, r, l, t, h, c: (f, s, r, re.sub(r"^\d+", lambda n: str(int(n[0]) + 2), l), t, h, c))
    yield "switches behind", panel_sub(text, lambda f, s, r, l, t, h, c: (f, max(f - 5, 0), r, l, t, h, c))
    yield "nothing copied", panel_sub(text, lambda f, s, r, l, t, h, c: (f, s, r, l, t, h, 0))
    yield "flips stalled", panel_sub(text, lambda f, s, r, l, t, h, c: (7, 7, r, l, t, h, c))
    yield "no flips", panel_sub(text, lambda f, s, r, l, t, h, c: (0, 0, r, l, t, h, c))


def doctored(text: str, mode: str):
    if DBUF.search(text):
        yield from doctored_dbuf(text)
        return
    if not SCAN.search(text):
        sys.exit("scan_red: the log has no `scan: N descriptors` line")
    yield "scan: not modelled", SCAN.sub(r"\1scan: not modelled on the ESP32-S3 yet (GDMA): the beam is not raced", text)
    yield "scan: no refresh", SCAN.sub(r"\1scan: no refresh in 150 ms (OUT_DSCR, GDMA channel 0)", text)
    yield "scan: implausible", SCAN.sub(r"\1scan: refresh 31000 us measured, 10000 us expected: implausible, not used", text)
    yield "no scan line", "\n".join(l for l in text.splitlines() if not SCAN.search(l)) + "\n"
    if mode == "nocard":
        yield "no pin check", "\n".join(l for l in text.splitlines() if "agrees with the address pins" not in l) + "\n"


def judge(a, log: Path) -> bool:
    cmd = ["uv", "run", "--quiet", str(HERE / "run.py"), a.mode, "--chip", "s3", "--image", str(a.image), "--replay", str(log)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    return f"EMU_{a.mode.upper()}_OK" in r.stdout


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mode", choices=["smoke", "nocard"], required=True)
    ap.add_argument("--image", type=Path, required=True)
    ap.add_argument("--log", type=Path, required=True)
    a = ap.parse_args()
    text = a.log.read_text(errors="replace")
    bad = []
    with tempfile.TemporaryDirectory() as d:
        if not judge(a, a.log):
            bad.append("the original log does not pass")
        for name, body in doctored(text, a.mode):
            p = Path(d) / "doctored.log"
            p.write_text(body)
            ok = judge(a, p)
            print(f"{'PASSED (wrong)' if ok else 'failed (right)'}  {name}")
            if ok:
                bad.append(name)
    if bad:
        sys.exit(f"SCAN_RED_FAIL: {bad}")
    print("SCAN_RED_OK")


if __name__ == "__main__":
    main()
