#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# ///
"""A read-only consistency check of a FAT32 card image (what `fsck.fat -n` would say), no mount.

    uv run tools/emu/fatcheck.py CARD.img

Walks the directory tree from the root and follows every chain: a cluster out of range, a
chain that loops or runs into a free cluster, two chains sharing a cluster (cross-linked),
a file whose chain is not the length its size needs, an allocated cluster no chain reaches
(lost), or FAT copies that differ: each is a problem. Prints `fatcheck: ok, ...` or the
problems (exit 1). The API lane (`run.py api`) runs it on the card the emulator wrote out.
"""
import mmap, struct, sys
from pathlib import Path

EOC = 0x0FFFFFF8
BAD = 0x0FFFFFF7


def check(path: Path) -> tuple[list[str], dict]:
    problems: list[str] = []
    with open(path, "rb") as f:
        m = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)
    bps, spc, reserved, nfats = struct.unpack_from("<HBHB", m, 11)
    root_ents, tot16, _media, fat16 = struct.unpack_from("<HHBH", m, 17)
    tot32, fat32, _flags, _ver, root_clus, fsinfo = struct.unpack_from("<IIHHIH", m, 32)
    if bps not in (512, 1024, 2048, 4096) or spc == 0 or fat16 != 0 or root_ents != 0:
        return [f"not a FAT32 volume (bytes/sector {bps}, sectors/cluster {spc}, FAT16 size {fat16})"], {}
    total = tot32 or tot16
    fat_off = reserved * bps
    data_sec = reserved + nfats * fat32
    nclus = (total - data_sec) // spc
    csize = bps * spc
    fat_bytes = fat32 * bps
    fats = [m[fat_off + i * fat_bytes: fat_off + (i + 1) * fat_bytes] for i in range(nfats)]
    for i in range(1, nfats):
        if fats[i][:4 * (nclus + 2)] != fats[0][:4 * (nclus + 2)]:
            problems.append(f"FAT copy {i + 1} differs from the first")
    fat = memoryview(fats[0]).cast("I")

    def nxt(c: int) -> int:
        return fat[c] & 0x0FFFFFFF

    def coff(c: int) -> int:
        return (data_sec + (c - 2) * spc) * bps

    owner: dict[int, str] = {}

    def chain(start: int, name: str) -> list[int]:
        out = []
        c = start
        while True:
            if not 2 <= c < nclus + 2:
                problems.append(f"{name}: cluster {c} out of range")
                break
            if c in owner:
                problems.append(f"{name}: cluster {c} cross-linked with {owner[c]}")
                break
            owner[c] = name
            out.append(c)
            v = nxt(c)
            if v >= EOC:
                break
            if v == 0:
                problems.append(f"{name}: chain runs into free cluster after {c}")
                break
            if v == BAD:
                problems.append(f"{name}: chain runs into a bad-cluster mark after {c}")
                break
            c = v
        return out

    stats = {"files": 0, "dirs": 1, "bytes": 0}

    def walk(start: int, where: str, depth: int):
        if depth > 32:
            problems.append(f"{where}: deeper than 32 levels")
            return
        clusters = chain(start, where or "/")
        lfn = ""
        for c in clusters:
            base = coff(c)
            for off in range(base, base + csize, 32):
                e = m[off:off + 32]
                if e[0] == 0:
                    return
                if e[0] == 0xE5:
                    lfn = ""
                    continue
                attr = e[11]
                if attr == 0x0F:
                    # A long name's piece (in reverse order).
                    part = (e[1:11] + e[14:26] + e[28:32]).decode("utf-16-le", "replace")
                    lfn = part.split("\x00")[0] + lfn
                    continue
                short = e[0:8].decode("latin-1").rstrip() + ("." + e[8:11].decode("latin-1").rstrip() if e[8:11].strip() else "")
                name, lfn = (lfn or short), ""
                if attr & 0x08:  # volume label
                    continue
                if short in (".", ".."):
                    continue
                first = (struct.unpack_from("<H", e, 20)[0] << 16) | struct.unpack_from("<H", e, 26)[0]
                size = struct.unpack_from("<I", e, 28)[0]
                full = f"{where}/{name}"
                if attr & 0x10:
                    stats["dirs"] += 1
                    if first == 0:
                        problems.append(f"{full}: a directory with no cluster")
                    else:
                        walk(first, full, depth + 1)
                else:
                    stats["files"] += 1
                    stats["bytes"] += size
                    need = (size + csize - 1) // csize
                    if first == 0:
                        if size:
                            problems.append(f"{full}: {size} B and no cluster")
                        continue
                    got = len(chain(first, full))
                    if got != need:
                        problems.append(f"{full}: {size} B needs {need} clusters, its chain has {got}")

    walk(root_clus, "", 0)
    lost = [c for c in range(2, nclus + 2) if nxt(c) != 0 and nxt(c) != BAD and c not in owner]
    if lost:
        problems.append(f"{len(lost)} lost clusters (allocated, in no chain), e.g. {lost[:8]}")
    free = sum(1 for c in range(2, nclus + 2) if nxt(c) == 0)
    stats |= {"clusters": nclus, "cluster_bytes": csize, "used": len(owner), "free": free}
    return problems, stats


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    problems, s = check(Path(sys.argv[1]))
    if problems:
        print(f"fatcheck: {len(problems)} problem(s) in {sys.argv[1]}:")
        for p in problems[:40]:
            print(f"  {p}")
        sys.exit(1)
    print(f"fatcheck: ok, {s['files']} files and {s['dirs']} directories, {s['used']} of {s['clusters']} clusters "
          f"of {s['cluster_bytes']} B in use, no cross-links, no lost clusters, the FAT copies agree")


if __name__ == "__main__":
    main()
