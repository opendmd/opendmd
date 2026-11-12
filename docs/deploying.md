# Deploying OpenDMD to the cabinet

This is the procedure for a player that lives in the cabinet rather than on
the bench: a card prepared on a laptop, a release firmware image with no
development credentials in it, installed over USB from a browser (or
espflash), and firmware and web app updates over WiFi afterwards. The
firmware is updated over the LAN through the card: a signed bundle is
staged on it and a small updater program copies it into place
(`docs/ota.md`; section 2, "The flash" and "Updating the firmware").

The short version:

```bash
# once per card, on a laptop with the card in a reader (mounted at $CARD)
cp -r <DMDoS pack>/gifs <DMDoS pack>/filelist.txt "$CARD"/
make pack ARGS="convert $CARD --verify"   # prepare every GIF (the playback cache, under cache/)
sync && umount "$CARD"

# once per board (and once more to move a board to flash table v4): over USB
make fw-release                            # dist/ (4mb, the DevKit): app, bootloader, table, full image, manifests (normal and offline, with its NVS image), signed bundle; updater, otadata, web app pack
make flasher                               # http://localhost:8765 in Chrome/Edge -> the board's Install button
#   or: make fw-release-flash PORT=/dev/ttyUSB0   (after `make fw-erase` if it ran anything else)
# join the OpenDMD-Setup WiFi, enter the network, then http://opendmd.local/

# a firmware update over WiFi (the board must already have table v4: v3 boards refuse v4 updates)
make fw-release && make fw-ota HOST=opendmd.local

# a web app update on its own, over WiFi
make web && make www-push HOST=opendmd.local
```

## 1. The card

### Filesystem: FAT32

The firmware mounts the card with ESP-IDF's FATFS over SPI, which reads FAT32
(FAT12/16 too) but **not exFAT**. A 32 GB (SDHC) card comes from the factory
as FAT32, and that factory layout is aligned to the card controller's erase
blocks; a generic `mkfs.vfat` with default options can misalign it and make
the card slower and wear it harder. So leave a FAT32 card's filesystem alone.
Cards of 64 GB and up are SDXC and ship as exFAT, which the player cannot
read; the player formats such a card itself (below).

What an exFAT card looks like: it is found and mounted (the log says
`microSD mounted at /sdcard (62.53 GB …)`), but nothing on it can be read
(`No such device`), so nothing plays. The panel's status card says **Card
unreadable: format in app**, the web app's Now page says **This card can’t
be read: it needs FAT32** with a **Format card…** button, and `/api/status`
has `sd.mounted` true, `sd.readable` false and `sd.error` "card not
readable: it needs FAT32; format it from the web app". The player tries the
card again once a minute, so a FAT32 card swapped in is picked up without a
restart.

**Format card…** erases the card and makes it FAT32 on the device (`POST
/api/card/format`; it refuses a card that can be read): 32 KB clusters, the
data area aligned to 4 MiB, two FATs. Then the card plays like any other:
add GIFs from the Library, or copy the pack onto it from a laptop (FAT32
reads everywhere). The display keeps its settings on the card in
`/opendmd.json` (written at the first boot with the card, from the
display's own settings): copying the pack never overwrites it, and a
card moved to another display brings them (docs/api.md, Settings). A card of any size works; 32 GB is only the Windows
formatter's limit for FAT32.

The card in the long-running test panel is a 32 GB card that was full-surface scanned with zero errors
and no slow regions. A card that stalls on a bad sector hangs the player —
that is how the original 128 GB DMDoS card failed — so a new card is worth a
contiguous read scan before it goes into the cabinet.

If a card starts failing in service, the player notices (three card read
errors within a minute) and restarts itself. After three such restarts
within 15 minutes it stops restarting, stays on the network without the
card and tries it again every minute; Settings says "Card errors: N,
retrying". That means: replace the card (and scan the old one before
trusting it again).

