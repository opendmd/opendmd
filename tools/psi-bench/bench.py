# /// script
# requires-python = ">=3.11"
# ///
"""Can a model write Psi from a prompt? The benchmark.

For each (model, prompt): the model gets a reference (--context) and the
prompt, writes a formula, and `psi check` answers; up to --rounds more
turns to fix errors or a formula too heavy for the panel. A compiled
formula's contact sheet is then judged against the prompt by a vision
model (1-5). Everything goes through `pi` (no tools, no project context).

    uv run tools/psi-bench/bench.py --context docs/psi-for-models.md --out target/psi-bench/base
    uv run tools/psi-bench/bench.py --summary target/psi-bench/base

Results: OUT/results.jsonl (one line a run), OUT/<model>/<id>/ (each
round's formula and checker output, the sheet), OUT/summary.md.
"""

import argparse
import concurrent.futures as cf
import json
import os
import re
import subprocess
import sys
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PSI = os.environ.get("PSI", str(ROOT / "opendmd-psi-cli/target/release/psi"))
MODELS = {
    "haiku": "anthropic/claude-haiku-5-5",
    "sonnet": "anthropic/claude-sonnet-5-5",
}
JUDGE = "anthropic/claude-sonnet-5-5"

PREAMBLE = """You write Formulas in Psi, the language of OpenDMD, a 128 x 32 \
LED panel. The reference below is everything there is to know about the \
language. Write one formula that does what the user asks, as well as the \
panel can show it, and light enough for the panel (the checker tells you \
its cost). When the request starts with "Over the GIF", the formula is \
drawn over an animated GIF that is playing (it is the panel's look).

Reply with exactly one ```psi code block holding the whole formula, and \
nothing else. A checker compiles it; if it reports a problem, reply with \
the whole corrected formula in the same way.

"""


def pi(model, session_dir, session_id, system, message, images=()):
    cmd = [
        "pi", "-p", "--no-tools", "--no-context-files", "--no-extensions",
        "--no-skills", "--no-prompt-templates", "--no-mcp", "--no-themes",
        "--model", model, "--session-dir", str(session_dir),
        "--session-id", session_id,
    ]
    if system is not None:
        cmd += ["--system-prompt", system]
    cmd += [f"@{p}" for p in images]
    cmd.append(message)
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=900)
    if r.returncode != 0:
        raise RuntimeError(f"pi failed: {r.stderr[-800:]}")
    return r.stdout


def cost(session_dir):
    total = 0.0
    tokens = 0
    for f in Path(session_dir).glob("*.jsonl"):
        for line in f.read_text().splitlines():
            try:
                e = json.loads(line)
            except json.JSONDecodeError:
                continue
            msg = e.get("message") if isinstance(e, dict) else None
            u = (msg or {}).get("usage") if isinstance(msg, dict) else None
            if u:
                total += u.get("cost", {}).get("total", 0.0)
                tokens += u.get("totalTokens", 0)
    return total, tokens


def code_of(reply):
    m = re.findall(r"```(?:psi)?[ \t]*\n(.*?)```", reply, re.S)
    if m:
        return m[-1].strip() + "\n"
    return reply.strip() + "\n"


def check(path, over, sheet):
    cmd = [PSI, "check", str(path), "--out", str(sheet), "--frames", "8", "--scale", "2"]
    if over:
        cmd.append("--over")
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    out = (r.stdout + r.stderr).strip()
    m = re.search(r"panel: ~([\d.]+) ms", out)
    ms = float(m.group(1)) if m else None
    return r.returncode == 0 and ms is not None, ms, out


def run_one(model_key, item, args, system):
    out = Path(args.out) / model_key / item["id"]
    out.mkdir(parents=True, exist_ok=True)
    sess = out / "session"
    sid = str(uuid.uuid4())
    over = item["kind"] == "overlay"
    budget = 12.0 if over else 8.0
    message = item["prompt"]
    rec = {"model": model_key, "id": item["id"], "kind": item["kind"], "rounds": 0}
    ok = False
    ms = None
    for rnd in range(args.rounds + 1):
        reply = pi(MODELS[model_key], sess, sid, system if rnd == 0 else None, message)
        src = code_of(reply)
        f = out / f"round{rnd}.psi"
        f.write_text(src)
        sheet = out / "sheet.png"
        ok, ms, report = check(f, over, sheet)
        (out / f"round{rnd}.check.txt").write_text(report)
        rec["rounds"] = rnd
        if ok and ms <= budget:
            break
        if ok:
            message = (f"The checker says:\n{report}\n\nIt compiles, but it is too heavy "
                       f"for the panel: ~{ms} ms a frame, and the budget is {budget:g} ms. "
                       "Make it lighter and reply with the whole formula again.")
        else:
            message = (f"The checker says:\n{report}\n\n"
                       "Fix it and reply with the whole formula again.")
    rec["compiles"] = ok
    rec["ms"] = ms
    rec["fits"] = bool(ok and ms is not None and ms <= budget)
    rec["code"] = src
    rec["cost"], rec["tokens"] = cost(sess)
    if ok and not args.no_judge:
        rec.update(judge(item, out, sheet, over))
    return rec


