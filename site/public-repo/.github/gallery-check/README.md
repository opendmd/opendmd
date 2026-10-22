# The formula gallery's check

`.github/workflows/gallery-check.yml` runs on every pull request to this
repository and checks the formulas it adds to `gallery/`. Everything under
`.github/` here is installed by OpenDMD's `tools/site-publish.sh` from the
main repository (`site/public-repo/`, and the checker's source from
`opendmd-psi/` and `opendmd-gallery/`): change it there.

## What it checks

`opendmd-gallery pr` reads the pull request's merge commit from git's
object store (`git diff-tree`, `git cat-file`; nothing is checked out) and
fails unless every change:

- adds or changes one file `gallery/<slug>.psi`: no other path, no deletion,
  no subfolder, no symbolic link, submodule or executable file, at most 20
  files;
- has a slug of lower-case letters, digits and single hyphens (≤ 48);
- is at most 4 KB of UTF-8 text with no control characters (a tab only in
  the formula), no invisible, joining or direction-changing characters (the
  "Trojan Source" ones: U+202A–202E, U+2066–2069, …), no private-use ones;
- starts with the header: `-- name:` (≤ 40 characters), `-- description:`
  (≤ 120), `-- author:` (≤ 40), `-- licence:` (CC0-1.0 or CC-BY-4.0), and
  optionally `-- values:` (`a=0.3 b=1`), with no `<` or `>` in them;
- compiles with OpenDMD's Psi compiler (the same crate the display runs) and
  costs at most 8 ms a frame on the display (`Program::esp32_us`).

It draws eight frames of each formula that passes into a PNG, kept as the
run's `formula-previews` artifact (14 days) for the reviewer, and writes a
summary on the run's page.

## Threat model

**Who:** anyone with a GitHub account can open a pull request, from a fork.
**What they control:** the whole pull request: the gallery file, and also
anything else in the tree, including this workflow file and the checker's
source in the pull request's copy.

**How the workflow is set up:**

- Trigger `pull_request` only, never `pull_request_target`. A fork's run
  gets a read-only `GITHUB_TOKEN` and no secrets: GitHub withholds them
  from fork runs no matter what the workflow file says.
- `permissions: contents: read` and nothing else. The workflow uses no
  secrets. Uploading an artifact needs no write permission.
- One third-party action, `actions/upload-artifact`, pinned by full commit
  SHA. No `actions/checkout`: the pull request is fetched with plain `git`
  as objects, never checked out.
- The checker is built from the **base branch's** copy (the merge commit's
  first parent), with `cargo build --locked --offline`: it depends on
  nothing but its own source and OpenDMD's Psi crate, both vendored here, so
  nothing is downloaded and no build script runs from the contribution.
- The contribution is only ever data: its files are read from git objects
  and parsed; a formula is compiled and evaluated by Psi's VM, which has no
  I/O (arithmetic over the pixel's position, the time and three sliders, a
  fixed-size program of at most 256 steps). Nothing the contributor wrote
  is executed as code.
- Untrusted text is printed escaped (`{:?}`) in the log, so it cannot start
  a workflow command, and as HTML entities in the job summary.

**What a malicious pull request can do:**

- Edit `.github/workflows/gallery-check.yml` (or the checker) in its own
  branch: under `pull_request` GitHub runs the pull request's version of the
  workflow, so such a pull request **can make its own check pass**. It
  still runs with a read-only token and no secrets, and can only spend CI
  minutes (10 minutes a run) on GitHub's machine. This is why the check is
  a help for the reviewer, not the gate: **the gate is the review.** A pull
  request that changes anything outside `gallery/*.psi` is never merged,
  whatever its check says (the "Files changed" tab shows it at a glance).
- Make the reviewer look at a picture (the artifact) or text: the summary
  escapes it, and the PNG is drawn by the checker, not supplied.
- Submit a formula that is offensive, misleading or copied: only a person
  can judge that.

**What it cannot do:**

- Get a token that can write to this repository, or any secret (there are
  none in this workflow; GitHub withholds the repository's from forks).
- Run its own code in the base branch's checker, or download anything into
  it (offline, locked, no dependencies).
- Reach the published site without a maintainer merging it. And after a
  merge, the site build (`tools/site.py` in the main repository, run by its
  release workflow) checks every gallery file again with the main
  repository's own copy of the checker and leaves out any that fail — a
  symlink is never followed, a bad file is never published — and renders
  every field as escaped text.

## For the maintainers

- Settings › Actions › General: "Require approval for all external
  contributors" (fork pull request workflows), so a first-time
  contributor's run waits for a maintainer to look at the pull request.
- A ruleset on `main` (Settings › Rules › Rulesets): require a pull request
  with one approving review, require the "Check the formulas" status check,
  block force pushes and deletions. The site publish pushes to `main`
  directly with its deploy key, so add "Deploy keys" to the ruleset's bypass
  list.
- Review checklist: only `gallery/*.psi` files changed; the check passed;
  the preview looks right and works on a 128 × 32 panel; the name and
  description are fine for everyone; the author owns it (or it is CC0 and
  credited); no duplicate of an existing formula.