The card runs at 20 MHz by default. With the opt-in `fast` card speed it
runs at 40 MHz and drops to 20 MHz after its first error; that slow clock is
remembered by the card's serial number, so it survives power cuts and only
goes away when a different card is inserted (Settings says "This card had
errors at Fast, so it runs at Safe now").

### The DMDoS pack

Copy the pack's `gifs/` tree and its `filelist.txt` to the card root, exactly
as they ship. This keeps the card working with stock DMDoS as well, and
`filelist.txt` is **curated**: it is not "every GIF on the card" (the
Halloween and XMAS folders are deliberately left to `filelist.hall` /
`filelist.xmas`), so never regenerate it with a directory scan. Copy the two
seasonal lists too if you want them.

```bash
CARD=/media/$USER/<card>            # wherever the reader mounted it
cp -r /path/to/pack/gifs /path/to/pack/filelist.txt "$CARD"/
```

### Prepare the card (the playback cache)

Users see GIFs and playlists; a **prepared copy** of a GIF
(`docs/cache-format.md`) is a cache: a standard GIF with its LZW capped at 9
bits and delta frames. The player looks for each GIF's copy at
`cache/<Category>/<xx>/<name>.gif` and plays it when it is there, otherwise
the GIF: playlists keep naming GIFs and nothing has to be configured. A copy
needs ~16 KB of heap (canvas included) against ~27 KB for the original
through the same decoder, is 0.61x its size on average, and lives in a
directory of ~15 entries, so the ESP32 never scans the 3,781-file
`gifs/Consoles` for it (each open there costs ~330 ms and shows as a late
frame).

```bash
# A copy of every GIF; --verify plays copy and original and compares every
# frame. Re-running skips files already prepared (the copy carries the GIF's
# mtime). gifs/ and filelist.txt are untouched.
make pack ARGS="convert $CARD --verify"
```

`make pack` is `opendmd-pack` built in release; the same arguments work on
the binary directly (`opendmd-pack convert --verify <card>`). That is the
whole step: no manifest to write. A GIF with no copy simply plays as a GIF.
Files added later from the web app (Library → **Add GIFs**, **Create Art**) are
prepared by the browser as they are uploaded, the player prepares the rest
itself at night, and **Settings → Device → Ready to play** shows how many of the
playlist's files are prepared and can prepare the rest in the background.

**Smooth motion (optional).** To switch it on for every GIF where it
helps (≥ 10% of the frame pairs move in steps of 2 px or more: ~1,000 of
the stock card's 11,047) before preparing, list them and convert — the
list is `/sets/.smooth`, which `convert` honours whenever it is on the
card, and which the web app's Library switch edits later:

```bash
make pack ARGS="smooth analyse $CARD --write"   # prints them, adds them to /sets/.smooth (68 s on 16 jobs)
make pack ARGS="convert $CARD --verify"         # their copies smooth, the rest plain
```

**A card prepared by an earlier build** has an `odm/` tree of the retired
pre-rendered `.odm` copies (and perhaps a `filelist.odm.txt`). The player
ignores both and logs each once at boot. Remove the tree on a laptop (`rm
-r "$CARD"/odm`) or over the network, a few seconds of work per request
until the reply says `"done":true`:

```bash
until curl -s -X DELETE "http://opendmd.local/api/files/odm?recursive=1" | grep -q '"done":true'; do :; done
curl -X DELETE http://opendmd.local/api/cache/failed   # the old failure list went with odm/; start clean
```

On a stock card the tree (7,970 copies, 9,662 entries with their
directories) takes about 60 requests, about 4 minutes, while the panel played (the deletes
hold the card, so expect late frames meanwhile).

The default playlist is `/filelist.txt` ("All files" in the app); `{"set":""}`
selects it, `{"set":"filelist.hall"}` a root manifest, `{"set":"<name>"}`
`/sets/<name>.txt`.

### Eject

```bash
sync && umount "$CARD"
```

Then pull the card. A plain unmount is enough; `udisksctl power-off` has
killed a USB reader on this machine before.

## 2. The firmware and the web app

### The flash

One layout: `4mb`, for the ESP32 DevKit V1 (`docs/ota.md` "The flash
layout").

| partition | offset | size | what |
|---|---|---|---|
| (bootloader) | 0x1000 | | the project's second-stage bootloader |
| (partition table) | 0x8000 | | `firmware/partitions.csv` |
| `nvs` | 0x9000 | 24 KB | WiFi and the secrets; a copy of the settings (their store is the card's `/opendmd.json`), `network` among them; the offline install writes it whole |
| `otadata` | 0xF000 | 8 KB | which app boots next; erased = `factory` |
| `phy_init` | 0x11000 | 4 KB | |
| `factory` | 0x20000 | 2,816 KB | the firmware |
| `ota_0` | 0x2E0000 | 384 KB | the updater (`ota/updater`), run only to install a firmware update |
| `www` | 0x340000 | 768 KB | the web app, one **www pack** (`docs/www-pack.md`) |

This is table **v4** (`docs/ota.md`; `/api/status` and `GET /api/firmware`
say `"layout":4`). v3 (1.0.0-beta.16 to 1.0.1) had a 2,560 KB `factory`,
`ota_0` at 0x2A0000 and a 1,024 KB `www` at 0x300000; v2 (1.0.0-beta.15
and earlier) a 2,304 KB `factory`, `ota_0` at 0x260000 and a 1,280 KB
`www` at 0x2C0000; v1 had one 2,752 KB app at 0x10000 and no `otadata`.
Moving a board to v4 takes one USB install (below), after which firmware
updates go over WiFi again.

### Moving a board to table v4 (once)

An update over WiFi never changes the partition table, and a v4 firmware
does not fit a v3 board's `factory` for long, so the layout is in every
update file and both sides refuse a mismatch before anything is written:

- **A v3 board** (1.0.0-beta.16 to 1.0.1) refuses a v4 `.odfw` (`ODFW44mb`).
  Its firmware does not know the magic and answers 422 "this update is for
  another flash layout than this display's: use the DevKit (4 MB) update
  file (opendmd.dev/install has the latest)" (1.0.0-beta.16's says "not an
  OpenDMD firmware bundle (bad magic)"); its updater, had a bundle got
  that far, logs "not a firmware bundle (bad magic)" and boots the firmware
  that is there (`make emu-api`'s boot C runs exactly that); its web app's
  **Update from file…** says "This file is for another flash layout than
  this display's. Use the DevKit (4 MB) update file".
- **Check for updates** on a v3 board reads the release's
  `variants.4mb.layout` (4, from the bundle's magic: `tools/site.py`) and
  offers nothing, firmware or web app: "Firmware X is out. This firmware
  needs a one-time USB reinstall on this display: use the installer at
  opendmd.dev/install (Update OpenDMD keeps your WiFi and settings)". The
  same holds for `beta.json` with **Get beta updates** on. (The feed's
  `?layout=` is the variant, `4mb`, for the counts only: the Worker
  answers every board the same file, and the web app decides.)
- **A v4 board** refuses a v3 one ("this update is for the older flash
  layout v3 and this DevKit (4 MB) has layout v4: use a release made for
  layout v4 (opendmd.dev/install has the latest)"), and its web app says
  "This file is for an older flash layout than this display has: get the
  latest from opendmd.dev/install".

So every board moves once, over USB, with any of:

- **The installer** (<https://opendmd.dev/install/> or `make flasher`): a
  board on OpenDMD answers Improv, so it offers **Update OpenDMD**, which
  writes the six parts and does not erase: WiFi and settings stay. (A board
  that does not answer gets **Install**: leave **Erase device** unticked to
  keep them.) The offline install (**Play from the card only, no WiFi**)
  works too, and writes `nvs-offline-4mb.bin` at 0x9000 as always: NVS did
  not move, so it is the same image as for v3.
- **`make fw-release-flash PORT=/dev/ttyUSB0`** (no `fw-erase` first): the
  pack and the updater at their new offsets, then bootloader, table,
  `otadata` erased and the app. Keeps NVS.
- **The full image**, `espflash write-bin 0x0 dist/opendmd-<v>-4mb-full.bin`
  (or `esptool write-flash 0 …`): 4 MB from offset 0, **NVS included** (it
  is 0xFF there), so the board comes up with no WiFi (the `OpenDMD-Setup`
  access point) and the WiFi password, the API key and the Follow RePlayOS
  code need entering again; the other settings come back from the card's
  `/opendmd.json`.

What the move writes over: the v3 updater (0x2A0000..0x300000) and the
first 256 KB of the v3 pack (0x300000..0x340000) become the end of the v4
`factory` (the new firmware is ~2.5 MB, so most of that is left as it was
until a bigger firmware uses it); the new updater goes to 0x2E0000 and the
pack to 0x340000, over the rest of the old pack. A move that leaves out the
pack (a bench `espflash flash` of the app alone) finds no pack at 0x340000
(the API and `/setup` work without one); `make www-push` or `/setup` puts
one there. The card is not touched. NVS (0x9000), `otadata` (0xF000) and
`phy_init` are where they were.

After the move, `GET /api/firmware` says `"layout":4` and the boot banner
`(4mb layout v4, …)`; updates over WiFi work as before, with v4 files.

**Back to v3** is the same move the other way: a v3 release's parts
(1.0.1's from its GitHub release: `gh release download v1.0.1`) written the
same way, without erasing (its installer manifest, or its parts by hand at
the v3 offsets: updater 0x2A0000, pack 0x300000, then bootloader, table,
`otadata` erased, app at 0x20000). NVS stays there too.

The web app is not on the card. `make web` packs it (every file gzipped as
`make web` always did) into `opendmd-web/www.pack` — ~0.5 MB — and the
firmware maps that partition at boot, checks its CRC and serves the app
straight out of flash: the UI and the setup page work with no card in the
slot. A `www/` directory left on a card by an older firmware is ignored and
can be deleted.

### Build a release

```bash
make fw-release
```

This is a `cargo build --release` **without** the `dev-credentials` feature:
`firmware/.env` is not read, the two WiFi variables are scrubbed from the
build environment, and the code that would compile them in does not exist in
this build. It runs `make web` first, then builds the layout (`<var>` is
`4mb`) and writes to `dist/`:

| file | flashed at | |
|---|---|---|
| `opendmd-<version>-<var>-app.bin` | 0x20000 | the app image; its descriptor carries `"<version> <var>"` (checked) |
| `updater.bin` | 0x2E0000 | the updater (`make updater`, `ota/updater`) |
| `ota_data_initial.bin` | 0xF000 | `otadata` erased (8 KB of 0xFF): the bootloader boots `factory` |
| `opendmd-<version>-www.pack` | 0x340000 | the web app |
| `bootloader-<var>.bin` | 0x1000 | its header carries the flash size (checked) |
| `partition-table-<var>.bin` | 0x8000 | |
| `opendmd-<version>-<var>-full.bin` | 0x0 | all six merged and padded to the flash size: `esptool write-flash 0 <it>` or any "flash one image" tool (it rewrites NVS too: the settings in it go) |
| `manifest-<var>.json` | | ESP Web Tools: the six parts and their offsets |
| `nvs-offline-<var>.bin` | 0x9000 | the offline install's NVS: the whole `nvs` partition (24 KB), namespace `opendmd`, string `network` = `off` ("Offline installs" below) |
| `manifest-<var>-offline.json` | | ESP Web Tools, the offline install: the six parts and `nvs-offline-<var>.bin`, seven in flash order |
| `opendmd-<version>-<var>.odfw` | | the signed bundle for an update over WiFi (`make fw-ota`; `make fw-sign`); its magic names the layout and table (`ODFW44mb`) |

The target fails if the app image leaves less than 64 KB of the
`factory` partition (2,883,584 B), if the updater
leaves less than 32 KB of its 393,216 B `ota_0`, or if the signed web app
pack leaves less than 64 KB of the `www` partition (786,432 B;
`WWW_PACK_SPARE`, also enforced by `make web`'s size lines); the packer
refuses a pack over the partition outright. Signing needs the private key, `~/.config/opendmd/opendmd.key`
(`uv run tools/fw-sign.py keygen` made it, once; it is never in the repo;
`FW_SIGN_KEY=` points elsewhere); the device checks bundles against
`firmware/keys/opendmd.pub`. The release build needs ~1.5 GB of scratch;
`FW_RELEASE_TARGET_DIR=/roomier/dir make fw-release` puts it elsewhere.

**The flash clock is 80 MHz by default** (the app's image header says so and
the app sets SPI0's clock from it at start-up; the updater and the
bootloader stay at 40). `FW_FLASH_FREQ=40 make fw-release` builds for a board
whose flash cannot run at 80 MHz (a board that crash-loops right after an
80 MHz install or update); the target checks the app header's clock
against it. A board that boot-loops after an 80 MHz install or update
recovers with one USB install of a 40 MHz build (`docs/ota.md`, "If a
board can't run its flash at 80 MHz").

### Releasing

A release is built and checked on the dev box, and published by GitHub
Actions: `.github/workflows/release.yml`, on the `v*` tag the dev box
pushes, is the one publisher (the GitHub release and its assets, the public
gallery repo, the site). `tools/release.sh` (`make release`) does the dev
box's half.

```bash
make release VERSION=1.0.1                        # dry run: builds and checks everything, publishes nothing
make release VERSION=1.0.1 PUBLISH=1              # the same, then pushes main and the tag; CI publishes; waits and checks
make release VERSION=1.0.1 PUBLISH=1 RESUME=1     # after the push: wait for CI's run again (or check it) and check what it published
make release VERSION=1.0.1 PUBLISH=local          # fallback, CI down: publish everything from the dev box
make release VERSION=1.0.1 PUBLISH=local RESUME=1 # after a local publish that stopped part way
```

**A dry run** (the default):

1. Plan: the version is newer than the latest release (a `-` suffix makes
   it a pre-release), its tag `v<version>` exists neither here nor on
   GitHub, the tree is clean and on main, GitHub's and the local mirror's
   main are in it, gh can push, and the signing key is there.
2. Bump: the version in every crate's `Cargo.toml` and `Cargo.lock`,
   committed as "Release <version>" (on a detached HEAD: main does not
   move) and tagged `v<version>` locally (an annotated tag whose message
   says `Publisher: ci`, or `Publisher: local` for `PUBLISH=local`).
3. Build: `make fw-release`, signed with
   `~/.config/opendmd/opendmd.key`, into
   `target/release/v<version>/dist` (`RELEASE_DIR=` moves it; with
   `SCRATCH` set, e.g. in a gitignored `local.mk`, it is
   `$SCRATCH/opendmd-release`); then the version in the images, manifests,
   bundles and www pack, at least 64 KB of factory spare, and the bundle and
   the pack verify against `firmware/keys/opendmd.pub`.
4. Assets: the GitHub release's files, `SHA256SUMS` and the release notes
   (`tools/release-notes.sh`).
5. Site: `make site` with the public gallery repo's contributions and
   earlier API pages, this release's API page frozen; the `release.json`,
   `beta.json` and installer checks (which files each feed names depends
   on the channel: "Betas" below), `make site-smoke`, `make
   site-deploy-check` (uploads nothing), and the tree still clean.

It then goes back to main and deletes the local tag: nothing has left the
machine.

**`PUBLISH=1`** (`--publish`) does all of the above (with
`PUBLISH=1` the plan also checks that the repository has the three secrets
below, by name), then:

1. main fast-forwarded to the release commit;
2. main and the tag pushed to GitHub (over https with gh's credentials) and
   to the local mirror `origin`. The tag push starts `release.yml`;
3. it waits for that run (`gh run watch`; a cold firmware build takes 30
   minutes or more; interrupting the wait is safe, `RESUME=1` waits again);
4. it checks what CI published: the GitHub release (not a draft; a
   pre-release for a `-` version), its assets (the same names as the local
   build; SHA256SUMS holds; the bundle and the www pack verify against
   `firmware/keys/opendmd.pub`), and opendmd.dev live
   (`tools/site-live-check.sh`: `install/beta.json` and
   `install/release.json` name the versions they should and serve the
   release's own bytes).

The build on the dev box is the check that the tag is releasable; what
ships is CI's build of the same commit. Builds are not byte-reproducible,
so the two differ in bytes, never in what they are checked for.

**What `release.yml` does on the tag** (its header has the details):
plans (the tag is the version in `firmware/Cargo.toml` and newer than every
other `v*` tag; every secret is set, or it fails before building), builds
and signs with the `OTA_SIGNING_KEY` secret, runs the same image, bundle,
pack and site checks, then publishes: the GitHub release through
`tools/release-publish.sh` (created as a draft, every asset uploaded, then
published: a pre-release for a `-` version, else marked latest), the public
gallery repo (`SITE_DEPLOY_KEY`), and the site (`make site-deploy-built`,
`CLOUDFLARE_API_TOKEN`), checked live (after waiting for `/api/health` to
report the tag, so it reads the new deploy). One release runs at a time,
whatever its tag. The release notes end with `Tag object: <id>` (the
annotated tag the release was built from) and `Publisher: ci` or
`Publisher: local`. A run that fails part way is rerun on GitHub as a
whole ("Re-run all jobs", or `gh run rerun <id>`; not "Re-run failed
jobs", which keeps the first attempt's plan): once the release is
published its assets are the release, so a rerun builds no firmware and
takes them as they are, provided they were built from the same tag object
(a moved or re-pushed tag is refused); a release whose notes say
`Publisher: local` is never adopted or completed by CI; a final is marked
latest only while it is the newest final; a draft left behind is completed;
same-digest assets are skipped and a published asset is never replaced;
the gallery repo is committed to only when something changed; the deploy is
the same site again. If a newer tag appears while a run is going, that run
skips the gallery repo and the deploy (the newer one's run does them).

**Who signs what.** The first-party key (`~/.config/opendmd/opendmd.key`,
whose public half is the committed `firmware/keys/opendmd.pub`) exists in
two places: on the dev box, and as the `OTA_SIGNING_KEY` secret. CI signs
everything that ships with `PUBLISH=1`: the `.odfw` bundle and the www pack
on the GitHub release and the site. The dev box's copy signs its own
check builds (dry runs, and the build before the tag goes out, which is
never uploaded) and everything `PUBLISH=local` publishes. Every bundle and
pack, either way, is checked to verify against `firmware/keys/opendmd.pub`
before it is published, and again (downloaded) after. CI's dry runs and
`ci.yml` sign with a throwaway key made in the job, checked against its own
public half.

**`PUBLISH=local`** (`--publish-local`) is the fallback for when Actions
is down. It refuses if `release.yml` has run for the tag at all, or if the
GitHub release exists (unless it is resuming a publish it started itself).
Its tag says `Publisher: local`, so a `release.yml` run that starts later
does nothing. It does the dry run's steps, the same pushes, then publishes
from the dev box: the GitHub release and assets (`tools/release-publish.sh`,
the same as CI), the gallery repo (`tools/gallery-repo-publish.sh` over
https with gh's credentials) and the site (`make site-deploy-built` with
cf's own login), checked live. If a `PUBLISH=1` tag was pushed but CI never
started a run for it, `PUBLISH=local RESUME=1` can take it over (no run and
no release exist); should CI run for it later after all, it finds the
release published and takes its assets as they are.

Each step prints its outcome, and the first failure stops it with what was
and wasn't published. A failure before the first push undoes the local
commit and tag. After that, rerun with `RESUME=1`: it skips whatever is
already done. A published tag is never moved, and nothing published on
GitHub is overwritten.

**What the owner sets up once** (Settings → Secrets and variables →
Actions on opendmd/opendmd; `OTA_SIGNING_KEY` and `SITE_DEPLOY_KEY` are
set, how to make them is in `release.yml`'s header):

- **`CLOUDFLARE_API_TOKEN`** (secret), used by `release.yml` and `site.yml`
  to deploy the `opendmd-site` Worker. In the Cloudflare dashboard: My
  Profile → API Tokens → Create Token → the **Edit Cloudflare Workers**
  template, then
  - Permissions: keep the template's (Account: Workers Scripts Edit,
    Account Settings Read, and the rest; User: User Details Read,
    Memberships Read; Zone: Workers Routes Edit) and add **Account → D1 →
    Edit** (the migrations `make site-deploy-built` applies before every
    deploy) and **Zone → DNS → Edit** (the Worker's custom domains,
    `opendmd.dev` and `www.opendmd.dev`, are asserted on every deploy);
  - Account Resources: Include → the one account;
  - Zone Resources: Include → Specific zone → `opendmd.dev`;
  - no client IP filtering (GitHub's runners have no fixed addresses).

  Then, on a machine logged in to gh, type it in at the prompt (so it
  never goes through a file, the shell history or anyone else):

  ```bash
  gh secret set CLOUDFLARE_API_TOKEN -R opendmd/opendmd
  ```
- **`CLOUDFLARE_ACCOUNT_ID`** (a repository *variable*, not a secret: the
  account's id, on the dashboard's Workers & Pages overview), so wrangler
  never has to pick an account: `gh variable set CLOUDFLARE_ACCOUNT_ID -R
  opendmd/opendmd --body <id>` (already set).

To try the token without releasing: run the **Site** workflow by hand
(Actions → Site → Run workflow, on main). It deploys main's docs with the
latest release's files, and checks the result live.

### The changelog

<https://opendmd.dev/changelog/> is written by hand in one file,
`site/pages/changelog.md`: the highlights of each release for a display's
owner, newest first. Add a release's entry at the top, on main, **before**
`make release` (the release needs a clean tree):

```markdown
## 1.2.0 · 2026-12-01

One line that sums the release up.

- Two to five things an owner would notice: new features, big fixes, speed.
```

- The heading is `## <version> · <YYYY-MM-DD>` (a leading `v` is
  accepted and dropped). `tools/site.py` (`changelog`) makes the rest from it: the date in
  words, **Beta** on a pre-release, **Latest** on the release the Install
  button installs, the link to the GitHub release, and the Atom feed
  (`/changelog/feed.xml`). A heading out of order or in another shape, or
  a group with nothing under it, fails `make site`. A tag that never had a
  GitHub release (0.3.0) is listed in `CHANGELOG_TAG_ONLY` and links to
  its tree instead.
- An entry for a version newer than the newest published release is **held
  back**: the page and the feed leave it out until the site is built with
  that release (`make site` says which entries it held back). So the entry
  can be written ahead, as the work merges, and nothing links to a release
  that is not out.
- A run of betas can be folded once their release is out, as "Road to 1.0"
  is: a `## <title>` heading that is not a version, a line under it, then
  each beta as `### <version> · <YYYY-MM-DD>` with a sentence or two.
- Words: GIFs, Formulas, playlists and the web app's own names for things.
  Nothing internal, nothing that is not released or linked from the site
  yet.

A release without an entry still goes out: `make release` prints a `NOTE`
in its plan, and `make site` and `make site-smoke` say `no entry for
<version>`. Nothing fails; add the entry on main afterwards and the next
site deploy shows it.

### Betas: the two update feeds

Releases are `X.Y.Z` (final) or `X.Y.Z-beta.N` (a beta of the next
`X.Y.Z`). Versions are ordered as semver orders them: `1.0.0-beta.16` <
`1.0.0` < `1.0.1-beta.1` < `1.0.1` (the firmware's
`opendmd_core::ota::version_older`, the web app's `older()`, `site.py`'s
`semver_key` and `release.sh`'s `semver_cmp` agree, and are tested on
that order).

The site publishes two feeds of the same shape (`docs/ota.md` "Check for
updates"):

- **`install/release.json`**: the newest **final** release. Players read
  it by default. Before the first final release exists it names the
  newest beta (as it always has), so players keep getting updates.
- **`install/beta.json`**: the newest release of all, beta or final.
  Players with **Get beta updates** on (Settings → Device → Firmware; the
  device setting `beta_updates`, off by default) read it instead. A beta
  player therefore gets the next betas and also a final release once it
  is newer than its beta. Switching it off downgrades nothing: the player
  stays on its beta until a newer final comes out.

What each kind of release does to them (`tools/release.sh` and
`release.yml` work it out from the tags, here and on GitHub):

| release | `release.json` | `beta.json` | Install button at /install/ |
|---|---|---|---|
| a beta, before any final exists (0.x finals such as v0.3.0 predate the channels and do not count) | this beta | this beta (the same file) | this beta |
| a final `X.Y.Z` | this release | this release (the same file) | this release |
| a beta after a final | stays on the last final | this beta | the last final, and a second "Or install the beta" button |

For a beta after a final, the site is built with the last final's files
as `DIST` (downloaded from its GitHub release and checked against its
`SHA256SUMS`: the final stays installable and its update files stay
reachable) and this beta's own build as `BETA_DIST`: `make site
DIST=<final's files> BETA_DIST=<the beta's dist>`. The beta's update
bundles, installer manifest and parts go under `install/firmware/beta/`
(its own bootloader, partition table and updater: a beta may change any of
them), the final's stay under `install/firmware/`. Without `BETA_DIST`
`beta.json` is a copy of `release.json` and there is no beta folder. The
docs on the site are the tree's (the beta's) either way, and the frozen
API page is the beta's (`dev/api/v<beta>.html`), so a beta's Settings →
About link has its page.

Between releases the Site workflow works out both by itself from the
published GitHub releases (`tools/site-releases.sh`). Locally, `make site
DIST=<final's files> BETA_DIST=<beta's files>` (each a GitHub release's
assets: `gh release download v<version> -D dir`). A site built from one
dist alone would point `beta.json` back at the final, which is harmless
(beta players see nothing newer) but hides the beta until the next deploy.

A release must be newer than every `v*` tag (release.sh and release.yml
both refuse otherwise). So once a beta of the next version is out (say
`1.1.0-beta.1`), a hotfix final below it (`1.0.1`) can't be released:
ship the fix as the next beta, or as `1.1.0`. Allowing it would need a
final that leaves `beta.json` on the newer beta.

The Worker counts `beta.json?v=&layout=` as an update check like
`release.json` (one kind, `check`, on /stats), and a beta's install
images under `install/firmware/beta/` as installs of that version.

### Installing from a browser (the flasher)

`flasher/index.html` is an install page built on
[ESP Web Tools](https://esphome.github.io/esp-web-tools/) (10.4.0, from
jsDelivr). It needs **Chrome or Edge** on a desktop (Web Serial; not
Firefox or Safari) and a secure context, which `localhost` is:

```bash
make fw-release    # once, for dist/
make flasher       # serves the page and dist/ on http://localhost:8765
```

Plug the DevKit in by USB, open <http://localhost:8765/>, click **Install**
and pick the serial port (a CH340, "USB Serial" / `ttyUSB0`).

**A board already running OpenDMD answers Improv Serial**
(`firmware/src/improv.rs`): the dialog shows its hostname and `OpenDMD
<version> (ESP32)`, **Update OpenDMD** when its version is not the
manifest's (no erase question: the settings stay), **Change Wi-Fi**,
**Visit device** (`http://<ip>/`), **Logs & Console** and **Erase User
Data** (a full install with erase). The manifest's `name` (`OpenDMD`) and
`version` must match what the firmware reports for that to work: both come
from the same `FW_VERSION`. A board that does not answer (anything else,
or OpenDMD before Improv) gets **Install** and the **Erase device** box,
which starts unticked (`new_install_prompt_erase`):

- **Erase ticked — a first install, or a board coming from DMDoS or any
  other firmware.** The whole chip is erased, then the bootloader, the
  partition table, the app and the web app are written. *Everything* on the
  board goes, NVS included: WiFi credentials, the API key and the Follow
  RePlayOS code (the settings — brightness and its maximum, playlist, the
  schedule and night window, Follow RePlayOS — come back from the card's
  `/opendmd.json` at the next boot with the card). The installer then waits up
  to `new_install_improv_wait_time` (15 s) for the new firmware to answer
  and asks for WiFi: the board scans, joins the network it is given (live,
  within 35 s), saves it and reports its URL. Skipped or unanswered, the
  board comes up as the `OpenDMD-Setup` access point (below). The card is
  not touched.
- **Erase unticked — a firmware update that keeps the settings.** ESP Web
  Tools writes only the six parts (the sectors they cover); NVS at 0x9000
  is not among them, so the WiFi, the schedule and every setting survive.
  This also moves a board from table v1, v2 or v3 (or the older dual-OTA one) to v4:
  NVS is at 0x9000 in every table, the new table replaces the old one, and
  `otadata` is written erased (`ota_data_initial.bin`), so whatever an older
  layout left at 0xF000-0x11000 cannot steer the bootloader: it boots
  `factory`.

#### Offline installs

The install page's **Play from the card only, no WiFi** box (under each
Install button: `flasher/index.html`, `tools/site.py` `OFFLINE_BOX`, both
driven by `flasher/offline.js`) swaps the button's `manifest` attribute to
`manifest-4mb-offline.json` while it is ticked; ESP Web Tools reads the
attribute when the button is clicked. That manifest is `manifest-4mb.json`
plus `nvs-offline-4mb.bin` at 0x9000 (seven parts, in flash order), with
`new_install_prompt_erase: true` and `new_install_improv_wait_time: 0`: no
WiFi form after the install, and no Improv detection before it, so a board
already on OpenDMD is not recognised through it (no Update, no Change
Wi-Fi: those need the box unticked).

`nvs-offline-4mb.bin` is the whole `nvs` partition (offset and size read
from `firmware/partitions.csv`), made by `make fw-release` with IDF v5.3's
`nvs_partition_gen` (`python -m esp_idf_nvs_partition_gen generate`, from
the IDF Python env under `~/.espressif/python_env/idf5.3_py3*_env`, which
the firmware's build sets up) from a CSV holding namespace `opendmd` and one
string, `network` = `off`. It is written whether or not **Erase device** is
ticked, so every setting NVS held goes (WiFi, the API key, the Follow code,
the name, panel timing). `fw-release` asserts both manifests (six parts and
seven, the NVS part at the partition's offset, the image the partition's
size, the Improv waits); `make flasher-smoke` also reads the image back as
an NVS partition.

The setting itself (`network` among the settings, `on` by default) is also the
card's: `{"network":"off"}` in `/opendmd.json` (the page offers that one
line as a download) turns WiFi off at the next boot with the card, and the
card wins over NVS once mounted. With `network` off the firmware starts no
WiFi, setup network or web server; Improv still answers over USB, and its
WifiSettings joins, saves the network, sets `network` on and restarts.

| installer action | NVS | card's `/opendmd.json` | result |
|---|---|---|---|
| **Update OpenDMD** (box unticked) | kept | kept | as it was: offline stays offline |
| **Install**, erase unticked, box unticked | kept | kept | as it was |
| **Install**, erase ticked, box unticked | erased: `network` absent, so on | kept | online, unless the card says `off` (the card wins once mounted) |
| **Install**, box ticked (erase either way) | replaced by `nvs-offline-4mb.bin` | kept | offline, unless the card says `on` |
| **Connect to Wi-Fi** / **Change Wi-Fi** (box unticked) | `network` = on, the credentials | rewritten at the next save | online after one restart |

The page cannot be tested end to end without a person at a real board (Web
Serial asks for a port by hand); `tools/web-smoke.py`-style checks cover
that it loads, the button renders and the manifest parses. The public
site hosts a version of it at <https://opendmd.dev/install/>
(`docs/guide/install.md`, built by `tools/site.py --dist dist` with the
current release's files under `install/firmware/`), so installing needs no
toolchain at all.

### The project site (opendmd.dev)

The site, installer and update check included, is
<https://opendmd.dev/>: the `opendmd-site` Worker on Cloudflare
(`site/worker/`) serving `make site`'s `target/site` as static assets.
There is no other host.

- **A release** (`release.yml`, on a `v*` tag) builds the site from the
  tag and deploys it with `make site-deploy-built`, then checks it live
  ("Releasing" above).
- **Between releases** the **Site** workflow (`site.yml`) deploys main: on
  every push to main that touches what the site is built from (the docs,
  `site/`, the web app and the crates it builds, the site's tools; not a
  release commit, whose release run deploys it), and by hand (Actions →
  Site → Run workflow). The pages and demo are main's; the installer,
  `install/release.json` and `install/beta.json` come only from the latest
  **published** GitHub releases (`tools/site-releases.sh`: the newest
  final, and a newer beta for `beta.json`; a tag whose release is still
  being made does not count), downloaded and checked against their
  `SHA256SUMS` and `firmware/keys/opendmd.pub`. So a deploy from main never
  offers a firmware newer than the latest release. While a `release.yml`
  run is queued or running it waits for it; if one starts, or the releases
  change, while it builds, it deploys nothing (the release's run deploys
  the site). It checks the result live (`tools/site-live-check.sh`). By
  hand it also takes `release` (and `beta`) to put the installer and feeds
  on earlier releases (a rollback; the next push to main puts them back on
  the latest).
- **From the maintainer's machine** (only for `PUBLISH=local`, or with CI
  down; the next CI deploy replaces it): `make fw-release` (or a release's
  files in `dist/`), then `make site-deploy` (refuses a dirty tree), or
  `make site` and `make site-deploy-built`. `make site-deploy-check` is a
  dry run. Build with `API_ARCHIVE=` (a checkout of the public gallery
  repo's `dev/api/`, as release.yml fetches it), or the deployed site
  lacks the earlier releases' API pages; `GALLERY=` (its `gallery/`) only
  fills the built gallery page, which is served until the gallery store
  below is ready. After the deploy, `make site-deploy-built` posts to
  `/api/gallery/sync` so the store holds the new build's examples at once.
- **The gallery store** lives in the D1 database `opendmd-site`, bound
  to the Worker as `DB` in `site/worker/cloudflare.config.ts`. Its schema
  is `site/worker/migrations/*.sql`, applied in order with `cf d1
  migrations apply` and recorded in the database's `d1_migrations` table:
  `make site-d1-migrate` applies the new ones, and `make
  site-deploy-built` runs it before every deploy, so a deploy whose
  Worker needs a new table has it first (0003 drops the short links'
  table: there are no anonymous short links since 2026-10-30). A schema
  change is a new numbered file, never an edit of an applied one. An API
  token for the deploy needs D1 Edit as well as Workers Scripts Edit.
- **Locally**, `make site-dev` and `make site-worker-test` run offline:
  `cf dev` keeps its own D1 in a SQLite file under
  `site/worker/.wrangler/state` (the migrations are applied to it first)
  and simulates the rate limiter; nothing in the Cloudflare account is
  touched. Delete that folder to start the local database again.
- **The gallery store** (`/formulas/`): the same D1 holds the gallery.
  The Worker seeds it with the deployed build's examples (from
  `formulas/gallery.json`) and, from a cron trigger every 5 minutes,
  syncs the public gallery repo's `gallery/`: it reads `main`'s commit from
  git's `info/refs` and, when it moved, that commit's tarball from GitHub
  (no token, no API rate limit), checks each changed file as the gallery
  checker does and draws its thumbnail. **A merged contribution is on
  opendmd.dev within about five minutes, with no deploy.** A file that
  fails keeps its last good version on the site; a removed file leaves it.
  `POST https://opendmd.dev/api/gallery/sync` runs a sync at once (anyone
  may; one every 10 s) and answers with what it did; `GET
  /api/gallery` shows the store: the commit synced, every entry and what
  was refused, and why. Until the store holds the build's examples and
  one sync, or if D1 fails, the built page is served (header `X-Gallery:
  built`, else `store`).
- **The public gallery repo** (github.com/opendmd/opendmd.github.io,
  serving nothing) takes Formula contributions by pull request in
  `gallery/`, checked by its `gallery-check.yml`. A release updates its
  own files with `tools/gallery-repo-publish.sh` (`make
  gallery-repo-publish`): the check and its vendored checker, the READMEs,
  and the release's frozen API page added to `dev/api/`. A merged
  contribution appears on the site within minutes (the gallery store,
  above).

### Installing with espflash (USB)

```bash
make fw-erase PORT=/dev/ttyUSB0           # a first install, or from any other firmware or the old layout
make fw-release-flash PORT=/dev/ttyUSB0   # web app pack, updater, then bootloader, table, app (otadata erased); then monitor
```

Flash at 115200 (the default here): faster rates give `verify_failed` on
these CH340 boards. The firmware's console (its log) is a different rate,
**921600 baud**: `make fw-monitor` and `tools/serial-capture.py` use it; any
other serial monitor must be set to 921600. The exception: when a host
talks to it at 115200 (the browser installer, which only speaks Improv at
that rate), the console switches to 115200, on trial until a valid Improv
frame arrives (3 s, else back to 921600), then **until the next restart**,
so the installer's "Logs & Console" is readable; the heartbeat then ends
`console at 115200 baud (Improv)`. **Reset device** in Logs & Console
restarts the board at 921600, so the console shows garbage from then on:
close the dialog and click Install again (it talks Improv, which switches
it back). Typing in the console does not do it. Without `fw-erase` this is the keep-settings path, like
the flasher's. On the bench, `make fw-flash` does the same with the bench
build (`dev-credentials`).

Put the card in and power up. The boot banner reads
`OpenDMD firmware v<version> (4mb layout v4, features […], app desc '<version> 4mb')`, then `www: pack
'<version>' verified, 37 entries, … B, … CRC pass N ms`, and on first boot
the player builds the default manifest's index on the card
(`filelist.txt.idx`; ~6 s for the full card) before it starts playing.

### Getting on the network

A release image has no WiFi credentials, so a fresh (or erased) board comes
up as an **open access point, `OpenDMD-Setup`**:

1. Join `OpenDMD-Setup` from a phone or laptop. The app opens as a "sign in
   to network" sheet; if it doesn't, browse to the address in the boot log
   (<http://192.168.71.1/>). `/setup`, the small embedded page, is always
   there too (and is what `/` shows in AP mode if the web app in flash is
   missing or damaged).
2. Enter the network name and password, **Save & reboot**. (The AP reboots
   the board after ten minutes regardless, so don't dawdle.)
3. The player joins the network as **<http://opendmd.local/>**.
4. Nothing to select for the prepared copies: the player finds them per
   file (section 1). A playlist is selected from the app, or with
   `curl -X PUT --json '{"set":"<name>"}' http://opendmd.local/api/config`; a
   name that does not exist on the card is refused with a 404 and nothing is
   saved. The first selection of a manifest builds its `.idx` (~6 s for the
   full card, the display holds its frame meanwhile).

Credentials live in NVS from then on and survive every update that does not
erase the chip.

### Updating the firmware

**Over WiFi** (a board already on table v4, with a card in): `make
fw-release`, then `make fw-ota HOST=opendmd.local`, or Settings → Device → Firmware → "Update from file…" with
`dist/opendmd-<version>-<variant>.odfw`, the board's own layout and
table: the device, the web app and `fw-ota` refuse any other. The
device receives the bundle onto the card (`/update/opendmd.bin`), checks
it (its header and layout, that it fits, the ESP image magic, the SHA-256,
the Ed25519 signature against the key compiled into the running firmware
and the image's app descriptor; anything wrong is a 422 and nothing changes), restarts into the updater,
which copies it into `factory` and restarts into the new firmware: ~45 s
from the end of the upload to playback. The panel says "Updating … Don't
unplug", then "Installing", which stays up while the updater installs. `GET
/api/firmware` then reports the new version; `/update/log.txt` on the card
has every step, and its end is `log` in `GET /api/firmware` (the Firmware
card's "Update log"). A bundle signed with another key is refused with a
422 as soon as its header is in. An older version is installed too (the
web app asks first). If the updater cannot finish
(the card pulled mid-copy), it keeps retrying from the card every 10 s with
the LED blinking fast; put the card back. `docs/ota.md` has the details.
The web app is separate: push it too when the API changed
(`make www-push HOST=…`).

Over USB, keeping the settings: bump `version` in `firmware/Cargo.toml`,
`make fw-release`, then either the flasher with **Erase** unticked, or
`make fw-release-flash PORT=...` (no `fw-erase`). The web app pack is
written too, so firmware and app stay in step.

**Pause the cache builder first.** A USB flash resets the chip wherever it
is, and a reset in the middle of a card write can leave a card that times
out at the next mount. So check `curl http://opendmd.local/api/cache`: flash
only while `auto.state` is `paused`, `idle` or `done` — it is `paused`
(`reason "playing"`) whenever the panel is playing, with `auto_prepare` at
its default — never while it is `running` (night mode "off", or the player
paused). If it is running, resume playback (`POST /api/pause` toggles) or
wait for the night window to end, and check again.

### Updating the web app (WiFi)

```bash
make web
make www-push HOST=opendmd.local          # WWW_PACK=dist/opendmd-<v>-www.pack for a release's
```

`make www-push` (`tools/www-push.sh`) PUTs the pack to `/api/www`, then reads
`GET /api/www` back and checks the device serves a verified pack with the
version in the pack's header. No reboot. The device checks the header before
it erases anything (a file that is not a pack, or whose length disagrees with
its header, is a 400 and the app in flash is untouched); with a card in, it
receives the pack onto the card and checks its CRC there before touching
flash (a cut-short or damaged upload then leaves the old app working), then
writes the partition, header last, and verifies the CRC before serving it. While
it writes (a few seconds for today's ~0.5 MB; it is received like any upload, so the
same 120 s limit applies; the panel keeps playing, about one late frame per
push) the API answers as usual, a browser that already
has the app open keeps working, and a page load waits for the write and then
loads the new app. Without a card, a cut-short
or damaged upload leaves the API up and every page answering **503** with the
reason in plain text (`/api/status` `www.error` says it too) until a good
pack is pushed; the setup page (`/setup`) can also write one (**Web app** →
Write to flash).

The web app and the firmware share the JSON API, so after a firmware update
that changed it, push the web app from the same tree (the flasher and
`fw-release-flash` already do).

## 3. What a healthy install looks like

`curl http://opendmd.local/api/status` — typical numbers on a stock card:

| field | `filelist.txt`, few files prepared (mostly GIFs) | every file played from a prepared copy |
|---|---|---|
| `version` / `variant` | the release, e.g. `1.0.0` / `4mb` | same |
| `manifest` | `/sdcard/filelist.txt` | `/sdcard/filelist.txt` (the copies are found per file) |
| `index_state` | `ready` | `ready` |
| `www_version` / `www` | the `git describe` of the `make web` that built the pack in flash; `www.ok` `true`, `www.error` `null` | same |
| `heap.free` | ~62-64 KB (originals through gifslim at cap 12) | ~76 KB (copies through gifslim at cap 9; 75.9-76.2 KB over 90 s idle on a 645-copy set) |
| `heap.floor` (sampled low-water) | ~25 KB over the first 10 min after boot | 38,204 B over the first 2.5 min after boot (boot to idle on the copy set), 27,032 B after 120 s of `api-load WORKERS=3` |
| `refresh_hz` | 100 | 100 |
| `deadline_misses` | a handful per few minutes idle (19 in 5 min, worst ~520 ms: opening a file in a big directory) | 0 idle; 2 (worst 3 ms) in 2 min of back-to-back downloads of the same set |
| `worst_lateness_ms` | up to ~550 | 0 idle, single digits under load |
| `sd.mounted` | `true` | `true` |

Each GIF opened without a copy pays a ~5-6 ms miss looking for one, and
opening a file in a big directory can show as a late frame.

A floor that keeps falling towards 5 KB (`config::HEAP_RESERVE`, where the
server starts answering 503) or misses by the hundred while nobody is using
the web app are the signs something is wrong. `uptime_secs` resetting on its
own means the board is rebooting: capture the serial log
(`make fw-capture PORT=... SECS=600`) before anything else.

## 4. When an update goes wrong

There is no bootloader rollback and no second app slot any more: the 4 MB
of flash hold one app and the web app. What to do instead:

- **The firmware does not boot, or misbehaves.** USB: flash the previous
  release (keep `dist/` somewhere safe; it is gitignored) with the flasher
  (Erase unticked keeps the settings) or `make fw-release-flash`. A board
  that is stuck in a boot loop is still reachable over USB: the ROM
  bootloader takes the flash whatever the app does.
- **The web app is broken or missing** (`/api/status` `www.ok` false, pages
  answer 503 with the reason): `make www-push HOST=...` a good pack, or
  write one from `/setup` in a browser. The API, the player and the setup
  page do not depend on it.
- **Nothing answers on the network**: USB, as above; `make fw-capture` for
  the serial log first if you want to know why.
