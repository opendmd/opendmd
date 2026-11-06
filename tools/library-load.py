# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45"]
# ///
"""What the Library's folders, thumbnails and the playlist editor cost a
real player: the local build served through `mock-device.py --proxy`
(nothing is written to the device's flash), driven in headless Chromium.

    uv run tools/library-load.py --device http://192.168.1.69 [--out target/library-load.json]

Needs `make web` first. Each run starts with an empty thumbnail store (a
fresh browser), so it measures first visits. It writes one playlist,
`loadtest`, and deletes it again; nothing else on the card changes.

Per scenario it reports, from Chrome's own network events (CDP):
  - card-file requests (/gifs/, /cache/) and the bytes the browser received;
  - the bytes the thumbnails used (js/thumbs.js `stats.bytes`) and what
    each cut-off read brought the browser (`per_cut`; whole reads, such as
    a Details preview's file, are left out: counting them made "tail per
    cut" 9,488 B on a test board). What the device sent after a cut never reaches the browser (the
    proxy stops reading when the browser hangs up, as the browser would):
    read it in the device's log, the "client went away" lines' byte counts;
  - new TCP connections (distinct connection ids; a cut read closes one);
  - 503s (the stream queue full, or the heap low);
and from /api/status, polled every 500 ms throughout: its latency (p50 /
max), the heap floor (`heap.floor`, lowest seen), frames late in play and
at GIF changes (`misses.steady` / `.switch`, the change over the
scenario) and `streams_aborted` (must stay 0: a thumbnail's cut is a
client that went away, not an abort).

Scenarios: Consoles opened (a screen of first frames); the same with a
GIF's Details open (its whole file in the same stream queue); a fling to
the end of Consoles; a playlist made by ticking three GIFs and saved; and
a phone on weak WiFi (Chrome's network emulation: 400 ms latency,
~400 kbit/s down) opening Arcade.
"""
import argparse
import json
import socket
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

REPO = Path(__file__).resolve().parents[1]
IN_VIEW = ("[...document.querySelectorAll('#lib-list .vrow:not([hidden])')].filter(r => { const t = r.getBoundingClientRect().top,"
           " l = document.querySelector('#lib-list').getBoundingClientRect(); return t >= l.top && t < l.bottom - 40; })")
TREE = "document.querySelector('#lib-list')?._tree"


def get_json(url: str, timeout: float = 10):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.loads(r.read())


