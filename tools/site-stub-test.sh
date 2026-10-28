#!/usr/bin/env bash
# tools/site-stub.py over a small fixture site: the GitHub Pages stub has a
# forwarding page for every page and the 404 page, keeps install/release.json
# and beta.json as JSON with absolute URLs on the new host, the installer's
# manifests with absolute part paths, art-index/ byte for byte, and nothing
# else. Run by `make test` (nothing leaves the machine).
#
# SITE_STUB_BROWSER=1 (make site-stub-test) also loads the stub in headless
# Chromium, served the way GitHub Pages serves it (404.html for an address it
# lacks), forwarding to a second local server standing in for opendmd.dev:
# a /f/#formula=… link must arrive with its Formula, a page at its own path.
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail() { echo "site-stub-test: FAIL: $*" >&2; exit 1; }

# The fixture: the shape of make site's output.
s=$tmp/site
mkdir -p "$s"/{guide,f,demo/js,docs,dev/api,install/firmware,art-index/v0,art-index/v1,art}
for p in index.html guide/web-app.html f/index.html demo/index.html docs/web-app.html dev/api.html dev/api/v1.0.0.html install/index.html 404.html; do
  printf '<!doctype html><title>%s</title><p>the site</p>\n' "$p" > "$s/$p"
done
printf 'export {}\n' > "$s/f/f.js"
printf 'export {}\n' > "$s/demo/js/app.js"
printf 'body{}\n' > "$s/site.css"
printf 'GIF89a' > "$s/art/a.gif"
printf '/*\n  X-Test: 1\n' > "$s/_headers"
printf '/a /b 301\n' > "$s/_redirects"
printf 'README.md\n' > "$s/.assetsignore"
printf '' > "$s/.nojekyll"
printf '# the site\n' > "$s/README.md"
printf 'odfw' > "$s/install/firmware/opendmd-1.0.0-4mb.odfw"
cat > "$s/install/release.json" <<'EOF'
{"version": "1.0.0", "firmware": "firmware/opendmd-1.0.0-4mb.odfw",
 "variants": {"4mb": {"firmware": "firmware/opendmd-1.0.0-4mb.odfw"}, "8mb": {"firmware": "firmware/opendmd-1.0.0-8mb.odfw"}},
 "www": "firmware/opendmd-1.0.0-www.pack", "www_version": "v1.0.0"}
EOF
sed 's/1\.0\.0/1.1.0-beta.1/g' "$s/install/release.json" > "$s/install/beta.json"
manifest='{"name": "OpenDMD", "version": "1.0.0", "builds": [{"chipFamily": "ESP32", "parts": [{"path": "bootloader-4mb.bin", "offset": 4096}, {"path": "opendmd-1.0.0-4mb-app.bin", "offset": 131072}]}]}'
printf '%s\n' "$manifest" > "$s/install/firmware/manifest-4mb.json"
printf '%s\n' "$manifest" > "$s/install/firmware/manifest.json"
printf '{"repo":"x","names":["Tetris"],"kinds":"1"}' > "$s/art-index/v1/GB.json"
printf '{"old":0}' > "$s/art-index/v0/GB.json"

uv run --quiet tools/site-stub.py "$s" "$tmp/stub" --to https://opendmd.dev >/dev/null
o=$tmp/stub

# Exactly these files: a page for every page, the kept JSON, the repo's two files.
want=$(printf '%s\n' .nojekyll 404.html README.md art-index/v0/GB.json art-index/v1/GB.json dev/api.html dev/api/v1.0.0.html \
  demo/index.html docs/web-app.html f/index.html guide/web-app.html index.html install/beta.json \
  install/firmware/manifest-4mb.json install/firmware/manifest.json install/index.html install/release.json | sort)
have=$(cd "$o" && find . -type f | sed 's|^\./||' | sort)
[ "$have" = "$want" ] || fail "the stub's files: $(diff <(echo "$want") <(echo "$have") | grep '^[<>]' | tr '\n' ' ')"

# Every page forwards: the script keeps path, query and hash; meta refresh and canonical name the page.
for p in index.html guide/web-app.html f/index.html demo/index.html docs/web-app.html dev/api.html dev/api/v1.0.0.html install/index.html; do
  url="https://opendmd.dev/${p%index.html}"
  grep -qF '<script>location.replace("https://opendmd.dev" + location.pathname + location.search + location.hash)</script>' "$o/$p" || fail "$p: no forwarding script"
  grep -qF "<meta http-equiv=\"refresh\" content=\"0; url=$url\">" "$o/$p" || fail "$p: no meta refresh to $url"
  grep -qF "<link rel=\"canonical\" href=\"$url\">" "$o/$p" || fail "$p: no canonical $url"
  ! grep -q 'the site' "$o/$p" || fail "$p: carries the old page's content"
done
grep -qF 'location.replace("https://opendmd.dev" + location.pathname' "$o/404.html" || fail "404.html does not forward"
! grep -q canonical "$o/404.html" || fail "404.html names a canonical page"

