# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45"]
# ///
"""Headless-Chromium check of the browser flasher (flasher/index.html).

    uv run tools/flasher-smoke.py            # needs dist/ from `make fw-release`

Lays the site out the way `make flasher` does (target/flasher-smoke/:
index.html, firmware/ with dist/'s manifest and the files it names), serves it on a free localhost port, and checks: ESP
Web Tools loads from jsDelivr (the pinned version) and defines
`<esp-web-install-button>`; the one Install button (the DevKit's, and
nothing for another layout) renders (headless Chromium has `navigator.serial`, so the
"unsupported" slot must not be what shows); the manifest (checked in dist/) parses, carries
its name ("OpenDMD": the firmware's Improv answer), names
six parts at the layout's offsets (0x1000/0x8000/0xF000/0x20000/
0x2A0000/0x300000, table v3; docs/ota.md), each one is
served, and the page lists the DevKit's; no part runs into the next and each set
ends inside its flash; the footer links the third-party notices, which are
served; no console or page errors.

The offline install (docs/deploying.md "Offline installs"): manifest-4mb-offline.json
is the same six parts plus nvs-offline-4mb.bin at the `nvs` partition's
offset (firmware/partitions.csv), seven in flash order, with the erase
question and no Improv wait; the NVS image is the partition's size and
parses as an NVS partition holding namespace `opendmd`, string `network` =
`off` and nothing else; each part is served. On the page, the box "Play from
the card only, no WiFi" (a real label) switches the button's `manifest`
attribute to the offline manifest and back, and its download link saves
`opendmd.json` holding exactly `{"network":"off"}` and a newline.

What it cannot do: the flash itself. Web Serial's port picker needs a
person, and there is no board on the other end of a headless browser.
"""
import functools
import http.server
import json
import os
import shutil
import struct
import sys
import zlib
import threading
from pathlib import Path

from playwright.sync_api import sync_playwright

REPO = Path(__file__).resolve().parents[1]
SITE = REPO / "target" / "flasher-smoke"
DIST = Path(os.environ.get("DIST") or REPO / "dist")
failures = 0
# The third-party notices fw-release puts in dist/, linked from the page's footer.
NOTICES = ("THIRD-PARTY-LICENSES-firmware.txt", "THIRD-PARTY-LICENSES-web.txt")


def check(ok: bool, what: str) -> None:
    global failures
    print(("  ok   " if ok else "  FAIL ") + what)
    failures += not ok


# Per flash layout: the manifest's name, its parts' offsets in the order
# written, and the flash size.
LAYOUTS = {
    "4mb": ("OpenDMD", [0x1000, 0x8000, 0xF000, 0x20000, 0x2A0000, 0x300000], 4 << 20),
}


# The layouts the page offers (the install button, the parts table).
SHOWN = tuple(LAYOUTS)
# Each layout's partition table: where the offline install's NVS image goes.
TABLES = {"4mb": REPO / "firmware" / "partitions.csv"}
# What the offline box's card file holds (flasher/index.html, tools/site.py OFFLINE_FILE).
OFFLINE_FILE = b'{"network":"off"}\n'


def nvs_partition(table: Path) -> tuple[int, int]:
    """The `nvs` partition's offset and size in a partitions CSV."""
    for line in table.read_text().splitlines():
        cols = [c.strip() for c in line.split("#")[0].split(",")]
        if len(cols) >= 5 and cols[0] == "nvs" and cols[2] == "nvs":
            return int(cols[3], 0), int(cols[4], 0)
    raise SystemExit(f"no nvs partition in {table}")


