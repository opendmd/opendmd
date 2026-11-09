#!/usr/bin/env bash
# A release: built and checked on this machine, published by CI
# (.github/workflows/release.yml on the tag this pushes; docs/deploying.md
# "Releasing").
#
#   tools/release.sh VERSION                         dry run: plan, bump, build, check, site; publishes nothing
#   tools/release.sh VERSION --publish               the same, then pushes main and the tag; CI publishes; waits and checks
#   tools/release.sh VERSION --publish --resume      after a push: waits for CI's run again and checks what it published
#   tools/release.sh VERSION --publish-local         fallback while CI is down: publishes everything from here
#   tools/release.sh VERSION --publish-local --resume   after a local publish that stopped part way
#   (make release VERSION=1.0.1 [PUBLISH=1|PUBLISH=local] [RESUME=1])
#
# 1. Plan: the version is semver (the site's grammar, tools/site.py
#    API_FROZEN: no "-" inside the pre-release part), newer than every v*
#    tag here and on GitHub; the tag exists nowhere; the tree is clean, on
#    main, and GitHub's and origin's main are in it; gh may push; the signing
#    key is there (only its size is read). --publish: the repository has the
#    secrets release.yml needs (their names only). --publish-local: no
#    release.yml run exists for the tag and no GitHub release either.
# 2. Bump: every opendmd crate's Cargo.toml and the opendmd-* entries of
#    every Cargo.lock, committed as "Release <version>" on a detached HEAD
#    (main does not move until publishing), and the annotated tag v<version>,
#    whose message says who publishes it: "Publisher: ci", or "Publisher:
#    local" (--publish-local; release.yml then skips the tag).
# 3. Build: make fw-release (the 4mb layout, signed with FW_SIGN_KEY, default
#    ~/.config/opendmd/opendmd.key) into a dist/ of this release's own, then
#    release.yml's checks: the version in the images, manifests, bundles and
#    the www pack; no $HOME in the images; the full images' sizes; 64 KB of
#    factory spare per layout; the bundle and pack verify against
#    firmware/keys/opendmd.pub.
# 4. Assets: release.yml's list, SHA256SUMS, and the release notes.
# 5. Site: make site with the public gallery repo's gallery/ and dev/api/
#    (a fresh sparse clone) and this release's API page frozen; release.yml's
#    release.json, beta.json and installer checks; site-smoke;
#    site-deploy-check (cf deploy --dry-run, uploads nothing); the tree clean
#    after it all.
#
# The channels (docs/deploying.md "Betas"): install/release.json names the
# newest final release, install/beta.json the newest release of all (what
# players with Get beta updates on read). A final X.Y.Z, or a beta before
# any final exists, updates both (and the installer). A beta after a final
# updates beta.json only: the site is built with the last final's files
# from its GitHub release (DIST, checked against its SHA256SUMS: read only)
# and this beta's (BETA_DIST), so release.json and the Install button stay
# on the final and the beta gets install/firmware/beta/ and its own button.
#
# 6. --publish: main fast-forwarded to the release commit; the commit and
#    the tag pushed to GitHub (https, gh's credentials) and to origin (the
#    local mirror). The tag starts release.yml, which builds the release
#    again, signs it with the OTA_SIGNING_KEY secret (the same key) and
#    publishes it: the GitHub release and its assets, the public gallery
#    repo, the site. This waits for that run (gh run watch), then checks
#    what it published: the release (not a draft, a pre-release for a "-"
#    version), its assets (this build's names; CI's bytes, as builds are not
#    byte-reproducible: SHA256SUMS, and the bundle and pack verify against
#    opendmd.pub), and opendmd.dev live (tools/site-live-check.sh: the feeds
#    name the right versions and serve the release's own bytes). The build
#    here is the check that the tag is releasable; CI's is what ships.
# 6. --publish-local (CI down): the same pushes (the tag says "Publisher:
#    local", so a release.yml run, should one start, does nothing); the
#    GitHub release and its assets from this build (tools/release-publish.sh:
#    a draft until every asset is up); the public gallery repo
#    (tools/gallery-repo-publish.sh over https); the site on Cloudflare (make
#    site-deploy-built, cf's OAuth login), checked live the same way.
#
# A dry run, or a publish that fails before anything is pushed, leaves the
# checkout as it found it: back on main, the local tag deleted. Once a push
# has happened the release commit and tag stay, and a rerun with --resume
# picks up: it skips a push already done; with --publish it watches or
# checks CI's run (a failed run is rerun on GitHub, which is safe:
# release.yml); with --publish-local it reuses this release's build
# (rebuilding only if it is missing) and skips the release, an asset or the
# deploy already done. Something on GitHub that differs (a tag on another
# object, a published asset with another digest) stops it: nothing is ever
# moved, deleted or overwritten. --publish-local --resume may also take over
# a "Publisher: ci" tag CI never ran (no run, no release): a late CI run
# then finds the release published and takes its assets as they are.
#
# Environment: RELEASE_DIR (default $SCRATCH/opendmd-release when SCRATCH
# is set, as the Makefile's local.mk can do, else target/release): this
# release's build goes in $RELEASE_DIR/v<version>. FW_SIGN_KEY,
# FW_RELEASE_TARGET_DIR: as for make fw-release. UPDATER_BUILD (default: one
# per checkout under $SCRATCH, else target/updater). RELEASE_TRAILERS: the
# release commit's trailer lines, newline-separated (default: none).
set -euo pipefail
export LC_ALL=C

GH_REPO=opendmd/opendmd
GH_URL=https://github.com/$GH_REPO.git
GALLERY_URL=https://github.com/opendmd/opendmd.github.io.git
SITE_URL=https://opendmd.dev
TRAILERS=${RELEASE_TRAILERS:-}

usage() { sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }

