#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow>=10"]
# ///
"""Build the OpenDMD Studio starter pack.

Reads the records in src/ (see README.md), checks them, writes
starter.dmdpack (an "opendmd-studio/1" bundle) and, with --sheets DIR, a
contact sheet and an LED-dot sample picture.

    uv run studio-starter/build.py                  # check + write the pack
    uv run studio-starter/build.py --check          # fail if the pack is stale
    uv run studio-starter/build.py --sheets out/    # also draw the pictures
"""
import argparse
import base64
import hashlib
import json
import re
import sys
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = HERE / "src"
FORMAT = "opendmd-studio/1"
ID_NAMESPACE = "opendmd-studio-starter"
STAMP = "2026-11-11T00:00:00Z"  # created/modified of every record: fixed, so builds repeat
PREFIX = {"piece": "pc", "palette": "pl", "stamp": "st", "font": "fn",
          "shade": "sh", "template": "tp", "pack": "pk"}
KINDS = ["palette", "font", "stamp", "template"]  # bundle order: referenced before referencing
AUTHOR = {"name": "OpenDMD", "account": None}
LICENCE = "CC0-1.0"
PACK = {"slug": "starter", "name": "OpenDMD Starter", "tags": ["starter", "retro"],
        "note": "Hardware palettes, original LED palettes, stamps, two pixel fonts and "
                "four templates to start from. All of it is ours and dedicated to the "
                "public domain (CC0)."}
MAX_STAMP = 128
PANEL = (128, 32)
ASCII = [chr(c) for c in range(32, 127)]
ATLAS_COLS = 16
KEYCHARS = ".123456789abcdefghijklmnopqrstuvwxyz"  # row character -> palette key 0..35


class Bad(Exception):
    pass


def need(cond, msg):
    if not cond:
        raise Bad(msg)


def record_id(kind, slug):
    h = hashlib.sha256(f"{ID_NAMESPACE}:{kind}/{slug}".encode()).hexdigest()
    return f"{PREFIX[kind]}_{h[:32]}"


def canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def pack_px(rows):
    """Rows of key characters -> deflated (zlib) indexed bytes, base64."""
    raw = bytes(KEYCHARS.index(c) for r in rows for c in r)
    return base64.b64encode(zlib.compress(raw, 9)).decode()


def unpack_px(px, w, h):
    raw = zlib.decompress(base64.b64decode(px))
    need(len(raw) == w * h, "px length")
    return [list(raw[y * w:(y + 1) * w]) for y in range(h)]


def check_rows(rows, w, h, keys, what):
    need(isinstance(rows, list) and len(rows) == h, f"{what}: {len(rows)} rows, expected {h}")
    for y, r in enumerate(rows):
        need(len(r) == w, f"{what}: row {y} is {len(r)} wide, expected {w}")
        for c in r:
            need(c in KEYCHARS, f"{what}: row {y} has a bad character {c!r}")
            k = KEYCHARS.index(c)
            need(k == 0 or k in keys, f"{what}: row {y} uses key {k}, not in the palette")


# ------------------------------------------------------------------ loading

def load():
    recs = {}
    for kind in KINDS:
        for path in sorted((SRC / f"{kind}s").glob("*.json")):
            slug = path.stem
            try:
                need(re.fullmatch(r"[a-z0-9]+(-[a-z0-9]+)*", slug), "file name is not a slug")
                src = json.loads(path.read_text(encoding="utf-8"))
                need(src.get("type") == kind, f"type is {src.get('type')!r}, expected {kind!r}")
                need(isinstance(src.get("name"), str) and 0 < len(src["name"]) <= 40, "name")
                tags = src.get("tags")
                need(isinstance(tags, list) and tags and all(
                    re.fullmatch(r"[a-z0-9][a-z0-9-]*", t) for t in tags), "tags")
                need(isinstance(src.get("note"), str) and src["note"], "note")
                need(src.get("author") == AUTHOR, "author")
                need(src.get("licence") == LICENCE, "licence")
                frm = src.get("from")
                need(isinstance(frm, list), "from")
                for f in frm:
                    need(set(f) == {"external", "note"} and f["external"] is False
                         and isinstance(f["note"], str) and f["note"], "from entry")
                need(set(src) == {"type", "name", "tags", "note", "author", "licence", "from", "body"},
                     "unexpected or missing field")
            except Bad as e:
                raise Bad(f"{path.relative_to(HERE)}: {e}") from None
            recs[(kind, slug)] = src
    return recs


