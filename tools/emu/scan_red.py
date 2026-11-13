#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# ///
"""Red checks of the ESP32-S3's scan judge (`run.py smoke|nocard --chip s3`): replay a passing
lane's log as it is (it must pass) and doctored the ways an unmodelled scan would show (each must
fail): the scan line replaced by "scan: not modelled", by "scan: no refresh in 150 ms", by an
implausible refresh, or missing; and for nocard, the address-pin check's lines missing.

    uv run tools/emu/scan_red.py --mode smoke|nocard --image FULL.bin --log PASSING.log

Prints SCAN_RED_OK when the judge passes the original and fails every doctored copy.
"""
import argparse, re, subprocess, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCAN = re.compile(r"(hub75_shim: )scan: \d+ descriptors, \d+ per row pair, refresh \d+ us[^\n]*")


def doctored(text: str, mode: str):
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
