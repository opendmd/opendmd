# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Fail if any function in the firmware ELF realigns its stack pointer.

    uv run tools/check-stack-realign.py <elf> [--allow REGEX]

The esp Rust toolchain's LLVM gives a function with a local aligned to more
than the 16-byte stack alignment a prologue like

    entry   a1, 352
    movi.n  a8, 63 ; movi.n a9, 64
    and     a8, a1, a8 ; sub a8, a9, a8
    add.n   a1, a1, a8          <- a1 moved without MOVSP

which breaks the windowed ABI: an interrupt between `entry` and the `add`
spills the caller's a0-a3 below the old a1, and `retw` reloads them from
below the new one. The caller then runs on garbage registers (the /api/play
crash; see firmware/src/chan.rs). `std::sync::mpsc::channel()` was the only
such function in the firmware (its channel is 64-byte aligned); this check
keeps any other from coming in unnoticed — a new dependency, a
`#[repr(align(32))]` local, a toolchain change.

It looks at the first 48 bytes of every function symbol in `.flash.text` and
`.iram0.text` that starts with `entry a1, …`, for an `and` that reads a1, or
any instruction that writes a1, other than `entry`/`movsp`/`retw`. It reads
the section bytes itself (objdump hides Rust code behind `$d` mapping
symbols) with just enough of the Xtensa encoding for that.

Exit 0 and one line when clean; exit 1 and the offending functions if not.
`--allow` names functions (a regex on the symbol) that may realign — the
`diag-heap` build's std-channel torture test keeps one on purpose.
"""
import argparse
import re
import struct
import subprocess
import sys


def sections(elf):
    """{name: (vma, bytes)} for the two code sections, from the ELF itself."""
    data = open(elf, "rb").read()
    if data[:4] != b"\x7fELF" or data[4] != 1:
        sys.exit(f"{elf}: not a 32-bit ELF")
    shoff, = struct.unpack_from("<I", data, 0x20)
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", data, 0x2E)
    hdrs = [struct.unpack_from("<IIIIIIIIII", data, shoff + i * shentsize) for i in range(shnum)]
    strtab = hdrs[shstrndx]
    names = data[strtab[4]: strtab[4] + strtab[5]]
    out = {}
    for h in hdrs:
        name = names[h[0]: names.index(b"\0", h[0])].decode()
        if name in (".flash.text", ".iram0.text") and h[1] == 1:  # PROGBITS
            out[name] = (h[3], data[h[4]: h[4] + h[5]])
    return out


def functions(elf):
    """(address, name) of every defined text symbol, via nm."""
    nm = "xtensa-esp32-elf-nm"
    try:
        text = subprocess.run([nm, "-n", "--defined-only", elf], capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError) as e:
        sys.exit(f"{nm} failed ({e}); source ~/.espup-env.sh first")
    for line in text.splitlines():
        parts = line.split(maxsplit=2)
        if len(parts) == 3 and parts[1] in ("T", "t"):
            yield int(parts[0], 16), parts[2]


def insns(code):
    """(offset, length, bytes) of each instruction, narrow or 24-bit."""
    i = 0
    while i + 2 <= len(code):
        op0 = code[i] & 0xF
        n = 2 if op0 >= 8 and op0 <= 0xD else 3  # narrow (density) opcodes: 8..D
        if i + n > len(code):
            return
        yield i, n, code[i: i + n]
        i += n


def realigns(code):
    """The offset of an instruction that moves a1 in the prologue, or None.

    Only the straight-line code from `entry` to the first control transfer
    (call, jump, branch, return) is looked at: that is where a realignment
    is, and past a jump linear decoding may walk into literals."""
    first = True
    for off, n, b in insns(code):
        op0 = b[0] & 0xF
        if first:
            first = False
            # entry a1, imm: op0=6, n=3, m=0 -> byte0 0x36; s (a1) in byte1's low nibble
            if not (n == 3 and b[0] == 0x36 and (b[1] & 0xF) == 1):
                return None
            continue
        if n == 3 and op0 in (5, 6, 7):  # CALLn; J/BZ/BI; B
            return None
        if n == 2 and op0 == 0xC and (b[0] >> 4) & 8:  # beqz.n / bnez.n
            return None
        if n == 2 and op0 == 0xD and (b[1] >> 4) == 0xF:  # ret.n / retw.n / break.n / nop.n
            return None
        if n == 3 and op0 == 0:
            op1, op2 = b[2] & 0xF, b[2] >> 4
            r, s, t = b[1] >> 4, b[1] & 0xF, b[0] >> 4
            if op1 == 0 and op2 == 0 and r == 0:  # ST0: callx/jx/ret/retw (movsp is r=1)
                return None
            # RRR arithmetic/logic (op1=0) writing ar.
            if op1 == 0 and op2 in (1, 2, 3, 8, 9, 0xA, 0xB, 0xC, 0xD, 0xE, 0xF):
                if op2 == 2 and r == s == t == 1:
                    continue  # or a1, a1, a1: GCC's no-op before a noreturn call
                if op2 == 1 and s == 1:  # and ar, a1, at: a1 masked for alignment
                    return off
                if r == 1:
                    return off
        if n == 2 and op0 in (0xA, 0xB) and (b[1] >> 4) == 1:  # add.n / addi.n a1, …
            return off
        if n == 2 and op0 == 0xD and (b[1] >> 4) == 0 and (b[0] >> 4) == 1:  # mov.n a1, as
            return off
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("elf")
    ap.add_argument("--allow", default=None, help="regex of symbols allowed to realign")
    args = ap.parse_args()
    allow = re.compile(args.allow) if args.allow else None
    secs = sections(args.elf)
    bad, allowed, seen = [], [], 0
    for addr, name in functions(args.elf):
        for vma, data in secs.values():
            if vma <= addr < vma + len(data):
                code = data[addr - vma: addr - vma + 48]
                if len(code) >= 3 and code[0] == 0x36 and (code[1] & 0xF) == 1:
                    seen += 1
                    off = realigns(code)
                    if off is not None:
                        (allowed if allow and allow.search(name) else bad).append((addr + off, name))
                break
    if bad:
        print(f"{args.elf}: {len(bad)} function(s) move the stack pointer without MOVSP "
              f"(windowed-ABI violation; see tools/check-stack-realign.py):")
        for a, n in bad:
            print(f"  0x{a:08x}  {n}")
        sys.exit(1)
    extra = f", {len(allowed)} allowed ({', '.join(sorted({n[:60] for _, n in allowed}))})" if allowed else ""
    print(f"stack realignment: none in {seen} windowed functions{extra}")


if __name__ == "__main__":
    main()
