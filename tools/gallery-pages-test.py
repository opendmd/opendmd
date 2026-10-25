# /// script
# requires-python = ">=3.11"
# dependencies = ["pyyaml>=6", "markdown>=3.6"]
# ///
"""The public repo's Pages workflow (site/public-repo/.github/workflows/
pages.yml), run as GitHub runs it on a push to main, against a local bare
repository standing in for opendmd.github.io:

    uv run tools/gallery-pages-test.py [--site DIR] [--keep DIR] [-v]

1. A release's site: tools/site.py's gallery page (`write_gallery`, the code
   `make site` runs) over the web app's examples and the contributions then
   in the public repo, beside a stand-in for the rest of the site; or, with
   `--site`, a real `make site` output (its formulas/ is rebuilt here with
   that gallery, as release.yml builds it). It is published to the bare
   repo with tools/site-publish.sh, which installs the workflow and the
   vendored checker and generator.
2. main's copy of the workflow is linted (push to main and dispatch only,
   `permissions: {}` with read-only build and pages/id-token deploy, the
   github-pages environment, pinned actions, no checkout, no `${{ }}` in a
   script) and its build job's `run:` steps executed with a runner's env.
   The upload and deploy actions are not run: the folder the upload step
   names is the artifact.
3. Determinism: the artifact must be the release's site, byte for byte
   (formulas/ included), whenever the gallery is the one the release built.
4. A contribution merged to main appears (its tile and thumbnail), and the
   artifact equals a release site built with that gallery.
5. Hostile files merged straight to main (a markup name, a link out of
   gallery/, a folder, a link at the top of the site) are left out, and the
   rest still deploys.

Nothing leaves the machine. Run by `make test`.
"""

import argparse
import filecmp
import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
WF = ".github/workflows/pages.yml"
GALLERY = ROOT / "opendmd-gallery"
SPIRAL = ("-- name: Spiral sweep\n-- description: A rainbow spiral turning around the middle\n-- author: @contributor\n"
          "-- licence: CC0-1.0\nloop 8\na2 = atan2(y - 0.5, (x - 0.5)*4)\nd = len((x - 0.5)*4, y - 0.5)\n"
          "hsv(frac(a2 + d*0.6 - t/8), 1, 1)\n")
RINGS = ("-- name: Rainbow rings\n-- description: Rings rippling out from the middle\n-- author: @someone\n"
         "-- licence: CC-BY-4.0\n-- values: a=0.3\nloop 4\nd = len((x - 0.5)*4, y - 0.5)\npal(rainbow, frac(d*1.5*(a + 0.5) - t/2))\n")
EVIL = SPIRAL.replace("Spiral sweep", "<script>alert(1)</script>")


def sh(*args, cwd=None, env=None, check=True) -> subprocess.CompletedProcess:
    e = {**os.environ, "GIT_AUTHOR_NAME": "Test", "GIT_AUTHOR_EMAIL": "test@example.invalid",
         "GIT_COMMITTER_NAME": "Test", "GIT_COMMITTER_EMAIL": "test@example.invalid", **(env or {})}
    r = subprocess.run([str(a) for a in args], cwd=cwd, env=e, capture_output=True, text=True)
    if check and r.returncode:
        raise SystemExit(f"{' '.join(map(str, args))}: exit {r.returncode}\n{r.stdout}{r.stderr}")
    return r


