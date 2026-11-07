#!/usr/bin/env bash
# A GitHub release's notes (release.yml, and tools/release.sh PUBLISH=local):
#
#   tools/release-notes.sh VERSION CHANNEL SHA256SUMS TAG_OBJECT PUBLISHER > notes.md
#
# CHANNEL: both, or beta (a beta after a final: docs/deploying.md "Betas").
# TAG_OBJECT: the annotated tag's object id the release was built from;
# PUBLISHER: ci or local. Both end the notes as lines of their own
# ("Tag object: <sha>", "Publisher: <who>"): release.yml takes a published
# release's assets as its build only when its tag object is still the tag's
# (never for a moved or re-pushed tag), and never adopts or completes a
# release whose notes say "Publisher: local" (tools/release-publish.sh
# refuses a release made by the other publisher). GitHub's generated notes
# (the changes since the last release) follow them.
set -euo pipefail
v=$1 channel=$2 sums=$3 tagobj=$4 publisher=$5
[[ $tagobj =~ ^[0-9a-f]{40}$ ]] || { echo "release-notes.sh: '$tagobj' is not a tag object id" >&2; exit 2; }
case $publisher in ci|local) ;; *) echo "release-notes.sh: publisher '$publisher' is not ci or local" >&2; exit 2 ;; esac
where='https://opendmd.dev/install/'
[ "$channel" != beta ] || where="$where (this beta under \"Or install the beta\")"
cat <<EOF
For the ESP32 DevKit V1 (4 MB); the browser installer is at $where.

| file | DevKit (4 MB) |
|---|---|
| update over WiFi (Update from file…) | \`opendmd-$v-4mb.odfw\` |
| whole flash at offset 0 | \`opendmd-$v-4mb-full.bin\` |
| ESP Web Tools manifest | \`manifest-4mb.json\` |

The DevKit build is for flash layout v3 (a bigger firmware partition; docs/deploying.md): a DevKit on layout v2 (1.0.0-beta.15 or earlier) refuses its update file and needs one USB install from https://opendmd.dev/install/ (Update OpenDMD, or Install with Erase unticked, keeps the WiFi and settings).

SHA-256:

\`\`\`
$(cat "$sums")
\`\`\`

Tag object: $tagobj
Publisher: $publisher
EOF
