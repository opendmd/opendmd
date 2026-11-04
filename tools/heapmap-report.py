#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# ///
"""Tables from a heap-map build's output (firmware/src/heapmap.rs, `FW_RELEASE_FEATURES=heapmap`).

    uv run tools/heapmap-report.py LOG_OR_JSON --elf ELF [--report N] [--sites 40]

LOG_OR_JSON: a console capture (the `heapmap:` lines; the emulator's log, tools/serial-capture.py's)
or the JSON of `GET /api/status?heapmap=1`. ELF: the image's ELF (the release build's
`target/xtensa-esp32-espidf/release/opendmd-firmware`), to name the Rust call sites with addr2line.
--report N: which census of the log (default: the last complete one).

Prints markdown: the checkpoints (free heap, largest block and the change at each start-up step),
the task table, the census (Rust vs C, free blocks, the largest C blocks) and the Rust call sites
grouped by owner (the first frame outside the allocator and std, its module the owner).
"""
import argparse, json, os, re, subprocess, sys
from collections import defaultdict
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
CP = re.compile(
    r"heapmap: cp (\d+) (.+?)\s+t=(\d+) ms \[([^\]]*)\] 8bit (\d+) \(([+-]?\d+)\) big (\d+) \| int (\d+) \(([+-]?\d+)\) big (\d+) \| dma (\d+) \(([+-]?\d+)\) big (\d+) \| rust (\d+) \(([+-]?\d+)\) in (\d+)"
)
TASK = re.compile(r"heapmap: task (\S+)\s+cpu(\S) p(\d+)\s+(\S+)\s+stack (\d+ B heap|static) unused (\d+) B")
SITE = re.compile(r"heapmap: site\s+(\d+)\s+(\d+) B in\s+(\d+):((?: 0x[0-9a-f]+)+)")
CENSUS = re.compile(r"heapmap: census at (\d+) s: (.*)")
REPORT = re.compile(r"heapmap: report (\d+) (?:at (\d+) s|end)")
SKIP = re.compile(
    r"^(alloc::|core::|std::|__rust|__rg_|<alloc::|<core::|<std::|hashbrown::|<hashbrown::|opendmd_firmware::heapmap|"
    r"<opendmd_firmware::heapmap|\?\?|_ZN|esp_backtrace)"
)


def toolchain_addr2line():
    for p in sorted(Path.home().glob(".espressif/tools/xtensa-esp-elf/*/xtensa-esp-elf/bin/xtensa-esp32-elf-addr2line")):
        return str(p)
    return "xtensa-esp32-elf-addr2line"


def symbolize(elf, pcs):
    """{pc: [(function, file:line), ...] innermost first}."""
    pcs = sorted({p for p in pcs if p})
    if not pcs or not elf:
        return {}
    out = subprocess.run([toolchain_addr2line(), "-a", "-f", "-C", "-i", "-e", elf],
                         input="\n".join(f"{p:#x}" for p in pcs), capture_output=True, text=True).stdout.splitlines()
    res, cur, i = {}, None, 0
    while i < len(out):
        l = out[i]
        if re.fullmatch(r"0x[0-9a-f]+", l):
            cur = int(l, 16)
            res[cur] = []
            i += 1
            continue
        fn = l
        loc = out[i + 1] if i + 1 < len(out) else ""
        res.setdefault(cur, []).append((fn, loc))
        i += 2
    return res


def short_loc(loc):
    loc = re.sub(r" \(discriminator \d+\)", "", loc)
    for root in ("/opendmd/", "/cargo/registry/src/", "/rustup/toolchains/"):
        if root in loc:
            loc = loc.split(root, 1)[1]
    return loc.split("/")[-2] + "/" + loc.split("/")[-1] if loc.count("/") > 1 else loc


def owner_of(frames):
    """(owner module, function, location) of the first frame outside the allocator and std."""
    for fn, loc in frames:
        if SKIP.match(fn):
            continue
        name = re.sub(r"<(.+?) as .+?>", r"\1", fn)
        name = re.sub(r"::\{\{closure\}\}", "", name)
        name = re.sub(r"::h[0-9a-f]{16}$", "", name)
        parts = [p for p in re.split(r"::", name.lstrip("<&")) if p]
        if parts and parts[0] in ("opendmd_firmware", "opendmd_core", "esp_idf_svc", "esp_idf_hal", "gifslim", "opendmd_psi"):
            mod = "::".join(parts[:2]) if len(parts) > 1 else parts[0]
        else:
            mod = parts[0] if parts else fn
        return mod, name, short_loc(loc)
    return "?", frames[0][0] if frames else "?", ""


