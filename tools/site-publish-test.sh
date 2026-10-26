#!/usr/bin/env bash
# tools/site-publish.sh against a local bare repository standing in for the
# public Pages repo: two publishes with a contribution merged in between, and
# a third with nothing new. gallery/'s formulas must survive every publish,
# .github/ must be ours (the gallery check, the Pages workflow, the gallery
# page's generator and the checker's source, which
# must build offline as the public repo's workflow builds it), and the site's
# own files must be replaced. Run by `make test`; nothing leaves the machine.
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail() { echo "site-publish-test: FAIL: $*" >&2; exit 1; }
g() { git -c user.name=Test -c user.email=test@example.invalid -c init.defaultBranch=main "$@"; }

# The public repo as it might be: an old site, an old workflow, a contribution.
g init -q --bare "$tmp/pub.git"
g clone -q "$tmp/pub.git" "$tmp/seed" 2>/dev/null
mkdir -p "$tmp/seed/gallery" "$tmp/seed/.github/workflows"
printf '<p>old</p>\n' > "$tmp/seed/index.html"
printf 'stale\n' > "$tmp/seed/stale.html"
mkdir -p "$tmp/seed/dev/api"
printf '<p>API of 0.9.0</p>\n' > "$tmp/seed/dev/api/v0.9.0.html"
printf 'name: old\n' > "$tmp/seed/.github/workflows/gallery-check.yml"
mkdir -p "$tmp/seed/art-index/v0" "$tmp/seed/art-index/v1"
printf '{"old":0}\n' > "$tmp/seed/art-index/v0/GB.json"
printf '{"names":"old"}\n' > "$tmp/seed/art-index/v1/GB.json"
printf -- '-- name: Theirs\n-- description: A contribution\n-- author: @someone\n-- licence: CC0-1.0\ngray(x)\n' > "$tmp/seed/gallery/theirs.psi"
g -C "$tmp/seed" add -A && g -C "$tmp/seed" commit -qm seed && g -C "$tmp/seed" push -q origin HEAD:main

site() { # a site build: its version, a page, and the public repo's API pages (as release.yml fetches them)
  rm -rf "$tmp/site"; mkdir -p "$tmp/site/install" "$tmp/site/f" "$tmp/site/dev/api"
  printf '<p>dev</p>\n' > "$tmp/site/dev/index.html"
  local pub="$tmp/seed"; [ ! -d "$tmp/look" ] || pub="$tmp/look"
  cp "$pub"/dev/api/v*.html "$tmp/site/dev/api/" 2>/dev/null || true
  printf '{"version": "9.9.9"}\n' > "$tmp/site/install/release.json"
  printf '<p>%s</p>\n' "$1" > "$tmp/site/index.html"
  printf '<p>share</p>\n' > "$tmp/site/f/index.html"
  mkdir -p "$tmp/site/art-index/v1"
  printf '{"names":"%s"}\n' "$1" > "$tmp/site/art-index/v1/GB.json"
}
publish() {
  # (An identity of its own: CI's checkout has none for site-publish.sh to copy.)
  GIT_AUTHOR_NAME=Test GIT_AUTHOR_EMAIL=test@example.invalid GIT_COMMITTER_NAME=Test GIT_COMMITTER_EMAIL=test@example.invalid \
    SITE_REPO_WORK="$tmp/work" SITE_PUBLISH_ALLOW_DIRTY=1 tools/site-publish.sh "$tmp/site" "$tmp/pub.git"
}
look() { rm -rf "$tmp/look"; g clone -q "$tmp/pub.git" "$tmp/look" 2>/dev/null; }

site v1
publish >/dev/null
look
[ "$(cat "$tmp/look/index.html")" = "<p>v1</p>" ] || fail "the site's page was not replaced"
[ ! -e "$tmp/look/stale.html" ] || fail "a file the site no longer has was kept"
[ -f "$tmp/look/gallery/theirs.psi" ] || fail "the first publish removed a contribution"
[ "$(cat "$tmp/look/dev/api/v0.9.0.html" 2>/dev/null)" = "<p>API of 0.9.0</p>" ] || fail "a publish removed an earlier release's API page"
[ "$(cat "$tmp/look/art-index/v0/GB.json" 2>/dev/null)" = '{"old":0}' ] || fail "a publish removed an art index format the site no longer builds"
[ "$(cat "$tmp/look/art-index/v1/GB.json" 2>/dev/null)" = '{"names":"v1"}' ] || fail "a publish did not refresh the art index"
cmp -s "$tmp/look/gallery/README.md" site/public-repo/gallery/README.md || fail "gallery/README.md is not ours"
cmp -s "$tmp/look/.github/workflows/gallery-check.yml" site/public-repo/.github/workflows/gallery-check.yml || fail "the workflow is not ours"
cmp -s "$tmp/look/.github/workflows/pages.yml" site/public-repo/.github/workflows/pages.yml || fail "the Pages workflow is not ours"
for f in opendmd-psi/src/lib.rs opendmd-core/src/fonts.rs opendmd-gallery/src/main.rs opendmd-gallery/Cargo.lock tools/gallery_page.py; do
  cmp -s "$tmp/look/.github/gallery-check/$f" "$f" || fail "the checker's source lacks $f"
done
[ ! -e "$tmp/look/.github/gallery-check/opendmd-psi/target" ] || fail "a build folder was vendored"

# A contribution merged in the public repo between two publishes.
g -C "$tmp/look" checkout -q main
printf -- '-- name: Merged\n-- description: Merged by a maintainer\n-- author: @other\n-- licence: CC-BY-4.0\ngray(y)\n' > "$tmp/look/gallery/merged.psi"
g -C "$tmp/look" add -A && g -C "$tmp/look" commit -qm "Add merged.psi" && g -C "$tmp/look" push -q origin HEAD:main

