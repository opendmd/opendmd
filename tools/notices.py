# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Generate (or check) the third-party licence notices the release ships:

    uv run tools/notices.py firmware --elf ELF --idf-build DIR --updater-build DIR \\
        --toolchain DIR [--check] [--out FILE]
    uv run tools/notices.py web [--check] [--out FILE]

firmware: licenses/THIRD-PARTY-LICENSES-firmware.txt, for everything the
installer writes but the web app: the firmware (the factory app), its
bootloader and the updater. What is listed comes from the release build:

- C and C++: the DWARF compile units of the three ELFs (readelf), each
  source file mapped to its ESP-IDF component by that build's
  project_description.json, then to packages by licenses/components.toml
  (its header says how; an unmapped component, an unclassified compile
  unit, a file whose own licence header no package covers, or a licence
  not accepted there fails). Licence texts are read from the ESP-IDF
  checkout, the components and the toolchain at generation time.
- Rust: cargo-about (the pinned CARGO_ABOUT, licenses/about.toml) over the
  firmware crate for xtensa-esp32-espidf, release features (none),
  cross-checked against the crates whose code is in the ELF; the standard
  library and the crates build-std links into it are listed as one entry.

web: licenses/THIRD-PARTY-LICENSES-web.txt, the web app in the www pack:
cargo-about over opendmd-web, opendmd-art-web and opendmd-psi-web for wasm32, the Rust
standard library, and the headings' font (Silkscreen, OFL 1.1).

