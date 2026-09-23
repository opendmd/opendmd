# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow", "numpy", "scipy"]
# ///
"""The five processing demos from docs/art-pipeline.md, rendered through
the LED look (led.py == opendmd-render, see check_led.py).

    ART_TMP=/tmp/opendmd-art uv run fetch_inputs.py
    ART_TMP=/tmp/opendmd-art uv run demos.py [a b c d e]

Writes out/<demo>_*.png (comparison sheets) and out/*.gif (the animations,
ignored by the repo's *.gif rule).
"""
import sys, time
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent))
import art  # noqa: E402
from art import W, H  # noqa: E402
from font import Bdf  # noqa: E402
from led import Led  # noqa: E402

OUT = Path(__file__).resolve().parent / "out"
SRC = art.SCRATCH / "src"
LED = Led(6)
LABEL_H = 16


def rgba(path):
    return np.asarray(Image.open(path).convert("RGBA"))


def sheet(name, rows, title=None):
    """rows: [(label, 32x128 frame | [frames])] -> LED renders, labelled."""
    tiles = []
    for label, fr in rows:
        frames = fr if isinstance(fr, list) else [fr]
        leds = [LED.render(f) for f in frames]
        tile = np.concatenate([np.pad(l, ((0, 0), (0, 6), (0, 0)), constant_values=40) for l in leds], 1)[:, :-6]
        tiles.append((label, tile))
    width = max(t.shape[1] for _, t in tiles)
    head = LABEL_H + 4 if title else 0
    im = Image.new("RGB", (width, head + sum(t.shape[0] + LABEL_H + 6 for _, t in tiles)), (28, 28, 30))
    d = ImageDraw.Draw(im)
    y = 0
    if title:
        d.text((4, 3), title, fill=(255, 220, 120))
        y = head
    for label, t in tiles:
        d.text((4, y + 2), label, fill=(230, 230, 230))
        im.paste(Image.fromarray(t), (0, y + LABEL_H))
        y += t.shape[0] + LABEL_H + 6
    # 256-colour PNG keeps the repo small; the LED look survives it.
    im.quantize(256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE).save(OUT / f"{name}.png", optimize=True)
    print("  wrote", f"out/{name}.png")


def strip(frames, n=4):
    idx = np.linspace(0, len(frames) - 1, n).round().astype(int)
    return [frames[i] for i in idx]


def flat_x(img, k=3):
    return Image.fromarray(img).resize((img.shape[1] * k, img.shape[0] * k), Image.NEAREST)


# --- (a) a 256x224 screenshot ---------------------------------------------------

