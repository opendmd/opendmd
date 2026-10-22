# /// script
# requires-python = ">=3.11"
# ///
"""Package the desktop simulator for download: the binary, a README.txt and
the licences, in one folder inside a zip (macOS, Windows) or a tar.gz (Linux).

    uv run tools/sim-package.py BINARY --platform macos-universal --out DIR

PLATFORM is macos-universal, windows-x86_64 or linux-x86_64; the archive is
DIR/opendmd-sim-<version>-<platform>.zip|.tar.gz, the version opendmd-sim's
Cargo.toml (--version overrides). release.yml runs this on each platform's
runner (for macOS after `lipo` has joined the two architectures), and
`make sim-dist` for the host. Nothing else goes in: no GIFs, no art.
"""

import argparse
import gzip
import io
import os
import re
import subprocess
import tarfile
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PLATFORMS = {
    # platform: (label, archive, executable name)
    "macos-universal": ("macOS (Apple silicon and Intel)", "zip", "opendmd-sim"),
    "windows-x86_64": ("Windows (x86-64)", "zip", "opendmd-sim.exe"),
    "linux-x86_64": ("Linux (x86-64)", "tar.gz", "opendmd-sim"),
}
SITE = "https://opendmd.github.io/dev/simulator.html"

README = """OpenDMD simulator {version}, {label}
{rule}

Plays GIFs in a window drawn as the LEDs of the 128x32 DMDoS display,
through the same engine the OpenDMD firmware runs.

Start it
--------

{start}

From a terminal:

    {run} <folder or GIF>      a card, any folder of GIFs, or one GIF
    {run} --demo               the built-in demo
    {run} --help               every option

Keys: Space next, P pause/resume, L LEDs/flat squares, Esc quit.

{first_run}
More: {site}

Licence: MIT or Apache-2.0, at your option (LICENSE-MIT.txt,
LICENSE-APACHE.txt). OpenDMD is an independent project, not made by or
affiliated with DMDoS.
"""

START = {
    "macos-universal": """Double-click opendmd-sim. It opens in Terminal, then asks for a
folder: choose a card (a folder with filelist.txt in it) or any folder of
GIFs. Cancel for the built-in demo of the panel's effects and live items.""",
    "windows-x86_64": """Double-click opendmd-sim.exe and choose a folder: a card (a folder with
filelist.txt in it) or any folder of GIFs. Cancel for the built-in demo of
the panel's effects and live items. You can also drop a folder, or a GIF,
onto opendmd-sim.exe. The console window beside it shows what is playing.""",
    "linux-x86_64": """Run ./opendmd-sim (or double-click it, if your file manager runs
programs) and choose a folder: a card (a folder with filelist.txt in it) or
any folder of GIFs. Cancel for the built-in demo of the panel's effects and
live items.""",
}

FIRST_RUN = {
    "macos-universal": """The first time
--------------

The simulator isn't signed with an Apple developer certificate, so macOS
won't open it with a double-click at first. Control-click (or right-click)
opendmd-sim, choose Open, then Open again. If macOS offers no Open button
(macOS 15 and later), try once, then go to System Settings > Privacy &
Security and click Open Anyway. From a terminal you can instead run:

    xattr -d com.apple.quarantine opendmd-sim
""",
    "windows-x86_64": """The first time
--------------

The simulator isn't code-signed, so Windows SmartScreen may say it
"protected your PC". Click More info, then Run anyway.
""",
    "linux-x86_64": """What it needs
-------------

glibc 2.34 or newer (Ubuntu 22.04, Debian 12, Fedora 35 and later) and an
X11 or Wayland desktop (libX11 and libXcursor, or libwayland-client and
libxkbcommon: every desktop has them). The folder picker is the desktop's
own, through xdg-desktop-portal, or zenity; without either, give the folder
on the command line.
""",
}


def sim_version() -> str:
    text = (ROOT / "opendmd-sim/Cargo.toml").read_text()
    m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
    if not m:
        raise SystemExit("cannot read the version from opendmd-sim/Cargo.toml")
    return m.group(1)


def mtime() -> int:
    """The commit's time, so a rebuild of a commit gives the same archive."""
    if "SOURCE_DATE_EPOCH" in os.environ:
        return int(os.environ["SOURCE_DATE_EPOCH"])
    try:
        out = subprocess.run(["git", "-C", str(ROOT), "log", "-1", "--format=%ct"],
                             capture_output=True, text=True, check=True).stdout.strip()
        return int(out)
    except (OSError, subprocess.CalledProcessError, ValueError):
        return int(time.time())


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("binary", type=Path)
    ap.add_argument("--platform", required=True, choices=PLATFORMS)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--version")
    a = ap.parse_args()
    if not a.binary.is_file():
        raise SystemExit(f"no {a.binary}")
    version = a.version or sim_version()
    label, kind, exe = PLATFORMS[a.platform]
    windows = a.platform.startswith("windows")
    title = f"OpenDMD simulator {version}, {label}"
    readme = README.format(version=version, label=label, rule="=" * len(title),
                           start=START[a.platform], run="opendmd-sim" if windows else "./opendmd-sim",
                           first_run=FIRST_RUN[a.platform], site=SITE)
    if windows:
        readme = readme.replace("\n", "\r\n")

    base = f"opendmd-sim-{version}-{a.platform}"
    files = [  # (name in the folder, bytes, executable)
        (exe, a.binary.read_bytes(), True),
        ("README.txt", readme.encode(), False),
        ("LICENSE-MIT.txt", (ROOT / "LICENSE-MIT").read_bytes(), False),
        ("LICENSE-APACHE.txt", (ROOT / "LICENSE-APACHE").read_bytes(), False),
    ]
    a.out.mkdir(parents=True, exist_ok=True)
    out = a.out / f"{base}.{kind}"
    when = mtime()
    if kind == "zip":
        stamp = time.gmtime(max(when, 315532800))[:6]  # zip times start in 1980
        with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            for name, data, x in files:
                info = zipfile.ZipInfo(f"{base}/{name}", stamp)
                info.create_system = 3  # Unix, so the mode below is honoured
                info.external_attr = (0o100755 if x else 0o100644) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                z.writestr(info, data)
    else:
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w", format=tarfile.PAX_FORMAT) as t:
            for name, data, x in files:
                info = tarfile.TarInfo(f"{base}/{name}")
                info.size, info.mtime, info.mode = len(data), when, 0o755 if x else 0o644
                info.uname = info.gname = ""
                t.addfile(info, io.BytesIO(data))
        out.write_bytes(gzip.compress(buf.getvalue(), compresslevel=9, mtime=when))
    print(f"{out}: {out.stat().st_size:,} B ({len(files[0][1]):,} B {exe})")


if __name__ == "__main__":
    main()
