# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright>=1.45"]
# ///
"""
CORS probe for OpenDMD art-sourcing candidates.

Serves a tiny static page from http://127.0.0.1:<port>/ (so the test page has a
real http origin, like the ESP32 device does) and, from *inside* that page,
for each candidate URL:
  - does fetch(url, {mode:'cors'}) and reports ok/status/error + the ACAO
    header as seen by JS (resp.headers.get(...))
  - for "image" kind candidates: loads <img crossOrigin="anonymous">, draws to
    canvas, tries getImageData -> readable yes/no
  - for "image" kind candidates: loads plain <img> (no crossOrigin), draws to
    canvas, tries getImageData -> confirms tainting (SecurityError) or not
Also listens on page.on("response") to record the real network-level
Access-Control-Allow-Origin header (or its absence) for every request the
page makes, keyed by a per-test cache-busting query param, so we can see the
header even when the JS-level read is blocked.

Usage:
    uv run cors_probe.py
Writes cors_results.json next to this script and prints a markdown table.
"""
import asyncio
import http.server
import json
import random
import string
import threading
from pathlib import Path

from playwright.async_api import async_playwright

HERE = Path(__file__).parent
RESULTS_PATH = HERE / "cors_results.json"

INDEX_HTML = b"""<!doctype html><html><head><title>cors probe host</title></head>
<body><h1>OpenDMD CORS probe host page</h1><p>Real http origin for testing.</p></body></html>"""


def start_static_server():
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.end_headers()
            self.wfile.write(INDEX_HTML)

        def log_message(self, *a):
            pass

    httpd = http.server.HTTPServer(("127.0.0.1", 0), Handler)
    port = httpd.server_address[1]
    t = threading.Thread(target=httpd.serve_forever, daemon=True)
    t.start()
    return httpd, port