def demo_a():
    print("(a) screenshot")
    big = rgba(SRC / "pp" / "SampleA.png")[..., :3]  # 918x515, drawn at 2x
    nat, k = art.native(big)
    print(f"  SampleA 918x515 -> detected scale {k}, native {nat.shape[1]}x{nat.shape[0]}")
    shot = np.ascontiguousarray(nat[0:224, 100:356])  # a SNES-sized 256x224 frame
    # Robustness of the scale detector on typical web copies of it.
    for label, im in [("2x NN", np.repeat(np.repeat(shot, 2, 0), 2, 1)),
                      ("3x NN", np.repeat(np.repeat(shot, 3, 0), 3, 1)),
                      ("2.5x bilinear", np.asarray(Image.fromarray(shot).resize((640, 560), Image.BILINEAR))),
                      ("2x NN, JPEG q85", np.asarray(_jpeg(np.repeat(np.repeat(shot, 2, 0), 2, 1), 85)))]:
        n, kk = art.native(im)
        same = n.shape == shot.shape and np.abs(n.astype(int) - shot).max() < 24
        print(f"  detect on {label}: scale {kk} -> {n.shape[1]}x{n.shape[0]}{'  (recovers the original)' if same else ''}")

    t = time.perf_counter()
    x2, y2 = art.best_window(shot, 256, 64)
    x1, y1 = art.best_window(shot, 128, 32)
    bb = art.busy_bbox(shot)
    print(f"  windows: 256x64 at y={y2}; 128x32 at ({x1},{y1}); busy bbox {bb}; {1000 * (time.perf_counter() - t):.1f} ms (numpy)")
    src = flat_x(shot, 3)
    d = ImageDraw.Draw(src)
    d.rectangle([0, 3 * y2, 3 * 256 - 1, 3 * (y2 + 64) - 1], outline=(255, 60, 60), width=3)
    d.rectangle([3 * x1, 3 * y1, 3 * (x1 + 128) - 1, 3 * (y1 + 32) - 1], outline=(60, 255, 60), width=3)
    src.save(OUT / "a_source_windows.png", optimize=True)
    print("  wrote out/a_source_windows.png (red: 256x64 for 2:1, green: 128x32 native)")

    band = shot[y2 : y2 + 64]
    fit = art.down_area(shot, 7)
    area = art.down_area(band, 2)
    rows = [
        (f"fit whole screen, area 7:1 -> {fit.shape[1]}x{fit.shape[0]} (what a naive resize gives)", art.place(fit)),
        ("busiest 256x64, nearest 2:1", art.down_nearest(band, 2)),
        (f"busiest 256x64, area-average 2:1 ({len(art.colours(area))} colours, new blends)", area),
        (f"busiest 256x64, mode 2:1 (pixel-art-aware; {len(art.colours(art.down_mode(band, 2)))} colours, all from the source)", art.down_mode(band, 2)),
        ("busiest 128x32 at native 1:1 (no resampling)", shot[y1 : y1 + 32, x1 : x1 + 128]),
        ("same, dominant (sky) colour keyed to black, the pack's usual look", key_bg(shot)[y1 : y1 + 32, x1 : x1 + 128]),
    ]
    sheet("a_screenshot_resample", rows, "(a) 256x224 screenshot -> 128x32   [Kenney Pixel Platformer, CC0]")

    # Quantisation on the one variant that invents colours.
    n_src = len(art.colours(shot))
    rows = [
        (f"area 2:1, exact ({len(art.colours(area))} colours)", area),
        ("median cut 16, no dither", art.q_mediancut([area], 16)[0]),
        ("NeuQuant 16 (color_quant 1.1.0, as the core)", art.q_neuquant([area], 16)[0]),
        (f"snap to the source's own palette ({n_src} colours), 16 picked for spread", art.q_native([area], 16, shot)[0]),
        ("median cut 16 + Floyd-Steinberg dither", art.q_mediancut([area], 16, dither=True)[0]),
        ("mode 2:1, median cut 16 (no dither)", art.q_mediancut([art.down_mode(band, 2)], 16)[0]),
        ("mode 2:1, source palette 16 picked for spread", art.q_native([art.down_mode(band, 2)], 16, shot)[0]),
    ]
    sheet("a_screenshot_quantise", rows, "(a) quantising the area-average result to 16 colours")


def key_bg(img, tol=12):
    """The most common colour (and anything within tol of it) -> black. A
    pale sky is most of the panel's power and glare; the pack's art is
    mostly sprites on black."""
    flat = img.reshape(-1, 3)
    cols, cnt = np.unique(flat, axis=0, return_counts=True)
    bg = cols[cnt.argmax()].astype(int)
    m = np.abs(img.astype(int) - bg).max(2) <= tol
    out = img.copy()
    out[m] = 0
    return out


def _jpeg(img, q):
    import io
    b = io.BytesIO()
    Image.fromarray(img).save(b, "JPEG", quality=q)
    return Image.open(io.BytesIO(b.getvalue())).convert("RGB")


# --- (b) a sprite sheet: pan, and walk cycles --------------------------------------

