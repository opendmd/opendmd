#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# ///
"""Red checks of the probe judges (`run.py probe`): replay a passing lane's log as it is (it
must pass) and doctored the ways a broken VM or JIT, or a probe short of heap, would show (each
must fail): VM goldens skipped past what is allowed, a skip said but not counted or counted but
not said, VM goldens stopped for the heap, the VM goldens line missing; kernels skipped, kernels
falling back to the VM, a kernel skip not counted, the kernel goldens line missing; and for the
no-card probe the floor kernel's line missing and the tiny kernel returning the wrong value.

    uv run tools/emu/probe_red.py --image FULL.bin --log PASSING.log [--chip s3|esp32] [--card]
        [--vm-skips N] [--jit-skips N]

`--card` judges the card probe's log (`make emu-probe`); without it the no-card probe's
(`make emu-s3-probe`). The skips are passed to the judge as the lane passes them.
Prints PROBE_RED_OK when the judge passes the original and fails every doctored copy.
"""
import argparse, re, subprocess, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
KG = re.compile(r"golden frames through kernels: (\d+) match the host/wasm hashes, 0 differ \(bit-identical\); 0 on the VM \[\]; (?:(\d+) skipped)?")
VG = re.compile(r"psi-probe: vm: golden frames: (\d+) match the host/wasm hashes, 0 differ; (\d+) skipped \(the probe's heap\), 0 not run \(heap under 40 KB\) of (\d+) cases")


def lines_without(text: str, needle: str) -> str:
    return "\n".join(l for l in text.splitlines() if needle not in l) + "\n"


def after(text: str, needle: str, extra: str) -> str:
    """`extra` as a line of its own just before the first line holding `needle`."""
    out, done = [], False
    for l in text.splitlines():
        if not done and needle in l:
            out.append(extra)
            done = True
        out.append(l)
    return "\n".join(out) + "\n"


TOTAL = re.compile(r"psi-probe: golden frames: (\d+) match the host/wasm hashes")


def total(text: str, delta: int) -> str:
    """The overall golden line's matches moved by `delta`, as the VM's or the kernels' moved:
    a doctored copy fails for its own reason, not for the sum."""
    return TOTAL.sub(lambda m: f"psi-probe: golden frames: {int(m[1]) + delta} match the host/wasm hashes", text)


def doctored(text: str, card: bool):
    k = KG.search(text)
    if not k:
        sys.exit("probe_red: the log has no passing `jit2: golden frames through kernels` line")
    v = VG.search(text)
    if not v:
        sys.exit("probe_red: the log has no passing `vm: golden frames` line")
    n, ks = int(k[1]), int(k[2] or 0)
    vn, vs, vc = int(v[1]), int(v[2]), int(v[3])
    vskip_line = "I (1) opendmd_firmware::psi_probe: psi-probe: golden plasma: skipped, no heap to compile it here (heap 9000 B free, largest block 8000 B)"
    kskip_line = "I (1) opendmd_firmware::psi_probe: psi-probe: jit2: golden plasma: skipped, no heap to compile it here (heap 9000 B free, largest block 8000 B)"
    def vline(ok, skip, not_run=0):
        return f"psi-probe: vm: golden frames: {ok} match the host/wasm hashes, 0 differ; {skip} skipped (the probe's heap), {not_run} not run (heap under 40 KB) of {vc} cases"
    # The VM's goldens.
    yield "VM golden skipped", total(after(VG.sub(vline(vn - 1, vs + 1), text), "psi-probe: vm: golden frames:", vskip_line), -1)
    yield "VM skip said, not counted", after(text, "psi-probe: vm: golden frames:", vskip_line)
    yield "VM skip counted, not said", VG.sub(vline(vn, vs + 1), text)
    yield "VM goldens stopped for the heap", total(VG.sub(vline(vn - 5, vs, 5), text), -5)
    yield "no VM goldens line", lines_without(text, "psi-probe: vm: golden frames:")
    # The JIT's.
    kskip = f"; {ks + n} skipped" if k[2] is not None else ""
    yield "kernels skipped", total(KG.sub(f"golden frames through kernels: 0 match the host/wasm hashes, 0 differ; 0 on the VM []{kskip}", text), -n)
    yield "kernels on the VM", total(KG.sub(lambda m: m[0].replace(f"{n} match", f"{n - 1} match").replace("; 0 on the VM []", "; 1 on the VM [(\"no executable memory\", 1)]"), text), -1)
    yield "kernel skip said, not counted", after(text, "jit2: golden frames through kernels", kskip_line)
    yield "no kernel goldens line", lines_without(text, "jit2: golden frames through kernels")
    if not card:
        yield "no floor kernel line", lines_without(text, "floor kernel against the VM at four times")
        yield "tiny kernel wrong", text.replace("returned 42 (want 42)", "returned 41 (want 42)")


def judge(a, log: Path) -> bool:
    cmd = ["uv", "run", "--quiet", str(HERE / "run.py"), "probe", "--chip", a.chip, "--image", str(a.image), "--replay", str(log),
           "--vm-skips", str(a.vm_skips), "--jit-skips", str(a.jit_skips)]
    if not a.card:
        cmd.append("--no-card")
    r = subprocess.run(cmd, capture_output=True, text=True)
    return "EMU_PROBE_OK" in r.stdout


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--image", type=Path, required=True)
    ap.add_argument("--log", type=Path, required=True)
    ap.add_argument("--chip", choices=["esp32", "s3"], default="s3")
    ap.add_argument("--card", action="store_true", help="the card probe's log (make emu-probe), not the no-card probe's")
    ap.add_argument("--vm-skips", type=int, default=0)
    ap.add_argument("--jit-skips", type=int, default=0)
    a = ap.parse_args()
    text = a.log.read_text(errors="replace")
    bad = []
    with tempfile.TemporaryDirectory() as d:
        if not judge(a, a.log):
            bad.append("the original log does not pass")
        for name, body in doctored(text, a.card):
            p = Path(d) / "doctored.log"
            p.write_text(body)
            ok = judge(a, p)
            print(f"{'PASSED (wrong)' if ok else 'failed (right)'}  {name}")
            if ok:
                bad.append(name)
    if bad:
        sys.exit(f"PROBE_RED_FAIL: {bad}")
    print("PROBE_RED_OK")


if __name__ == "__main__":
    main()