def load_site_py():
    sys.path.insert(0, str(ROOT / "tools"))
    spec = importlib.util.spec_from_file_location("site_build", ROOT / "tools/site.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def lint(wf: dict) -> list[str]:
    bad = []
    on = wf.get("on", wf.get(True))
    if on != {"push": {"branches": ["main"]}, "workflow_dispatch": None}:
        bad.append(f"trigger {on!r}")
    if wf.get("permissions") != {}:
        bad.append(f"top-level permissions {wf.get('permissions')!r}")
    if wf.get("concurrency") != {"group": "pages", "cancel-in-progress": False}:
        bad.append(f"concurrency {wf.get('concurrency')!r}")
    jobs = wf.get("jobs", {})
    if set(jobs) != {"build", "deploy"}:
        bad.append(f"jobs {sorted(jobs)}")
        return bad
    want = {"build": {"contents": "read"}, "deploy": {"pages": "write", "id-token": "write"}}
    for name, job in jobs.items():
        if job.get("permissions") != want[name]:
            bad.append(f"{name}: permissions {job.get('permissions')!r}")
        if job.get("if") != "github.ref == 'refs/heads/main'":
            bad.append(f"{name}: if {job.get('if')!r}")
        for st in job["steps"]:
            u = st.get("uses")
            if u and ("checkout" in u or not re.fullmatch(r"actions/[\w.-]+@[0-9a-f]{40}", u)):
                bad.append(f"{name}: uses {u}")
            if "${{" in st.get("run", ""):
                bad.append(f"{name}: an expression in the script of {st.get('name')!r}")
    if jobs["deploy"].get("environment", {}).get("name") != "github-pages" or jobs["deploy"].get("needs") != "build":
        bad.append("deploy: not the github-pages environment after build")
    if [s.get("uses", "").split("@")[0] for s in jobs["deploy"]["steps"]] != ["actions/deploy-pages"]:
        bad.append("deploy: runs more than actions/deploy-pages")
    if "environment" in jobs["build"]:
        bad.append("build: has an environment")
    return bad


def run_build(bare: Path, rt: Path, target: Path, py: Path) -> tuple[bool, str, Path | None]:
    """main's pages.yml, its build job's run: steps in order as a runner
    would (bash -e, the job's env, RUNNER_TEMP); returns (passed, output,
    the folder the upload step publishes)."""
    wf = yaml.safe_load(sh("git", "--git-dir", bare, "show", f"main:{WF}").stdout)
    bad = lint(wf)
    if bad:
        return False, "lint: " + "; ".join(bad), None
    if rt.exists():
        shutil.rmtree(rt)
    rt.mkdir(parents=True)
    # python3 as a runner has it: this interpreter, isolated (the steps say -I).
    shim = rt.parent / f"{rt.name}-bin"
    shim.mkdir(exist_ok=True)
    (shim / "python3").write_text(f'#!/bin/sh\nexec "{py}" "$@"\n')
    (shim / "python3").chmod(0o755)
    env = {**os.environ, "REPO_URL": f"file://{bare}", "RUNNER_TEMP": str(rt), "PATH": f"{shim}:{os.environ['PATH']}",
           "RUSTUP_TOOLCHAIN": os.environ.get("RUSTUP_TOOLCHAIN", "stable")}
    out, upload = "", None
    for st in wf["jobs"]["build"]["steps"]:
        if "uses" in st:
            if st["uses"].startswith("actions/upload-pages-artifact@"):
                upload = Path(st["with"]["path"].replace("${{ runner.temp }}", str(rt)))
            continue
        script = st["run"]
        if "cargo build" in script:  # one build folder for every run
            script = script.replace('"$RUNNER_TEMP/target"', str(target))
        script = script.replace('"$RUNNER_TEMP/target/release', f'"{target}/release')
        r = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", script], capture_output=True, text=True, env=env, cwd=rt)
        out += r.stdout + r.stderr
        if r.returncode:
            return False, out, None
    return True, out, upload


def dark_thumbs(thumbs: Path) -> list[str]:
    """The thumbnails with no lit pixel (r + g + b over 90, as site-smoke
    checks the page in Chromium): opendmd-gallery's 8-bit RGB, unfiltered."""
    dark = []
    for p in sorted(thumbs.glob("*.png")):
        b, i, idat = p.read_bytes(), 8, b""
        while i < len(b):
            n = int.from_bytes(b[i:i + 4], "big")
            if b[i + 4:i + 8] == b"IDAT":
                idat += b[i + 8:i + 8 + n]
            i += 12 + n
        raw, w = zlib.decompress(idat), int.from_bytes(b[16:20], "big")
        rows = [raw[k + 1:k + 1 + w * 3] for k in range(0, len(raw), w * 3 + 1)]
        if b[24:26] != b"\x08\x02" or any(raw[k] for k in range(0, len(raw), w * 3 + 1)) or \
                not any(sum(r[j:j + 3]) > 90 for r in rows for j in range(0, len(r), 3)):
            dark.append(p.stem)
    return dark


def diff_trees(a: Path, b: Path) -> list[str]:
    """Every path that differs between two trees (content, type, presence)."""
    out = []

    def walk(c: filecmp.dircmp, rel: str):
        out.extend(f"{rel}{n}: only in {a.name}" for n in c.left_only)
        out.extend(f"{rel}{n}: only in {b.name}" for n in c.right_only)
        out.extend(f"{rel}{n}: type" for n in c.common_funny + c.funny_files)
        _, mismatch, errors = filecmp.cmpfiles(c.left, c.right, c.common_files, shallow=False)
        out.extend(f"{rel}{n}: differs" for n in mismatch + errors)
        for n, sub in c.subdirs.items():
            walk(sub, f"{rel}{n}/")
    walk(filecmp.dircmp(a, b, ignore=[]), "")
    return sorted(out)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--site", type=Path, help="a real `make site` output to publish (default: a stand-in)")
    ap.add_argument("--keep", type=Path, help="work here and keep it")
    ap.add_argument("-v", action="store_true", help="print each run's output")
    a = ap.parse_args()
    tmp = Path(a.keep or tempfile.mkdtemp(prefix="gallery-pages-"))
    if a.keep and tmp.exists():
        shutil.rmtree(tmp)
    tmp.mkdir(parents=True, exist_ok=True)
    failures = 0

    def check(ok: bool, what: str, detail: str = "") -> None:
        nonlocal failures
        failures += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} {what}")
        if detail and (not ok or a.v):
            print("".join(f"       | {line}\n" for line in detail.strip().splitlines()[:40]))

    try:
        site_py = load_site_py()
        # The release's checker, as `make gallery-tool` builds it.
        sh("cargo", "build", "-q", "--release", "--locked", "--manifest-path", GALLERY / "Cargo.toml",
           "--target-dir", tmp / "release-target", env={"RUSTUP_TOOLCHAIN": os.environ.get("RUSTUP_TOOLCHAIN", "stable")})
        tool = tmp / "release-target/release/opendmd-gallery"
        www = ROOT / "opendmd-web/www"
        wm = {"w": 64, "h": 8, "src": "wordmark.png"}

        def release_site(name: str, contrib: Path) -> Path:
            """A release's site with the gallery `contrib`, as make site writes it."""
            out = tmp / name
            if a.site:
                shutil.copytree(a.site, out, symlinks=True)
                shutil.rmtree(out / "formulas")
                # The wordmark as the page has it (art.json is not published).
                m = re.search(r'--cols:(\d+);--rows:(\d+)"><img src="\.\./art/([^"]+)"', (a.site / "formulas/index.html").read_text())
                site_py.write_gallery(out, out / "demo", contrib, tool, {"w": m[1], "h": m[2], "src": m[3]})
            else:
                (out / "demo").mkdir(parents=True)
                shutil.copyfile(www / "formulas.json", out / "demo/formulas.json")
                (out / "install").mkdir()
                (out / "install/release.json").write_text('{"version": "9.9.9"}\n')
                (out / "index.html").write_text("<p>the site</p>\n")
                (out / ".nojekyll").write_text("")
                site_py.write_gallery(out, www, contrib, tool, wm)
            return out

        # The public repo before the release: a contribution already merged.
        bare = tmp / "opendmd.github.io.git"
        sh("git", "init", "-q", "--bare", "-b", "main", bare)
        seed = tmp / "seed"
        sh("git", "clone", "-q", bare, seed)
        (seed / "gallery").mkdir()
        (seed / "gallery/spiral-sweep.psi").write_text(SPIRAL)
        sh("git", "-C", seed, "checkout", "-q", "-b", "main")
        sh("git", "-C", seed, "add", "-A")
        sh("git", "-C", seed, "commit", "-q", "-m", "seed")
        sh("git", "-C", seed, "push", "-q", "origin", "HEAD:main")

        # 1. The release: its gallery is main's gallery/ at the time.
        g1 = tmp / "gallery-1"
        g1.mkdir()
        shutil.copyfile(seed / "gallery/spiral-sweep.psi", g1 / "spiral-sweep.psi")
        rel1 = release_site("release-1", g1)
        page = (rel1 / "formulas/index.html").read_text()
        check(page.count("<!-- gallery:begin -->") == 1 and "Spiral sweep" in page, "the release's gallery page has its markers and the contribution")
        thumbs = sorted((rel1 / "formulas").glob("*/*.png"))
        dark = dark_thumbs(rel1 / "formulas/examples") + dark_thumbs(rel1 / "formulas/thumbs")
        check(len(thumbs) > 20 and not dark, f"every thumbnail lit, those over a GIF over the sample picture ({len(thumbs)}, dark: {dark})")
        sh("tools/site-publish.sh", rel1, bare, cwd=ROOT, env={"SITE_REPO_WORK": str(tmp / "work"), "SITE_PUBLISH_ALLOW_DIRTY": "1"})

        target = tmp / "pages-target"
        py = Path(sys.executable)
        ok, out, art = run_build(bare, tmp / "run-1", target, py)
        check(ok and art is not None, "the workflow's build runs on the published main", out)
        if ok:
            d = diff_trees(rel1, art)
            check(not d, "determinism: the artifact is the release's site, byte for byte (formulas/ too)", "\n".join(d))
            check(not (art / ".github").exists() and not (art / "gallery").exists(), "the artifact has no .github/ or gallery/")

        # 2. A contribution merged to main (a squash merge: one commit on main).
        work = tmp / "maintainer"
        sh("git", "clone", "-q", bare, work)
        (work / "gallery/rainbow-rings.psi").write_text(RINGS)
        sh("git", "-C", work, "add", "-A")
        sh("git", "-C", work, "commit", "-q", "-m", "Add rainbow-rings.psi (#2)")
        sh("git", "-C", work, "push", "-q", "origin", "HEAD:main")
        ok, out, art = run_build(bare, tmp / "run-2", target, py)
        check(ok, "the build runs after a merge", out)
        if ok:
            page = (art / "formulas/index.html").read_text()
            check("<h3>Rainbow rings</h3>" in page and (art / "formulas/thumbs/rainbow-rings.png").is_file()
                  and "<h3>Spiral sweep</h3>" in page, "the merged contribution is on the page, with its thumbnail")
            g2 = tmp / "gallery-2"
            shutil.copytree(g1, g2)
            (g2 / "rainbow-rings.psi").write_text(RINGS)
            rel2 = release_site("release-2", g2)
            d = diff_trees(rel2, art)
            check(not d, "determinism: equal to a release built with that gallery", "\n".join(d))

        # 3. Hostile files merged straight to main (an admin's bypass, or a
        # mistake): markup in a name, links out of gallery/ and the site, a folder.
        (work / "gallery/evil.psi").write_text(EVIL)
        (work / "gallery/passwd.psi").symlink_to("/etc/passwd")
        (work / "gallery/folder.psi").mkdir()
        (work / "gallery/folder.psi/x.psi").write_text(SPIRAL)
        (work / "leak.html").symlink_to("/etc/passwd")
        (work / "formulas/thumbs").mkdir(parents=True, exist_ok=True)
        sh("git", "-C", work, "add", "-A")
        sh("git", "-C", work, "commit", "-q", "-m", "hostile")
        sh("git", "-C", work, "push", "-q", "origin", "HEAD:main")
        ok, out, art = run_build(bare, tmp / "run-3", target, py)
        check(ok, "the build still runs with hostile files on main", out)
        if ok:
            page = (art / "formulas/index.html").read_text()
            check("<script>" not in page and "alert(1)" not in page and not (art / "formulas/thumbs/evil.png").exists(),
                  "markup in a name: left out (not on the page, no thumbnail)", out)
            check("left out 'evil.psi'" in out and "left out 'passwd.psi': not a plain file" in out
                  and "left out 'folder.psi': not a plain file" in out, "the log names what was left out", out)
            check(not (art / "leak.html").exists() and not (art / "leak.html").is_symlink(), "a link at the top of the site: deleted, not followed")
            check(not any(p.is_symlink() for p in art.rglob("*")), "no link anywhere in the artifact")
            check("<h3>Rainbow rings</h3>" in page and "<h3>Spiral sweep</h3>" in page, "the good contributions are still there")

        # 4. A page without markers (a site from before this workflow): the build fails, nothing deploys.
        (work / "formulas/index.html").write_text("<p>an old page</p>\n")
        sh("git", "-C", work, "commit", "-q", "-am", "an old page")
        sh("git", "-C", work, "push", "-q", "origin", "HEAD:main")
        ok, out, _ = run_build(bare, tmp / "run-4", target, py)
        check(not ok and "gallery markers" in out, "a page without gallery markers: the build fails (no deploy)", out)
    finally:
        if not a.keep:
            shutil.rmtree(tmp, ignore_errors=True)
    print(f"gallery-pages-test: {'ok' if not failures else f'{failures} failures'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
