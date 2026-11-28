# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""The psi-refresh probe's rows (firmware/src/psi_probe/refresh.rs): fetch
them from a board, parse them, and put them into opendmd-psi/tests/measured.rs.

    uv run tools/psi-refresh.py collect HOST [--out DIR] [--version V] [--wait S]
        Polls http://HOST/filelist-psi-probe.txt (the report the probe writes on
        the card as it goes; OPENDMD_API_KEY=<key> if the board has one) until
        its done line, saving each copy to DIR/raw-latest.txt (and the final
        one to DIR/raw-<time>.txt), then writes DIR/results.json. Never writes
        to the board. Exit 0 only when the probe finished and every row came.

    uv run tools/psi-refresh.py parse FILE... [--out results.json]
        The same results from saved copies or console captures (any prefix
        before `psi-probe:` is dropped; a line whose #check does not match is
        dropped, never misread; the last good copy of each row wins).

    uv run tools/psi-refresh.py apply results.json [--repo DIR] [--dry-run]
        [--no-storm-front]
        Puts the numbers into opendmd-psi/tests/measured.rs and the commit
        each Formula's file was measured at into MEASURED_AT in
        opendmd-psi/tests/estimate.rs: refreshed rows take their file by
        include_str!, the retired rows go (RETIRE below), new rows come
        (ADD). Each row's file must still hash to what the board measured.
        It changes no estimate and no tolerance; its last lines say what to
        run next.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPORT = "filelist-psi-probe.txt"
DONE = "psi-probe: done"
EXPECTED_ROWS = 44  # refresh.rs ROWS (the report's `refresh rows: ... of N` line says too)

# Rows the refresh retires (table, name): no longer measured (see the
# refresh notes in the branch's report).
RETIRE = [
    ("JIT_EFFECTS", "Endless road Both"),  # experiments/road-life: superseded by the bundled road
    ("JIT_EFFECTS", "Endless road Full"),  # experiments/road-life: superseded by the bundled road
    ("CONSOLEB", "Big clock rate (bar in the mask)"),  # experiments/console-b: never bundled
    ("STEPS", "big clock (beta.10)"),  # an old spelling of Big clock; the current one is measured
    ("SPEED2", "Endless road (sprite car)"),  # road-cost's A/B of a road since replaced
    ("SPEED2", "Endless road (car drawn)"),
]
# Rows the refresh adds (table, name): measured by the probe, not in the
# tables before.
ADD = [("JIT_EFFECTS", "Endless road"), ("CONSOLEB", "Storm front")]

LINE = re.compile(r"(psi-probe: refresh(?:-failed)?\|.*) #([0-9a-f]{8})\s*$")
HEADER = re.compile(r"# OpenDMD (\S+) psi-probe report")
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def fnv32(b: bytes) -> int:
    h = 0x811C9DC5
    for c in b:
        h = ((h ^ c) * 0x01000193) & 0xFFFFFFFF
    return h