class Status(threading.Thread):
    """/api/status every 500 ms: latency, heap floor, misses, aborts."""

    def __init__(self, device: str):
        super().__init__(daemon=True)
        self.device, self.stop, self.lock = device, threading.Event(), threading.Lock()
        self.samples: list[tuple[float, dict]] = []

    def run(self):
        while not self.stop.is_set():
            t = time.time()
            try:
                s = get_json(self.device + "/api/status", 8)
                with self.lock:
                    self.samples.append(((time.time() - t) * 1000, s))
            except Exception as e:  # a missed poll is a data point too
                with self.lock:
                    self.samples.append(((time.time() - t) * 1000, {"error": str(e)}))
            self.stop.wait(0.5)

    def mark(self) -> int:
        with self.lock:
            return len(self.samples)

    def since(self, k: int) -> dict:
        with self.lock:
            part = self.samples[max(0, k - 1):]
        ok = [s for _, s in part if "error" not in s]
        lat = sorted(ms for ms, _ in part)
        first, last = (ok[0], ok[-1]) if ok else ({}, {})
        m = lambda s, k: (s.get("misses") or {}).get(k, 0)
        return {
            "status_ms_p50": round(lat[len(lat) // 2]) if lat else None,
            "status_ms_max": round(lat[-1]) if lat else None,
            "status_failed": len(part) - len(ok),
            "heap_floor": min(((s.get("heap") or {}).get("floor") or 1 << 30) for s in ok) if ok else None,
            "heap_free_min": min(((s.get("heap") or {}).get("free") or 1 << 30) for s in ok) if ok else None,
            "misses_steady": m(last, "steady") - m(first, "steady"),
            "misses_switch": m(last, "switch") - m(first, "switch"),
            "streams_aborted": last.get("streams_aborted", 0) - first.get("streams_aborted", 0),
        }


class Net:
    """Chrome's network events for card files on one page."""

    def __init__(self, page):
        self.req: dict[str, dict] = {}
        cdp = page.context.new_cdp_session(page)
        cdp.send("Network.enable")
        self.cdp = cdp
        card = lambda u: "/gifs/" in u or "/cache/" in u
        cdp.on("Network.requestWillBeSent", lambda e: card(e["request"]["url"]) and self.req.setdefault(e["requestId"], {"url": e["request"]["url"], "bytes": 0}))
        cdp.on("Network.responseReceived", lambda e: e["requestId"] in self.req and self.req[e["requestId"]].update(status=e["response"]["status"], conn=e["response"].get("connectionId")))
        cdp.on("Network.dataReceived", lambda e: e["requestId"] in self.req and self.req[e["requestId"]].update(bytes=self.req[e["requestId"]]["bytes"] + e.get("dataLength", 0)))
        cdp.on("Network.loadingFailed", lambda e: e["requestId"] in self.req and self.req[e["requestId"]].update(cut=e.get("canceled", False)))

    def mark(self) -> set:
        return set(self.req)

    def since(self, before: set) -> dict:
        new = [v for k, v in self.req.items() if k not in before]
        return {"requests": len(new), "received": sum(v["bytes"] for v in new),
                "connections": len({v.get("conn") for v in new if v.get("conn") is not None}),
                "r503": sum(v.get("status") == 503 for v in new), "cut": sum(bool(v.get("cut")) for v in new),
                "received_by_cut": sum(v["bytes"] for v in new if v.get("cut"))}


def thumbs(page) -> dict:
    return page.evaluate("import('./js/thumbs.js').then(m => ({ ...m.stats }))")


def drawn(page, timeout=120000):
    page.wait_for_function(f"{IN_VIEW}.length >= 6 && {IN_VIEW}.every(r => r.querySelector('canvas.th[data-done]'))", timeout=timeout)


def folder(page, cat):
    page.evaluate("location.hash = '#library'")
    page.wait_for_selector("#view-library.on")
    page.evaluate(f"location.hash = '#library/' + encodeURIComponent({json.dumps(cat)})")
    page.wait_for_function(f"(t => t && t.open.has({json.dumps(cat)}) && t.rows.some(r => r.path))({TREE})", timeout=120000)


def scenario(name, page, net, st, run) -> dict:
    n0, s0, t0, th0 = net.mark(), st.mark(), time.time(), thumbs(page)
    run()
    page.wait_for_timeout(1500)  # the tail settles, a poll or two lands
    th = thumbs(page)
    out = {"scenario": name, "seconds": round(time.time() - t0, 1), **net.since(n0), **st.since(s0),
           "thumb_bytes_used": th["bytes"] - th0["bytes"], "thumb_r503": th["r503"] - th0["r503"], "thumb_failed": th["failed"] - th0["failed"]}
    # What a cut read brought the browser (the first frame and what came with
    # it; whole reads, a Details preview's file, are left out).
    out["per_cut"] = round(out["received_by_cut"] / out["cut"]) if out["cut"] else None
    print(json.dumps(out))
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--device", required=True, help="the player, e.g. http://192.168.1.69")
    ap.add_argument("--out", default=str(REPO / "target" / "library-load.json"))
    a = ap.parse_args()
    device = a.device.rstrip("/")
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    proc = subprocess.Popen(["uv", "run", "-q", str(REPO / "tools" / "mock-device.py"), "--proxy", device, "--port", str(port), "--quiet"], cwd=REPO)
    base = f"http://127.0.0.1:{port}"
    for _ in range(50):
        try:
            get_json(base + "/api/status", 5)
            break
        except Exception:
            time.sleep(0.3)
    st = Status(device)
    st.start()
    results = []
    try:
        with sync_playwright() as pw:
            b = pw.chromium.launch()
            ctx = b.new_context(viewport={"width": 1280, "height": 800})
            page = ctx.new_page()
            net = Net(page)
            page.goto(base + "/#library")
            page.wait_for_selector("#lib-list", timeout=60000)

            def consoles():
                folder(page, "Consoles")
                drawn(page)
            results.append(scenario("Consoles, first screen", page, net, st, consoles))

            def details():
                folder(page, "Arcade")
                page.click("#lib-list .vrow:not([hidden]) >> nth=0")
                drawn(page)
                page.wait_for_function("document.querySelector('#lib-cap').dataset.shown", timeout=120000)
            results.append(scenario("Arcade with Details open", page, net, st, details))

            def fling():
                folder(page, "Consoles")
                page.evaluate("""async () => { const l = document.querySelector('#lib-list');
                    for (let i = 0; i < 30; i++) { l.scrollTop += 4000; await new Promise((r) => setTimeout(r, 12)); } }""")
                drawn(page)
            results.append(scenario("fling to the end of Consoles", page, net, st, fling))

            def save():
                page.evaluate("location.hash = '#sets'")
                page.click("#sets-new")
                page.click("#set-add")
                page.click("#set-add-list .vdir[data-cat=Arcade]")
                page.wait_for_selector("#set-add-list .vrow:not([hidden])", timeout=60000)
                for i in range(3):
                    page.click(f"#set-add-list .vrow:not([hidden]) >> nth={i} >> .t")
                page.click("#set-add-ticked")
                page.fill("#set-name", "loadtest")
                page.click("#set-save")
                page.wait_for_selector("#sets-list [data-path='/sets/loadtest.txt']", state="attached", timeout=60000)
            results.append(scenario("playlist ticked and saved", page, net, st, save))
            urllib.request.urlopen(urllib.request.Request(base + "/api/files/sets/loadtest.txt", method="DELETE"), timeout=30)
            ctx.close()

            phone = b.new_context(viewport={"width": 390, "height": 844}, device_scale_factor=3, is_mobile=True, has_touch=True)
            pp = phone.new_page()
            pnet = Net(pp)
            pnet.cdp.send("Network.emulateNetworkConditions", {"offline": False, "latency": 400, "downloadThroughput": 50_000, "uploadThroughput": 25_000})
            pp.goto(base + "/#library", timeout=180000)
            pp.wait_for_selector("#lib-list", timeout=180000)

            def weak():
                folder(pp, "Arcade")
                drawn(pp, 300000)
            results.append(scenario("phone, weak WiFi, Arcade", pp, pnet, st, weak))
            b.close()
    finally:
        st.stop.set()
        proc.terminate()
    Path(a.out).parent.mkdir(parents=True, exist_ok=True)
    Path(a.out).write_text(json.dumps(results, indent=1))
    bad = [r["scenario"] for r in results if r["streams_aborted"]]
    print(f"\nwrote {a.out}; streams_aborted {'rose in ' + ', '.join(bad) if bad else 'stayed 0'}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
