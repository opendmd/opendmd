# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45"]
# ///
"""Drive ejs.html in Playwright Chromium: load a local ROM via the file picker,
time boot, grab frames (raw framebuffer tap / canvas / RetroArch screenshot),
test fast-forward, pause and save states, and count bytes downloaded.

uv run grab.py --rom ROM --core gambatte [--data local|cdn] [--host 127.0.0.1]
               [--coi] [--threads] [--hidden] [--pdb] [--tag name] [--headed]
Vendored EmulatorJS data dir defaults to $EJS_DATA (see README.txt).
"""
import urllib.parse, argparse, base64, functools, json, os, threading, time
from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler
from pathlib import Path
from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
# Where the vendored emulator files were fetched to (a scratch dir outside the repo).
SCRATCH = os.environ.get("SCRATCH_DIR", "/tmp/scratch")
EJS_DATA = Path(os.environ.get("EJS_DATA", f"{SCRATCH}/vendor/ejs-data"))
BINJGB = Path(os.environ.get("BINJGB_DIR", f"{SCRATCH}/vendor/binjgb"))


def make_handler(coi):
    class H(SimpleHTTPRequestHandler):
        def translate_path(self, path):
            p = path.split("?", 1)[0].split("#", 1)[0]
            if p.startswith("/ejs/"):
                return str(EJS_DATA / p[len("/ejs/"):])
            if p.startswith("/binjgb/"):
                return str(BINJGB / p[len("/binjgb/"):])
            return str(HERE / p.lstrip("/"))

        def end_headers(self):
            if coi:
                self.send_header("Cross-Origin-Opener-Policy", "same-origin")
                self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
                self.send_header("Cross-Origin-Resource-Policy", "cross-origin")
            self.send_header("Cache-Control", "no-store")
            super().end_headers()

        def log_message(self, *a):
            pass
    return H