# ---- candidate URLs (real, checked working as of 2026-09-23) ----------------
CANDIDATES = [
    dict(source="Wikimedia Commons", kind="api",
         url="https://commons.wikimedia.org/w/api.php?action=query&titles=File:Sonic%20the%20Hedgehog%20logo.svg&prop=imageinfo&iiprop=url&format=json&origin=*",
         note="MediaWiki Action API WITH origin=*"),
    dict(source="Wikimedia Commons", kind="api",
         url="https://commons.wikimedia.org/w/api.php?action=query&titles=File:Sonic%20the%20Hedgehog%20logo.svg&prop=imageinfo&iiprop=url&format=json",
         note="MediaWiki Action API WITHOUT origin="),
    dict(source="Wikimedia Commons", kind="image",
         url="https://thumb.wikimedia.org/wikipedia/commons/thumb/0/09/Sonic_the_Hedgehog_logo.svg/330px-Sonic_the_Hedgehog_logo.svg.png",
         note="thumb.wikimedia.org PNG thumbnail of Sonic logo SVG"),
    dict(source="Wikimedia Commons", kind="image",
         url="https://upload.wikimedia.org/wikipedia/commons/0/09/Sonic_the_Hedgehog_logo.svg",
         note="upload.wikimedia.org original SVG file"),
    dict(source="Wikipedia", kind="api",
         url="https://en.wikipedia.org/w/api.php?action=query&titles=Sonic%20the%20Hedgehog%20(character)&prop=pageimages&format=json&pithumbsize=300&origin=*",
         note="Action API with origin=*"),
    dict(source="Wikipedia", kind="api",
         url="https://en.wikipedia.org/api/rest_v1/page/summary/Sonic_the_Hedgehog_(character)",
         note="REST API page summary (returns thumbnail url)"),
    dict(source="OpenGameArt", kind="image",
         url="https://opengameart.org/sites/default/files/styles/thumbnail/public/sonic_asalga.png",
         note="asset file from opengameart.org/content/sonic"),
    dict(source="Lospec", kind="api",
         url="https://lospec.com/palette-list/sweetie-16.json",
         note="palette JSON API"),
    dict(source="Lospec", kind="image",
         url="https://cdn.lospec.com/palette-examples/sweetie-16-palette-example-palette-example-by-grafxkid-58ffe3.png",
         note="gallery example image on cdn.lospec.com"),
    dict(source="The Spriters Resource", kind="image",
         url="https://www.spriters-resource.com/media/assets/20/21628.png?updated=1755472540",
         note="Sonic 1 (Genesis) sheet, actual CDN file URL"),
    dict(source="Tenor", kind="api",
         url="https://tenor.googleapis.com/v2/search?q=sonic",
         note="v2 search with NO key"),
    dict(source="Tenor", kind="api",
         url="https://tenor.googleapis.com/v2/search?q=sonic&key=LIVDSRZULELA&client_key=opendmd_test",
         note="v2 search with published demo key (LIVDSRZULELA)"),
    dict(source="Giphy", kind="api",
         url="https://api.giphy.com/v1/gifs/search?q=sonic",
         note="search with NO key"),
    dict(source="Giphy", kind="api",
         url="https://api.giphy.com/v1/gifs/search?q=sonic&api_key=dc6zaTOxFJmzC",
         note="search with public beta key (dc6zaTOxFJmzC)"),
    dict(source="Giphy", kind="image",
         url="https://media.giphy.com/media/XJtM2nNFCzT3etvzOB/200.gif",
         note="media CDN gif file (no key needed)"),
    dict(source="IGDB", kind="api",
         url="https://api.igdb.com/v4/games",
         note="no Client-ID / bearer token"),
    dict(source="IGDB", kind="image",
         url="https://images.igdb.com/igdb/image/upload/t_cover_big/co1r7f.jpg",
         note="images.igdb.com cover image (no auth needed for images)"),
    dict(source="TheGamesDB", kind="api",
         url="https://api.thegamesdb.net/v1/Games/ByGameName?apikey=test&name=sonic",
         note="bad/placeholder apikey"),
    dict(source="TheGamesDB", kind="image",
         url="https://cdn.thegamesdb.net/images/original/boxart/front/121-1.jpg",
         note="cdn.thegamesdb.net box art"),
    dict(source="ScreenScraper", kind="api",
         url="https://www.screenscraper.fr/api2/jeuInfos.php?output=json",
         note="no devid/devpassword"),
    dict(source="RAWG", kind="api",
         url="https://api.rawg.io/api/games?search=sonic",
         note="no key"),
    dict(source="RAWG", kind="image",
         url="https://media.rawg.io/media/games/bbf/bbf8d74ab64440ad76294cff2f4d9cfa.jpg",
         note="media.rawg.io game image (Sonic Mania)"),
    dict(source="itch.io", kind="image",
         url="https://img.itch.zone/aW1nLzEwNDg4OTQxLnBuZw==/original/VIWm9w.png",
         note="img.itch.zone cover image"),
    dict(source="SteamGridDB", kind="api",
         url="https://www.steamgriddb.com/api/v2/search/autocomplete/sonic",
         note="no bearer key"),
    dict(source="SteamGridDB", kind="image",
         url="https://cdn2.steamgriddb.com/logo_thumb/6f1b6f1fc5764effbc0537c6324397cc.png",
         note="cdn2.steamgriddb.com logo (Mario & Sonic)"),
    dict(source="GitHub", kind="image",
         url="https://raw.githubusercontent.com/libretro-thumbnails/Sega_-_Mega_Drive_-_Genesis/master/Named_Titles/Sonic%20The%20Hedgehog%203%20(USA).png",
         note="raw.githubusercontent.com file (also = libretro-thumbnails via GitHub)"),
    dict(source="GitHub", kind="api",
         url="https://api.github.com/repos/libretro-thumbnails/Sega_-_Mega_Drive_-_Genesis/contents/Named_Titles",
         note="api.github.com contents listing, unauthenticated"),
    dict(source="libretro-thumbnails", kind="image",
         url="https://thumbnails.libretro.com/Sega%20-%20Mega%20Drive%20-%20Genesis/Named_Titles/Sonic%20The%20Hedgehog%203%20(USA).png",
         note="thumbnails.libretro.com CDN mirror"),
    dict(source="archive.org", kind="api",
         url="https://archive.org/metadata/SonicTheHedgehogSegaGenesisHiResScans",
         note="metadata endpoint"),
    dict(source="archive.org", kind="image",
         url="https://archive.org/download/SonicTheHedgehogSegaGenesisHiResScans/thumbnail.png",
         note="download endpoint (302s to a CDN node)"),
    dict(source="images.weserv.nl", kind="image",
         url="https://images.weserv.nl/?url=upload.wikimedia.org/wikipedia/commons/0/09/Sonic_the_Hedgehog_logo.svg&w=200",
         note="public CORS-adding image proxy, re-serving the Wikimedia SVG"),
]