def parse_log(path, which):
    lines = [ANSI.sub("", l.rstrip("\n")) for l in open(path, errors="replace")]
    cps = []
    reports = []
    cur = None
    for l in lines:
        m = CP.search(l)
        if m:
            g = m.groups()
            cps.append(dict(k=int(g[0]), name=g[1].strip(), t_ms=int(g[2]), task=g[3], free=int(g[4]), d=int(g[5]),
                            largest=int(g[6]), free_int=int(g[7]), d_int=int(g[8]), largest_int=int(g[9]),
                            free_dma=int(g[10]), d_dma=int(g[11]), largest_dma=int(g[12]), rust=int(g[13]),
                            d_rust=int(g[14]), rust_allocs=int(g[15])))
            continue
        m = REPORT.search(l)
        if m and m.group(2):
            cur = dict(seq=int(m[1]), t_s=int(m[2]), tasks=[], sites=[], lines=[])
            continue
        if cur is None:
            continue
        if "heapmap:" in l:
            cur["lines"].append(l[l.index("heapmap:"):])
        m = TASK.search(l)
        if m:
            st = m[5]
            cur["tasks"].append(dict(name=m[1], core=m[2], prio=int(m[3]), state=m[4],
                                     stack=int(st.split()[0]) if st != "static" else 0, unused=int(m[6])))
        m = SITE.search(l)
        if m:
            cur["sites"].append(dict(site=int(m[1]), live=int(m[2]), count=int(m[3]), pcs=[int(x, 16) for x in m[4].split()]))
        m = CENSUS.search(l)
        if m:
            cur["census"] = m[2]
        if re.search(r"heapmap: report \d+ end", l):
            reports.append(cur)
            cur = None
    rep = None
    if reports:
        rep = reports[which] if which is not None else reports[-1]
    return cps, rep, len(reports)


def parse_json(path):
    j = json.load(open(path))
    h = j.get("heapmap", j)
    cps = []
    last = {}
    for i, c in enumerate(h.get("checkpoints", [])):
        d = lambda k: c[k] - last[k] if last else 0
        cps.append(dict(k=i, name=c["name"], t_ms=c["t_ms"], task=c["task"], free=c["free"], d=d("free"), largest=c["largest"],
                        free_int=c["free_int"], d_int=d("free_int"), largest_int=c["largest_int"], free_dma=c["free_dma"],
                        d_dma=d("free_dma"), largest_dma=c["largest_dma"], rust=c["rust"], d_rust=d("rust"), rust_allocs=c["rust_allocs"]))
        last = c
    w = h.get("walk", {})
    rep = dict(seq=0, t_s=h.get("t_ms", 0) // 1000,
               tasks=[dict(name=t["name"], core=str(t["core"]) if t["core"] >= 0 else "*", prio=t["prio"], state=t["state"],
                           stack=t["stack"], unused=t["unused"]) for t in h.get("tasks", [])],
               sites=[dict(site=s["site"], live=s["live"], count=s["count"], pcs=[int(p, 16) for p in s["pcs"]]) for s in h.get("sites", [])],
               census=json.dumps({k: v for k, v in w.items() if k != "top_c"}), lines=[f"top_c: {w.get('top_c')}"],
               rust=h.get("rust"))
    return cps, rep, 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("input")
    ap.add_argument("--elf")
    ap.add_argument("--report", type=int)
    ap.add_argument("--sites", type=int, default=48)
    a = ap.parse_args()
    if a.input.endswith(".json"):
        cps, rep, n = parse_json(a.input)
    else:
        cps, rep, n = parse_log(a.input, a.report)

    print("## Checkpoints\n")
    print("| # | step | task | t ms | 8bit free | delta | largest | DMA free | DMA delta | rust live | rust delta |")
    print("|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for c in cps:
        print(f"| {c['k']} | {c['name']} | {c['task']} | {c['t_ms']} | {c['free']:,} | {c['d']:+,} | {c['largest']:,} | "
              f"{c['free_dma']:,} | {c['d_dma']:+,} | {c['rust']:,} | {c['d_rust']:+,} |")
    if not rep:
        print("\n(no complete census in the input)")
        return
    print(f"\n## Report {rep['seq']} at {rep['t_s']} s (of {n})\n")
    print("| task | core | prio | state | stack B | unused B | used B |")
    print("|---|---|---:|---|---:|---:|---:|")
    tot = 0
    for t in sorted(rep["tasks"], key=lambda t: -t["stack"]):
        tot += t["stack"]
        used = t["stack"] - t["unused"] if t["stack"] else ""
        print(f"| {t['name']} | {t['core']} | {t['prio']} | {t['state']} | {t['stack'] or 'static'} | {t['unused']} | {used} |")
    print(f"| **heap stacks** | | | | **{tot:,}** | | |")
    print("\n### Census\n")
    print(rep.get("census", ""))
    print()
    for l in rep["lines"]:
        if re.search(r"heapmap: (region|free blocks|c blocks|cblk|sites )|top_c", l):
            print("    " + l)
    sites = rep["sites"][: a.sites]
    sym = symbolize(a.elf, [p for s in sites for p in s["pcs"]]) if a.elf else {}
    by_owner = defaultdict(lambda: [0, 0, []])
    print("\n### Rust call sites (most live bytes first)\n")
    print("| site | live B | allocs | owner | first frame outside alloc/std | where |")
    print("|---:|---:|---:|---|---|---|")
    for s in sites:
        frames = [f for p in s["pcs"] for f in sym.get(p, [])]
        own, fn, loc = owner_of(frames) if frames else ("?", " ".join(f"{p:#x}" for p in s["pcs"]), "")
        by_owner[own][0] += s["live"]
        by_owner[own][1] += s["count"]
        by_owner[own][2].append(s["site"])
        print(f"| {s['site']} | {s['live']:,} | {s['count']} | {own} | `{fn[:90]}` | {loc} |")
    print("\n### Rust live bytes by owner (the sites above)\n")
    print("| owner | live B | allocs | sites |")
    print("|---|---:|---:|---|")
    for own, (b, n_, ss) in sorted(by_owner.items(), key=lambda kv: -kv[1][0]):
        print(f"| {own} | {b:,} | {n_} | {', '.join(map(str, ss[:12]))} |")


if __name__ == "__main__":
    main()