# PUBLISH: 0 (a dry run), 1 (--publish: CI publishes), 1 with LOCAL=1 (--publish-local).
VERSION= PUBLISH=0 LOCAL=0 RESUME=0 ANY_BRANCH=0
for a in "$@"; do
  case $a in
    --publish) PUBLISH=1 ;;
    --publish-local) PUBLISH=1 LOCAL=1 ;;
    --resume) RESUME=1 ;;
    # Tests of this script only (a dry run from a branch); never with --publish.
    --any-branch) ANY_BRANCH=1 ;;
    -h|--help) usage ;;
    -*) echo "unknown option $a" >&2; usage ;;
    *) [ -z "$VERSION" ] || usage; VERSION=${a#v} ;;
  esac
done
[ -n "$VERSION" ] || usage
MODE=dry; [ "$PUBLISH" = 0 ] || MODE=ci; [ "$LOCAL" = 0 ] || MODE=local
# What the annotated tag says (release.yml skips a "Publisher: local" tag).
PUBLISHER=ci; [ "$LOCAL" = 0 ] || PUBLISHER=local

# ---- output ---------------------------------------------------------------

DONE=()        # what this run published, in order
PUBLISHING=0   # 1 once the first outward-facing step has started
CUR=           # the step running
step() { CUR=$1; printf '\n\033[1m==> %s\033[0m\n' "$1"; }
pass() { printf 'PASS  %s\n' "$*"; }
skip() { printf 'SKIP  %s\n' "$*"; }
info() { printf '      %s\n' "$*"; }
die()  { printf 'FAIL  %s\n' "$*" >&2; exit 1; }
published() { DONE+=("$1"); pass "$1"; }

# git with gh's credentials, for GitHub over https (its ssh key is refused).
ghgit() { GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=credential.helper GIT_CONFIG_VALUE_0='!gh auth git-credential' git "$@"; }

# ---- semver ---------------------------------------------------------------

valid_version() { [[ $1 =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$ ]]; }

# semver_cmp A B: prints -1, 0 or 1 (semver.org precedence).
semver_cmp() {
  local a=$1 b=$2 ap= bp= i x y
  [[ $a == *-* ]] && ap=${a#*-}
  [[ $b == *-* ]] && bp=${b#*-}
  local -a ac bc
  IFS=. read -ra ac <<<"${a%%-*}"; IFS=. read -ra bc <<<"${b%%-*}"
  for i in 0 1 2; do
    (( 10#${ac[i]} > 10#${bc[i]} )) && { echo 1; return; }
    (( 10#${ac[i]} < 10#${bc[i]} )) && { echo -1; return; }
  done
  [ -z "$ap" ] && [ -z "$bp" ] && { echo 0; return; }
  [ -z "$ap" ] && { echo 1; return; }
  [ -z "$bp" ] && { echo -1; return; }
  local -a ai bi
  IFS=. read -ra ai <<<"$ap"; IFS=. read -ra bi <<<"$bp"
  for (( i = 0; i < ${#ai[@]} && i < ${#bi[@]}; i++ )); do
    x=${ai[i]} y=${bi[i]}
    [ "$x" = "$y" ] && continue
    if [[ $x =~ ^[0-9]+$ && $y =~ ^[0-9]+$ ]]; then
      (( 10#$x > 10#$y )) && echo 1 || echo -1; return
    fi
    [[ $x =~ ^[0-9]+$ ]] && { echo -1; return; }
    [[ $y =~ ^[0-9]+$ ]] && { echo 1; return; }
    [[ $x > $y ]] && echo 1 || echo -1; return
  done
  (( ${#ai[@]} > ${#bi[@]} )) && echo 1 || { (( ${#ai[@]} < ${#bi[@]} )) && echo -1 || echo 0; }
}

# ---- setup ----------------------------------------------------------------

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"
TAG=v$VERSION
KIND=release; [[ $VERSION == *-* ]] && KIND=prerelease
if [ -z "${RELEASE_DIR:-}" ]; then
  if [ -n "${SCRATCH:-}" ]; then RELEASE_DIR=$SCRATCH/opendmd-release; else RELEASE_DIR=$ROOT/target/release; fi
fi
REL=$RELEASE_DIR/$TAG
DIST=$REL/dist
SITE=$REL/site
ASSETS=$REL/assets
export FW_SIGN_KEY=${FW_SIGN_KEY:-$HOME/.config/opendmd/opendmd.key}
# idf.py's build dir names its source dir: one per checkout.
if [ -n "${SCRATCH:-}" ]; then
  export UPDATER_BUILD=${UPDATER_BUILD:-$SCRATCH/opendmd-updater-$(printf '%s' "$ROOT" | md5sum | cut -c1-8)}
else
  export UPDATER_BUILD=${UPDATER_BUILD:-$ROOT/target/updater}
fi
# make's own variables for every make below.
MK=(DIST="$DIST" UPDATER_BUILD="$UPDATER_BUILD" FW_SIGN_KEY="$FW_SIGN_KEY")

START_REF=$(git symbolic-ref -q --short HEAD || true)   # the branch this run started on
START=$(git rev-parse HEAD)
RELEASE_COMMIT=        # the commit the tag names
TAG_PUBLISHER=         # the tag's "Publisher:" (ci or local), once there is a tag
CREATED_TAG=0          # this run made the local tag (and the detached commit)

on_exit() {
  local rc=$?
  set +e
  if [ "$CREATED_TAG" = 1 ] && [ "$PUBLISHING" = 0 ]; then
    # Nothing left the machine: put the checkout back as it was.
    echo
    [ "$PUBLISH" = 1 ] && echo "Nothing was published; undoing the local release commit and tag."
    if [ "$(git rev-parse -q --verify "refs/tags/$TAG^{commit}")" = "$RELEASE_COMMIT" ]; then
      git tag -d "$TAG" >/dev/null && info "local tag $TAG deleted"
    fi
    if [ -n "$START_REF" ] && [ "$(git rev-parse HEAD)" = "$RELEASE_COMMIT" ]; then
      git switch -q "$START_REF" && info "back on $START_REF at $(git rev-parse --short HEAD)" ||
        echo "could not switch back to $START_REF (local changes?): git switch $START_REF" >&2
    fi
  fi
  if [ "$rc" != 0 ]; then
    echo
    echo "release.sh $TAG stopped${CUR:+ at \"$CUR\"} (exit $rc)."
    if [ "$PUBLISH" = 1 ]; then
      if [ ${#DONE[@]} -eq 0 ]; then
        echo "Published: nothing."
      else
        echo "Published:"; printf '  - %s\n' "${DONE[@]}"
      fi
      if [ "$PUBLISHING" = 1 ]; then
        echo "Not published: \"$CUR\" and every step after it."
        echo "Fix the cause and rerun: tools/release.sh $VERSION $( [ "$MODE" = local ] && echo --publish-local || echo --publish ) --resume"
      fi
    fi
  fi
  exit "$rc"
}
trap on_exit EXIT

mkdir -p "$REL"
exec > >(tee -a "$REL/release.log") 2>&1
echo "release.sh $TAG ($KIND) $(case $MODE in dry) echo 'dry run' ;; ci) echo 'publish (CI publishes)' ;; local) echo 'publish from this machine (PUBLISH=local)' ;; esac)$( [ "$RESUME" = 1 ] && echo ', resume' ) at $(date -u +%FT%TZ)"
echo "checkout $ROOT, build in $REL"

# ---- 1. plan --------------------------------------------------------------

step "1. Plan"
[ "$RESUME" = 0 ] || [ "$PUBLISH" = 1 ] || die "--resume goes with --publish or --publish-local"
[ "$ANY_BRANCH" = 0 ] || [ "$PUBLISH" = 0 ] || die "--any-branch is for dry runs only"
valid_version "$VERSION" || die "'$VERSION' is not a version this project releases: X.Y.Z or X.Y.Z-pre.N (letters, digits and dots after the '-')"
pass "version $VERSION ($KIND, tag $TAG)"
for t in git gh jq uv make rsync curl sha256sum; do command -v $t >/dev/null || die "$t is not installed"; done
gh auth status >/dev/null 2>&1 || die "gh is not logged in (gh auth login)"
PUSH_REPOS=("$GH_REPO"); [ "$MODE" != local ] || PUSH_REPOS+=(opendmd/opendmd.github.io)
for r in "${PUSH_REPOS[@]}"; do
  [ "$(gh api "repos/$r" --jq .permissions.push 2>/dev/null)" = true ] || die "gh's account cannot push to $r"
done
pass "gh logged in, may push to ${PUSH_REPOS[*]}"
if [ "$MODE" = ci ]; then
  # release.yml refuses a real release without them (before building); find out now, before the push.
  secrets=$(gh secret list -R "$GH_REPO" --json name --jq '.[].name') || die "cannot list $GH_REPO's secrets (gh needs admin on it)"
  for n in OTA_SIGNING_KEY SITE_DEPLOY_KEY CLOUDFLARE_API_TOKEN; do
    grep -qx "$n" <<<"$secrets" || die "$GH_REPO has no $n secret, which release.yml needs to publish (docs/deploying.md \"Releasing\"); or PUBLISH=local while CI cannot"
  done
  pass "$GH_REPO has the secrets release.yml publishes with (OTA_SIGNING_KEY, SITE_DEPLOY_KEY, CLOUDFLARE_API_TOKEN; names only)"
fi

[ -z "$(git status --porcelain)" ] || { git status --short; die "the working tree is not clean"; }
pass "the tree is clean"
! git submodule status | grep -q '^[-+]' || { git submodule status; die "a submodule is missing or not at the committed commit: git submodule update --init"; }
pass "the submodules are checked out at their committed commits"
if [ "$ANY_BRANCH" = 1 ]; then
  info "--any-branch: on ${START_REF:-a detached HEAD}, not checking for main"
else
  [ "$START_REF" = main ] || die "not on main (on ${START_REF:-a detached HEAD})"
  pass "on main"
fi

ghgit fetch -q "$GH_URL" +refs/heads/main:refs/remotes/github/main || die "cannot fetch main from $GH_URL"
git fetch -q origin main || die "cannot fetch main from origin"
REMOTE_TAGS=$(ghgit ls-remote --tags "$GH_URL" 'refs/tags/v*') || die "cannot list $GH_URL's tags"
remote_tag_obj() { awk -v r="refs/tags/$1" '$2 == r { print $1 }' <<<"$REMOTE_TAGS"; }
# The local tag's "Publisher:" line: local, or ci (also a tag from before the line).
tag_publisher() { git cat-file -p "refs/tags/$TAG" | sed '1,/^$/d' | grep -qx 'Publisher: local' && echo local || echo ci; }
# release.yml's runs for the tag (a tag push or a dispatch from it), newest first: id, status, conclusion.
ci_runs() { gh run list -R "$GH_REPO" --workflow release.yml --branch "$TAG" --limit 20 --json databaseId,status,conclusion,createdAt \
  --jq 'sort_by(.createdAt) | reverse | .[] | [.databaseId, .status, .conclusion] | @tsv'; }

LOCAL_OLD=$(sed -n 's/^version *= *"\(.*\)"/\1/p' firmware/Cargo.toml | head -1)
if [ "$RESUME" = 1 ]; then
  # After a publish that stopped: the release commit is main, tagged.
  git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || die "--resume: there is no local tag $TAG (nothing to resume; run without --resume)"
  [ "$(git cat-file -t "refs/tags/$TAG")" = tag ] || die "--resume: $TAG is not an annotated tag"
  RELEASE_COMMIT=$(git rev-parse "refs/tags/$TAG^{commit}")
  [ "$START" = "$RELEASE_COMMIT" ] || die "--resume: HEAD ($START) is not $TAG's commit ($RELEASE_COMMIT)"
  [ "$(git rev-parse main)" = "$RELEASE_COMMIT" ] || die "--resume: main is not $TAG's commit"
  [ "$LOCAL_OLD" = "$VERSION" ] || die "--resume: firmware/Cargo.toml says $LOCAL_OLD, not $VERSION"
  TAG_PUBLISHER=$(tag_publisher)
  if [ "$MODE" = ci ] && [ "$TAG_PUBLISHER" = local ]; then
    die "--resume: $TAG says \"Publisher: local\": CI does not publish it; resume with --publish-local"
  fi
  obj=$(remote_tag_obj "$TAG")
  [ -z "$obj" ] || [ "$obj" = "$(git rev-parse "refs/tags/$TAG")" ] ||
    die "GitHub's $TAG is $obj, not the local tag $(git rev-parse "refs/tags/$TAG"): not touching it"
  pass "resuming $TAG at $(git rev-parse --short "$RELEASE_COMMIT")"
else
  ! git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || die "the tag $TAG already exists here (a published tag is never moved or re-released; --resume continues a publish that stopped)"
  [ -z "$(remote_tag_obj "$TAG")" ] || die "the tag $TAG already exists on GitHub"
  pass "$TAG exists neither here nor on GitHub"
  for r in refs/remotes/github/main refs/remotes/origin/main; do
    git merge-base --is-ancestor "$r" HEAD || die "${r#refs/remotes/} ($(git rev-parse --short "$r")) is not in HEAD: pull or rebase first"
  done
  pass "GitHub's and origin's main are in HEAD ($(git rev-list --count refs/remotes/github/main..HEAD) commits ahead of GitHub)"
  latest=
  while read -r t; do
    v=${t#v}; valid_version "$v" || continue
    if [ -z "$latest" ] || [ "$(semver_cmp "$v" "$latest")" = 1 ]; then latest=$v; fi
  done < <(git tag -l 'v*'; awk '{ sub("refs/tags/", "", $2); sub(/\^\{\}$/, "", $2); print $2 }' <<<"$REMOTE_TAGS")
  [ -z "$latest" ] || [ "$(semver_cmp "$VERSION" "$latest")" = 1 ] || die "$VERSION is not newer than the latest release, $latest"
  pass "$VERSION is newer than the latest release, ${latest:-none}"
fi

# The channel: the newest final release other than this one (tags here and on GitHub).
# 0.x finals (v0.3.0) predate the channels and have no GitHub release: not counted.
FINAL=
while read -r t; do
  v=${t#v}; valid_version "$v" || continue
  [[ $v == *-* ]] || [ "$v" = "$VERSION" ] || [[ $v == 0.* ]] && continue
  if [ -z "$FINAL" ] || [ "$(semver_cmp "$v" "$FINAL")" = 1 ]; then FINAL=$v; fi
done < <(git tag -l 'v*'; awk '{ sub("refs/tags/", "", $2); sub(/\^\{\}$/, "", $2); print $2 }' <<<"$REMOTE_TAGS")
if [ "$KIND" = prerelease ] && [ -n "$FINAL" ]; then
  [ "$(semver_cmp "$VERSION" "$FINAL")" = 1 ] || die "$VERSION is not newer than the final release $FINAL"
  CHANNEL=beta REL_VERSION=$FINAL
  st=$(GH_REPO=$GH_REPO tools/release-tags.sh state "v$FINAL") || die "cannot read the final release v$FINAL"
  [ "${st%% *}" = published ] || die "the final release v$FINAL is '$st', not a published GitHub release to take its files from"
  pass "channel: beta (a beta after the final $FINAL): beta.json names $VERSION; release.json and the installer stay on $FINAL"
else
  CHANNEL=both REL_VERSION=$VERSION
  pass "channel: both ($( [ "$KIND" = release ] && echo 'a final release' || echo 'a beta before any final release')): release.json and beta.json name $VERSION"
fi

if [ "$MODE" = local ]; then
  # The fallback never races CI: no run of release.yml for the tag, and no
  # release (unless resuming a publish this fallback started itself).
  runs=$(ci_runs) || die "cannot list release.yml's runs for $TAG"
  [ -z "$runs" ] || die "release.yml has run for $TAG (run $(head -1 <<<"$runs" | cut -f1): $(head -1 <<<"$runs" | cut -f2,3 | tr '\t' ' ')): CI publishes this tag, not PUBLISH=local (rerun it on GitHub: it is safe)"
  rstate=$(GH_REPO=$GH_REPO tools/release-tags.sh state "$TAG") || die "cannot read the GitHub release $TAG"
  if [ "$rstate" != none ]; then
    # Only one this fallback started: its notes say "Publisher: local".
    [ "$RESUME" = 1 ] && [[ $rstate == *local ]] ||
      die "the GitHub release $TAG already exists ($rstate): PUBLISH=local only makes a release (or, with RESUME=1, finishes one it started)"
    pass "no release.yml run for $TAG; its GitHub release ($rstate) was made by an earlier PUBLISH=local (resuming)"
  else
    pass "no release.yml run and no GitHub release for $TAG"
  fi
fi

[ -f "$FW_SIGN_KEY" ] || die "no signing key at $FW_SIGN_KEY"
[ "$(stat -c %s "$FW_SIGN_KEY")" -eq 32 ] || die "$FW_SIGN_KEY is not a 32-byte key"
[ $(( 8#$(stat -c %a "$FW_SIGN_KEY") & 8#077 )) -eq 0 ] || die "$FW_SIGN_KEY is readable by others (mode $(stat -c %a "$FW_SIGN_KEY")): chmod 600 it"
pass "signing key present (32 bytes, mode $(stat -c %a "$FW_SIGN_KEY"); its contents are never read here)"

# ---- 2. bump --------------------------------------------------------------

if [ "$RESUME" = 1 ]; then
  step "2. Bump"; skip "resuming: $TAG is $(git rev-parse --short "$RELEASE_COMMIT")"
else
  step "2. Bump $LOCAL_OLD -> $VERSION"
  mapfile -t TOMLS < <(git ls-files '*Cargo.toml' | grep -E '^(firmware|opendmd-[a-z-]+)/Cargo.toml$')
  [ ${#TOMLS[@]} -gt 0 ] || die "no Cargo.toml files found"
  for f in "${TOMLS[@]}"; do
    v=$(awk '/^\[package\]/ { p = 1; next } /^\[/ { p = 0 } p && /^version *=/ { gsub(/.*= *"|".*/, ""); print; exit }' "$f")
    [ "$v" = "$LOCAL_OLD" ] || die "$f's version is '$v', not firmware's $LOCAL_OLD (every crate carries the same version)"
  done
  # Detached: main stays where it is until publishing fast-forwards it.
  git switch -q --detach
  for f in "${TOMLS[@]}"; do
    awk -v old="$LOCAL_OLD" -v new="$VERSION" '
      /^\[package\]/ { p = 1 } /^\[/ && !/^\[package\]/ { p = 0 }
      p && !done && $0 == "version = \"" old "\"" { $0 = "version = \"" new "\""; done = 1 }
      { print }' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
    lock=${f%.toml}.lock
    [ -f "$lock" ] || continue
    awk -v old="$LOCAL_OLD" -v new="$VERSION" '
      /^name = "opendmd-/ { o = 1; print; next }
      o && $0 == "version = \"" old "\"" { $0 = "version = \"" new "\"" }
      { o = 0; print }' "$lock" > "$lock.tmp" && mv "$lock.tmp" "$lock"
  done
  for f in "${TOMLS[@]}"; do
    grep -qx "version = \"$VERSION\"" "$f" || die "$f was not bumped"
    lock=${f%.toml}.lock
    [ ! -f "$lock" ] || ! grep -A1 '^name = "opendmd-' "$lock" | grep -qx "version = \"$LOCAL_OLD\"" || die "$lock still names $LOCAL_OLD"
  done
  git add -- "${TOMLS[@]}" $(for f in "${TOMLS[@]}"; do [ -f "${f%.toml}.lock" ] && echo "${f%.toml}.lock"; done)
  git diff --cached --stat
  if [ -n "$TRAILERS" ]; then git commit -q -m "Release $VERSION" -m "$TRAILERS"; else git commit -q -m "Release $VERSION"; fi
  RELEASE_COMMIT=$(git rev-parse HEAD)
  git tag -a "$TAG" -m "OpenDMD $VERSION" -m "Publisher: $PUBLISHER" "$RELEASE_COMMIT"
  CREATED_TAG=1
  [ -z "$(git status --porcelain)" ] || die "the tree is not clean after the bump"
  TAG_PUBLISHER=$PUBLISHER
  pass "committed \"Release $VERSION\" ($(git rev-parse --short HEAD), detached; main unchanged) and tagged $TAG locally (Publisher: $PUBLISHER)"
fi

# ---- 3-5. build, assets, site ---------------------------------------------

STAMP=$REL/built
built_ok() {
  [ -f "$STAMP" ] && [ "$(cat "$STAMP")" = "$RELEASE_COMMIT" ] && [ -f "$ASSETS/SHA256SUMS" ] &&
    (cd "$ASSETS" && sha256sum --quiet -c SHA256SUMS) &&
    [ "$(jq -r .version "$SITE/install/beta.json" 2>/dev/null)" = "$VERSION" ] &&
    [ "$(jq -r .version "$SITE/install/release.json" 2>/dev/null)" = "$REL_VERSION" ]
}

if [ "$RESUME" = 1 ] && [ "$MODE" = ci ]; then
  step "3-5. Build, assets, site"
  skip "resuming after the push: CI builds what ships; this checkout's build was the check before the tag went out"
elif [ "$RESUME" = 1 ] && built_ok; then
  step "3-5. Build, assets, site"
  skip "this release's build in $REL is complete (assets match SHA256SUMS, the site is $VERSION's)"
else
  rm -f "$STAMP"

  step "3. Build (make fw-release, signed)"
  rm -rf "$DIST"
  make fw-release "${MK[@]}"
  pass "make fw-release"

  step "3b. Check the version in the images, the manifests and the www pack"
  for v in 4mb; do
    app="$DIST/opendmd-$VERSION-$v-app.bin"
    desc="$(dd if="$app" bs=1 skip=48 count=32 2>/dev/null | tr -d '\000')"
    [ "$desc" = "$VERSION $v" ] || die "$app's descriptor is '$desc', want '$VERSION $v'"
    pass "$v app descriptor: '$desc'"
    manifest="$(jq -r .version "$DIST/manifest-$v.json")"
    [ "$manifest" = "$VERSION" ] || die "manifest-$v.json says $manifest, want $VERSION"
    pass "manifest-$v.json version $manifest"
    # The offline install (docs/deploying.md "Offline installs"): the same
    # version, the six parts plus the NVS image at 0x9000, the image the
    # nvs partition's size (firmware/partitions.csv).
    off="$DIST/manifest-$v-offline.json" nvs="$DIST/nvs-offline-$v.bin"
    read -r nvs_off nvs_size < <(awk -F'[[:space:]]*,[[:space:]]*' '$1 == "nvs" && $3 == "nvs" { print $4, $5 }' firmware/partitions.csv)
    nvs_off=$(( nvs_off )) nvs_size=$(( nvs_size ))
    [ "$(jq -r .version "$off")" = "$VERSION" ] || die "manifest-$v-offline.json is not $VERSION"
    jq -e --slurpfile m "$DIST/manifest-$v.json" --arg nvs "nvs-offline-$v.bin" --argjson off "$nvs_off" '
      (.builds | length) == 1 and (.builds[0].parts | length) == 7 and ($m[0].builds[0].parts | length) == 6
      and (.builds[0].parts | del(.[2])) == $m[0].builds[0].parts and .builds[0].parts[2] == {path: $nvs, offset: $off}
      and .new_install_prompt_erase == true and .new_install_improv_wait_time == 0 and .name == $m[0].name' "$off" >/dev/null ||
      die "manifest-$v-offline.json is not manifest-$v.json's six parts and nvs-offline-$v.bin at $nvs_off (erase asked, no Improv wait)"
    [ "$(stat -c %s "$nvs")" -eq "$nvs_size" ] && [ "$(od -An -tx4 -N4 "$nvs" | tr -d ' ')" = fffffffe ] ||
      die "nvs-offline-$v.bin is not a $nvs_size B NVS partition image"
    pass "manifest-$v-offline.json: the six parts and nvs-offline-$v.bin at $nvs_off ($nvs_size B, an NVS image)"
    uv run --quiet tools/fw-sign.py inspect "$DIST/opendmd-$VERSION-$v.odfw" | tee "$REL/inspect-$v.txt"
    grep -qx "variant  $v" "$REL/inspect-$v.txt" || die "opendmd-$VERSION-$v.odfw is not a $v bundle"
    grep -qx "layout   v[0-9]* (current)" "$REL/inspect-$v.txt" || die "opendmd-$VERSION-$v.odfw is not for this tree's $v partition table"
    uv run --quiet tools/fw-sign.py verify "$DIST/opendmd-$VERSION-$v.odfw" --pub firmware/keys/opendmd.pub >/dev/null ||
      die "opendmd-$VERSION-$v.odfw does not verify against firmware/keys/opendmd.pub"
    pass "opendmd-$VERSION-$v.odfw: a $v bundle, verifies against the committed firmware/keys/opendmd.pub"
    ! strings "$app" "$DIST/updater.bin" "$DIST/bootloader-$v.bin" | grep -n "$HOME" || die "the release names $HOME"
    pass "$v images do not name $HOME"
    full="$DIST/opendmd-$VERSION-$v-full.bin"
    want=$(( ${v%mb} * 1024 * 1024 ))
    [ "$(stat -c %s "$full")" -eq "$want" ] || die "$full is $(stat -c %s "$full") B, want $want (the $v flash)"
    pass "opendmd-$VERSION-$v-full.bin: $want B"
    # The factory partition the app goes in (firmware/partitions.csv).
    factory=2621440
    size=$(stat -c %s "$app"); spare=$(( factory - size ))
    [ "$spare" -ge 65536 ] || die "$v app is $size B: $spare B spare of the $factory B factory partition, need 65536"
    pass "$v app $size B of $factory B factory: $spare B spare (>= 65536)"
  done
  pack="$DIST/opendmd-$VERSION-www.pack"
  wv="$(uv run --quiet python -I -c 'import sys; b = open(sys.argv[1], "rb").read(64); assert b[:4] == b"ODWP"; print(b[20:20 + b[18]].decode())' "$pack")"
  [ "$wv" = "v$VERSION" ] || die "the www pack's version is '$wv', want 'v$VERSION' (a dirty tree or another tag?)"
  uv run --quiet tools/fw-sign.py verify-www "$pack" --pub firmware/keys/opendmd.pub >/dev/null ||
    die "opendmd-$VERSION-www.pack is not signed with the committed firmware/keys/opendmd.pub's key"
  pass "www pack: '$wv', signed (verifies against firmware/keys/opendmd.pub)"

  step "4. Collect this version's assets"
  rm -rf "$ASSETS"; mkdir -p "$ASSETS"
  files=("opendmd-$VERSION-www.pack" ota_data_initial.bin updater.bin THIRD-PARTY-LICENSES-firmware.txt THIRD-PARTY-LICENSES-web.txt)
  for v in 4mb; do
    files+=("opendmd-$VERSION-$v-app.bin" "opendmd-$VERSION-$v-full.bin" "opendmd-$VERSION-$v.odfw"
            "bootloader-$v.bin" "partition-table-$v.bin" "manifest-$v.json" "manifest-$v-offline.json" "nvs-offline-$v.bin")
  done
  for f in "${files[@]}"; do cp "$DIST/$f" "$ASSETS/"; done
  (cd "$ASSETS" && sha256sum -- * | tee SHA256SUMS)
  tools/release-notes.sh "$VERSION" "$CHANNEL" "$ASSETS/SHA256SUMS" "$(git rev-parse "refs/tags/$TAG")" "$PUBLISHER" > "$REL/notes.md"
  pass "$(( ${#files[@]} + 1 )) assets (with SHA256SUMS) in $ASSETS; notes in $REL/notes.md"

  step "5. The public site"
  # The demo card's Kenney (CC0) animations: made from the sprites committed
  # in the art pipeline (offline), as release.yml "The art demos' GIFs" does.
  out=opendmd-web/experiments/art/process/out
  gifs=(b_walk.gif b_pan.gif d_scroll.gif)
  have_gifs() { local f; for f in "${gifs[@]}"; do [ -f "$1/$f" ] || return 1; done; }
  if have_gifs $out; then
    skip "the art demos' GIFs are there"
  else
    (cd opendmd-web/experiments/art/process && export ART_TMP=$RELEASE_DIR/art PYTHONDONTWRITEBYTECODE=1 &&
      uv run --quiet fetch_inputs.py kenney && uv run --quiet demos.py b d && git checkout -- out)
    pass "the art demos' GIFs made"
  fi
  # The gallery's contributions and the earlier API pages: the public repo,
  # read-only over https (release.yml "Fetch the formula gallery ...").
  g=$REL/site-gallery
  rm -rf "$g"
  git clone -q --filter=blob:none --no-checkout "$GALLERY_URL" "$g"
  git -C "$g" sparse-checkout set --no-cone '/gallery/*.psi' '/dev/api/v*.html'
  git -C "$g" checkout -q
  mkdir -p "$g/gallery" "$g/dev/api"
  pass "public gallery repo: $(find "$g/gallery" -maxdepth 1 -name '*.psi' | wc -l) gallery files, $(find "$g/dev/api" -maxdepth 1 -name 'v*.html' | wc -l) earlier API pages"
  # The release.json side: this build, or (a beta after a final) the final's
  # files from its GitHub release, downloaded here (read only) and checked.
  if [ "$CHANNEL" = beta ]; then
    FINAL_DIST=$REL/final-dist
    rm -rf "$FINAL_DIST"; mkdir -p "$FINAL_DIST"
    gh release download "v$FINAL" -R "$GH_REPO" -D "$FINAL_DIST" || die "cannot download v$FINAL's assets"
    (cd "$FINAL_DIST" && sha256sum --quiet -c SHA256SUMS) || die "v$FINAL's assets do not match its SHA256SUMS"
    for v in 4mb; do
      [ "$(jq -r .version "$FINAL_DIST/manifest-$v.json")" = "$FINAL" ] || die "v$FINAL's manifest-$v.json is not $FINAL"
      uv run --quiet tools/fw-sign.py verify "$FINAL_DIST/opendmd-$FINAL-$v.odfw" --pub firmware/keys/opendmd.pub >/dev/null ||
        die "v$FINAL's opendmd-$FINAL-$v.odfw does not verify against firmware/keys/opendmd.pub"
      uv run --quiet tools/fw-sign.py inspect "$FINAL_DIST/opendmd-$FINAL-$v.odfw" > "$REL/inspect-final-$v.txt"
    done
    uv run --quiet tools/fw-sign.py verify-www "$FINAL_DIST/opendmd-$FINAL-www.pack" --pub firmware/keys/opendmd.pub >/dev/null ||
      die "v$FINAL's opendmd-$FINAL-www.pack does not verify against firmware/keys/opendmd.pub"
    pass "v$FINAL's assets from its GitHub release (SHA256SUMS, manifests, bundles and www pack verify) in $FINAL_DIST"
    SITE_DISTS=(DIST="$FINAL_DIST" BETA_DIST="$DIST")
  else
    SITE_DISTS=(DIST="$DIST")
  fi
  rm -rf "$SITE"
  make site "${SITE_DISTS[@]}" SITE="$SITE" GALLERY="$g/gallery" API_ARCHIVE="$g/dev/api" API_FREEZE=yes
  pass "make site (${SITE_DISTS[*]})"

  step "5b. Check the site's release.json, beta.json and installer"
  inst=$SITE/install
  # check_feed FILE VER DIR SRC INSPECT: install/FILE names VER's bundles and
  # www pack under install/DIR/, byte for byte SRC's (a dist), each layout
  # the one its bundle is for (INSPECT: fw-sign.py inspect's output, per layout).
  check_feed() {
    local file=$1 ver=$2 dir=$3 src=$4 insp=$5 f l v
    cat "$inst/$file"
    [ "$(jq -r .version "$inst/$file")" = "$ver" ] || die "$file's version is not $ver"
    [ "$(jq -r .firmware "$inst/$file")" = "$dir/opendmd-$ver-4mb.odfw" ] || die "$file's firmware is not the 4mb bundle"
    for v in 4mb; do
      f="$(jq -r ".variants[\"$v\"].firmware" "$inst/$file")"
      [ "$f" = "$dir/opendmd-$ver-$v.odfw" ] || die "$file's variants.$v.firmware is '$f'"
      cmp "$inst/$f" "$src/opendmd-$ver-$v.odfw" || die "$inst/$f is not $src's"
      l="$(jq -r ".variants[\"$v\"].layout" "$inst/$file")"
      grep -Eq "^layout   v$l( |\$)" "$insp-$v.txt" || die "$file's variants.$v.layout is '$l', not the bundle's table"
    done
    f="$(jq -r .www "$inst/$file")"
    [ "$f" = "$dir/opendmd-$ver-www.pack" ] && cmp "$inst/$f" "$src/opendmd-$ver-www.pack" || die "$file's www is '$f', not $src's pack"
    pass "$file: version $ver, every layout's bundle and the www pack under install/$dir/ ($src's)"
  }
  # check_offline DIR BUTTON SRC: SRC's offline install (its manifest and
  # NVS image) under install/DIR/ byte for byte, and BUTTON's box that
  # swaps to it. SRC is this build (it has one: 3b) or a final's release
  # (one from before the offline install has none: then none on the page).
  check_offline() {
    local dir=$1 button=$2 src=$3 f
    if [ ! -f "$src/manifest-4mb-offline.json" ]; then
      [ "$src" != "$DIST" ] || die "$DIST has no manifest-4mb-offline.json"
      [ ! -e "$inst/$dir/manifest-4mb-offline.json" ] && ! grep -q "data-offline-for=\"$button\"" "$inst/index.html" ||
        die "install/$dir/ has an offline install $src does not have"
      skip "install page: $src has no offline install (a release from before it): no box for $button"
      return
    fi
    for f in manifest-4mb-offline.json nvs-offline-4mb.bin; do
      cmp "$inst/$dir/$f" "$src/$f" || die "install/$dir/$f is not $src's"
    done
    grep -q "data-manifest-offline=\"$dir/manifest-4mb-offline.json\"" "$inst/index.html" &&
      grep -q "data-offline-for=\"$button\"" "$inst/index.html" || die "the install page has no offline box for $button"
    pass "install page: $button's offline box, install/$dir/manifest-4mb-offline.json and nvs-offline-4mb.bin ($src's)"
  }
  if [ "$CHANNEL" = beta ]; then
    check_feed release.json "$FINAL" firmware "$FINAL_DIST" "$REL/inspect-final"
    check_feed beta.json "$VERSION" firmware/beta "$DIST" "$REL/inspect"
    INSTALL_SRC=$FINAL_DIST
    grep -q 'manifest="firmware/beta/manifest-4mb.json"' "$inst/index.html" || die "the install page has no beta button"
    cmp "$inst/firmware/beta/manifest-4mb.json" "$DIST/manifest-4mb.json" || die "install/firmware/beta/manifest-4mb.json is not this beta's"
    pass "install page: the beta's own button, its 4mb manifest under firmware/beta/"
    check_offline firmware/beta install-beta-4mb "$DIST"
  else
    check_feed release.json "$VERSION" firmware "$DIST" "$REL/inspect"
    cmp "$inst/beta.json" "$inst/release.json" || die "beta.json is not release.json (no beta is newer than $VERSION)"
    [ ! -e "$inst/firmware/beta" ] && ! grep -q 'beta-install' "$inst/index.html" || die "the site has a beta installer, with no beta newer than $VERSION"
    pass "beta.json is release.json; no beta installer"
    INSTALL_SRC=$DIST
  fi
  grep -q 'manifest="firmware/manifest-4mb.json"' "$inst/index.html" || die "the install page has no 4mb button"
  grep -q "OpenDMD $REL_VERSION\." "$inst/index.html" || die "the install page does not name $REL_VERSION"
  ! grep -q '8mb' "$inst/index.html" || die "the install page names an 8mb build"
  [ -f "$inst/firmware/manifest-4mb.json" ] || die "install/firmware/manifest-4mb.json is missing"
  ! ls "$inst/firmware" | grep -q 8mb || die "install/firmware has 8mb files"
  cmp "$inst/firmware/manifest-4mb.json" "$INSTALL_SRC/manifest-4mb.json" || die "install/firmware/manifest-4mb.json is not $REL_VERSION's"
  cmp "$inst/firmware/manifest.json" "$INSTALL_SRC/manifest-4mb.json" || die "install/firmware/manifest.json is not the 4mb manifest"
  pass "install page: $REL_VERSION's 4mb button, its manifest published, no 8mb files"
  check_offline firmware install-4mb "$INSTALL_SRC"
  [ -f "$SITE/dev/api/$TAG.html" ] || die "the site has no frozen dev/api/$TAG.html"
  grep -q "OpenDMD $VERSION" "$SITE/dev/api/$TAG.html" || die "dev/api/$TAG.html does not name $VERSION"
  grep -q "api/$TAG.html" "$SITE/dev/api.html" || die "dev/api.html does not link $TAG.html"
  pass "this release's API page frozen: dev/api/$TAG.html, linked from dev/api.html"

  step "5c. site-smoke"
  make site-smoke "${SITE_DISTS[@]}" SITE="$SITE"
  pass "make site-smoke"

  step "5d. site-deploy-check (cf deploy --dry-run: uploads nothing)"
  make site-deploy-check SITE="$SITE"
  pass "make site-deploy-check"

  step "5e. The tree is clean after the build"
  git status --porcelain
  [ -z "$(git status --porcelain)" ] || die "the build left the tree dirty"
  pass "the tree is clean"
  echo "$RELEASE_COMMIT" > "$STAMP"
fi

if [ "$PUBLISH" = 0 ]; then
  step "Dry run done"
  pass "every build and check step passed for $TAG; nothing was pushed or published"
  info "assets: $ASSETS"
  info "site:   $SITE"
  info "to publish: tools/release.sh $VERSION --publish (make release VERSION=$VERSION PUBLISH=1): rebuilds from a fresh release commit, pushes the tag, CI publishes"
  exit 0
fi

# ---- 6. publish -----------------------------------------------------------

PUBLISHING=1

step "6a. main -> the release commit"
if [ "$(git rev-parse main)" = "$RELEASE_COMMIT" ]; then
  skip "main is already $TAG's commit"
else
  # Only from where this run started: a main that moved meanwhile stops it.
  git update-ref -m "release $TAG" refs/heads/main "$RELEASE_COMMIT" "$START" || die "main moved since the start ($START); nothing published"
  git switch -q main
  published "local main fast-forwarded to $(git rev-parse --short "$RELEASE_COMMIT") (Release $VERSION)"
fi
[ -z "$(git status --porcelain)" ] || die "the tree is not clean"

# push_to NAME URL GIT...: main and the tag to a remote, each only if missing.
push_to() {
  local name=$1 url=$2; shift 2
  local tracking=refs/remotes/$name/main refs=() obj
  "$@" fetch -q "$url" "+refs/heads/main:$tracking" || die "cannot fetch $name's main"
  if git merge-base --is-ancestor "$RELEASE_COMMIT" "$tracking"; then
    skip "$name's main has the release commit"
  elif git merge-base --is-ancestor "$tracking" "$RELEASE_COMMIT"; then
    refs+=("$RELEASE_COMMIT:refs/heads/main")
  else
    die "$name's main ($(git rev-parse --short "$tracking")) and the release have diverged: not pushing"
  fi
  obj=$("$@" ls-remote "$url" "refs/tags/$TAG" | awk '{ print $1 }')
  if [ -z "$obj" ]; then
    refs+=("refs/tags/$TAG:refs/tags/$TAG")
  elif [ "$obj" = "$(git rev-parse "refs/tags/$TAG")" ]; then
    skip "$name has the tag $TAG"
  else
    die "$name's $TAG is $obj, not the local tag: never moved"
  fi
  [ ${#refs[@]} -gt 0 ] || return 0
  "$@" push -q --atomic "$url" "${refs[@]}" || die "push to $name failed"
  published "pushed to $name: ${refs[*]}"
}

step "6b. Push to GitHub$( [ "$TAG_PUBLISHER" = ci ] && echo ' (the tag starts release.yml)')"
push_to github "$GH_URL" ghgit
step "6c. Push to origin (the local mirror)"
push_to origin origin git

if [ "$MODE" = ci ]; then
  # ---- 6d-6f (--publish): release.yml publishes; wait for it, then check.
  step "6d. CI: release.yml's run for $TAG"
  run=
  for _ in $(seq 60); do
    run=$(ci_runs | head -1) || true
    [ -z "$run" ] || break
    sleep 5
  done
  [ -n "$run" ] || die "no release.yml run for $TAG after 5 minutes: see https://github.com/$GH_REPO/actions/workflows/release.yml (CI down? then PUBLISH=local RESUME=1 takes over, as long as no run starts)"
  id=$(cut -f1 <<<"$run")
  url="https://github.com/$GH_REPO/actions/runs/$id"
  info "$url"
  if [ "$(cut -f2 <<<"$run")" != completed ]; then
    info "waiting for it (a cold firmware build takes 30 minutes or more; interrupting this is safe: RESUME=1 waits again)"
    gh run watch "$id" -R "$GH_REPO" --interval 30 --exit-status >/dev/null || true
    run=$(ci_runs | awk -F'\t' -v id="$id" '$1 == id')
  fi
  [ "$(cut -f2 <<<"$run")" = completed ] || die "run $id is $(cut -f2 <<<"$run")"
  [ "$(cut -f3 <<<"$run")" = success ] ||
    die "release.yml's run for $TAG ended '$(cut -f3 <<<"$run")': $url. Fix the cause, rerun the whole run (gh run rerun $id -R $GH_REPO, not --failed, which keeps the first attempt's plan; safe: a rerun never replaces a published asset), then RESUME=1 to check"
  published "release.yml published $TAG ($url)"

  step "6e. Check the GitHub release CI made"
  json=$(gh release view "$TAG" -R "$GH_REPO" --json isDraft,isPrerelease,assets) || die "no GitHub release $TAG"
  [ "$(jq -r .isDraft <<<"$json")" = false ] || die "$TAG is still a draft"
  want_pre=false; [ "$KIND" = prerelease ] && want_pre=true
  [ "$(jq -r .isPrerelease <<<"$json")" = "$want_pre" ] || die "$TAG's isPrerelease is not $want_pre"
  have=$(jq -r '.assets[] | select(.state == "uploaded") | .name' <<<"$json" | sort)
  want=$( [ ! -d "$ASSETS" ] || find "$ASSETS" -maxdepth 1 -type f -printf '%f\n' | sort)
  if [ -n "$want" ]; then
    [ "$have" = "$want" ] || die "$TAG's assets are not this build's set: $(diff <(echo "$want") <(echo "$have") | grep '^[<>]' | tr '\n' ' ')"
  fi
  pub=$REL/published
  rm -rf "$pub"; mkdir -p "$pub"
  gh release download "$TAG" -R "$GH_REPO" -D "$pub" -p SHA256SUMS -p "opendmd-$VERSION-4mb.odfw" -p "opendmd-$VERSION-www.pack" -p manifest-4mb.json \
    -p manifest-4mb-offline.json -p nvs-offline-4mb.bin || die "cannot download $TAG's assets"
  for f in manifest-4mb-offline.json nvs-offline-4mb.bin; do [ -f "$pub/$f" ] || die "$TAG has no $f"; done
  (cd "$pub" && sha256sum --quiet -c --ignore-missing SHA256SUMS) || die "$TAG's assets do not match its SHA256SUMS"
  [ "$(jq -r .version "$pub/manifest-4mb.json")" = "$VERSION" ] || die "$TAG's manifest-4mb.json is not $VERSION"
  [ "$(jq -r .version "$pub/manifest-4mb-offline.json")" = "$VERSION" ] || die "$TAG's manifest-4mb-offline.json is not $VERSION"
  # (CI's own build: the image's size and page header, as 3b checks this one's.)
  read -r nvs_off nvs_size < <(awk -F'[[:space:]]*,[[:space:]]*' '$1 == "nvs" && $3 == "nvs" { print $4, $5 }' firmware/partitions.csv)
  [ "$(stat -c %s "$pub/nvs-offline-4mb.bin")" -eq $(( nvs_size )) ] && [ "$(od -An -tx4 -N4 "$pub/nvs-offline-4mb.bin" | tr -d ' ')" = fffffffe ] &&
    jq -e --argjson off $(( nvs_off )) '.builds[0].parts[2] == {path: "nvs-offline-4mb.bin", offset: $off} and (.builds[0].parts | length) == 7' "$pub/manifest-4mb-offline.json" >/dev/null ||
    die "$TAG's manifest-4mb-offline.json and nvs-offline-4mb.bin are not the offline install (the NVS image at the nvs partition, its size)"
  uv run --quiet tools/fw-sign.py verify "$pub/opendmd-$VERSION-4mb.odfw" --pub firmware/keys/opendmd.pub >/dev/null ||
    die "$TAG's opendmd-$VERSION-4mb.odfw does not verify against firmware/keys/opendmd.pub"
  uv run --quiet tools/fw-sign.py verify-www "$pub/opendmd-$VERSION-www.pack" --pub firmware/keys/opendmd.pub >/dev/null ||
    die "$TAG's www pack does not verify against firmware/keys/opendmd.pub"
  pass "$TAG: published ($KIND), $(wc -l <<<"$have") assets, SHA256SUMS holds; the bundle and the www pack verify against firmware/keys/opendmd.pub"

  step "6f. opendmd.dev serves it (tools/site-live-check.sh)"
  if [ "$CHANNEL" = beta ]; then
    gh release download "v$FINAL" -R "$GH_REPO" -p SHA256SUMS -O "$pub/SHA256SUMS-final" --clobber || die "cannot download v$FINAL's SHA256SUMS"
    SITE_LIVE_WAIT=30 tools/site-live-check.sh "$VERSION" "$FINAL" "$pub/SHA256SUMS-final" "$pub/SHA256SUMS" || die "opendmd.dev does not serve $TAG as it should"
  else
    SITE_LIVE_WAIT=30 tools/site-live-check.sh "$VERSION" "$VERSION" "$pub/SHA256SUMS" || die "opendmd.dev does not serve $TAG as it should"
  fi
  pass "opendmd.dev: beta.json $VERSION, release.json $REL_VERSION, the release's own bytes"
else
  # ---- 6d-6f (--publish-local): everything from here.
  step "6d. GitHub release $TAG on $GH_REPO (tools/release-publish.sh)"
  # GUARD_CI_RUNS: a release.yml run that starts meanwhile (a takeover of a
  # tag CI was slow to pick up) stops this before the release is made or published.
  out=$(GH_REPO=$GH_REPO PUBLISHER=local GUARD_CI_RUNS=1 tools/release-publish.sh "$TAG" "$ASSETS" "$REL/notes.md") || { echo "$out"; die "tools/release-publish.sh failed"; }
  echo "$out"
  what=$(sed -n 's/^published: //p' <<<"$out" | tail -1)
  if [ "$what" = nothing ]; then skip "the release and every asset were already there"; else published "GitHub release: $what"; fi

  step "6e. The public gallery repo (opendmd/opendmd.github.io)"
  rm -rf "$REL/gallery-repo"
  out=$(GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=credential.helper GIT_CONFIG_VALUE_0='!gh auth git-credential' \
    GALLERY_REPO_DESCRIBE="$TAG" GALLERY_REPO_WORK="$REL/gallery-repo" \
    tools/gallery-repo-publish.sh "$SITE" "$GALLERY_URL") || { echo "$out"; die "tools/gallery-repo-publish.sh failed"; }
  echo "$out"
  case $out in *"no changes"*) skip "the gallery repo already has $TAG's files" ;; *) published "gallery repo updated: $out" ;; esac

  step "6f. The site on Cloudflare (opendmd.dev)"
  # (beta.json names this release whatever the channel; release.json this
  # one, or for a beta after a final, the final still.)
  live() {
    if [ "$CHANNEL" = beta ]; then
      tools/site-live-check.sh "$VERSION" "$FINAL" "$REL/final-dist/SHA256SUMS" "$ASSETS/SHA256SUMS"
    else
      tools/site-live-check.sh "$VERSION" "$VERSION" "$ASSETS/SHA256SUMS"
    fi
  }
  if SITE_LIVE_WAIT=0 live >/dev/null 2>&1; then
    skip "$SITE_URL already serves $TAG (beta.json $VERSION, release.json $REL_VERSION, this build's bytes)"
  else
    make site-deploy-built SITE="$SITE" || die "make site-deploy-built failed"
    SITE_LIVE_VERSION=$(git describe --always --dirty --tags --match 'v*') live || die "deployed, but $SITE_URL does not serve $TAG as it should"
    published "site deployed: $SITE_URL/install/beta.json says $VERSION, release.json $REL_VERSION"
  fi
fi

step "Published $TAG"
printf '  - %s\n' "${DONE[@]:-nothing new (everything was already done)}"
info "https://github.com/$GH_REPO/releases/tag/$TAG"
info "$SITE_URL/install/"