FETCH_JS = """
async (url) => {
  const busted = url + (url.includes('?') ? '&' : '?') + '_cbf=' + MARKER;
  try {
    const resp = await fetch(busted, {mode: 'cors', cache: 'no-store'});
    let acao = null;
    try { acao = resp.headers.get('access-control-allow-origin'); } catch (e) {}
    let bodyOk = true;
    try { await resp.clone().text(); } catch (e) { bodyOk = false; }
    return {ok: resp.ok, status: resp.status, resp_type: resp.type, acao_from_js: acao, error: null, body_readable: bodyOk};
  } catch (e) {
    return {ok: false, status: null, resp_type: null, acao_from_js: null, error: String(e), body_readable: false};
  }
}
"""

IMG_CORS_JS = """
(url) => {
  const busted = url + (url.includes('?') ? '&' : '?') + '_cbc=' + MARKER;
  return new Promise((resolve) => {
    const img = new Image();
    img.crossOrigin = 'anonymous';
    const timer = setTimeout(() => resolve({readable: null, note: 'timeout'}), 12000);
    img.onload = () => {
      clearTimeout(timer);
      try {
        const c = document.createElement('canvas');
        c.width = img.naturalWidth || 1; c.height = img.naturalHeight || 1;
        const ctx = c.getContext('2d');
        ctx.drawImage(img, 0, 0);
        ctx.getImageData(0, 0, 1, 1);
        resolve({readable: true, note: null});
      } catch (e) {
        resolve({readable: false, note: String(e)});
      }
    };
    img.onerror = () => { clearTimeout(timer); resolve({readable: false, note: 'img load error (blocked or 404)'}); };
    img.src = busted;
  });
}
"""

IMG_PLAIN_JS = """
(url) => {
  const busted = url + (url.includes('?') ? '&' : '?') + '_cbp=' + MARKER;
  return new Promise((resolve) => {
    const img = new Image();
    const timer = setTimeout(() => resolve({tainted: null, note: 'timeout'}), 12000);
    img.onload = () => {
      clearTimeout(timer);
      try {
        const c = document.createElement('canvas');
        c.width = img.naturalWidth || 1; c.height = img.naturalHeight || 1;
        const ctx = c.getContext('2d');
        ctx.drawImage(img, 0, 0);
        ctx.getImageData(0, 0, 1, 1);
        resolve({tainted: false, note: 'getImageData succeeded (NOT tainted!)'});
      } catch (e) {
        resolve({tainted: true, note: String(e)});
      }
    };
    img.onerror = () => { clearTimeout(timer); resolve({tainted: null, note: 'img load error'}); };
    img.src = busted;
  });
}
"""


def marker():
    return "".join(random.choices(string.ascii_lowercase + string.digits, k=10))