Both files are committed. --check regenerates in memory and fails when the
committed file differs (stale) — `make fw-release` checks the firmware's,
CI checks the web one; `make notices` rewrites both. Nothing in the output
depends on this machine's paths or on the OpenDMD version.
"""

import argparse
import difflib
import fnmatch
import gzip
import hashlib
import json
import os
import re
import subprocess
import textwrap
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LIC = ROOT / "licenses"
OUT = {"firmware": LIC / "THIRD-PARTY-LICENSES-firmware.txt", "web": LIC / "THIRD-PARTY-LICENSES-web.txt"}
WIDTH = 78

# How the release build names its sources (Makefile FW_RELEASE_REMAP).
REMAP = {"/opendmd": ROOT, "/cargo": Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")),
         "/rustup": Path(os.environ.get("RUSTUP_HOME", Path.home() / ".rustup"))}

RUST_STD = {
    "name": "Rust standard library",
    "url": "https://github.com/rust-lang/rust",
    "license": "Apache-2.0",
    "note": ("std, core, alloc and the crates they are built from{crates}: MIT OR Apache-2.0 "
             "(compiler-builtins' compiler-rt parts: Apache-2.0 WITH LLVM-exception), used here "
             "under Apache-2.0."),
}


def die(msg: str) -> None:
    raise SystemExit(f"notices: {msg}")


# --- Licence texts -------------------------------------------------------------

@dataclass
class Text:
    title: str
    body: str
    users: list[str] = field(default_factory=list)


class Texts:
    """Licence texts, numbered in order of first use, one copy of each."""

    def __init__(self) -> None:
        self.items: list[Text] = []
        self.keys: dict[str, int] = {}

    @staticmethod
    def key(body: str) -> str:
        # Whitespace aside, and anything after the end of the terms (the
        # Apache licence's appendix is in some copies and not others).
        norm = " ".join(body.split())
        cut = norm.find("END OF TERMS AND CONDITIONS")
        if cut > 0:
            norm = norm[:cut]
        return hashlib.sha256(norm.encode()).hexdigest()

    def add(self, title: str, body: str, user: str) -> int:
        body = clean(body)
        k = self.key(body)
        if k not in self.keys:
            self.items.append(Text(title, body))
            self.keys[k] = len(self.items)
        n = self.keys[k]
        if user not in self.items[n - 1].users:
            self.items[n - 1].users.append(user)
        return n


def clean(text: str) -> str:
    lines = [ln.rstrip() for ln in text.replace("\r\n", "\n").replace("\t", "    ").split("\n")]
    while lines and not lines[0]:
        lines.pop(0)
    while lines and not lines[-1]:
        lines.pop()
    out, blank = [], 0
    for ln in lines:
        blank = blank + 1 if not ln else 0
        if blank <= 1:
            out.append(ln)
    return "\n".join(out)


def header_comment(text: str) -> str:
    """The leading comment block of a C source file (a /* ... */ block or a
    run of // lines), comment markers and rules of dashes removed."""
    lines = text.lstrip("\n").split("\n")
    if lines[0].lstrip().startswith("//"):
        block = [ln for ln in iter_while(lines, lambda s: s.lstrip().startswith("//"))]
    elif lines[0].lstrip().startswith("/*"):
        block = []
        for ln in lines:
            block.append(ln)
            if "*/" in ln:
                break
    else:
        die("no leading comment block")
    out = []
    for ln in block:
        s = re.sub(r"^\s*(/\*+|\*+/|\*+|//+|/)", "", ln.rstrip())
        s = re.sub(r"(\*+/|/)$", "", s)
        out.append("" if re.fullmatch(r"[\s/*=-]*", s) else (s[1:] if s.startswith(" ") else s).rstrip())
    return "\n".join(out)


def iter_while(lines: list[str], ok):
    for ln in lines:
        if not ok(ln):
            return
        yield ln


def rst_section(text: str, title: str) -> str:
    lines = text.split("\n")
    try:
        i = lines.index(title)
    except ValueError:
        die(f"no section '{title}'")
    out = []
    for j in range(i + 2, len(lines)):
        nxt = lines[j + 1] if j + 1 < len(lines) else ""
        if (lines[j] and re.fullmatch(r"[=\-*~]{3,}", nxt)) or lines[j].startswith(".. _"):
            break
        out.append(lines[j])
    return "\n".join(out)


# --- The firmware: DWARF compile units ----------------------------------------

def compile_units(elf: Path, readelf: str) -> list[str]:
    """Every compile unit's source path (DW_AT_name joined to DW_AT_comp_dir)."""
    r = subprocess.run([readelf, "--debug-dump=info", "--dwarf-depth=1", str(elf)],
                       capture_output=True, text=True)
    if r.returncode != 0:
        die(f"readelf failed on {elf}: {r.stderr.strip()[:300]}")
    out = []
    for block in r.stdout.split("DW_TAG_compile_unit")[1:]:
        name = comp = None
        for line in block.split("\n")[:20]:
            m = re.search(r"DW_AT_(name|comp_dir)\s*:\s*(?:\(indirect[^)]*\):\s*)?(.*)$", line)
            if m and m.group(1) == "name" and name is None:
                name = m.group(2).strip()
            elif m and m.group(1) == "comp_dir" and comp is None:
                comp = m.group(2).strip()
        if name:
            out.append(name if name.startswith("/") or not comp else f"{comp.rstrip('/')}/{name}")
    if len(out) < 10:
        die(f"{elf}: {len(out)} compile units; built without debug info?")
    return out


def unremap(path: str) -> str:
    for k, v in REMAP.items():
        if path == k or path.startswith(k + "/"):
            return str(v) + path[len(k):]
    return path


@dataclass
class RustUnit:
    kind: str  # "registry", "std", "ours"
    name: str
    version: str = ""


def rust_unit(path: str) -> RustUnit | None:
    if ".rs/@/" not in path and not path.endswith(".rs"):
        return None
    src = path.split(".rs/@/")[0] + ".rs"
    m = re.search(r"/registry/src/[^/]+/([A-Za-z0-9_.+-]+?)-(\d+\.\d+\.\d+[^/]*)/", src)
    if m:
        return RustUnit("registry", m.group(1), m.group(2))
    m = re.search(r"/lib/rustlib/src/rust/library/(.+?)/src/", src)
    if m:
        return RustUnit("std", m.group(1).split("/")[-1])
    return RustUnit("ours", src)


def licence_families(expr: str) -> list[set[str]]:
    """An SPDX expression as its alternatives, each a set of families."""
    alts = []
    for alt in re.split(r"\s+OR\s+", expr.replace("(", " ").replace(")", " ").strip()):
        fams = set()
        for part in re.split(r"\s+AND\s+", alt):
            fams.add(family(part.split(" WITH ")[0].strip()))
        alts.append(fams)
    return alts


def family(spdx: str) -> str:
    s = spdx.strip()
    for pre, fam in (("BSD-", "BSD"), ("GPL-", "GPL"), ("LGPL-", "GPL"), ("AGPL-", "GPL"), ("MIT", "MIT"),
                     ("Apache-", "Apache"), ("ISC", "ISC"), ("CC0", "CC0"), ("LicenseRef-FatFs", "BSD"),
                     ("Zlib", "Zlib")):
        if s.startswith(pre):
            return fam
    return s


WORDING = [  # (family, wording in a licence notice without an SPDX tag)
    ("GPL", r"GNU (Lesser |Library )?General Public License"),
    ("ISC", r"Permission to use, copy, modify, and(/or)? distribute this software"),
    ("BSD", r"Redistribution and use (of \w+ )?in source and binary forms|under the terms of (the )?BSD license"),
    ("MIT", r"Permission is hereby granted, free of charge"),
    ("Apache", r"Licensed under the Apache License"),
]


def file_licence(path: Path) -> list[set[str]] | None:
    """The licence a source file's own header states: alternatives of families."""
    try:
        head = path.read_text(errors="replace")[:6000]
    except OSError:
        return None
    m = re.search(r"SPDX-License-Identifier:\s*(.+?)\s*(?:\*/)?\s*$", head, re.M)
    if m:
        return licence_families(m.group(1))
    prose = " ".join(" ".join(re.sub(r"^\s*(/\*+|\*+/?|//+|#|;)", "", ln) for ln in head.split("\n")).split())
    found = {fam for fam, rx in WORDING if re.search(rx, prose, re.I)}
    if not found:
        return None
    if "GPL" in found and len(found) > 1:
        # A dual licence ("... GPL v2 ... Alternatively, ... BSD"): either alone.
        return [{f} for f in sorted(found)]
    return [found]


class Firmware:
    def __init__(self, a: argparse.Namespace) -> None:
        self.cfg = tomllib.loads((LIC / "components.toml").read_text())
        self.packages = {p["id"]: p for p in self.cfg["package"]}
        self.rules = {r["match"]: r["packages"] for r in self.cfg["rule"]}
        self.accepted = set(self.cfg["accepted"])
        self.toolchain = a.toolchain
        self.readelf = a.readelf
        for p in self.packages.values():
            if p["license"] not in self.accepted:
                die(f"package {p['id']}: licence {p['license']} is not accepted (components.toml)")
        for pid in (pid for r in self.rules.values() for pid in r):
            if pid != "opendmd" and pid not in self.packages:
                die(f"a rule names package {pid}, which components.toml does not define")
        fw_build = a.idf_build
        self.images = [
            ("firmware", a.elf, fw_build / "project_description.json"),
            ("bootloader", fw_build / "bootloader/bootloader.elf", fw_build / "bootloader/project_description.json"),
            ("updater", a.updater_build / "opendmd-updater.elf", a.updater_build / "project_description.json"),
        ]
        for _, elf, desc in self.images:
            for f in (elf, desc):
                if not f.is_file():
                    die(f"no {f}: run make fw-release first")
        self.idf_path: Path | None = None
        self.comp_dirs: dict[str, Path] = {}  # component name -> dir (any image)
        # package id -> {"images": set, "components": set}
        self.used: dict[str, dict[str, set[str]]] = {}
        self.rust: dict[str, set[RustUnit]] = {}
        self.problems: list[str] = []

    def component_of(self, path: str, comps: list[tuple[str, str, Path]]) -> tuple[str, str] | None:
        """(component, path within it) for a source file, by longest directory."""
        real = os.path.realpath(path)
        best = None
        for name, d, _ in comps:
            for cand in (path, real):
                if cand.startswith(d + "/") and (best is None or len(d) > len(best[1])):
                    best = (name, d, cand[len(d) + 1:])
        return (best[0], best[2]) if best else None

    def rule_for(self, comp: str, rel: str, in_idf: bool) -> list[str] | None:
        full = f"{comp}/{rel}" if rel else comp
        best = None
        for m in self.rules:
            if full == m or full.startswith(m + "/"):
                if best is None or len(m) > len(best):
                    best = m
        if best is not None:
            return self.rules[best]
        return list(self.cfg["idf_default"]) if in_idf else None

    def covered(self, pkgs: list[str], lic: list[set[str]]) -> bool:
        have = {family(self.packages[p]["license"].split(" WITH ")[0]) for p in pkgs if p != "opendmd"}
        if "opendmd" in pkgs:
            have |= {"MIT", "Apache"}
        return any(alt <= have for alt in lic)

    def scan(self) -> None:
        for image, elf, desc_path in self.images:
            desc = json.loads(desc_path.read_text())
            idf = Path(desc["idf_path"]).resolve()
            if self.idf_path is None:
                self.idf_path = idf
            comps = []
            for name, info in desc["build_component_info"].items():
                if name and info.get("dir"):
                    d = info["dir"].rstrip("/")
                    comps.append((name, d, Path(d)))
                    comps.append((name, os.path.realpath(d), Path(d)))
                    self.comp_dirs.setdefault(name, Path(d))
            for cu in compile_units(elf, self.readelf):
                ru = rust_unit(cu)
                if ru:
                    self.rust.setdefault(image, set()).add((ru.kind, ru.name, ru.version))
                    continue
                path = unremap(cu)
                hit = self.component_of(path, comps)
                if hit:
                    comp, rel = hit
                else:
                    comp = next((b["component"] for b in self.cfg["blob"]
                                 if fnmatch.fnmatch(cu, b["glob"]) or fnmatch.fnmatch(os.path.basename(cu), b["glob"])),
                                None)
                    rel = ""
                    if comp is None:
                        self.problems.append(f"{image}: compile unit {cu} is in no component and matches no [[blob]]")
                        continue
                cdir = self.comp_dirs.get(comp)
                in_idf = cdir is not None and str(cdir.resolve()).startswith(str(idf / "components") + "/")
                pkgs = self.rule_for(comp, rel, in_idf)
                if pkgs is None:
                    self.problems.append(f"{image}: component {comp} ({rel or cu}) has no rule in components.toml")
                    continue
                if hit and os.path.isfile(path):
                    lic = file_licence(Path(path))
                    if lic is not None and not self.covered(pkgs, lic):
                        shown = " OR ".join("+".join(sorted(a)) for a in lic)
                        self.problems.append(f"{image}: {comp}/{rel} says {shown}, not covered by {pkgs} "
                                             f"(add a [[rule]] to components.toml)")
                for p in pkgs:
                    if p == "opendmd":
                        continue
                    u = self.used.setdefault(p, {"images": set(), "components": set()})
                    u["images"].add(image)
                    u["components"].add(comp)

    # --- package details ---

    def resolve(self, ref: str, comp_dir: Path | None) -> Path:
        kind, _, rest = ref.partition(":")
        if kind == "idf":
            return self.idf_path / rest
        if kind == "comp":
            if comp_dir is None:
                die(f"{ref}: the package's component is not in this build")
            return comp_dir / rest
        if kind == "toolchain":
            return self.toolchain / "share/licenses" / rest
        if kind == "repo":
            return ROOT / rest
        die(f"unknown text source {ref}")

    def text_of(self, ref: str, comp_dir: Path | None) -> str:
        kind, _, rest = ref.partition(":")
        if kind == "header":
            return header_comment(self.read(self.resolve(rest, comp_dir)))
        if kind == "lines":
            src, _, span = rest.rpartition(":")
            lo, hi = (int(x) for x in span.split("-"))
            return "\n".join(self.read(self.resolve(src, comp_dir)).split("\n")[lo - 1:hi])
        if kind == "section":
            src, _, title = rest.partition(":")
            src2, _, title2 = title.partition(":")
            return rst_section(self.read(self.resolve(f"{src}:{src2}", comp_dir)), title2)
        return self.read(self.resolve(ref, comp_dir))

    @staticmethod
    def read(p: Path) -> str:
        if not p.is_file():
            die(f"no {p} (a licence text components.toml names)")
        return p.read_text(errors="replace")

    def version(self, p: dict, comp_dir: Path | None) -> str:
        v = p.get("version", "")
        if not v:
            return ""
        if v == "idf":
            t = (self.idf_path / "tools/cmake/version.cmake").read_text()
            parts = [re.search(rf"IDF_VERSION_{k} (\d+)", t).group(1) for k in ("MAJOR", "MINOR", "PATCH")]
            return ".".join(parts)
        kind, _, rest = v.partition(":")
        if kind == "sbom":
            m = re.search(r"^version:\s*'?([^'\n]+)'?", self.read(self.resolve(rest, comp_dir)), re.M)
            return m.group(1).strip() if m else ""
        if kind == "yaml":
            m = re.search(r"^version:\s*\"?([^\"\n]+)\"?", self.read(self.resolve(rest, comp_dir)), re.M)
            return m.group(1).strip() if m else ""
        if kind == "json":
            return json.loads(self.read(self.resolve(rest, comp_dir))).get("version", "")
        return v

    def comp_dir_for(self, pid: str) -> Path | None:
        for comp in sorted(self.used.get(pid, {}).get("components", ())):
            if comp in self.comp_dirs:
                return self.comp_dirs[comp]
        return None


# --- cargo-about -----------------------------------------------------------------

def cargo_about(about: str, version: str, crate_dir: Path, target: str) -> dict:
    v = subprocess.run([about, "--version"], capture_output=True, text=True).stdout.strip()
    if v != f"cargo-about {version}":
        die(f"need cargo-about {version}, have '{v or 'none'}' ({about}): the official release binary, "
            f"or cargo install --locked cargo-about@{version}; CARGO_ABOUT=<path>")
    env = dict(os.environ, RUSTUP_TOOLCHAIN="stable")
    r = subprocess.run([about, "generate", "--fail", "--locked", "--format", "json", "--target", target,
                        "-c", str(LIC / "about.toml")], cwd=crate_dir, capture_output=True, text=True, env=env)
    if r.returncode != 0:
        die(f"cargo-about failed for {crate_dir.name} ({target}):\n{r.stderr.strip()[-3000:]}")
    return json.loads(r.stdout)


def crate_entries(abouts: list[dict], texts: Texts) -> list[tuple[str, str, str, str, str]]:
    """(name, version, licence(s), repository, text numbers) per crate, sorted.
    A crate under two licences at once (unicode-ident: MIT AND Unicode-3.0)
    lists both."""
    crates: dict[tuple[str, str], dict] = {}
    for d in abouts:
        for lic in d["licenses"]:
            for u in lic["used_by"]:
                c = u["crate"]
                n = texts.add(lic["name"], lic["text"], f"{c['name']} {c['version']}")
                e = crates.setdefault((c["name"], c["version"]), {"ids": [], "nums": [], "repo": c.get("repository") or ""})
                if n not in e["nums"]:
                    e["ids"].append(lic["id"])
                    e["nums"].append(n)
    return sorted(((k[0], k[1], " AND ".join(e["ids"]), e["repo"], ", ".join(map(str, e["nums"])))
                   for k, e in crates.items()), key=lambda c: (c[0].lower(), c[1]))


# --- Rendering ---------------------------------------------------------------------

def wrap(s: str, indent: str = "") -> str:
    return textwrap.fill(" ".join(s.split()), WIDTH, initial_indent=indent, subsequent_indent=indent,
                         break_on_hyphens=False, break_long_words=False)


def render(title: str, intro: str, packages: list[dict], crates: list, texts: Texts) -> str:
    out = [title, "=" * len(title), "", wrap(intro), ""]
    if packages:
        out += ["Components", "----------", ""]
        for p in packages:
            head = p["name"] + (f" {p['version']}" if p.get("version") else "")
            out.append(f"{head}: {p['license']}" + (f" [{p['text']}]" if p.get("text") else ""))
            if p.get("url"):
                out.append(f"  {p['url']}")
            if p.get("note"):
                out.append(wrap(p["note"], "  "))
            if p.get("in"):
                out.append(wrap(p["in"], "  "))
            out.append("")
    if crates:
        out += ["Rust crates", "-----------", ""]
        w = max(len(f"{c[0]} {c[1]}") for c in crates)
        for name, ver, lic, repo, n in crates:
            out.append(f"  {f'{name} {ver}':<{w}}  {lic} [{n}]" + (f"  {repo}" if repo else ""))
        out.append("")
    out += ["Licence texts", "-------------", ""]
    for i, t in enumerate(texts.items, 1):
        out.append("-" * WIDTH)
        out.append(f"[{i}] {t.title}")
        out.append(wrap("Used by: " + ", ".join(t.users), "    "))
        out += ["", t.body, ""]
    return "\n".join(out).rstrip() + "\n"


def write_or_check(kind: str, text: str, a: argparse.Namespace) -> None:
    out: Path = a.out or OUT[kind]
    gz = len(gzip.compress(text.encode(), 9, mtime=0))
    if a.check:
        have = out.read_text() if out.is_file() else ""
        if have != text:
            diff = list(difflib.unified_diff(have.split("\n"), text.split("\n"), str(out), "regenerated", n=1, lineterm=""))
            print("\n".join(diff[:60]))
            die(f"{out.relative_to(ROOT)} is stale: run `make notices` (after `make fw-release` for the firmware) "
                f"and commit it")
        print(f"{out.relative_to(ROOT)}: up to date ({len(text):,} B, {gz:,} B gzipped)")
        return
    out.write_text(text)
    print(f"{out.relative_to(ROOT)}: {len(text):,} B ({gz:,} B gzipped)")


INTRO_FW = ("OpenDMD is MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE). The firmware the installer writes - "
            "the firmware itself, its bootloader and the updater - is built with the open-source software "
            "below, each used under the licence named; the full texts follow, numbered. The web app the "
            "player serves has its own list (THIRD-PARTY-LICENSES-web.txt). Generated from the release build "
            "by tools/notices.py.")
INTRO_WEB = ("OpenDMD is MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE). The web app the player serves "
             "(the www pack) is built with the open-source software below, each used under the licence "
             "named; the full texts follow, numbered. The firmware has its own list "
             "(THIRD-PARTY-LICENSES-firmware.txt). Generated by tools/notices.py.")


def std_crates(units: set, about_crates: set[tuple[str, str]], cargo_home: Path) -> list[str]:
    """The registry crates in the ELF that the firmware's own tree does not
    have: the ones build-std links in. Each must offer Apache-2.0."""
    out = []
    for kind, name, ver in sorted(units):
        if kind != "registry" or (name, ver) in about_crates:
            continue
        toml = next(iter(sorted((cargo_home / "registry/src").glob(f"*/{name}-{ver}/Cargo.toml"))), None)
        if toml is None:
            die(f"{name} {ver} is linked into the firmware but is not in its crate tree, and its source "
                f"is not under {cargo_home}/registry")
        lic = (tomllib.loads(toml.read_text()).get("package", {}).get("license") or "")
        if "Apache-2.0" not in lic:
            die(f"{name} {ver} (linked in by the standard library) is '{lic}', not available under Apache-2.0")
        out.append(f"{name} {ver}")
    return out


def firmware(a: argparse.Namespace) -> None:
    fw = Firmware(a)
    fw.scan()
    for p in fw.cfg["package"]:
        if p.get("always"):
            fw.used.setdefault(p["id"], {"images": set(), "components": set()})["images"].add(p["always"])
    if fw.problems:
        die("the firmware's components are not all accounted for:\n  " + "\n  ".join(fw.problems))
    about = cargo_about(a.about, a.about_version, ROOT / "firmware", "xtensa-esp32-espidf")
    about_crates = {(c["package"]["name"], c["package"]["version"]) for c in about["crates"]}
    units = set().union(*fw.rust.values())
    linked = {(n, v) for k, n, v in units if k == "registry"}
    std = std_crates(units, about_crates, REMAP["/cargo"])
    # The other way: every crate in the ELF is in the tree (or std's), so the list is complete.
    missing = sorted(linked - about_crates - {tuple(s.split(" ")) for s in std})
    if missing:
        die(f"crates in the ELF but not in the notices: {missing}")

    texts = Texts()
    order = [p["id"] for p in fw.cfg["package"]]
    packages = []
    for pid in order:
        if pid not in fw.used:
            continue
        p = fw.packages[pid]
        cdir = fw.comp_dir_for(pid)
        nums = []
        for ref in p["texts"]:
            body = fw.text_of(ref, cdir)
            if p.get("expect") and p["expect"] not in body:
                die(f"package {pid}: '{p['expect']}' is not in {ref}: a moved text?")
            nums.append(texts.add(spdx_title(p["license"]), body, p["name"]))
        images = sorted(fw.used[pid]["images"], key=["firmware", "bootloader", "updater"].index)
        packages.append({"name": p["name"], "version": fw.version(p, cdir), "license": p.get("license_name", p["license"]),
                         "url": p.get("url"), "note": p.get("note"),
                         "text": ", ".join(map(str, nums)), "in": "In: " + ", ".join(images) + "."})
    crates = crate_entries([about], texts)
    std_entry = rust_std_entry(texts, std)
    packages.append(std_entry)
    write_or_check("firmware", render("Third-party licences: the OpenDMD firmware", INTRO_FW, packages,
                                      crates, texts), a)


def spdx_title(spdx: str) -> str:
    return {"Apache-2.0": "Apache License 2.0", "MIT": "MIT License", "BSD-3-Clause": 'BSD 3-Clause License',
            "BSD-2-Clause": "BSD 2-Clause License", "ISC": "ISC License", "LicenseRef-FatFs": "FatFs licence",
            "OFL-1.1": "SIL Open Font License 1.1",
            "GPL-3.0-or-later WITH GCC-exception-3.1": "GCC Runtime Library Exception 3.1"}.get(spdx, spdx)


def rust_std_entry(texts: Texts, crates: list[str]) -> dict:
    n = texts.add("Apache License 2.0", (ROOT / "LICENSE-APACHE").read_text(), RUST_STD["name"])
    listed = f" ({', '.join(crates)})" if crates else ""
    return {"name": RUST_STD["name"], "license": RUST_STD["license"], "url": RUST_STD["url"],
            "note": RUST_STD["note"].format(crates=listed), "text": str(n)}


def web(a: argparse.Namespace) -> None:
    abouts = [cargo_about(a.about, a.about_version, ROOT / c, "wasm32-unknown-unknown")
              for c in ("opendmd-web", "opendmd-art-web", "opendmd-psi-web")]
    texts = Texts()
    packages = []
    font = ROOT / "opendmd-web/www/fonts/OFL.txt"
    n = texts.add("SIL Open Font License 1.1", font.read_text(), "Silkscreen")
    packages.append({"name": "Silkscreen (the headings' font, a subset)", "license": "OFL-1.1",
                     "url": "https://github.com/google/fonts/tree/main/ofl/silkscreen",
                     "note": "Jason Kottke's pixel font, fonts/silkscreen.woff2 (also inlined in the firmware's "
                             "setup page).", "text": str(n)})
    packages.append({"name": "Game title lists (the site's art-index/)", "license": "credit, no licence text",
                     "url": "https://github.com/libretro-thumbnails",
                     "note": "The game names Create Art searches are taken from the file names of the "
                             "libretro-thumbnails collections, which publish no licence; the web app fetches "
                             "them from the project site when you search, and the pictures from those "
                             "collections. Neither is bundled."})
    crates = crate_entries(abouts, texts)
    packages.append(rust_std_entry(texts, []))
    write_or_check("web", render("Third-party licences: the OpenDMD web app", INTRO_WEB, packages, crates,
                                 texts), a)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="kind", required=True)
    for kind in ("firmware", "web"):
        p = sub.add_parser(kind)
        p.add_argument("--check", action="store_true", help="fail if the committed file is stale")
        p.add_argument("--out", type=Path, help=f"default {OUT[kind].relative_to(ROOT)}")
        p.add_argument("--about", default=os.environ.get("CARGO_ABOUT", "cargo-about"))
        p.add_argument("--about-version", default="0.9.2")
        if kind == "firmware":
            p.add_argument("--elf", type=Path, required=True, help="the release firmware ELF")
            p.add_argument("--idf-build", type=Path, required=True,
                           help="esp-idf-sys's out/build (project_description.json, bootloader/)")
            p.add_argument("--updater-build", type=Path, required=True, help="the updater's idf.py build dir")
            p.add_argument("--toolchain", type=Path, required=True, help="the xtensa-esp-elf toolchain dir")
            p.add_argument("--readelf", default="readelf", help="GNU readelf (the toolchain's, or the host's)")
    a = ap.parse_args()
    firmware(a) if a.kind == "firmware" else web(a)


if __name__ == "__main__":
    main()