def ref(recs, kind, slug, where):
    need((kind, slug) in recs, f"{where}: no {kind} called {slug!r}")
    return record_id(kind, slug)


def palette_keys(recs, slug):
    return {c["k"] for c in recs[("palette", slug)]["body"]["colours"]}


# ------------------------------------------------------------------ bodies

def body_palette(recs, slug, b):
    cols = b.get("colours")
    need(set(b) == {"colours"} and isinstance(cols, list) and 2 <= len(cols) <= 255, "colours")
    seen = set()
    for c in cols:
        need(set(c) <= {"k", "hex", "name"} and isinstance(c.get("k"), int) and 1 <= c["k"] <= 255,
             f"colour {c}")
        need(c["k"] not in seen, f"key {c['k']} twice")
        seen.add(c["k"])
        need(re.fullmatch(r"#[0-9a-f]{6}", c.get("hex", "")), f"hex {c.get('hex')!r}")
    return {"colours": cols}


def body_stamp(recs, slug, b):
    need(set(b) == {"w", "h", "palette", "frames"}, "body fields")
    w, h = b["w"], b["h"]
    need(1 <= w <= MAX_STAMP and 1 <= h <= MAX_STAMP, "size")
    pal = ref(recs, "palette", b["palette"], "palette")
    keys = palette_keys(recs, b["palette"])
    need(max(keys) < len(KEYCHARS), "palette has keys the row characters cannot name")
    need(1 <= len(b["frames"]) <= 64, "frames")
    frames = []
    for i, f in enumerate(b["frames"]):
        need(set(f) <= {"ms", "rows"}, f"frame {i} fields")
        check_rows(f["rows"], w, h, keys, f"frame {i}")
        out = {"px": pack_px(f["rows"])}
        if "ms" in f:
            need(isinstance(f["ms"], int) and 20 <= f["ms"] <= 10000, f"frame {i} ms")
            out["ms"] = f["ms"]
        frames.append(out)
    timed = sum("ms" in f for f in frames)
    need(timed in (0, len(frames)), "give every frame a time or none")
    need(any(c != "." for r in b["frames"][0]["rows"] for c in r), "first frame is empty")
    return {"w": w, "h": h, "palette": pal, "frames": frames}