async def main():
    httpd, port = start_static_server()
    base = f"http://127.0.0.1:{port}/"
    print(f"[+] static host page at {base}")

    results = []
    async with async_playwright() as p:
        browser = await p.chromium.launch()
        page = await browser.new_page()

        network_log = []  # list of dicts: url, status, headers

        def on_response(resp):
            try:
                network_log.append({
                    "url": resp.url,
                    "status": resp.status,
                })
                # headers fetched async below via request; store handle
            except Exception:
                pass

        async def on_response_async(resp):
            try:
                headers = await resp.all_headers()
                network_log.append({"url": resp.url, "status": resp.status, "headers": headers})
            except Exception as e:
                network_log.append({"url": resp.url, "status": None, "headers": {}, "err": str(e)})

        page.on("response", lambda resp: asyncio.create_task(on_response_async(resp)))

        await page.goto(base)

        for cand in CANDIDATES:
            entry = dict(cand)
            m = marker()
            fetch_js = FETCH_JS.replace("MARKER", json.dumps(m))
            try:
                fetch_result = await page.evaluate(fetch_js, cand["url"])
            except Exception as e:
                fetch_result = {"ok": False, "status": None, "error": f"evaluate failed: {e}"}
            entry["fetch"] = fetch_result

            if cand["kind"] == "image":
                mc = marker()
                mp = marker()
                img_cors_js = IMG_CORS_JS.replace("MARKER", json.dumps(mc))
                img_plain_js = IMG_PLAIN_JS.replace("MARKER", json.dumps(mp))
                try:
                    entry["img_crossorigin"] = await page.evaluate(img_cors_js, cand["url"])
                except Exception as e:
                    entry["img_crossorigin"] = {"readable": None, "note": f"evaluate failed: {e}"}
                try:
                    entry["img_plain"] = await page.evaluate(img_plain_js, cand["url"])
                except Exception as e:
                    entry["img_plain"] = {"tainted": None, "note": f"evaluate failed: {e}"}
                await asyncio.sleep(0.05)
                # find network responses matching this test's markers
                entry["network_fetch"] = [r for r in network_log if f"_cbf={m}" in r["url"]]
                entry["network_img_cors"] = [r for r in network_log if f"_cbc={mc}" in r["url"]]
                entry["network_img_plain"] = [r for r in network_log if f"_cbp={mp}" in r["url"]]
            else:
                await asyncio.sleep(0.05)
                entry["network_fetch"] = [r for r in network_log if f"_cbf={m}" in r["url"]]

            results.append(entry)
            print(f"  probed [{cand['source']:22s}] {cand['kind']:5s} -> "
                  f"fetch:{entry['fetch'].get('status')}/{entry['fetch'].get('error')}")

        await browser.close()
    httpd.shutdown()

    RESULTS_PATH.write_text(json.dumps(results, indent=2))
    print(f"\n[+] wrote {RESULTS_PATH}")
    print_markdown(results)


def acao_for(entry, netkey):
    for r in entry.get(netkey, []):
        headers = r.get("headers") or {}
        val = headers.get("access-control-allow-origin")
        if val is not None:
            return val
    return None


def print_markdown(results):
    print()
    print("| source | kind | fetch result | ACAO header | img+canvas readable | notes |")
    print("|---|---|---|---|---|---|")
    for e in results:
        fr = e["fetch"]
        if fr.get("error"):
            fetch_str = f"blocked: {fr['error'][:60]}"
        else:
            fetch_str = f"{fr.get('status')} {'ok' if fr.get('ok') else 'not-ok'}"
        acao = acao_for(e, "network_fetch") or fr.get("acao_from_js") or "(none)"
        if e["kind"] == "image":
            ic = e.get("img_crossorigin", {})
            ip = e.get("img_plain", {})
            readable = f"cors-img: {ic.get('readable')}; plain-img tainted: {ip.get('tainted')}"
        else:
            readable = "n/a (api)"
        note = e.get("note", "")
        print(f"| {e['source']} | {e['kind']} | {fetch_str} | {acao} | {readable} | {note} |")


if __name__ == "__main__":
    asyncio.run(main())
