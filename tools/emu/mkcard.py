#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# ///
"""Build the emulator's SD card image, rootless: tools/ci-card.py's synthetic card
(prepared copies and all) copied into a FAT32 image with mtools. No mount, no sudo.

    uv run tools/emu/mkcard.py [--out IMG] [--root DIR] [--size-mb 256] [--force]
    uv run tools/emu/mkcard.py --rig    # the owner's rig's scale: $EMU_DIR/card/rig.img

--rig: the synthetic card plus ~10,000 more GIFs over 30 more folders (three of 1,200,
five of 600, the rest ~150) and a prepared copy of each in its bucket
(`cache/<folder>/<xx>/`, ~4,500 buckets): what the FAT index's build walks on the rig
(31 GB, ~10,100 GIFs, ~10,100 copies). Tiny valid GIFs, hard-linked in the tree. A
1 GB image of 4 KB clusters.

Default out: $EMU_DIR/card/sd.img (EMU_DIR: as the Makefile sets it, else $SCRATCH/opendmd-emu-sim, else target/emu-sim).
Cached: rebuilt only when missing or --force. Needs mtools (mformat, mcopy).
"""
import argparse, os, shutil, subprocess, sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
EMU_DIR = Path(os.environ.get("EMU_DIR") or (Path(os.environ["SCRATCH"]) / "opendmd-emu-sim" if os.environ.get("SCRATCH") else Path(__file__).resolve().parents[2] / "target" / "emu-sim"))


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
    ap.add_argument("--rig", action="store_true", help="rig-scale card (see above)")
    a = ap.parse_args()
    if a.rig:
        return rig(a)
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


def fnv(b: bytes) -> int:
    h = 0x811C9DC5
    for c in b:
        h = ((h ^ c) * 0x01000193) & 0xFFFFFFFF
    return h


RIG_FOLDERS = [1200] * 3 + [600] * 5 + [150] * 22


def rig(a):
    out = a.out if a.out != EMU_DIR / "card" / "sd.img" else EMU_DIR / "card" / "rig.img"
    base = a.root
    root = EMU_DIR / "card" / "rig-root"
    inputs = [REPO / "tools" / "ci-card.py", Path(__file__).resolve()]
    if out.exists() and not a.force and not any(p.stat().st_mtime > out.stat().st_mtime for p in inputs):
        print(f"rig card image up to date: {out}")
        return
    if not (base / "filelist.txt").exists():
        subprocess.run(["uv", "run", "--quiet", str(REPO / "tools" / "ci-card.py"), str(base)], check=True)
    if root.exists():
        shutil.rmtree(root)
    shutil.copytree(base, root)
    gif = next(p for p in sorted((root / "gifs" / "Other").glob("*.gif")))
    copy = next(p for p in sorted((root / "cache" / "Other").rglob("*.gif")))
    files = copies = 0
    for i, n in enumerate(RIG_FOLDERS):
        cat = f"Rig_Folder_{i:02d}"
        (root / "gifs" / cat).mkdir(parents=True)
        for j in range(n):
            name = f"RIG_{i:02d}_Some_Longer_Title_Here_{j:04d}.gif"
            os.link(gif, root / "gifs" / cat / name)
            b = root / "cache" / cat / f"{fnv(name.encode()) & 0xFF:02x}"
            b.mkdir(parents=True, exist_ok=True)
            os.link(copy, b / name)
            files += 1
            copies += 1
    buckets = sum(1 for c in (root / "cache").iterdir() if c.is_dir() for b in c.iterdir() if b.is_dir())
    print(f"rig tree: {files} more GIFs in {len(RIG_FOLDERS)} more folders, {copies} more copies, {buckets} buckets in all")
    out.parent.mkdir(parents=True, exist_ok=True)
    tmp = out.with_suffix(".tmp")
    tmp.unlink(missing_ok=True)
    size_mb = 1024
    with open(tmp, "wb") as f:
        f.truncate(size_mb * 1024 * 1024)
    env = {**os.environ, "MTOOLS_SKIP_CHECK": "1"}
    subprocess.run(["mformat", "-i", str(tmp), "-F", "-c", "8", "-v", "OPENDMD", "-T", str(size_mb * 2048), "::"], check=True, env=env)
    entries = sorted(str(p) for p in root.iterdir())
    subprocess.run(["mcopy", "-s", "-Q", "-m", "-i", str(tmp), *entries, "::"], check=True, env=env)
    os.replace(tmp, out)
    print(f"rig card image: {out} ({size_mb} MB FAT32, 4 KB clusters, from {root})")


main()