# The JSON stays JSON, with absolute URLs; art-index byte for byte.
uv run --quiet --no-project python - "$s" "$o" <<'EOF' || fail "the kept JSON (above)"
import json, sys
from pathlib import Path
s, o = Path(sys.argv[1]), Path(sys.argv[2])
fw = "https://opendmd.dev/install/firmware/"
for name, ver in (("release.json", "1.0.0"), ("beta.json", "1.1.0-beta.1")):
    d = json.loads((o / "install" / name).read_text())
    want = {"version": ver, "firmware": f"{fw}opendmd-{ver}-4mb.odfw",
            "variants": {"4mb": {"firmware": f"{fw}opendmd-{ver}-4mb.odfw"}, "8mb": {"firmware": f"{fw}opendmd-{ver}-8mb.odfw"}},
            "www": f"{fw}opendmd-{ver}-www.pack", "www_version": f"v{ver}"}
    assert d == want, (name, d)
for name in ("manifest-4mb.json", "manifest.json"):
    m = json.loads((o / "install/firmware" / name).read_text())
    assert [p["path"] for p in m["builds"][0]["parts"]] == [f"{fw}bootloader-4mb.bin", f"{fw}opendmd-1.0.0-4mb-app.bin"], m
    assert [p["offset"] for p in m["builds"][0]["parts"]] == [4096, 131072] and m["version"] == "1.0.0", m
for f in ("art-index/v0/GB.json", "art-index/v1/GB.json"):
    assert (o / f).read_bytes() == (s / f).read_bytes(), f
EOF

# A release.json from the real site, if one is built: still the update check's shape.
if [ -f target/site/install/release.json ]; then
  uv run --quiet tools/site-stub.py target/site "$tmp/real" >/dev/null
  uv run --quiet --no-project python - target/site "$tmp/real" <<'EOF' || fail "the real site's stub (above)"
import json, sys
from pathlib import Path
s, o = Path(sys.argv[1]), Path(sys.argv[2])
a, b = json.loads((s / "install/release.json").read_text()), json.loads((o / "install/release.json").read_text())
fw = "https://opendmd.dev/install/"
assert b["version"] == a["version"] and b["firmware"] == fw + a["firmware"] and b["www"] == fw + a["www"], b
assert {k: v["firmware"] for k, v in b["variants"].items()} == {k: fw + v["firmware"] for k, v in a["variants"].items()}, b
pages = {p.relative_to(s).as_posix() for p in s.rglob("*.html")}
stubs = {p.relative_to(o).as_posix() for p in o.rglob("*.html")}
assert pages == stubs, pages ^ stubs
assert not [p for p in o.rglob("*") if p.suffix in (".js", ".css", ".wasm", ".gif", ".png", ".bin", ".odfw", ".pack")]
EOF
  real=" and the real site's"
fi

browser=skipped
if [ "${SITE_STUB_BROWSER:-}" = 1 ]; then
  uv run --quiet --with 'playwright>=1.45' --no-project python - "$s" "$tmp" <<'EOF' || fail "headless (above)"
import functools, http.server, subprocess, sys, threading, urllib.parse
from pathlib import Path
from playwright.sync_api import sync_playwright
site, tmp = Path(sys.argv[1]), Path(sys.argv[2])

class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a): pass

class Pages(Quiet):  # GitHub Pages: 404.html, status 404, for an address it lacks
    def send_error(self, code, message=None, explain=None):
        if code != 404:
            return super().send_error(code, message, explain)
        body = (Path(self.directory) / "404.html").read_bytes()
        self.send_response(404); self.send_header("Content-Type", "text/html"); self.send_header("Content-Length", str(len(body)))
        self.end_headers(); self.wfile.write(body)

def serve(root, cls):
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(cls, directory=str(root)))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{srv.server_port}"

new = serve(site, Quiet)  # the new host serves the full site
subprocess.run(["uv", "run", "--quiet", "tools/site-stub.py", str(site), str(tmp / "stub-local"), "--to", new], check=True, stdout=subprocess.DEVNULL)
old = serve(tmp / "stub-local", Pages)
formula = "#" + urllib.parse.urlencode({"formula": "loop 4\ngray(frac(x + t/4))", "name": "Fade <b>", "a": "0.5"})
cases = [("/f/" + formula, "/f/" + formula), ("/f/index.html" + formula, "/f/index.html" + formula),
         ("/guide/web-app.html?x=1#top", "/guide/web-app.html?x=1#top"), ("/", "/"),
         ("/demo/js/app.js?v=2#h", "/demo/js/app.js?v=2#h"), ("/no/such/page#frag", "/no/such/page#frag")]
bad = []
with sync_playwright() as p:
    b = p.chromium.launch(headless=True)
    for path, want in cases:
        page = b.new_page()
        page.goto(old + path)
        page.wait_for_url(new + "/**", timeout=10000)
        if page.url != new + want:
            bad.append(f"{path} -> {page.url}")
        page.close()
    b.close()
for x in bad:
    print("  FAIL", x)
sys.exit(1 if bad else 0)
EOF
  browser="ok (/f/#formula= keeps its Formula, pages and unknown addresses keep path, query and hash)"
fi
echo "site-stub-test: ok (a forwarding page for every page and the 404, release.json/beta.json absolute, manifests absolute, art-index kept, nothing else${real:-}; headless: $browser)"