def save_png(data_url, path):
    path.write_bytes(base64.b64decode(data_url.split(",", 1)[1]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rom", required=True)
    ap.add_argument("--core", default="gambatte")
    ap.add_argument("--data", default="local", choices=["local", "cdn"])
    ap.add_argument("--host", default="127.0.0.1", help="bind+navigate host; use a LAN IP to test non-secure context")
    ap.add_argument("--port", type=int, default=8791)
    ap.add_argument("--coi", action="store_true", help="send COOP/COEP headers")
    ap.add_argument("--threads", action="store_true")
    ap.add_argument("--hidden", action="store_true", help="move the emulator off-screen")
    ap.add_argument("--pdb", action="store_true", help="force preserveDrawingBuffer")
    ap.add_argument("--engine", default="ejs", choices=["ejs", "binjgb"])
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--racfg", default="", help="extra retroarch.cfg lines, ';'-separated, e.g. 'audio_sync = false'")
    ap.add_argument("--suspendaudio", action="store_true", help="simulate autoplay-blocked AudioContext")
    ap.add_argument("--start", type=int, default=60, help="first capture at this emulated frame")
    ap.add_argument("--step", type=int, default=60, help="emulated frames between captures")
    ap.add_argument("--n", type=int, default=10)
    ap.add_argument("--tag", default=None)
    ap.add_argument("--no-extras", action="store_true", help="skip ff/pause/state tests")
    a = ap.parse_args()
    tag = a.tag or a.core
    out = HERE / "out"
    out.mkdir(exist_ok=True)

    srv = ThreadingHTTPServer((a.host, a.port), make_handler(a.coi))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    q = f"core={a.core}&data={a.data}" + ("&threads=1" if a.threads else "") + ("&hidden=1" if a.hidden else "") + ("&pdb=1" if a.pdb else "") + ("&suspendaudio=1" if a.suspendaudio else "") + (("&racfg=" + urllib.parse.quote(a.racfg)) if a.racfg else "")
    url = f"http://{a.host}:{a.port}/ejs.html?{q}"
    R = {"url": url, "rom": Path(a.rom).name}
    if a.engine == "binjgb":
        return run_binjgb(a, srv, tag, out)

    with sync_playwright() as p:
        b = p.chromium.launch(headless=not a.headed,
                              args=["--autoplay-policy=user-gesture-required", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"])
        ctx = b.new_context(viewport={"width": 800, "height": 700})
        page = ctx.new_page()
        reqs = []
        page.on("requestfinished", lambda r: reqs.append(r))
        page.on("console", lambda m: m.type in ("error", "warning") and print("  console", m.type, m.text[:200]))
        page.on("pageerror", lambda e: print("  pageerror", e))
        page.goto(url)
        R["env"] = page.evaluate("({secure: grab.secure, sab: grab.sab, coi: grab.coi})")
        t_wall = time.time()
        page.set_input_files("#rom", a.rom)   # same path as a user picking a file: no upload
        try:
            page.wait_for_function("grab.tFirstUpload > 0 && window.EJS_emulator && EJS_emulator.gameManager", timeout=60000)
        except Exception:
            print("STALLED before first frame:", page.evaluate("({uploads: grab.uploads, tStart: grab.tStart, gm: !!(window.EJS_emulator && EJS_emulator.gameManager), audio: grab.audioState(), text: document.body.innerText.slice(-300)})"))
            raise SystemExit(2)
        R["ms_file_to_game_start"] = page.evaluate("grab.tStart - grab.t0")
        R["ms_file_to_first_frame"] = page.evaluate("grab.tFirstUpload - grab.t0")
        R["glType"] = page.evaluate("grab.glType")
        R["audio_ctx_state"] = page.evaluate("grab.audioState()")
        R["dims"] = page.evaluate("grab.dims()")
        R["upload_sigs"] = page.evaluate("grab.uploadSig")

        caps = []
        for i in range(a.n):
            target = a.start + i * a.step
            page.wait_for_function(f"grab.frame() >= {target}", timeout=120000, polling=5)
            fr = page.evaluate("grab.frame()")
            c = page.evaluate("grab.captureRaw()")
            save_png(c.pop("png"), out / f"{tag}_{i:02d}_f{fr}.png")
            c["frame"] = fr
            caps.append(c)
        R["raw_captures"] = caps
        R["raw_ms_avg"] = sum(c["ms"] for c in caps) / len(caps)
        R["raw_copy_ms_avg"] = sum(c["copyMs"] for c in caps) / len(caps)

        # Compare with the other two capture paths (one each, saved for inspection)
        cv = page.evaluate("grab.captureCanvas()")
        save_png(cv.pop("png"), out / f"{tag}_canvas.png")
        R["canvas_capture"] = cv
        ra = page.evaluate("grab.captureRetroarch()")
        save_png(ra.pop("png"), out / f"{tag}_retroarch.png")
        R["retroarch_capture"] = ra
        # a few more timings of canvas + retroarch paths
        R["canvas_ms_x5"] = [page.evaluate("grab.captureCanvas().ms") for _ in range(5)]
        R["retroarch_ms_x3"] = [page.evaluate("grab.captureRetroarch().then(r => r.ms)") for _ in range(3)]

        if not a.no_extras:
            def fps(sec=2.0):
                f0 = page.evaluate("grab.frame()"); t0 = time.time(); time.sleep(sec)
                return (page.evaluate("grab.frame()") - f0) / (time.time() - t0)
            R["fps_normal"] = fps()
            page.evaluate("EJS_emulator.gameManager.setFastForwardRatio(0); EJS_emulator.gameManager.toggleFastForward(1)")
            R["fps_fastforward_unlimited"] = fps()
            page.evaluate("EJS_emulator.gameManager.toggleFastForward(0)")
            page.evaluate("EJS_emulator.gameManager.toggleMainLoop(0)")
            f0 = page.evaluate("grab.frame()"); time.sleep(1); R["frames_advanced_while_paused"] = page.evaluate("grab.frame()") - f0
            page.evaluate("EJS_emulator.gameManager.toggleMainLoop(1)")
            # save state, run on, load it twice; after each load wait 10 emulated frames and grab.
            # Deterministic core + working loadState => both grabs identical and equal to the grab
            # taken 10 frames after the save.
            gm = "EJS_emulator.gameManager"
            def grab_after(n=10):
                f = page.evaluate("grab.frame()")
                page.wait_for_function(f"grab.frame() >= {f + n}", polling=1)
                return page.evaluate("grab.captureRaw()")
            t = time.time(); n = page.evaluate(f"(window._st = {gm}.getState()).length")
            R["state_bytes"] = n; R["state_save_ms"] = (time.time() - t) * 1000
            before = grab_after()
            time.sleep(1.5)
            t = time.time(); page.evaluate(f"{gm}.loadState(window._st)"); R["state_load_ms"] = (time.time() - t) * 1000
            after = grab_after()
            time.sleep(1.0)
            page.evaluate(f"{gm}.loadState(window._st)")
            after2 = grab_after()
            R["state_restore_png_equal"] = before["png"] == after["png"]
            R["state_two_loads_equal"] = after["png"] == after2["png"]
            if not R["state_two_loads_equal"]:
                for nm, c in (("before", before), ("after", after), ("after2", after2)):
                    save_png(c["png"], out / f"{tag}_state_{nm}.png")
            R["audio_ctx_state_end"] = page.evaluate("grab.audioState()")

        by_host = {}
        total = 0
        for r in reqs:
            try:
                s = r.sizes()["responseBodySize"]
            except Exception:
                s = 0
            host = r.url.split("/")[2]
            by_host[host] = by_host.get(host, 0) + s
            total += s
        R["bytes_total"] = total
        R["bytes_by_host"] = by_host
        R["requests"] = sorted(((r.url.replace(f"http://{a.host}:{a.port}", ""), r.sizes()["responseBodySize"]) for r in reqs), key=lambda x: -x[1])[:12]
        b.close()
    srv.shutdown()
    (out / f"{tag}_report.json").write_text(json.dumps(R, indent=1))
    print(json.dumps({k: v for k, v in R.items() if k not in ("raw_captures",)}, indent=1))
    print("raw captures:", [(c["frame"], c["w"], c["h"], round(c["ms"], 1)) for c in R["raw_captures"]])


def run_binjgb(a, srv, tag, out):
    url = f"http://{a.host}:{a.port}/binjgb.html"
    R = {"url": url, "rom": Path(a.rom).name}
    with sync_playwright() as p:
        b = p.chromium.launch(headless=not a.headed)
        page = b.new_page()
        reqs = []
        page.on("requestfinished", lambda r: reqs.append(r))
        page.on("pageerror", lambda e: print("  pageerror", e))
        page.goto(url)
        page.set_input_files("#rom", a.rom)
        page.wait_for_function("bj.ready", timeout=30000)
        R["ms_module_ready"] = page.evaluate("bj.tModule")
        R["ms_file_to_first_frame"] = page.evaluate("bj.tFirstFrame")
        caps = []
        done = 1
        for i in range(a.n):
            target = a.start + i * a.step
            t = time.time(); page.evaluate(f"bj.runFrames({target - done})"); run_ms = (time.time() - t) * 1000
            done = target
            c = page.evaluate("bj.capture()")
            save_png(c.pop("png"), out / f"{tag}_{i:02d}_f{target}.png")
            c.update(frame=target, run_ms=run_ms); caps.append(c)
        R["captures"] = caps
        R["capture_ms_avg"] = sum(c["ms"] for c in caps) / len(caps)
        t = time.time(); page.evaluate("bj.runFrames(3600)"); R["fps_unthrottled"] = 3600 / (time.time() - t)
        st = page.evaluate("(window._st = bj.saveState()).length")
        page.evaluate("bj.runFrames(10)"); a1 = page.evaluate("bj.capture().png")
        page.evaluate("bj.runFrames(500)"); page.evaluate("bj.loadState(window._st)")
        page.evaluate("bj.runFrames(10)"); a2 = page.evaluate("bj.capture().png")
        R["state_bytes"] = st; R["state_restore_png_equal"] = a1 == a2
        R["bytes_total"] = sum(r.sizes()["responseBodySize"] for r in reqs)
        R["requests"] = [(r.url.split("/", 3)[-1], r.sizes()["responseBodySize"]) for r in reqs]
        b.close()
    srv.shutdown()
    (out / f"{tag}_report.json").write_text(json.dumps(R, indent=1))
    print(json.dumps({k: v for k, v in R.items() if k != "captures"}, indent=1))
    print("captures:", [(c["frame"], c["w"], c["h"], round(c["ms"], 2), round(c["run_ms"], 1)) for c in R["captures"]])


if __name__ == "__main__":
    main()