def body_font(recs, slug, b):
    need(set(b) == {"cell", "advance", "line", "glyphs"}, "body fields")
    cw, ch = b["cell"]["w"], b["cell"]["h"]
    need(1 <= cw <= 16 and 1 <= ch <= 16, "cell")
    g = b["glyphs"]
    missing = [c for c in ASCII if c not in g]
    extra = [c for c in g if c not in ASCII]
    need(not missing, f"missing glyphs {''.join(missing)!r}")
    need(not extra, f"glyphs outside ASCII 32-126: {extra}")
    seen = {}
    for c in ASCII:
        check_rows(g[c], cw, ch, {1}, f"glyph {c!r}")
        ink = any("1" in r for r in g[c])
        need(ink == (c != " "), f"glyph {c!r}: {'empty' if not ink else 'space has ink'}")
        key = tuple(g[c])
        need(key not in seen, f"glyphs {seen.get(key)!r} and {c!r} are identical")
        seen[key] = c
    rows_n = -(-len(ASCII) // ATLAS_COLS)
    atlas = [["."] * (ATLAS_COLS * cw) for _ in range(rows_n * ch)]
    for i, c in enumerate(ASCII):
        ox, oy = (i % ATLAS_COLS) * cw, (i // ATLAS_COLS) * ch
        for y, r in enumerate(g[c]):
            atlas[oy + y][ox:ox + cw] = r
    need(isinstance(b["advance"], int) and cw <= b["advance"] <= 2 * cw, "advance")
    need(isinstance(b["line"], int) and ch <= b["line"] <= 2 * ch, "line")
    return {"cell": {"w": cw, "h": ch}, "line": b["line"], "first": 32, "count": len(ASCII),
            "adv": [b["advance"]] * len(ASCII),
            "atlas": {"cols": ATLAS_COLS, "w": ATLAS_COLS * cw, "h": rows_n * ch,
                      "px": pack_px(["".join(r) for r in atlas])}}


def body_template(recs, slug, b):
    need(set(b) == {"w", "h", "palette", "plays", "frames", "layers", "guides"}, "body fields")
    need((b["w"], b["h"]) == PANEL, "size is not 128x32")
    pal = ref(recs, "palette", b["palette"], "palette")
    keys = palette_keys(recs, b["palette"])
    need(max(keys) < len(KEYCHARS), "palette has keys the row characters cannot name")
    n = len(b["frames"])
    need(1 <= n <= 256 and all(set(f) == {"ms"} and isinstance(f["ms"], int) for f in b["frames"]),
         "frames")
    need(all(f["ms"] >= 20 for f in b["frames"]) if n > 1 else True, "frame times")
    need(1 <= len(b["layers"]) <= 16, "layers")
    layers = []
    for layer in b["layers"]:
        need(set(layer) == {"name", "moves", "cels"}, "layer fields")
        need(len(layer["cels"]) == (n if layer["moves"] else 1),
             f"layer {layer['name']!r}: one cel per frame if it moves, else one")
        for i, cel in enumerate(layer["cels"]):
            check_rows(cel, *PANEL, keys, f"layer {layer['name']!r} cel {i}")
        layers.append({"name": layer["name"], "moves": layer["moves"],
                       "cels": [pack_px(c) for c in layer["cels"]]})
    guides = []
    for g in b["guides"]:
        need(g.get("kind") in ("safe", "text", "art"), f"guide kind {g.get('kind')!r}")
        want = {"kind", "name", "x", "y", "w", "h"} | (
            {"font", "align", "sample"} if g["kind"] == "text" else set())
        need(set(g) == want, f"guide {g.get('name')!r} fields")
        need(g["x"] >= 0 and g["y"] >= 0 and g["w"] > 0 and g["h"] > 0
             and g["x"] + g["w"] <= PANEL[0] and g["y"] + g["h"] <= PANEL[1],
             f"guide {g['name']!r} is off the panel")
        out = dict(g)
        if g["kind"] == "text":
            need(g["align"] in ("left", "center", "right"), "align")
            out["font"] = ref(recs, "font", g["font"], f"guide {g['name']!r}")
            f = recs[("font", g["font"])]["body"]
            need(f["cell"]["h"] <= g["h"], f"guide {g['name']!r} is shorter than its font")
            need(len(g["sample"]) * f["advance"] - 1 <= g["w"],
                 f"guide {g['name']!r}: the sample does not fit")
        guides.append(out)
    return {"w": PANEL[0], "h": PANEL[1], "palette": pal, "plays": b["plays"],
            "frames": b["frames"], "layers": layers, "guides": guides}


BODY = {"palette": body_palette, "stamp": body_stamp, "font": body_font, "template": body_template}


def build(recs):
    out = []
    for (kind, slug), src in recs.items():
        try:
            body = BODY[kind](recs, slug, src["body"])
        except Bad as e:
            raise Bad(f"src/{kind}s/{slug}.json: {e}") from None
        except (KeyError, TypeError) as e:
            raise Bad(f"src/{kind}s/{slug}.json: malformed body ({e!r})") from None
        out.append(envelope(kind, slug, src["name"], src["tags"], src["note"], src["from"], body))
    names = {}
    for r in out:
        need((r["type"], r["name"]) not in names, f"two {r['type']} records are named {r['name']!r}")
        names[(r["type"], r["name"])] = 1
    members = [{"id": r["id"], "rev": r["rev"]} for r in out]
    pack = envelope("pack", PACK["slug"], PACK["name"], PACK["tags"], PACK["note"], [],
                    {"members": members})
    return {"format": FORMAT, "root": pack["id"], "records": out + [pack]}


def envelope(kind, slug, name, tags, note, frm, body):
    return {"type": kind, "id": record_id(kind, slug), "rev": 1,
            "hash": "sha256-" + hashlib.sha256(canonical(body)).hexdigest(),
            "name": name, "tags": tags, "note": note, "author": AUTHOR, "licence": LICENCE,
            "from": frm, "created": STAMP, "modified": STAMP, "body": body}


def bundle_text(bundle):
    """One record per line: stable, and a diff shows which record changed."""
    lines = [json.dumps(r, separators=(",", ":"), ensure_ascii=False) for r in bundle["records"]]
    return ('{"format":%s,"root":%s,"records":[\n' % (json.dumps(bundle["format"]),
            json.dumps(bundle["root"])) + ",\n".join(lines) + "\n]}\n")


# ------------------------------------------------------------------ pictures
# Everything below draws from the built bundle, the way a loader would.

class Lib:
    def __init__(self, bundle):
        self.recs = bundle["records"]
        self.by_id = {r["id"]: r for r in self.recs}

    def of(self, kind):
        return [r for r in self.recs if r["type"] == kind]

    def get(self, kind, slug):
        return self.by_id[record_id(kind, slug)]

    def rgb(self, pal_id):
        return {c["k"]: tuple(int(c["hex"][i:i + 2], 16) for i in (1, 3, 5))
                for c in self.by_id[pal_id]["body"]["colours"]}

    def stamp_frame(self, st, frame=0):
        b = st["body"]
        return unpack_px(b["frames"][frame]["px"], b["w"], b["h"]), self.rgb(b["palette"])

    def glyph(self, font, ch):
        b = font["body"]
        if "_atlas" not in b:
            b["_atlas"] = unpack_px(b["atlas"]["px"], b["atlas"]["w"], b["atlas"]["h"])
        i = ord(ch) - b["first"]
        if not 0 <= i < b["count"]:
            i = ord("?") - b["first"]
        ox, oy = (i % b["atlas"]["cols"]) * b["cell"]["w"], (i // b["atlas"]["cols"]) * b["cell"]["h"]
        return [row[ox:ox + b["cell"]["w"]] for row in b["_atlas"][oy:oy + b["cell"]["h"]]], b["adv"][i]


class Grid:
    """A small RGB pixel grid; None is unlit."""

    def __init__(self, w, h):
        self.w, self.h = w, h
        self.p = [[None] * w for _ in range(h)]

    def put(self, x, y, c):
        if 0 <= x < self.w and 0 <= y < self.h:
            self.p[y][x] = c

    def blit(self, px, rgb, x, y):
        for yy, row in enumerate(px):
            for xx, k in enumerate(row):
                if k:
                    self.put(x + xx, y + yy, rgb[k])

    def text(self, lib, font, s, x, y, colour):
        for ch in s:
            g, adv = lib.glyph(font, ch)
            for yy, row in enumerate(g):
                for xx, k in enumerate(row):
                    if k:
                        self.put(x + xx, y + yy, colour)
            x += adv
        return x

    def stamp(self, lib, slug, x, y, frame=0):
        self.blit(*lib.stamp_frame(lib.get("stamp", slug), frame), x, y)

    def draw(self, img, ox, oy, scale, led=False):
        px = img.load()
        for y in range(self.h):
            for x in range(self.w):
                c = self.p[y][x]
                if led:
                    c = c or (14, 14, 16)
                    for dy in range(scale - 1):
                        for dx in range(scale - 1):
                            px[ox + x * scale + dx, oy + y * scale + dy] = c
                elif c is not None:
                    for dy in range(scale):
                        for dx in range(scale):
                            px[ox + x * scale + dx, oy + y * scale + dy] = c


def text_width(lib, font, s):
    return sum(lib.glyph(font, ch)[1] for ch in s) - 1


def template_grid(lib, tp, frame=0, samples=False):
    b = tp["body"]
    rgb = lib.rgb(b["palette"])
    g = Grid(b["w"], b["h"])
    for layer in b["layers"]:
        cel = layer["cels"][frame if layer["moves"] else 0]
        g.blit(unpack_px(cel, b["w"], b["h"]), rgb, 0, 0)
    if samples:
        for gd in b["guides"]:
            if gd["kind"] == "text":
                font = lib.by_id[gd["font"]]
                tw = text_width(lib, font, gd["sample"])
                x = gd["x"] + {"left": 0, "center": (gd["w"] - tw) // 2, "right": gd["w"] - tw}[gd["align"]]
                g.text(lib, font, gd["sample"], x, gd["y"], (255, 255, 255))
            elif gd["kind"] == "art":
                for x in range(gd["x"], gd["x"] + gd["w"], 2):
                    for y in range(gd["y"], gd["y"] + gd["h"], 2):
                        g.put(x, y, (60, 64, 80))
    return g


def sample_grid(lib):
    """A 128x32 scene made only of the pack's stamps and fonts."""
    g = Grid(*PANEL)
    tiny = lib.get("font", "tiny-3x5")
    p16 = lib.rgb(record_id("palette", "panel-16"))
    g.text(lib, tiny, "1UP", 1, 2, p16[5])
    for i, d in enumerate("04250"):
        g.stamp(lib, "digits-5x7", 14 + i * 6, 1, int(d))
    g.stamp(lib, "coin", 48, 0)
    g.text(lib, tiny, "x07", 58, 2, p16[7])
    g.stamp(lib, "crown", 74, 1)
    g.text(lib, tiny, "HI", 85, 2, p16[3])
    for i, s in enumerate(("heart-full", "heart-full", "heart-half", "heart-empty")):
        g.stamp(lib, s, 96 + i * 8, 1)
    row = (("alien-jelly", 2, 11), ("alien-stalker", 14, 11), ("alien-fang", 25, 12),
           ("saucer", 38, 12), ("wisp", 54, 11), ("explosion", 70, 10), ("gem", 80, 13),
           ("key", 89, 11), ("potion", 97, 11), ("power-cap", 107, 11), ("star", 119, 12))
    for s, x, y in row:
        g.stamp(lib, s, x, y, 1 if s == "explosion" else 0)
    g.stamp(lib, "text-insert-coin", 2, 24)
    g.stamp(lib, "bullet", 73, 18)
    g.stamp(lib, "ship", 70, 24)
    g.stamp(lib, "arrow-left", 98, 24)
    g.stamp(lib, "arrow-right", 110, 24)
    g.stamp(lib, "cursor", 121, 23)
    return g


def draw_sheets(bundle, outdir):
    from PIL import Image

    lib = Lib(bundle)
    tiny, arcade = lib.get("font", "tiny-3x5"), lib.get("font", "arcade-5x7")
    BG, INK, DIM, CELL = (10, 10, 12), (235, 235, 240), (120, 126, 140), (22, 22, 27)
    WIDTH, M = 1560, 16
    ops = []  # (callable(img)), laid out with a moving cursor
    cur = {"x": M, "y": M, "rowh": 0}

    def label(s, x, y, colour=INK, scale=2, font=None):
        g = Grid(len(s) * 6 + 1, 8)
        g.text(lib, font or arcade, s, 0, 0, colour)
        ops.append(lambda img, g=g, x=x, y=y, scale=scale: g.draw(img, x, y, scale))

    def newline(gap=10):
        cur["x"] = M
        cur["y"] += cur["rowh"] + gap
        cur["rowh"] = 0

    def heading(s):
        newline(14)
        label(s, M, cur["y"], (255, 216, 0), 3)
        cur["rowh"] = 24
        newline(8)

    def place(w, h):
        if cur["x"] + w > WIDTH - M:
            newline()
        x, y = cur["x"], cur["y"]
        cur["x"] += w + 12
        cur["rowh"] = max(cur["rowh"], h)
        return x, y

    def rect(x, y, w, h, c):
        ops.append(lambda img: img.paste(c, (x, y, x + w, y + h)))

    heading("PALETTES  %d" % len(lib.of("palette")))
    for p in lib.of("palette"):
        cols = p["body"]["colours"]
        sw = 20 if len(cols) <= 16 else 14 if len(cols) <= 64 else 10
        per = 16 if len(cols) <= 16 else 32
        rows_n = -(-len(cols) // per)
        w = max(per * sw if len(cols) > per else len(cols) * sw, len(p["name"]) * 12)
        x, y = place(w + 8, 22 + rows_n * sw + 8)
        label(p["name"], x, y)
        rect(x, y + 20, min(len(cols), per) * sw + 4, rows_n * sw + 4, CELL)
        for i, c in enumerate(cols):
            rgb = tuple(int(c["hex"][j:j + 2], 16) for j in (1, 3, 5))
            rect(x + 2 + (i % per) * sw, y + 22 + (i // per) * sw, sw - 1, sw - 1, rgb)

    heading("STAMPS  %d  (4x, every frame)" % len(lib.of("stamp")))
    for st in lib.of("stamp"):
        b = st["body"]
        n = len(b["frames"])
        w = max(n * (b["w"] * 4 + 8) + 4, len(st["name"]) * 8)
        x, y = place(w, b["h"] * 4 + 8 + 16)
        rect(x, y, n * (b["w"] * 4 + 8) + 4, b["h"] * 4 + 8, CELL)
        for f in range(n):
            g = Grid(b["w"], b["h"])
            g.blit(*lib.stamp_frame(st, f), 0, 0)
            ops.append(lambda img, g=g, x=x + 6 + f * (b["w"] * 4 + 8), y=y + 4: g.draw(img, x, y, 4))
        label(st["name"], x, y + b["h"] * 4 + 12, DIM, 2, tiny)

    heading("FONTS  %d  (4x)" % len(lib.of("font")))
    for f in lib.of("font"):
        newline(4)
        label(f["name"], M, cur["y"])
        cur["rowh"] = 18
        newline(4)
        fb = f["body"]
        text = "".join(ASCII)
        lines = [text[0:32], text[32:64], text[64:95], "The quick brown fox jumps over the lazy dog. 0123456789"]
        for ln in lines:
            g = Grid(text_width(lib, f, ln) + 1, fb["cell"]["h"])
            g.text(lib, f, ln, 0, 0, INK)
            x, y = M, cur["y"]
            rect(x, y, g.w * 4 + 8, g.h * 4 + 8, CELL)
            ops.append(lambda img, g=g, x=x + 4, y=y + 4: g.draw(img, x, y, 4))
            cur["rowh"] = g.h * 4 + 8
            newline(2)

    heading("TEMPLATES  %d  (2x: as drawn, then with the guides' sample text)" % len(lib.of("template")))
    for tp in lib.of("template"):
        x, y = place(2 * (256 + 8) + 8, 64 + 8 + 18)
        for i, smp in enumerate((False, True)):
            g = template_grid(lib, tp, 0, smp)
            xx = x + i * (256 + 12)
            rect(xx, y, 256 + 8, 64 + 8, (0, 0, 0))
            ops.append(lambda img, g=g, xx=xx, y=y: g.draw(img, xx + 4, y + 4, 2))
        label(tp["name"], x, y + 76, DIM)

    heading("SAMPLE  (stamps and fonts only, 4x LED dots)")
    sg = sample_grid(lib)
    x, y = place(512 + 16, 128 + 16)
    rect(x, y, 512 + 16, 128 + 16, (0, 0, 0))
    ops.append(lambda img: sg.draw(img, x + 8, y + 8, 4, led=True))
    newline()

    img = Image.new("RGB", (WIDTH, cur["y"] + M), BG)
    for op in ops:
        op(img)
    outdir.mkdir(parents=True, exist_ok=True)
    img.save(outdir / "starter-sheet.png")
    smp = Image.new("RGB", (512, 128), (0, 0, 0))
    sg.draw(smp, 0, 0, 4, led=True)
    smp.save(outdir / "starter-sample.png")
    return [outdir / "starter-sheet.png", outdir / "starter-sample.png"]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", type=Path, default=HERE / "starter.dmdpack")
    ap.add_argument("--check", action="store_true", help="do not write; fail if --out is stale")
    ap.add_argument("--sheets", type=Path, metavar="DIR", help="draw starter-sheet.png and starter-sample.png here")
    a = ap.parse_args()
    try:
        recs = load()
        bundle = build(recs)
    except Bad as e:
        sys.exit(f"studio-starter: {e}")
    text = bundle_text(bundle)
    assert json.loads(text) == bundle
    counts = {k: sum(r["type"] == k for r in bundle["records"]) for k in KINDS}
    summary = ", ".join(f"{n} {k}s" for k, n in counts.items())
    if a.check:
        if not a.out.exists() or a.out.read_text(encoding="utf-8") != text:
            sys.exit(f"studio-starter: {a.out} is stale; run `make studio-starter`")
        print(f"studio-starter: ok, {a.out.name} is current ({summary})")
    else:
        a.out.write_text(text, encoding="utf-8")
        print(f"studio-starter: ok, wrote {a.out} ({len(text.encode())} bytes: {summary})")
    if a.sheets:
        for p in draw_sheets(bundle, a.sheets):
            print(f"studio-starter: drew {p}")


if __name__ == "__main__":
    main()
