#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# ///
"""Red checks of the no-card probe judge (`run.py probe --no-card`): replay a passing lane's log
as it is (it must pass) and doctored the ways a broken JIT would show (each must fail):
kernels skipped, kernels falling back to the VM, the kernel goldens line missing, the floor
kernel's line missing, the tiny kernel returning the wrong value.

    uv run tools/emu/probe_red.py --image FULL.bin --log PASSING.log [--chip s3|esp32]

Prints PROBE_RED_OK when the judge passes the original and fails every doctored copy.
"""
import argparse, os, re, subprocess, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
KG = re.compile(r"golden frames through kernels: (\d+) match the host/wasm hashes, 0 differ \(bit-identical\); 0 on the VM \[\]")


def doctored(text: str):
    m = KG.search(text)
    if not m:
        sys.exit("probe_red: the log has no passing `jit2: golden frames through kernels` line")
    n = int(m[1])
    yield "kernels skipped", KG.sub(f"golden frames through kernels: 0 match the host/wasm hashes, 0 differ; 0 on the VM []", text).replace("; 0 skipped (the probe's heap)", f"; {n} skipped (the probe's heap)")
    yield "kernels on the VM", KG.sub(f"golden frames through kernels: {n - 1} match the host/wasm hashes, 0 differ; 1 on the VM [(\"no executable memory\", 1)]", text)
    yield "no kernel goldens line", "\n".join(l for l in text.splitlines() if "jit2: golden frames through kernels" not in l) + "\n"
    yield "no floor kernel line", "\n".join(l for l in text.splitlines() if "floor kernel against the VM at four times" not in l) + "\n"
    yield "tiny kernel wrong", text.replace("returned 42 (want 42)", "returned 41 (want 42)")


def judge(image: Path, log: Path, chip: str) -> bool:
    r = subprocess.run(["uv", "run", "--quiet", str(HERE / "run.py"), "probe", "--no-card", "--chip", chip, "--image", str(image), "--replay", str(log)],
                       capture_output=True, text=True)
    return "EMU_PROBE_OK" in r.stdout


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--image", type=Path, required=True)
    ap.add_argument("--log", type=Path, required=True)
    ap.add_argument("--chip", choices=["esp32", "s3"], default="s3")
    a = ap.parse_args()
    text = a.log.read_text(errors="replace")
    bad = []
    with tempfile.TemporaryDirectory() as d:
        if not judge(a.image, a.log, a.chip):
            bad.append("the original log does not pass")
        for name, body in doctored(text):
            p = Path(d) / "doctored.log"
            p.write_text(body)
            ok = judge(a.image, p, a.chip)
            print(f"{'PASSED (wrong)' if ok else 'failed (right)'}  {name}")
            if ok:
                bad.append(name)
    if bad:
        sys.exit(f"PROBE_RED_FAIL: {bad}")
    print("PROBE_RED_OK")


if __name__ == "__main__":
    main()