def fnv64(b: bytes) -> int:
    h = 0xCBF29CE484222325
    for c in b:
        h = ((h ^ c) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def parse_text(text: str, into: dict, failed: dict, meta: dict) -> None:
    """Rows from one copy of the report (or a console capture) into `into`
    (key: table|name|look) and `failed`; `meta`: the firmware version, done."""
    for raw in text.splitlines():
        line = ANSI.sub("", raw)
        m = HEADER.search(line)
        if m:
            meta["firmware"] = m.group(1)
        m = re.search(r"psi-probe: refresh rows: (\d+) measured, (\d+) failed, of (\d+)", line)
        if m:
            meta["of"] = int(m.group(3))
        if DONE in line and "refresh" in line:
            meta["done"] = line[line.index(DONE):].strip()
        i = line.find("psi-probe: refresh")
        if i < 0:
            continue
        m = LINE.match(line[i:])
        if not m:
            continue
        body, check = m.group(1), int(m.group(2), 16)
        if fnv32(body.encode()) != check:
            meta["bad_lines"] = meta.get("bad_lines", 0) + 1
            continue
        f = body.split("|")
        if f[0] == "psi-probe: refresh-failed":
            _, table, name, look, why = f[:4] + ["|".join(f[4:])]
            failed[f"{table}|{name}|{look}"] = {"table": table, "name": name, "look": look, "why": why}
            continue
        _, table, name, look, stem, src, values = f
        h, n = src.split("/")
        vals = {}
        for kv in values.split():
            k, v = kv.split("=")
            vals[k] = int(v)
        key = f"{table}|{name}|{look}"
        into[key] = {
            "table": table, "name": name, "look": None if look == "-" else look,
            "file": f"opendmd-web/formulas/{stem}.psi", "src_fnv64": h, "src_len": int(n),
            "us": vals.get("us"), "values": vals,
        }
        failed.pop(key, None)


def results_of(texts: list[str]) -> dict:
    rows, failed, meta = {}, {}, {}
    for t in texts:
        parse_text(t, rows, failed, meta)
    return {
        "firmware": meta.get("firmware"),
        "done": meta.get("done"),
        "bad_lines": meta.get("bad_lines", 0),
        "of": meta.get("of"),
        "rows": sorted(rows.values(), key=lambda r: (r["table"], r["name"], r["look"] or "")),
        "failed": list(failed.values()),
    }


# The calibration rows' numbers before (measured.rs, unchanged sources).
CAL_BEFORE = {"base gray(x)": 678, "sin (+add)": 1346, "noise (+mul)": 4540, "Rings": 6614, "p0 big": 1429, "p0 tween": 818}


def summary(res: dict) -> str:
    out = [f"firmware {res['firmware']}; done: {res['done'] or 'NO'}; "
           f"{len(res['rows'])} rows, {len(res['failed'])} failed, {res['bad_lines']} lines failed their check"]
    for r in res["rows"]:
        v = r["values"]
        extra = " ".join(f"{k}={v[k]}" for k in ("jit", "vm", "est", "est_still", "est_jit_still") if k in v)
        if r["table"].startswith("CAL_") and r["name"] in CAL_BEFORE:
            b = CAL_BEFORE[r["name"]]
            extra += f"  (calibration: {b} before, {(r['us'] - b) * 100 / b:+.0f} %)"
        out.append(f"  {r['table']:<15} {r['name']:<16} {r['look'] or '':<13} us={r['us']:>6}  {extra}")
    for f in res["failed"]:
        out.append(f"  FAILED {f['table']} {f['name']} {f['look']}: {f['why']}")
    return "\n".join(out)


def cmd_collect(a) -> int:
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    url = f"http://{a.host}/{REPORT}"
    hdr = {"User-Agent": "psi-refresh"}
    if os.environ.get("OPENDMD_API_KEY"):
        hdr["Authorization"] = f"Bearer {os.environ['OPENDMD_API_KEY']}"
    deadline = time.time() + a.wait
    text, res = "", None
    while True:
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=hdr), timeout=20) as r:
                text = r.read().decode("utf-8", "replace")
        except (urllib.error.URLError, OSError) as e:
            print(f"{time.strftime('%H:%M:%S')} {url}: {e}", flush=True)
            text = ""
        if text:
            (out / "raw-latest.txt").write_text(text)
            res = results_of([text])
            mine = res["firmware"] == a.version
            print(f"{time.strftime('%H:%M:%S')} report of {res['firmware']}: {len(text.splitlines())} lines, "
                  f"{len(res['rows'])} rows, {len(res['failed'])} failed, done: {'yes' if res['done'] else 'no'}"
                  + ("" if mine else f" (not {a.version}: waiting for its probe)"), flush=True)
            if mine and res["done"]:
                break
        if time.time() > deadline:
            print(f"gave up after {a.wait} s", file=sys.stderr)
            break
        time.sleep(a.every)
    if text:
        (out / f"raw-{time.strftime('%Y%m%d-%H%M%S')}.txt").write_text(text)
    if res is None:
        return 1
    (out / "results.json").write_text(json.dumps(res, indent=1) + "\n")
    print(summary(res))
    print(f"wrote {out / 'results.json'}")
    ok = res["firmware"] == a.version and res["done"] and not res["failed"] and len(res["rows"]) == (res["of"] or EXPECTED_ROWS)
    if not ok:
        print(f"NOT COMPLETE: want {res['of'] or EXPECTED_ROWS} rows from {a.version}, none failed, and its done line", file=sys.stderr)
    return 0 if ok else 1


def cmd_parse(a) -> int:
    res = results_of([Path(f).read_text(errors="replace") for f in a.files])
    Path(a.out).write_text(json.dumps(res, indent=1) + "\n")
    print(summary(res))
    print(f"wrote {a.out}")
    return 0 if res["done"] and not res["failed"] and len(res["rows"]) == (res["of"] or EXPECTED_ROWS) else 1


# --- apply: a small reader for the tables' Rust ---------------------------

TOKEN = re.compile(
    r'\s+|//[^\n]*|"(?:[^"\\]|\\.)*"|include_str!\(\s*"(?:[^"\\]|\\.)*"\s*\)|[A-Za-z_][A-Za-z0-9_]*|[0-9][0-9_]*|.',
    re.S,
)


def table_span(src: str, name: str) -> tuple[int, int]:
    """The text between a table's `= &[` and its closing `];`."""
    m = re.search(rf"(?:pub )?const {name}: &\[[^\n]*= &\[\n", src)
    if not m:
        raise SystemExit(f"no table {name}")
    depth, i = 1, m.end()
    for t in TOKEN.finditer(src, m.end()):
        s = t.group(0)
        if s in "([":
            depth += 1
        elif s in ")]":
            depth -= 1
            if depth == 0:
                return m.end(), t.start()
        i = t.end()
    raise SystemExit(f"table {name} does not end")