def judge(item, out, sheet, over):
    sess = out / "judge"
    note = (" The formula is drawn over a sample picture standing in for the GIF."
            if over else "")
    msg = (f"This image is a contact sheet of an animation on a 128 x 32 LED panel: "
           f"8 frames spread across its loop, left to right, then the next row.{note}\n\n"
           f"The request was: \"{item['prompt']}\"\n\n"
           "Score how well the animation does what was asked, 1 to 5 (1: nothing like it, "
           "3: recognisably that but crude or with something wrong, 5: just what was asked, "
           "nicely done). Be strict: does the motion between frames make sense for the request (things that should move, move; things that should track each other, do)? Judge only from what you see. Reply with JSON only: "
           '{"score": N, "why": "one sentence"}')
    try:
        reply = pi(JUDGE, sess, str(uuid.uuid4()), "You judge pixel-art animations.", msg, [sheet])
        m = re.search(r"\{.*\}", reply, re.S)
        j = json.loads(m.group(0))
        c, _ = cost(sess)
        return {"score": int(j["score"]), "why": j.get("why", ""), "judge_cost": c}
    except Exception as e:  # noqa: BLE001
        return {"score": None, "why": f"judge failed: {e}"}


def summary(out):
    out = Path(out)
    recs = [json.loads(l) for l in (out / "results.jsonl").read_text().splitlines() if l]
    lines = ["# Psi benchmark: " + out.name, ""]
    lines.append("| model | kind | runs | compiles | fits | first try | mean score | ≥4 | cost $ |")
    lines.append("|---|---|---:|---:|---:|---:|---:|---:|---:|")
    for m in sorted({r["model"] for r in recs}):
        for kind in ["shader", "scene", "overlay", "all"]:
            rs = [r for r in recs if r["model"] == m and (kind == "all" or r["kind"] == kind)]
            if not rs:
                continue
            sc = [r["score"] if r.get("score") else 1 for r in rs]
            lines.append(
                f"| {m} | {kind} | {len(rs)} | {sum(r['compiles'] for r in rs)} | "
                f"{sum(r['fits'] for r in rs)} | "
                f"{sum(r['fits'] and r['rounds'] == 0 for r in rs)} | "
                f"{sum(sc)/len(sc):.2f} | {sum(s >= 4 for s in sc)} | "
                f"{sum(r['cost'] + r.get('judge_cost', 0) for r in rs):.2f} |")
    lines += ["", "Score 1 counted for a formula that never compiled.", "",
              "| model | id | rounds | ms | score | why |", "|---|---|---:|---:|---:|---|"]
    for r in sorted(recs, key=lambda r: (r["model"], r["kind"], r["id"])):
        lines.append(f"| {r['model']} | {r['id']} | {r['rounds']} | {r['ms']} | "
                     f"{r.get('score')} | {r.get('why', '').replace('|', '/')} |")
    (out / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines[:20]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--context", help="the reference given to the model")
    ap.add_argument("--out", required=False)
    ap.add_argument("--models", default="haiku,sonnet")
    ap.add_argument("--only", help="comma-separated prompt ids")
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--no-judge", action="store_true")
    ap.add_argument("--summary")
    args = ap.parse_args()
    if args.summary:
        summary(args.summary)
        return
    if not (args.context and args.out):
        ap.error("--context and --out are needed")
    items = json.loads((Path(__file__).parent / "prompts.json").read_text())
    if args.only:
        keep = set(args.only.split(","))
        items = [i for i in items if i["id"] in keep]
    system = PREAMBLE + Path(args.context).read_text()
    Path(args.out).mkdir(parents=True, exist_ok=True)
    res = Path(args.out) / "results.jsonl"
    jobs = [(m, i) for m in args.models.split(",") for i in items]
    with cf.ThreadPoolExecutor(args.jobs) as ex, res.open("a") as fh:
        futs = {ex.submit(run_one, m, i, args, system): (m, i["id"]) for m, i in jobs}
        for fu in cf.as_completed(futs):
            m, iid = futs[fu]
            try:
                rec = fu.result()
            except Exception as e:  # noqa: BLE001
                print(f"{m} {iid}: FAILED {e}", file=sys.stderr)
                continue
            fh.write(json.dumps(rec) + "\n")
            fh.flush()
            print(f"{m:6} {iid:15} compiles={rec['compiles']} ms={rec['ms']} "
                  f"rounds={rec['rounds']} score={rec.get('score')} ${rec['cost']:.3f}")
    summary(args.out)


if __name__ == "__main__":
    main()
