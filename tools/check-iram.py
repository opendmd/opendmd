# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Fail if a function that must run from IRAM was linked into flash.

    uv run tools/check-iram.py <elf>

Code in flash runs through the 32 KB flash cache, and the board's flash is
DIO at 40 MHz: a refill is microseconds. Which hot loops collide in the
cache depends on the whole image's layout, so a change anywhere can make
the same frame 2-3x dearer (psi-speed's VM loops; psi-perf 2026-11-02: the
raced pair writer cost 1.2 ms a frame from flash in one build against 0.4 in
another). The hot paths that matter are therefore placed in IRAM
(`#[link_section = ".iram1.…"]`, `IRAM_ATTR`), and a refactor that renames,
inlines or moves one would put it back in flash without a sound. This
check names them and fails the build if any is missing or outside
`.iram0.text`.

Each entry is a regex matched against nm's demangled symbol names; at least
one symbol must match, and every match must be in IRAM. Extend REQUIRED when
something new is placed.
"""
import re
import subprocess
import sys

# (regex on the demangled name, why it is in IRAM)
REQUIRED = [
    (r"^opendmd_psi::vm::frame::<128>$", "the VM's frame loop: every pixel step's dispatch"),
    (r"^<opendmd_psi::vm::Rows>::begin::<128>$", "the VM's per-row entry"),
    (r"^<opendmd_psi::vm::Rows>::finish$", "the VM's per-row compare"),
    (r"^opendmd_psi::vm::fused::<128>$", "the fused runs"),
    (r"^opendmd_firmware::panel::beam::pair$", "the raced pair hook"),
    (r"^opendmd_firmware::panel::write_pair$", "the pair writer"),
    (r"^opendmd_firmware::panel::write_row$", "the row writer"),
    (r"^hub75_draw_span$", "the shim's span writer"),
    (r"^hub75_draw_row_pair$", "the shim's row-pair writer"),
    (r"^hub75_scan_pos$", "the scan read, twice a raced pair"),
    (r"^opendmd_firmware::wake::give$", "the display's wake, run in the esp_timer ISR"),
]


def sections(elf):
    """(start, end) of .iram0.text, from readelf."""
    out = subprocess.run(["xtensa-esp32-elf-readelf", "-S", "-W", elf], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        m = re.search(r"\]\s+\.iram0\.text\s+\S+\s+([0-9a-f]+)\s+[0-9a-f]+\s+([0-9a-f]+)", line)
        if m:
            a = int(m.group(1), 16)
            return a, a + int(m.group(2), 16)
    sys.exit("no .iram0.text section in the ELF")


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    elf = sys.argv[1]
    try:
        lo, hi = sections(elf)
        text = subprocess.run(["xtensa-esp32-elf-nm", "-C", "--defined-only", elf], capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError) as e:
        sys.exit(f"binutils failed ({e}); source ~/.espup-env.sh first")
    syms = []
    for line in text.splitlines():
        parts = line.split(maxsplit=2)
        if len(parts) == 3 and parts[1] in ("T", "t"):
            syms.append((int(parts[0], 16), parts[2].strip()))
    # A build without Psi (no `psi` feature) has no VM to place.
    psi = any(n.startswith("opendmd_psi::") for _, n in syms)
    bad = []
    for pat, why in REQUIRED:
        if not psi and "opendmd_psi" in pat:
            continue
        r = re.compile(pat)
        hits = [(a, n) for a, n in syms if r.search(n)]
        if not hits:
            bad.append(f"  {pat}: no such function ({why}): renamed or inlined?")
        for a, n in hits:
            if not lo <= a < hi:
                bad.append(f"  {n} at {a:#x} is in flash, not IRAM ({why})")
    if bad:
        print("IRAM placement: hot functions outside IRAM:\n" + "\n".join(bad))
        sys.exit(1)
    print(f"IRAM placement: {len(REQUIRED)} hot functions in .iram0.text ({hi - lo:,} B)")


if __name__ == "__main__":
    main()