def tuples(src: str, lo: int, hi: int) -> list[dict]:
    """The top-level tuples between lo and hi: span and fields (text)."""
    out, depth, cur, fstart = [], 0, None, None
    for t in TOKEN.finditer(src, lo, hi):
        s = t.group(0)
        if s.isspace() or s.startswith("//"):
            continue
        if s == "(":
            depth += 1
            if depth == 1:
                cur = {"start": t.start(), "fields": []}
                fstart = t.end()
                continue
        elif s == ")":
            depth -= 1
            if depth == 0:
                tail = src[fstart:t.start()].strip()
                if tail:
                    cur["fields"].append(tail)
                cur["end"] = t.end()
                out.append(cur)
                continue
        elif s == "," and depth == 1:
            cur["fields"].append(src[fstart:t.start()].strip())
            fstart = t.end()
    return out


def rust_str(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def unstr(s: str) -> str:
    assert s.startswith('"'), s
    return json.loads(s) if "\\u" not in s else s[1:-1]


def render(fields: list[str], multiline: bool) -> str:
    if multiline:
        return "(\n" + "".join(f"        {f},\n" for f in fields) + "    )"
    return "(" + ", ".join(fields) + ")"


def line_bounds(src: str, start: int, end: int, comments: bool) -> tuple[int, int]:
    """A tuple's whole lines (and its trailing comma), with the `//` lines
    just above it when `comments`."""
    a = src.rfind("\n", 0, start) + 1
    b = src.find("\n", end) + 1
    if comments:
        while True:
            p = src.rfind("\n", 0, a - 1) + 1
            if a > 0 and src[p:a].strip().startswith("//"):
                a = p
            else:
                break
    return a, b


def git(repo: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, text=True).stdout.strip()


def cmd_apply(a) -> int:
    repo = Path(a.repo).resolve()
    res = json.loads(Path(a.results).read_text())
    if not res.get("done") or res.get("failed"):
        print("the results are not a finished probe with no failed rows", file=sys.stderr)
        return 1
    # (The CAL_ rows are the board's calibration, in no table.)
    rows = {(r["table"], r["name"], r["look"]): r for r in res["rows"] if not r["table"].startswith("CAL_")}
    add = [x for x in ADD if not (a.no_storm_front and x[1] == "Storm front")]
    mpath, epath = repo / "opendmd-psi/tests/measured.rs", repo / "opendmd-psi/tests/estimate.rs"
    msrc, esrc = mpath.read_text(), epath.read_text()

    # Each row's file as measured: its bytes must hash to the board's; the
    # commit it was measured at is the last to change it (on HEAD).
    at = {}
    bad = []
    for r in rows.values():
        f = repo / r["file"]
        b = f.read_bytes()
        if f"{fnv64(b):016x}" != r["src_fnv64"] or len(b) != r["src_len"]:
            bad.append(f"{r['table']} {r['name']}: {r['file']} is not the text measured (changed since the probe's build)")
            continue
        at[r["file"]] = git(repo, "log", "-1", "--format=%h", "--", r["file"])
    if bad:
        print("\n".join(bad), file=sys.stderr)
        return 1

    edits = []  # (start, end, text) on msrc
    measured_at = {}  # key -> (path, commit); None: remove
    done = set()
    tables = ["STEPS", "LOOKS", "SPEED2", "EFFECTS", "JIT_EFFECTS", "JIT_LOOKS", "CONSOLEB", "PAINT_FORMULAS"]
    for table in tables:
        lo, hi = table_span(msrc, table)
        insert_at = None
        for t in tuples(msrc, lo, hi):
            fl = t["fields"]
            name = unstr(fl[0])
            looks = table in ("LOOKS", "JIT_LOOKS")
            look = f"{unstr(fl[2])} {fl[3]}" if looks else None
            multiline = "\n" in msrc[t["start"]:t["end"]]
            if (table, name) in RETIRE:
                s, e = line_bounds(msrc, t["start"], t["end"], comments=True)
                edits.append((s, e, ""))
                insert_at = insert_at if insert_at is not None else s
                measured_at[f"{table} {name}"] = None
                done.add((table, name, look))
                continue
            r = rows.get((table, name, look))
            if r is None:
                continue
            nf = list(fl)
            nf[1] = f'include_str!("../../{r["file"]}")'
            if table == "PAINT_FORMULAS":
                nf[2], nf[3] = str(r["values"]["vm"]), str(r["values"]["jit"])
            else:
                nf[-1] = str(r["us"])
            s, e = t["start"], t["end"]
            edits.append((s, e, render(nf, multiline)))
            key = f"{table} {name}" + (f" {look}" if table == "LOOKS" else "")
            measured_at[key] = (r["file"], at[r["file"]])
            done.add((table, name, look))
        # New rows: where the first retired row of the table was, else last.
        new = [x for x in add if x[0] == table]
        for _, name in new:
            r = rows.get((table, name, None))
            if r is None:
                print(f"no measurement for the new row {table} {name}", file=sys.stderr)
                return 1
            text = render([rust_str(name), f'include_str!("../../{r["file"]}")', str(r["us"])], True)
            pos = insert_at if insert_at is not None else msrc.rfind("\n", lo, hi) + 1
            edits.append((pos, pos, f"    {text},\n"))
            measured_at[f"{table} {name}"] = (r["file"], at[r["file"]])
            done.add((table, name, None))
    missing = [k for k in rows if k not in done]
    if missing:
        print(f"rows measured but not in any table (left out): {missing}", file=sys.stderr)
    # Overlapping edits would be a bug in the above.
    edits.sort(key=lambda x: (x[0], x[1]))
    for (s1, e1, _), (s2, _, _) in zip(edits, edits[1:]):
        assert e1 <= s2 or s1 == s2 == e1, "overlapping edits"
    for s, e, text in reversed(edits):
        msrc = msrc[:s] + text + msrc[e:]
    # Constants no row uses any more (the pinned sources of the retired and
    # refreshed rows).
    for const in ("ROAD_SPRITE", "ROAD_DRAWN", "STARFIELD", "CUBE"):
        uses = sum(len(re.findall(rf"\b{const}\b", p.read_text() if p != mpath else msrc))
                   for p in (repo / "opendmd-psi/tests").glob("*.rs"))
        if uses == 1:
            msrc = re.sub(r'pub const ' + const + r': &str = "(?:[^"\\]|\\.)*";\n', "", msrc)

    # MEASURED_AT in estimate.rs.
    lo, hi = table_span(esrc, "MEASURED_AT")
    eedits = []
    for t in tuples(esrc, lo, hi):
        key = unstr(t["fields"][0])
        if key not in measured_at:
            continue
        v = measured_at.pop(key)
        s, e = line_bounds(esrc, t["start"], t["end"], comments=False)
        if v is None:
            eedits.append((s, e, ""))
        else:
            eedits.append((s, e, f"    ({rust_str(key)}, {rust_str(v[0])}, {rust_str(v[1])}),\n"))
    tail = "".join(f"    ({rust_str(k)}, {rust_str(v[0])}, {rust_str(v[1])}),\n" for k, v in measured_at.items() if v)
    end = esrc.rfind("\n", lo, hi) + 1
    eedits.append((end, end, tail))
    for s, e, text in sorted(eedits, key=lambda x: x[0], reverse=True):
        esrc = esrc[:s] + text + esrc[e:]

    if a.dry_run:
        import difflib
        for p, new in ((mpath, msrc), (epath, esrc)):
            sys.stdout.writelines(difflib.unified_diff(p.read_text().splitlines(True), new.splitlines(True), str(p), str(p)))
        return 0
    mpath.write_text(msrc)
    epath.write_text(esrc)
    snap = repo / "opendmd-psi/tests/neon-city-measured.psi"
    still = [p for p in (repo / "opendmd-psi/tests").glob("*.rs") if "neon-city-measured.psi" in p.read_text()]
    print(f"updated {mpath} and {epath}")
    print("next: the table comments (probe names, dates, which rows are from which run) by hand;")
    if snap.exists() and not still:
        print(f"      {snap} is no longer used (git rm it, and the stale test's doc line naming it);")
    print("      make psi-stale (no stale rows), then")
    print("      cd opendmd-psi && RUSTUP_TOOLCHAIN=stable cargo test --test estimate -- --nocapture")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("collect")
    c.add_argument("host")
    c.add_argument("--out", default=".")
    c.add_argument("--version", default="1.1.0-beta.4.probe1", help="the probe build's firmware version")
    c.add_argument("--wait", type=int, default=1200, help="seconds to wait for the done line")
    c.add_argument("--every", type=int, default=15, help="seconds between reads")
    p = sub.add_parser("parse")
    p.add_argument("files", nargs="+")
    p.add_argument("--out", default="results.json")
    ap_ = sub.add_parser("apply")
    ap_.add_argument("results")
    ap_.add_argument("--repo", default=str(Path(__file__).resolve().parent.parent))
    ap_.add_argument("--dry-run", action="store_true")
    ap_.add_argument("--no-storm-front", action="store_true", help="leave Storm front out of CONSOLEB")
    a = ap.parse_args()
    return {"collect": cmd_collect, "parse": cmd_parse, "apply": cmd_apply}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