site v2
publish >/dev/null
look
[ "$(cat "$tmp/look/index.html")" = "<p>v2</p>" ] || fail "the second publish did not replace the page"
[ -f "$tmp/look/gallery/theirs.psi" ] && [ -f "$tmp/look/gallery/merged.psi" ] || fail "the second publish lost a contribution"
cmp -s "$tmp/look/.github/workflows/gallery-check.yml" site/public-repo/.github/workflows/gallery-check.yml || fail "the second publish lost the workflow"
[ "$(git -C "$tmp/look" log --format=%s -1)" = "Site v9.9.9, docs of $(date -u +%F)" ] || fail "commit message: $(git -C "$tmp/look" log --format=%s -1)"

# Nothing new: no commit.
out=$(publish)
[ "$out" = "site: no changes" ] || fail "a publish with nothing new said: $out"

# dev/api/: the API page as each release described it.
api() { # a site build with these API pages changed: <version>=<content> (- removes it) ...
  site "api"
  for kv in "$@"; do
    f="$tmp/site/dev/api/v${kv%%=*}.html"
    if [ "${kv#*=}" = - ]; then rm -f "$f"; else printf '<p>%s</p>\n' "${kv#*=}" > "$f"; fi
  done
}
refused() { # publish (with these variables) must be refused, saying this
  local want=$1; shift
  if ( [ $# -eq 0 ] || export "$@"; publish ) >/dev/null 2>"$tmp/err"; then return 1; fi
  grep -q "$want" "$tmp/err" || { cat "$tmp/err" >&2; return 1; }
}
# From main between releases ("docs of <date>"): no page for its version;
# an earlier release's page the repo lacks is added, one it has is kept.
api 9.9.9=unreleased 0.9.0=changed 0.8.0=older
publish >/dev/null
look
[ ! -e "$tmp/look/dev/api/v9.9.9.html" ] || fail "a publish from main froze an API page for its unreleased version"
[ "$(cat "$tmp/look/dev/api/v0.9.0.html")" = "<p>API of 0.9.0</p>" ] || fail "a publish replaced an earlier release's API page"
[ "$(cat "$tmp/look/dev/api/v0.8.0.html" 2>/dev/null)" = "<p>older</p>" ] || fail "a missing earlier release's API page was not added"
# A release's site without its own page is refused.
api 0.9.0=changed
refused "has no dev/api/v9.9.9.html" SITE_PUBLISH_DESCRIBE=v9.9.9 || fail "a release's site without its API page was not refused"
# A site lacking a published release's page (built without API_ARCHIVE) is refused.
api 0.8.0=-
refused "lacks the published API pages dev/api/v0.8.0.html" || fail "a site without a published API page was not refused"
# From the release's tag: its own page is added, then replaced by a republish.
api 9.9.9=release 0.9.0=changed
SITE_PUBLISH_DESCRIBE=v9.9.9 publish >/dev/null
look
[ "$(cat "$tmp/look/dev/api/v9.9.9.html" 2>/dev/null)" = "<p>release</p>" ] || fail "the release's publish did not freeze its API page"
[ "$(git -C "$tmp/look" log --format=%s -1)" = "Site v9.9.9" ] || fail "release commit message: $(git -C "$tmp/look" log --format=%s -1)"
api 9.9.9=republished
SITE_PUBLISH_DESCRIBE=v9.9.9 publish >/dev/null
look
[ "$(cat "$tmp/look/dev/api/v9.9.9.html")" = "<p>republished</p>" ] || fail "a republish of the release did not replace its own API page"
[ "$(cat "$tmp/look/dev/api/v0.9.0.html")" = "<p>API of 0.9.0</p>" ] && [ -f "$tmp/look/dev/api/v0.8.0.html" ] ||
  fail "a release's publish removed or replaced an earlier release's API page"
# A later publish from main keeps the release's page as it is.
api 9.9.9=main-again
publish >/dev/null
look
[ "$(cat "$tmp/look/dev/api/v9.9.9.html")" = "<p>republished</p>" ] || fail "a publish from main replaced the release's API page"
site v3

# A site that brings its own gallery/ is refused (it would be skipped).
mkdir -p "$tmp/site/gallery"
refused "the site has a gallery/" || fail "a site with a gallery/ was not refused"

# An untracked file beside the checker's source is refused (it would not be vendored).
rmdir "$tmp/site/gallery"
stray=opendmd-gallery/src/stray-site-publish-test.rs
trap 'rm -rf "$tmp" "$stray"' EXIT
: > "$stray"
refused "untracked files in the checker's source" || { rm -f "$stray"; fail "a publish with an untracked file in the checker's source was not refused"; }
rm -f "$stray"

# The vendored checker builds as the public workflow builds it: offline,
# locked, from an empty cargo home, and checks a contribution.
CARGO_HOME="$tmp/cargo" RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-stable} cargo build -q --release --locked --offline \
  --manifest-path "$tmp/look/.github/gallery-check/opendmd-gallery/Cargo.toml" --target-dir "$tmp/target" ||
  fail "the vendored checker does not build offline"
"$tmp/target/release/opendmd-gallery" check "$tmp/look/gallery/merged.psi" 2>/dev/null || fail "the vendored checker refused a good file"
echo "site-publish-test: ok (publishes kept gallery/, .github/ and earlier releases' dev/api/ pages and every art-index/ format, refreshed; only a release froze its own; the vendored checker builds offline)"
