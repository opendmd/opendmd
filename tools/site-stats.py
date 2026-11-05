# /// script
# requires-python = ">=3.11"
# ///
"""The site's usage counts in the terminal (make site-stats): the numbers
/stats shows, read from the opendmd-site D1 (the counts table,
site/worker/migrations/0006_counts.sql).

    uv run tools/site-stats.py                  production, through cf d1 query (the owner's login)
    uv run tools/site-stats.py --sqlite FILE    a local D1's SQLite file (cf dev's, under
                                                site/worker/.wrangler/state), opened read-only

Read-only: it only ever sends SELECT statements (checked before each one
goes), and a local file is opened with mode=ro.
"""

import argparse
import datetime
import json
import re
import sqlite3
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKER = ROOT / "site/worker"


def d1_id() -> str:
    m = re.search(r'DB: \{[^}]*\bid: "([0-9a-f-]{36})"', (WORKER / "cloudflare.config.ts").read_text())
    if not m:
        raise SystemExit("cloudflare.config.ts: no DB binding with an id")
    return m.group(1)


def read_only(sql: str) -> str:
    s = sql.strip()
    if not re.fullmatch(r"select\b[^;]*", s, re.I | re.S) or re.search(r"\b(insert|update|delete|drop|alter|create|replace|pragma|attach|vacuum)\b", s, re.I):
        raise SystemExit(f"refusing to send anything but one SELECT: {s[:60]}")
    return s


def rows_of(out) -> list[dict]:
    """The rows in what cf d1 query prints (the D1 API's result: a list of
    {results: [...]}, maybe inside {result: ...})."""
    if isinstance(out, dict):
        for k in ("result", "results"):
            if k in out:
                return rows_of(out[k])
        return [out]
    if isinstance(out, list):
        if all(isinstance(x, dict) and "results" in x for x in out):
            return [r for x in out for r in x["results"]]
        return [x for x in out if isinstance(x, dict)]
    return []


class Remote:
    def __init__(self):
        self.id = d1_id()

    def query(self, sql: str) -> list[dict]:
        r = subprocess.run(["npx", "--no-install", "cf", "d1", "query", self.id, "--sql", read_only(sql)],
                           cwd=WORKER, capture_output=True, text=True, stdin=subprocess.DEVNULL)
        if r.returncode:
            raise SystemExit(f"cf d1 query failed ({r.returncode}):\n{r.stderr or r.stdout}")
        # (any notice cf prints before the JSON skipped)
        starts = [k for k in (r.stdout.find("["), r.stdout.find("{")) if k >= 0]
        text = r.stdout[min(starts):] if starts else ""
        try:
            return rows_of(json.loads(text))
        except json.JSONDecodeError:
            raise SystemExit(f"cf d1 query printed something that isn't JSON:\n{r.stdout[:400]}")


class Local:
    def __init__(self, path: Path):
        self.db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
        self.db.row_factory = sqlite3.Row

    def query(self, sql: str) -> list[dict]:
        return [dict(r) for r in self.db.execute(read_only(sql))]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--sqlite", type=Path, help="a local D1's SQLite file instead of production")
    a = ap.parse_args()
    db = Local(a.sqlite) if a.sqlite else Remote()
    today = datetime.datetime.now(datetime.UTC).date()
    since30 = (today - datetime.timedelta(days=29)).isoformat()
    tot = {r["kind"]: r for r in db.query(
        "select kind, sum(n) as n, sum(case when day >= '%s' then n else 0 end) as recent, min(day) as first "
        "from counts group by kind" % since30)}
    i, c = tot.get("install", {}), tot.get("check", {})
    first = min([x for x in (i.get("first"), c.get("first")) if x], default=None)
    print(f"OpenDMD usage (opendmd.dev/stats){f', counted since {first}' if first else ''}; days are UTC, today {today}")
    print(f"  installs        {i.get('n') or 0:>8}  ({i.get('recent') or 0} in the last 30 days)")
    print(f"  update checks   {c.get('n') or 0:>8}  ({c.get('recent') or 0} in the last 30 days)")
    weeks = db.query("select strftime('%Y-%W', day) as week, min(day) as first, sum(n) as n from counts "
                     "where kind = 'install' and day >= date('now', '-182 days') group by week order by week")
    if weeks:
        print("\ninstalls a week (the last 26 weeks with any)")
        top = max(w["n"] for w in weeks)
        for w in weeks:
            print(f"  {w['first']}  {w['n']:>6}  {'#' * max(1, round(30 * w['n'] / top))}")
    for kind, title, since in (("check", "update checks by version, last 30 days", since30), ("install", "installs by release, all time", "0000")):
        rows = db.query(f"select version, variant, sum(n) as n from counts where kind = '{kind}' and day >= '{since}' "
                        "group by version, variant order by version desc, variant")
        if rows:
            print(f"\n{title}")
            for r in rows:
                print(f"  {r['version']:<24} {r['variant']:<8} {r['n']:>6}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