def demo_b():
    print("(b) sprite sheet")
    tiles = rgba(SRC / "pp" / "Tilemap" / "tilemap_packed.png")  # 360x162
    ob = art.on_black(tiles)
    _, y = art.best_window(ob, tiles.shape[1], 32, centre_bias=0)
    span = tiles.shape[1] - W
    xs = list(range(0, span + 1))  # one way, 1 px/frame (the GIF loops back to the start)
    pan = [np.ascontiguousarray(ob[y : y + 32, x : x + W]) for x in xs]
    art.save_gif(OUT / "b_pan.gif", pan, [1000] + [40] * (len(pan) - 2) + [1000])
    print(f"  pan: band y={y}, {len(pan)} frames at 40 ms (1 px/frame = 25 px/s) -> out/b_pan.gif")

    packed = rgba(SRC / "pp" / "Tilemap" / "tilemap-characters_packed.png")  # 216x72, no spacing
    print(f"  packed sheet (sprites touch): {len(art.components(packed))} blobs for 27 sprites -> needs a grid fallback")
    chars = rgba(SRC / "pp" / "Tilemap" / "tilemap-characters.png")  # 224x74, 1 px spacing
    t = time.perf_counter()
    boxes = art.components(chars, gap=0)
    groups = art.cycles(chars, boxes, thr=0.75)
    print(f"  segmentation: {len(boxes)} sprites -> {len(groups)} cycles {[len(g) for g in groups]} "
          f"in {1000 * (time.perf_counter() - t):.0f} ms (numpy/scipy)")
    # Contact sheet of what segmentation found.
    cs = flat_x(art.on_black(chars, (50, 50, 60)), 3)
    d = ImageDraw.Draw(cs)
    palette = [(255, 80, 80), (80, 255, 80), (80, 160, 255), (255, 220, 60), (255, 80, 255), (80, 255, 255)]
    for gi, g in enumerate(groups):
        for b in g:
            d.rectangle([3 * b[0], 3 * b[1], 3 * b[2] - 1, 3 * b[3] - 1], outline=palette[gi % 6], width=2)
    cs.save(OUT / "b_segments.png", optimize=True)

    def walker(g, x, step, ground=31):
        b = g[(step // 8) % len(g)]
        spr = chars[b[1] : b[3], b[0] : b[2]]
        return spr, x, ground - spr.shape[0] + 1

    frames = []
    walkers = [g for g in groups if len(g) >= 2]
    for s, x in enumerate(range(-24, W + 1)):
        f = np.zeros((H, W, 3), np.uint8)
        for n, g in enumerate(walkers[:4]):  # a little parade, 30 px apart
            spr, xx, yy = walker(g, x - n * 30, s + n * 3)
            f = art.place(spr, x=xx, y=yy, bg=(0, 0, 0)) if n == 0 else _over(f, spr, xx, yy)
        frames.append(f)
    art.save_gif(OUT / "b_walk.gif", frames, 40)
    print(f"  walk: {len(frames)} frames at 40 ms, cycle frame every 8 -> out/b_walk.gif")
    sheet("b_spritesheet", [("pan: 128x32 window sliding across the busiest band (4 of the frames)", strip(pan)),
                            ("walk cycles found by transparent-background segmentation, walking across", strip(frames[20:], 4))],
          "(b) sprite sheet -> pan, and per-sprite walk cycles   [Kenney Pixel Platformer, CC0]")


def _over(f, spr, x, y):
    layer = art.place(spr, x=x, y=y, bg=(0, 0, 0))
    m = np.zeros((H, W), bool)
    ih, iw = spr.shape[:2]
    sub = spr[..., 3] > 127
    for yy in range(ih):
        for xx in range(iw):
            if sub[yy, xx] and 0 <= y + yy < H and 0 <= x + xx < W:
                m[y + yy, x + xx] = True
    return np.where(m[..., None], layer, f)


# --- (c) logos -------------------------------------------------------------------

def lift(src, dark=70, max_cover=0.6):
    """A black wordmark on a black panel disappears. At full resolution,
    before resampling: unless the dark, low-saturation opaque pixels fill
    most of the logo's box (a backing plate — the Mega Drive logo's black
    square should stay black), flip their lightness so they become white."""
    rgb = src[..., :3].astype(int)
    op = src[..., 3] > 127
    luma = rgb @ np.array([0.299, 0.587, 0.114])
    sat = rgb.max(2) - rgb.min(2)
    m = op & (luma < dark) & (sat < 60)
    if not m.any() or m.sum() / m.size > max_cover:  # share of the whole (trimmed) logo box
        return src, False
    out = src.copy()
    out[..., :3][m] = 255 - rgb[m]
    return out, True


def fit_logo(path, margin=1):
    src = rgba(path)
    # Trim transparent borders first.
    ys, xs = np.nonzero(src[..., 3] > 16)
    src = src[ys.min() : ys.max() + 1, xs.min() : xs.max() + 1]
    src, lifted = lift(src)
    h = H - 2 * margin
    w = round(src.shape[1] * h / src.shape[0])
    if w > W - 2 * margin:
        w = W - 2 * margin
        h = round(src.shape[0] * w / src.shape[1])
    small = art.resize_area(src, w, h)
    return art.place(art.on_black(small)), (w, h), lifted


def demo_c():
    print("(c) logos")
    rows = []
    for name in ["sega", "snes", "megadrive", "saturn", "3ds"]:
        f, (w, h), lifted = fit_logo(SRC / f"{name}.png")
        n = len(art.colours(f))
        note = ", dark strokes lifted to white" if lifted else ""
        rows.append((f"{name}: fit {w}x{h}, area-average, exact ({n} colours{note})", f))
        rows.append((f"{name}: median cut 16 / NeuQuant 16 / spread-pick 8 (no dither)",
                     [art.q_mediancut([f], 16)[0], art.q_neuquant([f], 16)[0], art.q_native([f], 8, f)[0]]))
    sheet("c_logos", rows, "(c) logo -> fit to height, <=16 colours   [Wikimedia Commons, public domain]")


# --- (d) a tall image -> vertical scroll -------------------------------------------

def demo_d():
    print("(d) tall scroll")
    a, _ = art.native(rgba(SRC / "pp" / "SampleA.png")[..., :3])
    b, _ = art.native(rgba(SRC / "pp" / "SampleB.png")[..., :3])
    tall = np.concatenate([b[0:215], a[0:215]], 0)  # 459x430, sample text/logo rows dropped
    x, _ = art.best_window(tall, W, tall.shape[0], centre_bias=0)
    col = np.ascontiguousarray(tall[:, x : x + W])
    ys = range(0, col.shape[0] - H + 1)
    frames = [np.ascontiguousarray(col[y : y + H]) for y in ys]
    delays = [1500] + [40] * (len(frames) - 2) + [1500]  # the pack's scrolls hold 1.5 s at the ends
    n = art.save_gif(OUT / "d_scroll.gif", frames, delays)
    print(f"  column x={x}, {len(frames)} frames, 1 px/frame at 40 ms ({len(frames) * 0.04 + 2.9:.1f} s), {n} colours -> out/d_scroll.gif")
    flat_x(col, 2).save(OUT / "d_source_column.png", optimize=True)
    sheet("d_scroll", [("vertical scroll, native pixels, 1 px per 40 ms frame (5 of the frames)", strip(frames, 5)[:3]),
                       ("", strip(frames, 5)[3:])],
          "(d) tall image -> vertical scroll like the pack's *_SCROLL_* entries   [Kenney, CC0]")


# --- (e) text fallback ---------------------------------------------------------------

PLATFORM_COLOURS = {  # (background top, background bottom, tag colour)
    "GB": ((48, 98, 48), (15, 56, 15), (210, 240, 90)),
    "MD": ((20, 20, 60), (0, 0, 10), (80, 140, 255)),
    "SNES": ((60, 50, 90), (15, 10, 30), (180, 160, 255)),
    "SATURN": ((10, 30, 70), (0, 5, 20), (120, 200, 255)),
    "3DS": ((90, 10, 10), (20, 0, 0), (255, 80, 60)),
}


def title_card(platform, title, big, tiny):
    top, bot, accent = [np.array(c) for c in PLATFORM_COLOURS[platform]]
    t = np.linspace(0, 1, H)[:, None, None]
    bg = ((top * (1 - t) + bot * t) // 8 * 8).astype(np.uint8)  # banded, few colours
    bg = np.broadcast_to(bg, (H, W, 3)).copy()

    def stamp(f, m, x, y, colour, shadow=True):
        for dy, dx, c in ([(1, 1, (0, 0, 0))] if shadow else []) + [(0, 0, colour)]:
            yy, xx = np.nonzero(m)
            yy, xx = yy + y + dy, xx + x + dx
            ok = (yy >= 0) & (yy < H) & (xx >= 0) & (xx < W)
            f[yy[ok], xx[ok]] = c
        return f

    words = title.split()
    if big.width(title) <= W - 4:
        lines = [title]
    else:  # the most balanced two-line split that fits, if any
        splits = [(" ".join(words[:i]), " ".join(words[i:])) for i in range(1, len(words))]
        fits = [sp for sp in splits if max(big.width(x) for x in sp) <= W - 4]
        lines = list(min(fits, key=lambda sp: max(big.width(x) for x in sp))) if fits else [title, "", ""]
    if len(lines) == 1:  # tag + one line
        f = stamp(bg.copy(), tiny.mask(platform), 2, 1, accent, shadow=False)
        m = big.mask(title)
        return [stamp(f, m, (W - m.shape[1]) // 2, 11, (255, 255, 255))], None
    if len(lines) == 2 and all(lines):  # two lines, no tag
        f = bg.copy()
        for i, ln in enumerate(lines):
            m = big.mask(ln)
            f = stamp(f, m, (W - m.shape[1]) // 2, i * 16, (255, 255, 255))
        return [f], None
    # Too long: tag, then a marquee at 1 px/frame.
    m = big.mask(title)
    frames = []
    for x in range(W, -m.shape[1] - 1, -1):
        f = stamp(bg.copy(), tiny.mask(platform), 2, 1, accent, shadow=False)
        frames.append(stamp(f, m, x, 11, (255, 255, 255)))
    return frames, 30


def demo_e():
    print("(e) text fallback")
    big = Bdf(SRC / "spleen-2.1.0" / "spleen-8x16.bdf")
    tiny = Bdf(SRC / "tom-thumb.bdf")
    rows = []
    for plat, title in [("GB", "Tobu Tobu Girl"), ("MD", "Streets of Rage 2"), ("SATURN", "Panzer Dragoon"),
                        ("3DS", "Kid Icarus Uprising"), ("SNES", "The Legend of Zelda: A Link to the Past")]:
        frames, delay = title_card(plat, title, big, tiny)
        fn = f"e_{plat}_{title.split(':')[0].replace(' ', '_')}.gif"
        art.save_gif(OUT / fn, frames, delay or 5000)
        rows.append((f"{plat} \"{title}\" -> {len(frames)} frame(s){' marquee 1 px / 30 ms' if delay else ''}",
                     frames if len(frames) == 1 else strip(frames[len(frames) // 5 : -len(frames) // 5], 3)))
    sheet("e_title_cards", rows, "(e) no art: title in Spleen 8x16 (BSD-2), platform in Tom Thumb 3x5 (MIT/CC0)")


# --- (f) a real title screen from the emulator prototype ---------------------------

def demo_f():
    print("(f) emulator frame: Miniplanets REMIX (MD homebrew, zlib) title, 320x224")
    shot = rgba(Path(__file__).resolve().parents[1] / "emulator" / "out" / "md_gpgx_04_f480.png")[..., :3]
    nat, k = art.native(shot)
    x1, y1 = art.best_window(nat, 128, 32)
    x2, y2 = art.best_window(nat, 256, 64)
    print(f"  native scale {k}; 128x32 window at ({x1},{y1}); 256x64 at ({x2},{y2})")
    band = nat[y2 : y2 + 64, x2 : x2 + 256]
    logo = nat[8:72]  # the wordmark rows, picked by eye, as a user would drag it
    lx = art.best_window(logo, 256, 64)[0]
    rows = [
        ("auto: busiest 128x32 at 1:1", nat[y1 : y1 + 32, x1 : x1 + 128]),
        ("auto: busiest 256x64, mode 2:1", art.down_mode(band, 2)),
        ("user drags to the wordmark: 256x64, mode 2:1", art.down_mode(logo[:, lx : lx + 256], 2)),
        ("same, area 2:1 (for comparison)", art.down_area(logo[:, lx : lx + 256], 2)),
    ]
    sheet("f_emulator_title", rows, "(f) a real MD title screen grabbed natively by the emulator prototype")


if __name__ == "__main__":
    OUT.mkdir(exist_ok=True)
    want = sys.argv[1:] or list("abcdef")
    for k in want:
        globals()[f"demo_{k}"]()
