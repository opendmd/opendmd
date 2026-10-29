#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# ///
"""Build the emulator's SD card image, rootless: tools/ci-card.py's synthetic card
(prepared copies and all) copied into a FAT32 image with mtools. No mount, no sudo.

    uv run tools/emu/mkcard.py [--out IMG] [--root DIR] [--size-mb 256] [--force]

Default out: $EMU_DIR/card/sd.img (EMU_DIR defaults to ~/scratch/opendmd-emu-sim).
Cached: rebuilt only when missing or --force. Needs mtools (mformat, mcopy).
"""
import argparse, os, shutil, subprocess, sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
EMU_DIR = Path(os.environ.get("EMU_DIR", "~/scratch/opendmd-emu-sim"))


def need_mtools():
    missing = [t for t in ("mformat", "mcopy") if not shutil.which(t)]
    if missing:
        sys.exit(f"mkcard: {', '.join(missing)} not found. Install mtools (e.g. `sudo apt install mtools`); "
                 "this script does not install system packages.")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path, default=EMU_DIR / "card" / "sd.img")
    ap.add_argument("--root", type=Path, default=EMU_DIR / "card" / "root", help="card tree (made by ci-card.py if absent)")
    ap.add_argument("--size-mb", type=int, default=256)
    ap.add_argument("--force", action="store_true")
    a = ap.parse_args()
    need_mtools()
    # Stale when the card's recipe changed since it was built: ci-card.py (what
    # is on the card) or this script. Then both the tree and the image are remade.
    inputs = [REPO / "tools" / "ci-card.py", Path(__file__).resolve()]
    stale = a.out.exists() and any(p.stat().st_mtime > a.out.stat().st_mtime for p in inputs)
    if stale:
        print(f"card image older than {', '.join(p.name for p in inputs)}: rebuilding")
    if a.out.exists() and not a.force and not stale:
        print(f"card image up to date: {a.out}")
        return
    if a.force or stale or not (a.root / "filelist.txt").exists():
        if a.root.exists():
            shutil.rmtree(a.root)
        print(f"==> synthetic card tree -> {a.root}")
        subprocess.run(["uv", "run", "--quiet", str(REPO / "tools" / "ci-card.py"), str(a.root)], check=True)
    a.out.parent.mkdir(parents=True, exist_ok=True)
    tmp = a.out.with_suffix(".tmp")
    tmp.unlink(missing_ok=True)
    with open(tmp, "wb") as f:
        f.truncate(a.size_mb * 1024 * 1024)  # sparse
    env = {**os.environ, "MTOOLS_SKIP_CHECK": "1"}
    sectors = a.size_mb * 2048
    subprocess.run(["mformat", "-i", str(tmp), "-F", "-v", "OPENDMD", "-T", str(sectors), "::"], check=True, env=env)
    entries = sorted(str(p) for p in a.root.iterdir())
    subprocess.run(["mcopy", "-s", "-Q", "-m", "-i", str(tmp), *entries, "::"], check=True, env=env)
    os.replace(tmp, a.out)
    print(f"card image: {a.out} ({a.size_mb} MB FAT32 from {a.root})")


main()