def nvs_entries(img: bytes) -> dict[tuple[str, str], object]:
    """An NVS partition image (ESP-IDF's format, v2 pages) read back:
    (namespace, key) -> value, for the u8 and string entries; raises on a
    bad page header, entry CRC or string CRC."""
    names, raw = {0: ""}, []
    for page in range(0, len(img), 4096):
        p = img[page:page + 4096]
        state, = struct.unpack_from("<I", p)
        if state == 0xFFFFFFFF:
            continue  # empty
        if state not in (0xFFFFFFFE, 0xFFFFFFFC) or p[8] != 0xFE:
            raise ValueError(f"page at {page:#x}: state {state:#x}, version {p[8]:#x}")
        if struct.unpack_from("<I", p, 28)[0] != zlib.crc32(p[4:28], 0xFFFFFFFF):
            raise ValueError(f"page at {page:#x}: header CRC")
        bitmap = p[32:64]
        i = 0
        while i < 126:
            if (bitmap[i // 4] >> (2 * (i % 4))) & 3 != 2:  # not written
                i += 1
                continue
            e = p[64 + 32 * i:64 + 32 * (i + 1)]
            ns, typ, span = e[0], e[1], e[2]
            if struct.unpack_from("<I", e, 4)[0] != zlib.crc32(e[:4] + e[8:32], 0xFFFFFFFF):
                raise ValueError(f"page at {page:#x}, entry {i}: CRC")
            key = e[8:24].split(b"\0")[0].decode()
            if typ == 0x21:  # a string: its bytes in the next span-1 entries
                size, _, crc = struct.unpack_from("<HHI", e, 24)
                data = p[64 + 32 * (i + 1):64 + 32 * (i + 1) + size]
                if zlib.crc32(data, 0xFFFFFFFF) != crc or data[-1:] != b"\0":
                    raise ValueError(f"{key}: string CRC")
                raw.append((ns, key, data[:-1].decode()))
            elif typ == 0x01:  # a u8: a namespace's index when ns is 0
                if ns == 0:
                    names[e[24]] = key
                else:
                    raw.append((ns, key, e[24]))
            else:
                raw.append((ns, key, ("type", typ)))
            i += max(span, 1)
    return {(names.get(ns, f"#{ns}"), key): v for ns, key, v in raw}


def main() -> int:
    manifests = {v: DIST / f"manifest-{v}.json" for v in LAYOUTS}
    offlines = {v: DIST / f"manifest-{v}-offline.json" for v in LAYOUTS}
    if not all(p.exists() for p in (*manifests.values(), *offlines.values())):
        print(f"no {DIST}/manifest-4mb.json and manifest-4mb-offline.json: run make fw-release first")
        return 2
    ms = {v: json.loads(p.read_text()) for v, p in manifests.items()}
    oms = {v: json.loads(p.read_text()) for v, p in offlines.items()}
    shutil.rmtree(SITE, ignore_errors=True)
    (SITE / "firmware").mkdir(parents=True)
    shutil.copy(REPO / "flasher" / "index.html", SITE / "index.html")
    shutil.copy(REPO / "flasher" / "offline.js", SITE / "offline.js")
    # ESP Web Tools, served beside the page (flasher/esp-web-tools/), as `make flasher` does.
    shutil.copytree(REPO / "flasher" / "esp-web-tools", SITE / "esp-web-tools")
    shutil.copy(REPO / "licenses" / "THIRD-PARTY-LICENSES-installer.txt", SITE / "esp-web-tools")
    all_parts: dict[str, list] = {}
    for v, m in ms.items():
        shutil.copy(manifests[v], SITE / "firmware" / manifests[v].name)
        parts = all_parts[v] = m["builds"][0]["parts"]
        for part in parts:
            shutil.copy(DIST / part["path"], SITE / "firmware" / part["path"])
        name, want, flash = LAYOUTS[v]
        offsets = [p["offset"] for p in parts]
        check(m["builds"][0]["chipFamily"] == "ESP32" and offsets == want,
              f"manifest-{v}: ESP32, parts at {[hex(o) for o in offsets]}")
        check(m.get("name") == name and m.get("version") == ms["4mb"]["version"], f"manifest-{v}: {m.get('name')!r} {m.get('version')}")
        check(m.get("new_install_prompt_erase") is True, f"manifest-{v}: the installer asks whether to erase")
        spans = sorted((p["offset"], p["offset"] + (DIST / p["path"]).stat().st_size) for p in parts)
        check(all(e <= n for (_, e), (n, _) in zip(spans, spans[1:] + [(flash, 0)])),
              f"manifest-{v}: no part runs into the next, all inside {flash >> 20} MB (ends {[hex(e) for _, e in spans]})")
        check(any(p["path"] == f"bootloader-{v}.bin" for p in parts) and any(p["path"] == f"partition-table-{v}.bin" for p in parts),
              f"manifest-{v}: its own bootloader and partition table")
        check(isinstance(m.get("new_install_improv_wait_time"), int) and m["new_install_improv_wait_time"] > 0
              and not any("nvs" in p["path"] for p in parts),
              f"manifest-{v}: waits for the board's Improv answer ({m.get('new_install_improv_wait_time')} s), writes no NVS")
        # The offline install: the six parts and the NVS image, in flash order.
        om = oms[v]
        shutil.copy(offlines[v], SITE / "firmware" / offlines[v].name)
        oparts = all_parts[f"{v}-offline"] = om["builds"][0]["parts"]
        for part in oparts:
            shutil.copy(DIST / part["path"], SITE / "firmware" / part["path"])
        nvs_off, nvs_size = nvs_partition(TABLES[v])
        nvs = {"path": f"nvs-offline-{v}.bin", "offset": nvs_off}
        ooffsets = [p["offset"] for p in oparts]
        check(len(om["builds"]) == 1 and om["builds"][0]["chipFamily"] == "ESP32" and len(oparts) == 7
              and oparts[:2] + oparts[3:] == parts and oparts[2] == nvs and ooffsets == sorted(ooffsets),
              f"manifest-{v}-offline: the six parts and {nvs['path']} at {nvs_off:#x}, in flash order ({[hex(o) for o in ooffsets]})")
        check((om.get("name"), om.get("version")) == (m.get("name"), m.get("version")) and om.get("new_install_prompt_erase") is True
              and om.get("new_install_improv_wait_time") == 0,
              f"manifest-{v}-offline: {om.get('name')!r} {om.get('version')}, asks whether to erase, no Improv wait "
              f"({om.get('new_install_improv_wait_time')!r})")
        img = (DIST / nvs["path"]).read_bytes()
        try:
            entries = nvs_entries(img)
        except ValueError as e:
            entries = {"error": str(e)}
        check(len(img) == nvs_size and entries == {("opendmd", "network"): "off"},
              f"{nvs['path']}: {len(img):,} B (the nvs partition is {nvs_size:,} B), an NVS image holding {entries}")
    for name in NOTICES:
        shutil.copy(DIST / name, SITE / "firmware" / name)

    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args) -> None:
            pass

    handler = functools.partial(Quiet, directory=str(SITE))
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_port}"
    # localhost, not 127.0.0.1: both are secure contexts, localhost is what
    # `make flasher` prints.
    base = base.replace("127.0.0.1", "localhost")
    errors: list[str] = []
    with sync_playwright() as pw:
        browser = pw.chromium.launch(headless=True)
        page = browser.new_page(viewport={"width": 1280, "height": 900}, accept_downloads=True)
        page.on("console", lambda msg: msg.type == "error" and errors.append(msg.text))
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.on("response", lambda r: r.status >= 400 and errors.append(f"HTTP {r.status} {r.url}"))
        loaded: list[str] = []
        page.on("request", lambda r: loaded.append(r.url))
        page.goto(base + "/")
        page.wait_for_function("customElements.get('esp-web-install-button') !== undefined", timeout=30000)
        check(True, "ESP Web Tools 10.4.0 loaded from the page's own esp-web-tools/, <esp-web-install-button> defined")
        check(not any("jsdelivr" in u for u in loaded), f"nothing loaded from a CDN ({[u for u in loaded if 'jsdelivr' in u][:2]})")
        check(page.evaluate("'serial' in navigator && window.isSecureContext"), "Web Serial present, secure context")
        page.wait_for_timeout(1500)
        for v in SHOWN:
            m = ms[v]
            visible = page.locator(f"#install-activate-{v}").is_visible()
            unsupported = page.evaluate(f"document.querySelector('#install-{v}').hasAttribute('install-unsupported')")
            manifest_attr = page.get_attribute(f"#install-{v}", "manifest")
            label = page.inner_text(f"#install-activate-{v}") if visible else ""
            check(visible and not unsupported and manifest_attr == f"firmware/manifest-{v}.json",
                  f"the {v} Install button renders ({label!r}, {manifest_attr}; visible {visible}, unsupported {unsupported})")
            page.wait_for_function(f"document.querySelectorAll('#parts-{v} tr').length === 6", timeout=10000)
            rows = page.eval_on_selector_all(f"#parts-{v} tr", "rs => rs.map(r => r.innerText.replace(/\\s+/g, ' ').trim())")
            last = hex(LAYOUTS[v][1][-1]).upper().replace("0X", "0x")
            check(len(rows) == 6 and rows[-1].endswith(last), f"the page lists the {v} parts: {rows}")
            check(page.inner_text(f"#version-{v}") == f"{m['name']} {m['version']}", f"the page shows {page.inner_text(f'#version-{v}')!r}")
            for part in {p["path"]: p for p in all_parts[v] + all_parts[f"{v}-offline"]}.values():
                r = page.request.get(f"{base}/firmware/{part['path']}")
                check(r.status == 200 and len(r.body()) == (DIST / part["path"]).stat().st_size,
                      f"served: firmware/{part['path']} ({len(r.body()):,} B)")
            r = page.request.get(f"{base}/firmware/manifest-{v}-offline.json")
            check(r.status == 200 and r.json() == oms[v], f"served: firmware/manifest-{v}-offline.json")
            # The box swaps the button's manifest, and back (a real label: found by its words).
            box = page.get_by_label("Play from the card only, no WiFi")
            check(box.count() == 1 and page.get_attribute(f"#offline-{v}", "data-offline-for") == f"install-{v}"
                  and not box.is_checked(), f"the {v} offline box: one, labelled, unticked")
            box.check()
            on = page.get_attribute(f"#install-{v}", "manifest")
            box.uncheck()
            off = page.get_attribute(f"#install-{v}", "manifest")
            check(on == f"firmware/manifest-{v}-offline.json" and off == f"firmware/manifest-{v}.json",
                  f"the {v} offline box: ticked the button reads {on}, unticked {off}")
            about = page.inner_text(f"#offline-about-{v}")
            check(page.get_attribute(f"#offline-{v}", "aria-describedby") == f"offline-about-{v}"
                  and all(w in about for w in ("erases", "Connect to Wi-Fi", "unticked", "opendmd.json", '"on"'))
                  and not any(w in about for w in ("NVS", "manifest", "partition", "player")),
                  "the offline box says what it does and how to undo it, in plain words")
            with page.expect_download() as dl:
                page.click(f"#offline-file-{v}")
            got = Path(dl.value.path()).read_bytes()
            check(dl.value.suggested_filename == "opendmd.json" and got == OFFLINE_FILE,
                  f"the card file downloads as {dl.value.suggested_filename}: {got!r}")
        check(page.inner_text("#board-4mb h2") == "ESP32 DevKit V1" and page.locator(".board").count() == 1
              and page.locator("esp-web-install-button").count() == 1,
              "one choice, named for the DevKit")
        shown = page.inner_text("main") + page.evaluate("[...document.querySelectorAll('[manifest],[id]')].map(e => e.id + (e.getAttribute('manifest') || '')).join(' ')")
        check("8mb" not in shown and "8 MB" not in shown and "Not sure which" not in shown,
              "the page offers no 8 MB build")
        hrefs = page.eval_on_selector_all("footer a", "as => as.map(a => a.getAttribute('href'))")
        for name in NOTICES:
            r = page.request.get(f"{base}/firmware/{name}")
            check(f"firmware/{name}" in hrefs and r.status == 200 and b"Third-party licences" in r.body(),
                  f"the footer links firmware/{name}, served ({len(r.body()):,} B)")
        text = page.inner_text("main")
        check("Erase device" in text and "Chrome, Edge" in text and "NVS" in text
              and "Change Wi-Fi" in text,
              "the page explains the browser, the erase, what it takes and the WiFi step")
        page.screenshot(path=str(SITE / "flasher.png"), full_page=True)
        browser.close()
    srv.shutdown()
    check(not errors, f"zero console/page errors {errors}")
    print(f"\n{'FAILED: ' + str(failures) if failures else 'all checks passed'} (screenshot: {SITE / 'flasher.png'})")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
