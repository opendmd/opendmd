# /// script
# requires-python = ">=3.11"
# dependencies = ["pyyaml>=6"]
# ///
"""The public repo's gallery check, run as GitHub runs it on
`pull_request_target`, against pull requests to a local bare repository:

    uv run tools/gallery-check-test.py [--keep DIR] [-v]

The bare repo gets this checkout's site/public-repo/ and the vendored
checker (tools/gallery-repo-publish.sh, against a stand-in site). Each case is a
fork branch with GitHub's `refs/pull/<n>/merge` (main as the first parent)
and `refs/pull/<n>/head`; the workflow taken from **main** (the base: what
`pull_request_target` runs, whatever the pull request did to its own copy)
is linted (the trigger, `permissions: {}`, no checkout, actions pinned by
SHA, no `${{ }}` in a script) and its `run:` steps executed with the
event's env. Cases that edit the workflow or the checker in the pull
request must still fail. Nothing leaves the machine: the repo URL is a
file:// one and the API URL goes nowhere. Run by `make test`.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tools/fixtures"
HDR = "-- author: @contributor\n-- licence: CC0-1.0\nname \"Spiral sweep\"\nabout \"A rainbow spiral turning around the middle\"\n"
FORMULA = "loop 8\na2 = atan2(y - 0.5, (x - 0.5)*4)\nd = len((x - 0.5)*4, y - 0.5)\nhsv(frac(a2 + d*0.6 - t/8), 1, 1)\n"
GOOD = HDR + FORMULA
BROKEN = HDR + "loop 8\ngray(tme)\n"


def sh(*args: str, cwd: Path | None = None, check: bool = True, env: dict | None = None) -> subprocess.CompletedProcess:
    e = {**os.environ, "GIT_AUTHOR_NAME": "Contributor", "GIT_AUTHOR_EMAIL": "c@example.invalid",
         "GIT_COMMITTER_NAME": "Contributor", "GIT_COMMITTER_EMAIL": "c@example.invalid", **(env or {})}
    return subprocess.run(args, cwd=cwd, check=check, capture_output=True, text=True, env=e)


def write(root: Path, rel: str, text: str | bytes) -> None:
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(text.read_bytes() if isinstance(text, Path) else text if isinstance(text, bytes) else text.encode())


# (title, edit(fork), expect pass?, a phrase the output must have)
def cases():
    def symlink(f: Path):
        (f / "gallery/link.psi").symlink_to("../.github/workflows/gallery-check.yml")

    def rm_readme(f: Path):
        sh("git", "rm", "-q", "gallery/README.md", cwd=f)

    def workflow_edit(f: Path):
        write(f, ".github/workflows/gallery-check.yml", "name: Gallery check\non: pull_request_target\njobs:\n  check:\n"
              "    name: Gallery check (trusted)\n    runs-on: ubuntu-24.04\n    steps:\n      - run: exit 0\n")
        write(f, "gallery/broken.psi", BROKEN)

    def checker_edit(f: Path):
        lib = f / ".github/gallery-check/opendmd-gallery/src/lib.rs"
        text = lib.read_text().replace("pub fn check_file(slug: &str, bytes: &[u8]) -> Result<Entry, Vec<String>> {",
                                       "pub fn check_file(slug: &str, bytes: &[u8]) -> Result<Entry, Vec<String>> {\n"
                                       "    if true { return check_file_as(slug, b\"-- author: x\\n-- licence: CC0-1.0\\nname \\\"x\\\"\\nabout \\\"x\\\"\\ngray(x)\\n\", true); }")
        lib.write_text(text)
        write(f, "gallery/broken.psi", BROKEN)

    return [
        ("a good formula", lambda f: write(f, "gallery/spiral-sweep.psi", GOOD), True, "ok   \"gallery/spiral-sweep.psi\""),
        ("outside gallery/: path traversal, a folder, a dotted name", lambda f: (write(f, "x/evil.psi", GOOD), write(f, "gallery/sub/nested.psi", GOOD),
                                                                               write(f, "gallery/..psi", GOOD)), False, "only files in gallery/"),
        ("a huge file", lambda f: write(f, "gallery/huge.psi", GOOD + "-" * 40000), False, "36864 at most"),
        ("over 4,500 bytes once compiled (7.7 KB of source: tables, palettes, paint colours)", lambda f: write(f, "gallery/long-compiled.psi", FIXTURES / "gallery-over-4500.psi"), False, "4863 bytes once prepared, 4500 at most"),
        ("heavy on the VM but light with the JIT (the gate counts the tier)", lambda f: write(f, "gallery/jit-only.psi", FIXTURES / "gallery-jit-only.psi"), True, "ok   \"gallery/jit-only.psi\""),
        ("a 12 KB file that compiles small", lambda f: write(f, "gallery/long.psi", GOOD + "-- padding padding\n" * 600), True, "ok   \"gallery/long.psi\""),
        ("HTML in the author", lambda f: write(f, "gallery/html.psi", GOOD.replace("@contributor", "<img src=x onerror=alert(1)>")), False, "may not contain < or >"),
        ("HTML in the name", lambda f: write(f, "gallery/html2.psi", GOOD.replace("Spiral sweep", "<img src=x onerror=alert(1)>")), False, "doesn't compile"),
        ("the old header", lambda f: write(f, "gallery/old.psi", "-- name: Spiral sweep\n-- description: A spiral\n-- author: @contributor\n-- licence: CC0-1.0\n" + FORMULA), False, "the header is two lines now"),
        ("-- bundle in a contribution", lambda f: write(f, "gallery/bundle.psi", HDR.replace("CC0-1.0\n", "CC0-1.0\n-- bundle\n", 1) + FORMULA), False, "-- bundle"),
        ("bidi characters", lambda f: write(f, "gallery/bidi.psi", GOOD.replace("Spiral sweep", "Spiral " + chr(0x202E) + "teews")), False, "direction"),
        ("does not compile", lambda f: write(f, "gallery/broken.psi", BROKEN), False, "doesn't compile"),
        ("too heavy", lambda f: write(f, "gallery/heavy.psi", HDR + "loop 8\nn = noise(x*8, y*4, t) + noise(x*4, y*8, t) + noise(x*2, y*2, t)\n"
                                       "m = 1/(x + 0.1) + 1/(y + 0.2) + 1/(x + y + 0.3)\npal(fire, frac(n + m))\n"), False, "too heavy"),
        ("a good formula plus index.html", lambda f: (write(f, "gallery/spiral-sweep.psi", GOOD), write(f, "index.html", "pwned")), False, "\"index.html\": only files in gallery/"),
        ("the PR rewrites the workflow to pass, with a broken formula", workflow_edit, False, "doesn't compile"),
        ("the PR rewrites the checker to pass, with a broken formula", checker_edit, False, "doesn't compile"),
        ("a symlink", symlink, False, "symbolic link"),
        ("a licence not on the list", lambda f: write(f, "gallery/gpl.psi", GOOD.replace("CC0-1.0", "GPL-3.0")), False, "licence must be one of"),
        ("binary content", lambda f: write(f, "gallery/image.psi", b"\x89PNG\r\n\x1a\n\0\0"), False, "not plain text"),
        ("deleting a file", rm_readme, False, "removes a file"),
        ("posing as OpenDMD", lambda f: write(f, "gallery/fake.psi", GOOD.replace("@contributor", "Ｏｐｅｎ DMD")), False, "OpenDMD's or a maintainer's"),
        ("Windows line endings", lambda f: write(f, "gallery/crlf.psi", GOOD.replace("\n", "\r\n")), False, "Windows line endings"),
    ]


def lint(wf: dict) -> list[str]:
    bad = []
    on = wf.get("on", wf.get(True))
    if list(on) != ["pull_request_target"] or on["pull_request_target"] != {"branches": ["main"]}:
        bad.append(f"trigger {on!r}")
    if wf.get("permissions") != {}:
        bad.append(f"permissions {wf.get('permissions')!r}")
    for job in wf["jobs"].values():
        if job.get("permissions") not in (None, {}):
            bad.append("a job asks for permissions")
        for st in job["steps"]:
            u = st.get("uses")
            if u and ("checkout" in u or not re.fullmatch(r"[\w.-]+/[\w.-]+@[0-9a-f]{40}", u)):
                bad.append(f"uses {u}")
            if "${{" in st.get("run", ""):
                bad.append(f"an expression in the script of {st.get('name')!r}")
    return bad


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--keep", type=Path, help="work here and keep it (previews, summaries)")
    ap.add_argument("-v", action="store_true", help="print each run's output")
    a = ap.parse_args()
    tmp = Path(a.keep or tempfile.mkdtemp(prefix="gallery-check-"))
    if a.keep and tmp.exists():
        shutil.rmtree(tmp)
    tmp.mkdir(parents=True, exist_ok=True)
    failures = 0
    try:
        bare = tmp / "opendmd.github.io.git"
        sh("git", "init", "-q", "--bare", "-b", "main", str(bare))
        site = tmp / "site"
        write(site, "install/release.json", '{"version": "9.9.9"}\n')
        write(site, "index.html", "<p>site</p>\n")
        sh("tools/gallery-repo-publish.sh", str(site), str(bare), cwd=ROOT,
           env={"GALLERY_REPO_WORK": str(tmp / "work"), "GALLERY_REPO_ALLOW_DIRTY": "1"})
        fork = tmp / "fork"
        sh("git", "clone", "-q", str(bare), str(fork))
        target = tmp / "cargo-target"
        for n, (title, edit, want_ok, phrase) in enumerate(cases(), 1):
            sh("git", "checkout", "-q", "-B", f"pr-{n}", "origin/main", cwd=fork)
            edit(fork)
            sh("git", "add", "-A", cwd=fork)
            sh("git", "commit", "-q", "--allow-empty", "-m", title, cwd=fork)
            head = sh("git", "rev-parse", "HEAD", cwd=fork).stdout.strip()
            sh("git", "checkout", "-q", "--detach", "origin/main", cwd=fork)
            sh("git", "merge", "-q", "--no-ff", "--no-edit", "-m", f"Merge {head} into main", head, cwd=fork)
            sh("git", "push", "-q", "-f", "origin", f"HEAD:refs/pull/{n}/merge", f"{head}:refs/pull/{n}/head", cwd=fork)
            ok, out = run_workflow(bare, tmp / f"run-{n}", n, head, target)
            good = ok == want_ok and phrase in out
            failures += not good
            print(f"  {'ok  ' if good else 'FAIL'} PR {n}, {title}: {'passed' if ok else 'failed'}")
            if a.v or not good:
                print("".join(f"       | {l}\n" for l in out.strip().splitlines() if l.startswith(("ok ", "FAIL", "note", "::error"))))
        # A merge GitHub hasn't redone for the latest push: the check says re-run.
        ok, out = run_workflow(bare, tmp / "run-stale", 1, "f" * 40, target)
        good = not ok and "re-run" in out
        failures += not good
        print(f"  {'ok  ' if good else 'FAIL'} a stale merge (head moved on): {'passed' if ok else 'failed, asks to re-run'}")
    finally:
        if not a.keep:
            shutil.rmtree(tmp, ignore_errors=True)
    print(f"gallery-check-test: {'ok' if not failures else f'{failures} failures'}")
    return 1 if failures else 0


def run_workflow(bare: Path, rt: Path, pr: int, head: str, target: Path) -> tuple[bool, str]:
    """The base's workflow (main's copy), its run: steps in order, as a
    runner would: bash -e, the job's env, RUNNER_TEMP. Stops at the first
    failing step. Returns (passed, output)."""
    text = sh("git", "--git-dir", str(bare), "show", "main:.github/workflows/gallery-check.yml").stdout
    wf = yaml.safe_load(text)
    bad = lint(wf)
    if bad:
        return False, "lint: " + "; ".join(bad)
    rt.mkdir(parents=True)
    env = {"PR": str(pr), "HEAD_SHA": head, "REPO_URL": f"file://{bare}", "API_URL": "http://127.0.0.1:9/repos/x",
           "RUNNER_TEMP": str(rt), "GITHUB_STEP_SUMMARY": str(rt / "summary.md"), "GALLERY_WAIT": "0"}
    out = ""
    for st in wf["jobs"]["check"]["steps"]:
        if "run" not in st:
            continue
        script = st["run"]
        if "cargo build" in script:  # one build folder for every case (the same base checker)
            script = script.replace('"$RUNNER_TEMP/target"', str(target))
        script = script.replace('"$RUNNER_TEMP/target/release', f'"{target}/release')
        r = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", script], capture_output=True, text=True,
                           env={**os.environ, **env}, cwd=rt)
        out += r.stdout + r.stderr
        if r.returncode != 0:
            return False, out
    return True, out


if __name__ == "__main__":
    sys.exit(main())
