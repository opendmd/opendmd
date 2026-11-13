# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Hostile pictures for the web smoke's sprite import (tools/web-smoke.py
psi_import_checks): written by a red-team pass that never saw
js/fimport.js, from its contract only. hostile_fixtures(canary) -> [{name,
file, mime, data, expect: import|refuse|either, why}]. Every picture is
drawn here (Pillow and hand-written PNG chunks); nothing is fetched.
"""
import base64
import io
import random
import struct
import zlib
from urllib.parse import urlparse

from PIL import Image, ImageDraw

SVGNS = "http://www.w3.org/2000/svg"
XLINK = "http://www.w3.org/1999/xlink"


# ----------------------------------------------------------------- PNG utils
def chunk(typ, data=b"", crc=None):
    c = zlib.crc32(typ + data) & 0xFFFFFFFF if crc is None else crc
    return struct.pack(">I", len(data)) + typ + data + struct.pack(">I", c)


def pix(x, y, w, h):
    """Our own picture: gradient + disc, RGBA 0..255 ints."""
    r = int(255 * x / max(1, w - 1))
    g = int(255 * y / max(1, h - 1))
    b = 128
    a = 255
    cx, cy, rad = w / 2, h / 2, min(w, h) / 3
    d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
    if d < rad:
        r, g, b = 250, 40, 40
    elif d > min(w, h) * 0.48 and (x + y) % 3 == 0:
        a = 0
    return r, g, b, a


def build_png(w, h, color, depth, rows=None, plte=None, trns=None,
              interlace=0, pre=(), idat_raw=None, post=(), bad_crc=False):
    sig = b"\x89PNG\r\n\x1a\n"
    out = sig + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, color, 0, 0, interlace))
    for c in pre:
        out += c
    if plte is not None:
        out += chunk(b"PLTE", plte)
    if trns is not None:
        out += chunk(b"tRNS", trns)
    raw = idat_raw if idat_raw is not None else b"".join(b"\x00" + r for r in rows)
    comp = zlib.compress(raw, 9)
    out += chunk(b"IDAT", comp, crc=(0 if bad_crc else None))
    for c in post:
        out += c
    out += chunk(b"IEND")
    return out


def adam7_raw(w, h, bpp_bytes, pixel_bytes):
    passes = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4),
              (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
    out = bytearray()
    for xs, ys, dx, dy in passes:
        xsr = list(range(xs, w, dx))
        if not xsr:
            continue
        for y in range(ys, h, dy):
            out.append(0)
            for x in xsr:
                out += pixel_bytes(x, y)
    return bytes(out)


def png_bytes_pillow(img):
    b = io.BytesIO()
    img.save(b, "PNG")
    return b.getvalue()


def pillow_bytes(img, fmt, **kw):
    b = io.BytesIO()
    img.save(b, fmt, **kw)
    return b.getvalue()


def rgba_image(w, h):
    im = Image.new("RGBA", (w, h))
    im.putdata([pix(x, y, w, h) for y in range(h) for x in range(w)])
    return im


# ------------------------------------------------------------------ SVG utils
def svg_doc(inner, attrs='width="32" height="32" viewBox="0 0 32 32"', extra_ns="", pre=""):
    return (f'{pre}<svg xmlns="{SVGNS}" xmlns:xlink="{XLINK}" {extra_ns} {attrs}>'
            f'<rect width="32" height="32" fill="#2a6"/>'
            f'<circle cx="16" cy="16" r="9" fill="#e33"/>{inner}</svg>')


def hostile_fixtures(canary):
    rng = random.Random(20261113)
    fx = []

    def add(name, file, mime, data, expect, why):
        if isinstance(data, str):
            data = data.encode("utf-8")
        fx.append({"name": name, "file": file, "mime": mime, "data": data,
                   "expect": expect, "why": why})

    u = urlparse(canary)
    schemeless = "//" + u.netloc + u.path

    # ============================================================== PNG
    W, H = 24, 20
    # 16-bit RGBA
    rows = []
    for y in range(H):
        row = bytearray()
        for x in range(W):
            for v in pix(x, y, W, H):
                row += struct.pack(">H", v * 257)
        rows.append(bytes(row))
    add("png-16bit-rgba", "rgba16.png", "image/png",
        build_png(W, H, 6, 16, rows), "import", "16-bit RGBA must be reduced to 8-bit correctly")

    # 16-bit grey
    rows = [b"".join(struct.pack(">H", (x * 65535) // (W - 1)) for x in range(W)) for y in range(H)]
    add("png-16bit-grey", "grey16.png", "image/png",
        build_png(W, H, 0, 16, rows), "import", "16-bit greyscale ramp")

    # Adam7 interlaced RGBA 8-bit
    W7, H7 = 23, 19
    raw = adam7_raw(W7, H7, 4, lambda x, y: bytes(pix(x, y, W7, H7)))
    add("png-adam7-rgba", "interlaced.png", "image/png",
        build_png(W7, H7, 6, 8, idat_raw=raw, interlace=1), "import", "Adam7 interlaced RGBA (hand written)")

    # Adam7 greyscale 8 with odd size 5x5 (some passes empty-ish)
    raw = adam7_raw(5, 3, 1, lambda x, y: bytes([x * 50 + y]))
    add("png-adam7-tiny", "interlaced-tiny.png", "image/png",
        build_png(5, 3, 0, 8, idat_raw=raw, interlace=1), "import", "Adam7 5x3 where several passes are empty")

    # palette + tRNS partial/full
    pal = bytearray()
    for i in range(16):
        pal += bytes([i * 16, 255 - i * 16, (i * 37) % 256])
    trns = bytes([0, 40, 80, 120, 160, 200, 255, 255])  # shorter than palette: rest opaque
    rows = [bytes(((x + y) % 16) for x in range(W)) for y in range(H)]
    add("png-palette-trns", "pal.png", "image/png",
        build_png(W, H, 3, 8, rows, plte=bytes(pal), trns=trns), "import",
        "palette with tRNS: index0 fully transparent, others partial, tRNS shorter than PLTE")

    # 2-bit palette
    rows = []
    for y in range(8):
        bits = [(x + y) % 4 for x in range(8)]
        rows.append(bytes([(bits[0] << 6) | (bits[1] << 4) | (bits[2] << 2) | bits[3],
                           (bits[4] << 6) | (bits[5] << 4) | (bits[6] << 2) | bits[7]]))
    add("png-palette-2bit", "pal2.png", "image/png",
        build_png(8, 8, 3, 2, rows, plte=bytes([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0]),
                  trns=b"\x00"), "import", "2-bit palette with tRNS")

    # RGB with colour-key tRNS
    rows = [b"".join(bytes([255, 0, 0]) if (x // 2 + y // 2) % 2 else bytes([0, 0, 255]) for x in range(16))
            for y in range(16)]
    add("png-rgb-colorkey", "rgbkey.png", "image/png",
        build_png(16, 16, 2, 8, rows, trns=struct.pack(">HHH", 0, 0, 255)), "import",
        "truecolour with colour-key transparency (blue squares transparent)")

    # 1-bit grey
    rows = []
    for y in range(16):
        rows.append(bytes([0xAA if y % 2 else 0x55, 0xF0 if y < 8 else 0x0F]))
    add("png-1bit-grey", "bw.png", "image/png", build_png(16, 16, 0, 1, rows), "import", "1-bit greyscale")

    # grey+alpha 8
    rows = [b"".join(bytes([x * 10, 255 if x > 5 else 60]) for x in range(24)) for y in range(8)]
    add("png-grey-alpha", "ga.png", "image/png", build_png(24, 8, 4, 8, rows), "import", "greyscale+alpha")

    good = build_png(W, H, 6, 8, [b"".join(bytes(pix(x, y, W, H)) for x in range(W)) for y in range(H)])
    add("png-plain", "plain.png", "image/png", good, "import", "baseline control PNG")

    add("png-1x1", "one.png", "image/png",
        build_png(1, 1, 6, 8, [b"\xff\x00\x00\xff"]), "import", "1x1 pixel image")

    add("png-bad-crc", "badcrc.png", "image/png",
        build_png(W, H, 6, 8, [b"".join(bytes(pix(x, y, W, H)) for x in range(W)) for y in range(H)],
                  bad_crc=True), "either", "IDAT CRC wrong; browsers usually reject, some tolerate")

    add("png-truncated", "trunc.png", "image/png", good[:len(good) * 2 // 3], "either",
        "PNG cut off in the middle of IDAT")

    add("png-truncated-header", "trunc-hdr.png", "image/png", good[:20], "refuse",
        "PNG cut inside IHDR: cannot decode")

    # IHDR 5000x10 (valid zlib, tiny)
    add("png-5000x10", "wide.png", "image/png",
        build_png(5000, 10, 2, 8, idat_raw=(b"\x00" + bytes(5000 * 3)) * 10), "refuse",
        "wider than 4096; deflates to a few dozen bytes")
    add("png-1x4097", "tall.png", "image/png",
        build_png(1, 4097, 0, 8, idat_raw=(b"\x00\x80") * 4097), "refuse", "4097 px tall just over limit")

    # IHDR claims 5000x10 but IDAT is garbage (the 'lie' variant)
    lie = bytearray(good)
    lie[16:24] = struct.pack(">II", 5000, 10)
    ihdr_crc = zlib.crc32(bytes(lie[12:29])) & 0xFFFFFFFF
    lie[29:33] = struct.pack(">I", ihdr_crc)
    add("png-ihdr-lies-5000x10", "lie.png", "image/png", bytes(lie), "refuse",
        "IHDR rewritten to 5000x10 with valid CRC but small IDAT for a 24x20 image")

    # boundary 4096x4096 grey (decompressed 16 MB, compresses small)
    row = b"\x00" + bytes((x * 255) // 4095 for x in range(4096))
    add("png-4096x4096", "max.png", "image/png",
        build_png(4096, 4096, 0, 8, idat_raw=row * 4096), "either",
        "exactly at the pixel limit; 16 MB decoded from ~20 KB; must stay responsive (import or refuse ok)")

    # pixel-bomb: 4096 x 4096 RGBA 64 MB decoded
    row = b"\x00" + bytes([200, 30, 30, 255]) * 4096
    add("png-4096x4096-rgba", "bomb.png", "image/png",
        build_png(4096, 4096, 6, 8, idat_raw=row * 4096), "either",
        "64 MB decoded RGBA at the limit; must not hang or OOM the tab")

    # huge iTXt + zTXt
    itxt = b"Comment\x00\x00\x00\x00\x00" + (b"lorem ipsum dolor sit amet " * 130000)[:3_500_000]
    ztxt = b"Big\x00\x00" + zlib.compress(b"A" * 6_000_000, 9)
    add("png-huge-text-chunks", "text.png", "image/png",
        build_png(W, H, 6, 8, [b"".join(bytes(pix(x, y, W, H)) for x in range(W)) for y in range(H)],
                  pre=[chunk(b"iTXt", itxt), chunk(b"zTXt", ztxt)]), "import",
        "3.5 MB iTXt (+ zTXt that inflates to 6 MB) ahead of tiny pixel data; total < 8 MB")

    # over 8 MB valid PNG
    big = build_png(W, H, 6, 8, [b"".join(bytes(pix(x, y, W, H)) for x in range(W)) for y in range(H)],
                    pre=[chunk(b"tEXt", b"pad\x00" + b"x" * (8 * 1024 * 1024 + 10))])
    add("png-over-8mb", "huge.png", "image/png", big, "refuse", "valid picture padded above 8 MB")

    # APNG
    frames = []
    for i in range(3):
        im = Image.new("RGBA", (16, 16), (0, 0, 0, 0))
        ImageDraw.Draw(im).rectangle((i * 4, 2, i * 4 + 5, 12), fill=(250, 200, 0, 255))
        frames.append(im)
    apng = pillow_bytes(frames[0], "PNG", save_all=True, append_images=frames[1:], duration=100, loop=0)
    add("png-apng", "anim.png", "image/png", apng, "import", "animated PNG: first frame (or static) acceptable")

    # PNG with trailing JS / polyglot
    add("png-polyglot-trailer", "poly.png", "image/png",
        good + b"<script>window.__pwned=1</script>" + b"\x00" * 100, "import",
        "data after IEND is ignored")

    add("png-header-garbage", "garbage.png", "image/png",
        b"\x89PNG\r\n\x1a\n" + bytes(rng.randrange(256) for _ in range(3000)), "either",
        "valid signature followed by random noise")

    add("png-misnamed-jpg", "x.jpg", "image/jpeg", good, "import", "real PNG named .jpg, mime jpeg: magic wins")

    # ============================================================== JPEG
    rgb = rgba_image(40, 24).convert("RGB")
    cm = rgb.convert("CMYK")
    add("jpeg-cmyk", "cmyk.jpg", "image/jpeg", pillow_bytes(cm, "JPEG", quality=90), "either",
        "CMYK JPEG (Adobe inverted); some browsers decode, some fail")

    ex = Image.Exif()
    ex[0x0112] = 6
    add("jpeg-exif-orient6", "rot.jpg", "image/jpeg",
        pillow_bytes(rgb, "JPEG", quality=90, exif=ex.tobytes()), "import",
        "40x24 with EXIF Orientation=6; browsers honour it so the sprite becomes 24x40 upright")

    add("jpeg-progressive", "prog.jpg", "image/jpeg",
        pillow_bytes(rgb, "JPEG", quality=85, progressive=True), "import", "progressive JPEG")
    add("jpeg-grey", "grey.jpg", "image/jpeg", pillow_bytes(rgb.convert("L"), "JPEG", quality=85),
        "import", "greyscale JPEG")
    jp = pillow_bytes(rgb, "JPEG", quality=85)
    add("jpeg-truncated", "trunc.jpg", "image/jpeg", jp[:len(jp) // 2], "either", "JPEG cut in half")
    add("jpeg-4097x8", "wide.jpg", "image/jpeg",
        pillow_bytes(Image.new("RGB", (4097, 8), (10, 120, 200)), "JPEG", quality=50), "refuse",
        "4097 px wide JPEG")
    add("jpeg-misnamed-png", "x.png", "image/png", jp, "import", "JPEG named .png with png mime")

    # ============================================================== GIF
    frames = []
    for i in range(5):
        im = Image.new("P", (24, 24), 0)
        im.putpalette([0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255] + [0] * 756)
        d = ImageDraw.Draw(im)
        d.ellipse((i * 3, 4, i * 3 + 8, 12), fill=1 + i % 3)
        frames.append(im)
    add("gif-animated-transparent", "anim.gif", "image/gif",
        pillow_bytes(frames[0], "GIF", save_all=True, append_images=frames[1:], duration=80, loop=0,
                     transparency=0, disposal=2), "import",
        "5-frame animation, index 0 transparent, restore-to-background")
    still = Image.new("P", (20, 20), 0)
    still.putpalette([0, 0, 0, 255, 128, 0] + [0] * 762)
    ImageDraw.Draw(still).rectangle((4, 4, 15, 15), fill=1)
    g = pillow_bytes(still, "GIF")
    if g[:6] == b"GIF89a":
        g87 = b"GIF87a" + g[6:]
    else:
        g87 = g
    add("gif-87a", "old.gif", "image/gif", g87, "import", "GIF87a header, single frame, no extensions")
    # logical screen claims 5000x5000 (frame small)
    gl = bytearray(g)
    gl[6:10] = struct.pack("<HH", 5000, 5000)
    add("gif-screen-5000", "screen.gif", "image/gif", bytes(gl), "either",
        "logical screen descriptor says 5000x5000 but image data is 20x20")
    add("gif-5000x8", "wide.gif", "image/gif", pillow_bytes(Image.new("P", (5000, 8), 0), "GIF"), "refuse",
        "wider than 4096")
    add("gif-truncated", "trunc.gif", "image/gif", g[:len(g) // 2], "either", "GIF truncated")

    # ============================================================== WebP / BMP / others
    add("webp-lossless-alpha", "a.webp", "image/webp",
        pillow_bytes(rgba_image(32, 24), "WEBP", lossless=True), "import", "lossless WebP with alpha")
    add("webp-lossy", "l.webp", "image/webp", pillow_bytes(rgb, "WEBP", quality=70), "import", "lossy WebP")
    wf = [Image.new("RGBA", (16, 16), (255 * (i % 2), 80, 200, 255)) for i in range(3)]
    add("webp-animated", "anim.webp", "image/webp",
        pillow_bytes(wf[0], "WEBP", save_all=True, append_images=wf[1:], duration=100, lossless=True),
        "import", "animated WebP")
    add("webp-4097x8", "wide.webp", "image/webp",
        pillow_bytes(Image.new("RGB", (4097, 8), (1, 2, 3)), "WEBP"), "refuse", "4097 px wide WebP")
    add("bmp-24bit", "p.bmp", "image/bmp", pillow_bytes(rgb, "BMP"), "import", "24-bit BMP")
    add("bmp-4097x1", "wide.bmp", "image/bmp", pillow_bytes(Image.new("RGB", (4097, 1), (9, 9, 9)), "BMP"),
        "refuse", "4097 px wide BMP")
    bmp = bytearray(pillow_bytes(rgb, "BMP"))
    bmp[18:22] = struct.pack("<i", 100000)  # width lies
    add("bmp-lying-width", "lie.bmp", "image/bmp", bytes(bmp), "refuse",
        "BMP header claims 100000 px wide with tiny data")
    add("tiff-unsupported", "p.tiff", "image/tiff", pillow_bytes(rgb, "TIFF"), "refuse",
        "TIFF is not on the accepted list")
    add("ico-unsupported", "p.ico", "image/x-icon", pillow_bytes(rgb.resize((32, 32)), "ICO"), "refuse",
        "ICO is not on the accepted list")

    # ============================================================== non-images
    add("empty-file", "empty.png", "image/png", b"", "refuse", "zero bytes")
    add("text-file", "note.png", "image/png", b"hello this is not a picture\n" * 10, "refuse",
        "plain text named png")
    add("pdf-file", "doc.png", "image/png", b"%PDF-1.4\n1 0 obj<<>>endobj\ntrailer<<>>\n%%EOF", "refuse",
        "PDF magic")
    add("zip-file", "a.png", "image/png", b"PK\x03\x04" + b"\x00" * 100, "refuse", "ZIP magic")
    add("html-file", "page.svg", "image/svg+xml",
        b"<!DOCTYPE html><html><body><h1>hi</h1><script>window.__pwned=1</script></body></html>",
        "refuse", "HTML named svg")
    add("jpeg-magic-only", "m.jpg", "image/jpeg", b"\xff\xd8\xff", "refuse", "just the 3 JPEG magic bytes")
    add("gif-magic-only", "m.gif", "image/gif", b"GIF89a", "refuse", "just the GIF magic")
    add("riff-not-webp", "w.webp", "image/webp", b"RIFF\x24\x00\x00\x00WAVEfmt " + b"\x00" * 40, "refuse",
        "RIFF container that is WAV, not WebP")
    add("bm-not-bmp", "b.bmp", "image/bmp", b"BM" + bytes(rng.randrange(256) for _ in range(200)), "refuse",
        "starts with BM but random body")

    # ============================================================== SVG
    def S(name, body, expect, why, file=None, mime="image/svg+xml"):
        add(name, file or (name + ".svg"), mime, body, expect, why)

    S("svg-plain", svg_doc(""), "import", "control SVG")
    S("svg-script", svg_doc(
        f'<script>window.__pwned=1;fetch("{canary}?script");alert(1)</script>'
        f'<script xlink:href="{canary}?scriptsrc"/>'), "import", "script elements inline and external")
    S("svg-script-cdata", svg_doc(
        f'<script><![CDATA[window.__pwned=1;new Image().src="{canary}?cdata";alert(2)]]></script>'),
      "import", "script in CDATA")
    S("svg-onload", svg_doc("", attrs=f'width="32" height="32" viewBox="0 0 32 32" '
                                      f'onload="window.__pwned=1;fetch(\'{canary}?onload\')"'),
      "import", "onload on root")
    S("svg-onerror-and-friends", svg_doc(
        f'<image width="4" height="4" href="data:image/png;base64,AAAA" onerror="window.__pwned=1;alert(3)"/>'
        f'<rect width="4" height="4" onclick="alert(4)" onmouseover="alert(5)" onfocusin="alert(6)" '
        f'ONLOAD="alert(7)" OnClick="alert(8)"/>'
        f'<animate attributeName="x" begin="0s" onbegin="window.__pwned=1;alert(9)" onend="alert(10)"/>'),
      "import", "on* handlers on many elements, mixed case")
    S("svg-image-href", svg_doc(f'<image width="16" height="16" href="{canary}?image"/>'), "import",
      "external <image href>")
    S("svg-image-xlink", svg_doc(f'<image width="16" height="16" xlink:href="{canary}?xlink"/>'), "import",
      "external <image xlink:href>")
    S("svg-image-protocol-relative", svg_doc(f'<image width="16" height="16" href="{schemeless}?pr"/>'
                                             f'<image width="4" height="4" href="  {canary}?ws"/>'
                                             f'<image width="4" height="4" href="\t{canary}?tab"/>'),
      "import", "protocol-relative and whitespace-prefixed external image refs")
    S("svg-image-file-url", svg_doc('<image width="4" height="4" href="file:///etc/passwd"/>'
                                    '<image width="4" height="4" href="ftp://127.0.0.1:1/x"/>'),
      "import", "file: and ftp: image refs")
    tinypng = base64.b64encode(png_bytes_pillow(Image.new("RGBA", (4, 4), (0, 0, 255, 255)))).decode()
    S("svg-image-data-png", svg_doc(f'<image x="2" y="2" width="8" height="8" href="data:image/png;base64,{tinypng}"/>'),
      "import", "data:image/png on <image> is allowed and should stay")
    S("svg-image-data-svg", svg_doc(
        '<image width="8" height="8" href="data:image/svg+xml;utf8,&lt;svg xmlns=\'http://www.w3.org/2000/svg\' '
        f'onload=\'alert(1)\'&gt;&lt;image href=\'{canary}?nested\'/&gt;&lt;/svg&gt;"/>'),
      "import", "nested data:image/svg+xml in <image>: not on allow-list, must be removed")
    S("svg-data-html", svg_doc('<image width="8" height="8" href="data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg=="/>'),
      "import", "data:text/html on <image> must be stripped")
    S("svg-foreignobject", svg_doc(
        f'<foreignObject width="32" height="32"><body xmlns="http://www.w3.org/1999/xhtml">'
        f'<iframe src="{canary}?iframe"></iframe><img src="{canary}?img" onerror="alert(1)"/>'
        f'<script>window.__pwned=1</script><link rel="stylesheet" href="{canary}?link"/></body></foreignObject>'),
      "import", "foreignObject with XHTML iframe/img/script/link")
    S("svg-html-elements-in-svg", svg_doc(
        f'<iframe xmlns="http://www.w3.org/1999/xhtml" src="{canary}?i2"/>'
        f'<object xmlns="http://www.w3.org/1999/xhtml" data="{canary}?obj"/>'
        f'<embed xmlns="http://www.w3.org/1999/xhtml" src="{canary}?emb"/>'
        f'<audio xmlns="http://www.w3.org/1999/xhtml" src="{canary}?aud" autoplay="autoplay"/>'
        f'<video xmlns="http://www.w3.org/1999/xhtml" src="{canary}?vid" poster="{canary}?poster" autoplay="autoplay"/>'
        f'<link xmlns="http://www.w3.org/1999/xhtml" rel="stylesheet" href="{canary}?lnk"/>'),
      "import", "XHTML iframe/object/embed/audio/video/link as direct SVG children")
    S("svg-css-import", svg_doc("", ).replace("<rect", f'<style>@import url("{canary}?imp");</style><rect', 1),
      "import", "@import url() in <style>")
    S("svg-css-import-bare", svg_doc("").replace("<rect", f'<style>@import "{canary}?bare";</style><rect', 1),
      "import", "@import without url(): bare string")
    S("svg-css-import-upper", svg_doc("").replace(
        "<rect", f'<style>@IMPORT "{canary}?up"; @im/**/port "{canary}?cm";</style><rect', 1),
      "import", "@IMPORT uppercase and comment-split")
    S("svg-css-url", svg_doc("").replace(
        "<rect", f'<style>rect{{fill:url({canary}?fill)}} circle{{filter:url("{canary}?f#f")}} '
                 f'svg{{cursor:url({canary}?cur),auto;background:url(\'{canary}?bg\')}}</style><rect', 1),
      "import", "url() to canary in <style>: fill, filter, cursor, background")
    S("svg-css-url-style-attr", svg_doc(
        f'<rect width="8" height="8" style="fill:url({canary}?a);mask:url({canary}?m);'
        f'clip-path:url({canary}?c);marker-end:url({canary}?mk)"/>'),
      "import", "url() in style attributes")
    S("svg-css-url-escape", svg_doc("").replace(
        "<rect", f'<style>rect{{fill:\\75rl({canary}?esc);background:ur\\6c({canary}?esc2);}}</style><rect', 1),
      "import", "CSS escapes hiding url(")
    S("svg-css-imageset", svg_doc(
        f'<rect width="8" height="8" style="background:image-set(\'{canary}?is\' 1x);'
        f'content:image-set(\'{canary}?is2\' 1x);border-image-source:src(\'{canary}?src\')"/>'),
      "import", "network fetch via image-set()/src() with no url( token")
    S("svg-css-fontface", svg_doc("").replace(
        "<rect", f'<style>@font-face{{font-family:X;src:url({canary}?font)}} text{{font-family:X}}</style>'
                 f'<rect', 1) + "", "import", "@font-face src:url(canary)")
    S("svg-css-fontface-local-text", svg_doc(
        f'<style>@font-face{{font-family:Y;src:url("{canary}?font2") format("woff2")}}</style>'
        f'<text x="2" y="20" font-family="Y" font-size="12">Hi</text>'), "import",
      "@font-face plus <text> that actually uses it")
    S("svg-css-cdata-url", svg_doc(
        f'<style><![CDATA[ rect {{ fill: url({canary}?cd) }} @import "{canary}?cdi"; ]]></style>'),
      "import", "CDATA wrapped CSS")
    S("svg-css-url-fragment-ok", svg_doc(
        '<defs><linearGradient id="g"><stop offset="0" stop-color="#00f"/><stop offset="1" stop-color="#ff0"/>'
        '</linearGradient></defs><rect width="32" height="8" style="fill:url(#g)"/>'),
      "import", "legit url(#g) must remain")
    S("svg-use-external", svg_doc(f'<use href="{canary}?use#x"/><use xlink:href="{canary}?use2#x"/>'),
      "import", "external <use>")
    S("svg-use-data-svg", svg_doc(
        '<use href="data:image/svg+xml;utf8,&lt;svg xmlns=\'http://www.w3.org/2000/svg\'&gt;'
        f'&lt;image href=\'{canary}?usedata\'/&gt;&lt;/svg&gt;#x"/>'), "import", "<use> with data: svg")
    S("svg-use-local-ok", svg_doc('<defs><rect id="r" width="6" height="6" fill="#fff"/></defs>'
                                  '<use href="#r" x="2" y="2"/><use xlink:href="#r" x="20" y="2"/>'),
      "import", "legit local <use> refs")
    S("svg-use-bomb", svg_doc(
        '<defs><g id="a0"><rect width="1" height="1"/></g>' +
        "".join(f'<g id="a{i}"><use href="#a{i-1}"/><use href="#a{i-1}"/></g>' for i in range(1, 30)) +
        '</defs><use href="#a29"/>'), "either",
      "exponential <use> expansion (2^29 instances): must not hang the page")
    S("svg-use-self", svg_doc('<g id="s"><use href="#s"/></g><use href="#s"/>'), "either", "self-referencing <use>")
    S("svg-js-href", svg_doc(
        '<a href="javascript:alert(1)"><rect width="8" height="8"/></a>'
        '<a xlink:href="javascript:alert(2)"><rect x="8" width="8" height="8"/></a>'
        '<a href="JaVaScRiPt:alert(3)"><rect x="16" width="8" height="8"/></a>'
        '<a href="&#106;avascript:alert(4)"><rect y="8" width="8" height="8"/></a>'
        '<a href="java&#9;script:alert(5)"><rect x="8" y="8" width="8" height="8"/></a>'
        '<a href=" javascript:alert(6)"><rect x="16" y="8" width="8" height="8"/></a>'
        f'<a href="{canary}?a"><rect y="16" width="8" height="8"/></a>'
        '<a href="data:text/html,&lt;script&gt;alert(7)&lt;/script&gt;"><rect x="8" y="16" width="8" height="8"/></a>'),
      "import", "javascript: URLs with case/entity/tab/space obfuscation, external and data: links")
    S("svg-animate-href", svg_doc(
        f'<image width="8" height="8" href="#nothing"><animate attributeName="href" '
        f'values="#a;{canary}?anim" dur="1s" repeatCount="indefinite"/>'
        f'<set attributeName="xlink:href" to="{canary}?set" begin="0s"/></image>'
        f'<a href="#x"><set attributeName="href" to="javascript:alert(1)"/><rect width="4" height="4"/></a>'),
      "import", "animate/set rewriting href to canary / javascript:")
    S("svg-animate-misc", svg_doc(
        '<rect width="8" height="8"><animate attributeName="x" from="0" to="24" dur="1s" repeatCount="indefinite"/>'
        '<animateTransform attributeName="transform" type="rotate" from="0" to="360" dur="2s"/>'
        '<animateMotion path="M0,0 L10,10" dur="1s"/></rect>'),
      "import", "animation elements stripped; static picture remains")
    S("svg-filter-feimage", svg_doc(
        f'<filter id="f"><feImage href="{canary}?feimage"/><feImage xlink:href="{canary}?feimage2"/></filter>'
        '<rect width="32" height="32" filter="url(#f)"/>'), "import", "feImage external")
    S("svg-pattern-external", svg_doc(
        f'<pattern id="p" href="{canary}?pat#q" width="4" height="4" patternUnits="userSpaceOnUse"/>'
        f'<pattern id="p2" xlink:href="{canary}?pat2" width="4" height="4"/>'
        '<rect width="16" height="16" fill="url(#p)"/>'), "import", "pattern / gradient with external href")
    S("svg-gradient-external", svg_doc(
        f'<linearGradient id="lg" xlink:href="{canary}?grad#z"/><rect width="16" height="16" fill="url(#lg)"/>'),
      "import", "gradient inheriting from external href")
    S("svg-mask-clip-external", svg_doc(
        f'<rect width="32" height="32" mask="url({canary}?mask#m)" clip-path="url({canary}?clip#c)" '
        f'filter="url({canary}?filt#f)" fill="red"/>'), "import",
      "presentation attributes (not style) with url(canary)")
    S("svg-marker-external-attr", svg_doc(
        f'<path d="M2 2L30 30" stroke="#fff" marker-end="url({canary}?marker#m)" '
        f'fill="url({canary}?fillattr)"/>'), "import", "fill/marker attributes with url(canary)")
    S("svg-xml-stylesheet-pi", svg_doc("", pre=f'<?xml version="1.0"?><?xml-stylesheet href="{canary}?pi" type="text/css"?>\n'),
      "either", "xml-stylesheet processing instruction pointing at canary")
    S("svg-xml-stylesheet-pi-inline", svg_doc(
        "", pre=f'<?xml version="1.0"?><?xml-stylesheet type="text/xsl" href="{canary}?xsl"?>\n'),
      "either", "XSL stylesheet PI")
    S("svg-style-link-xhtml", svg_doc(
        f'<link xmlns="http://www.w3.org/1999/xhtml" rel="stylesheet" href="{canary}?link"/>'
        f'<style xmlns="http://www.w3.org/1999/xhtml">@import "{canary}?xhs";</style>'),
      "import", "XHTML-namespace <link>/<style> inside SVG")

    # DOCTYPE / entity family: refuse
    lol = '<!DOCTYPE svg [<!ENTITY a "aaaaaaaaaa">' + "".join(
        f'<!ENTITY {chr(98+i)} "&{chr(97+i)};&{chr(97+i)};&{chr(97+i)};&{chr(97+i)};&{chr(97+i)};&{chr(97+i)};&{chr(97+i)};&{chr(97+i)};&{chr(97+i)};&{chr(97+i)};">'
        for i in range(8)) + ']>'
    S("svg-billion-laughs", f'<?xml version="1.0"?>{lol}<svg xmlns="{SVGNS}" width="32" height="32"><text>&i;</text></svg>',
      "refuse", "entity expansion bomb with DOCTYPE")
    S("svg-external-entity", f'<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY x SYSTEM "{canary}?xxe">]>'
                             f'<svg xmlns="{SVGNS}" width="32" height="32"><text>&x;</text></svg>',
      "refuse", "external SYSTEM entity")
    S("svg-external-entity-file", f'<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY x SYSTEM "file:///etc/passwd">]>'
                                  f'<svg xmlns="{SVGNS}" width="32" height="32"><text>&x;</text></svg>',
      "refuse", "file:// SYSTEM entity")
    S("svg-param-entity", f'<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY % p SYSTEM "{canary}?param"> %p;]>'
                          f'<svg xmlns="{SVGNS}" width="32" height="32"/>', "refuse", "parameter entity")
    S("svg-doctype-only", f'<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "{canary}?dtd">'
                          f'<svg xmlns="{SVGNS}" width="32" height="32"><rect width="32" height="32"/></svg>',
      "refuse", "plain DOCTYPE with external DTD (even the standard SVG one)")
    S("svg-doctype-lowercase", f'<!doctype svg><svg xmlns="{SVGNS}" width="32" height="32"/>', "refuse",
      "lowercase doctype is invalid XML and a DOCTYPE anyway")
    S("svg-entity-in-comment", f'<?xml version="1.0"?><!-- <!ENTITY x "y"> -->'
                               f'<svg xmlns="{SVGNS}" width="32" height="32"><rect width="32" height="32" fill="#2a6"/></svg>',
      "either", "<!ENTITY inside a comment: strict substring check refuses, strict parse accepts")
    S("svg-doctype-in-text", svg_doc('<text x="1" y="10"><![CDATA[<!DOCTYPE foo>]]></text>'), "either",
      "<!DOCTYPE inside CDATA: substring check refuses, parser accepts")
    S("svg-undefined-entity", f'<svg xmlns="{SVGNS}" width="32" height="32"><text>&nbsp;&foo;</text></svg>',
      "refuse", "undefined entities are XML errors")

    # sizes (an SVG is drawn at its shape, 512 x 256 at most, whatever it says: a declared
    # size past 4096 may be refused or drawn smaller)
    S("svg-huge-size-no-viewbox", f'<svg xmlns="{SVGNS}" width="1e9" height="1e9"><rect width="100" height="100" fill="#e33"/></svg>',
      "either", "width/height 1e9 with no viewBox: must not allocate a giant canvas")
    S("svg-huge-px-size", f'<svg xmlns="{SVGNS}" width="999999px" height="999999px" viewBox="0 0 32 32"><circle cx="16" cy="16" r="10"/></svg>',
      "either", "huge declared size with viewBox")
    S("svg-percent-size", f'<svg xmlns="{SVGNS}" width="100%" height="100%"><circle cx="16" cy="16" r="10"/></svg>',
      "either", "percentage size with no viewBox: intrinsic size undefined")
    S("svg-zero-size", f'<svg xmlns="{SVGNS}" width="0" height="0" viewBox="0 0 32 32"><circle cx="16" cy="16" r="10"/></svg>',
      "either", "zero size")
    S("svg-negative-size", f'<svg xmlns="{SVGNS}" width="-5" height="-5" viewBox="0 0 32 32"><circle cx="16" cy="16" r="10"/></svg>',
      "either", "negative size")
    S("svg-no-size-no-viewbox", f'<svg xmlns="{SVGNS}"><circle cx="16" cy="16" r="10" fill="#e33"/></svg>',
      "either", "no size information at all (browsers default 300x150)")
    S("svg-tiny-viewbox-huge-width", f'<svg xmlns="{SVGNS}" width="5000" height="10" viewBox="0 0 5000 10"><rect width="5000" height="10" fill="#33e"/></svg>',
      "either", "5000 px wide declared size, over 4096 limit")
    S("svg-4097-wide", f'<svg xmlns="{SVGNS}" width="4097" height="8"><rect width="4097" height="8" fill="#33e"/></svg>',
      "either", "4097 px wide")
    S("svg-nan-size", f'<svg xmlns="{SVGNS}" width="NaN" height="Infinity" viewBox="0 0 32 32"><circle cx="16" cy="16" r="10"/></svg>',
      "either", "NaN/Infinity attributes")

    # namespaces
    S("svg-prefixed-root", f'<svg:svg xmlns:svg="{SVGNS}" width="32" height="32" viewBox="0 0 32 32">'
                           f'<svg:rect width="32" height="32" fill="#2a6"/><svg:circle cx="16" cy="16" r="9" fill="#e33"/>'
                           f'<svg:script>window.__pwned=1;fetch("{canary}?pfx")</svg:script></svg:svg>',
      "import", "svg:svg root via prefixed namespace; prefixed script must still be removed")
    S("svg-prefixed-nested-default", f'<s:svg xmlns:s="{SVGNS}" xmlns="http://www.w3.org/1999/xhtml" width="32" height="32">'
                                     f'<s:circle cx="16" cy="16" r="9" fill="#e33"/><script>alert(1)</script>'
                                     f'<s:image href="{canary}?mixed"/></s:svg>',
      "import", "prefixed SVG with default namespace XHTML (script in XHTML ns)")
    S("svg-prefix-trick-script", svg_doc(
        f'<x:script xmlns:x="{SVGNS}">window.__pwned=1;fetch("{canary}?x")</x:script>'
        f'<x:image xmlns:x="{SVGNS}" href="{canary}?xi" width="4" height="4"/>'),
      "import", "script/image via a locally declared prefix: filter must match by localName/namespace not literal tag")
    S("svg-wrong-namespace", f'<svg xmlns="http://www.w3.org/1999/xhtml" width="32" height="32"><script>window.__pwned=1</script>'
                             f'<img src="{canary}?wrongns"/></svg>',
      "refuse", "root named svg in the XHTML namespace")
    S("svg-no-namespace", '<svg width="32" height="32"><rect width="32" height="32" fill="red"/></svg>',
      "refuse", "svg root with no namespace")
    S("svg-wrong-root", f'<html xmlns="http://www.w3.org/1999/xhtml"><body><svg xmlns="{SVGNS}" width="32" height="32">'
                        f'<rect width="32" height="32"/></svg><script>window.__pwned=1</script></body></html>',
      "refuse", "XHTML page wrapping an svg: root is not svg")
    S("svg-html-wrapper-not-xml", f'<html><body><svg xmlns="{SVGNS}" width="32" height="32"><rect width="32" height="32"/></svg>'
                                  f'<img src="{canary}?html"></body></html>',
      "refuse", "tag-soup HTML wrapping svg (non-XML)")
    S("svg-svg-in-other-root", f'<doc xmlns="urn:x"><svg xmlns="{SVGNS}" width="32" height="32"/></doc>',
      "refuse", "svg only as a child element")
    S("svg-not-wellformed", f'<svg xmlns="{SVGNS}" width="32" height="32"><rect width="32" height="32"></svg>',
      "refuse", "unclosed element")
    S("svg-two-roots", f'<svg xmlns="{SVGNS}" width="8" height="8"/><svg xmlns="{SVGNS}" width="8" height="8"/>',
      "refuse", "two root elements")
    S("svg-trailing-script-after-root", f'<svg xmlns="{SVGNS}" width="32" height="32"><rect width="32" height="32"/></svg>'
                                        f'<script>alert(1)</script>', "refuse", "junk after root is not well-formed")
    S("svg-nested-svg", svg_doc(
        f'<svg x="4" y="4" width="8" height="8" onload="alert(1)"><image href="{canary}?nested" width="8" height="8"/></svg>'),
      "import", "nested <svg> with handler and external image")
    S("svg-switch-requiredext", svg_doc(
        f'<switch><foreignObject requiredExtensions="http://www.w3.org/1999/xhtml"><iframe xmlns="http://www.w3.org/1999/xhtml" src="{canary}?sw"/></foreignObject>'
        '<text x="2" y="12">fallback</text></switch>'), "import", "switch with foreignObject")

    # encodings / odd framing
    plain = svg_doc("")
    S("svg-utf8-bom", b"\xef\xbb\xbf" + plain.encode(), "import", "UTF-8 BOM")
    S("svg-leading-whitespace", "\n  \t " + plain, "import", "leading whitespace without XML declaration")
    S("svg-leading-comment", "<!-- hi -->" + plain, "import", "comment before root")
    S("svg-xml-decl-leading-space", " <?xml version=\"1.0\"?>" + plain, "either",
      "whitespace before XML declaration is a well-formedness error")
    S("svg-utf16-bom", ('<?xml version="1.0" encoding="UTF-16"?>' + plain).encode("utf-16"), "either",
      "UTF-16 with BOM")
    S("svg-utf16le-nobom", plain.encode("utf-16-le"), "either", "UTF-16LE without BOM")
    S("svg-latin1", ('<?xml version="1.0" encoding="ISO-8859-1"?>' + svg_doc('<text x="1" y="20">caf\xe9</text>')).encode("latin-1"),
      "either", "ISO-8859-1 declared with high bytes")
    S("svg-invalid-utf8", b'<svg xmlns="' + SVGNS.encode() + b'" width="32" height="32"><text>\xff\xfe\xfa</text></svg>',
      "either", "invalid UTF-8 inside text")
    S("svg-nul-bytes", plain.encode() + b"\x00\x00", "either", "trailing NULs")
    S("svg-control-chars", svg_doc('<text>&#1;&#8;</text>'), "either", "numeric references to illegal XML 1.0 control characters")
    S("svg-wrong-mime-png-body", plain, "import", "SVG sent as image/png named .png: magic wins",
      file="really-svg.png", mime="image/png")
    S("svg-xml-prolog-only-then-eof", '<?xml version="1.0"?>', "refuse", "no root element")
    S("svg-mime-text-plain", plain, "import", "SVG with text/plain mime and .txt name", file="vector.txt", mime="text/plain")

    # nesting / size
    depth = 5000
    S("svg-deep-nesting-5000", svg_doc("<g>" * depth + '<rect width="8" height="8" fill="#fff"/>' + "</g>" * depth),
      "either", "5000 nested <g>: must not stack overflow or hang")
    depth2 = 100000
    S("svg-deep-nesting-100k", svg_doc("<g>" * depth2 + "</g>" * depth2), "either",
      "100000 nested <g> (~700 KB): recursion limit test, must not hang or crash")
    S("svg-many-elements", svg_doc("".join(f'<rect x="{i % 32}" y="{(i // 32) % 32}" width="1" height="1" fill="#{(i * 37) % 4096:03x}"/>' for i in range(12000))),
      "either", "12000 small rects (~1 MB-ish): near the size limit, heavy but valid")
    big = svg_doc("<!--" + "x" * 1_100_000 + "-->")
    S("svg-1.1mb", big, "refuse", "SVG text > 1 MB (padding in a comment)")
    S("svg-just-under-1mb", svg_doc("<!--" + "x" * (1_000_000 - 400) + "-->"), "import",
      "just under 1 MB limit (1,048,576): must be accepted")
    S("svg-1mb-attr-bomb", svg_doc('<rect width="8" height="8" fill="#fff" data-x="' + "A" * 900000 + '"/>'),
      "either", "900 KB attribute value")
    S("svg-filter-blur-bomb", svg_doc(
        '<filter id="b" x="-1000%" y="-1000%" width="2100%" height="2100%"><feGaussianBlur stdDeviation="500"/>'
        '<feMorphology radius="200"/><feTurbulence baseFrequency="0.01" numOctaves="10"/></filter>'
        '<rect width="32" height="32" filter="url(#b)"/>'), "either",
      "filter that is expensive to render; 5s timeout / responsiveness test")
    S("svg-pattern-explosion", svg_doc(
        '<pattern id="p" width="0.001" height="0.001" patternUnits="userSpaceOnUse"><rect width="0.0005" height="0.0005"/></pattern>'
        '<rect width="32" height="32" fill="url(#p)" transform="scale(1000)"/>'), "either",
      "microscopic pattern tile: render-time blowup")
    S("svg-huge-stroke-dash", svg_doc(
        '<path d="M0 0L32 32" stroke="#fff" stroke-dasharray="0.0001" stroke-width="2"/>'), "either",
      "dash array so fine it needs ~10^6 segments")

    # sanity: bad image header in svg data uri
    S("svg-image-data-garbage", svg_doc('<image width="8" height="8" href="data:image/png;base64,####notbase64####"/>'),
      "import", "malformed data: URI image")

    return fx


if __name__ == "__main__":
    import time
    t = time.time()
    fs = hostile_fixtures("http://127.0.0.1:1/imp-canary")
    print(len(fs), "fixtures", round(time.time() - t, 2), "s")
