# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl==1.5.0", "aiohttp==3.12.15", "pillow"]
# ///
"""Device test D4 (P1-PLAN 8.8): GIF sends with real card writes, on a
panel running a Connect build, through a Connect server. **Run only with
the owner's OK, on the spare first** (then the rig, the owner told): it
pairs a controller on the panel and writes files to its card
(`/gifs/Sent/d4-*.gif`, removed afterwards with --clean).

    uv run tools/connect/d4_send.py --lan http://<panel> --server https://connect.opendmd.dev [--runs 3] [--size-kb 200] [--copy]
    uv run tools/connect/d4_send.py --lan http://<panel> --server ... --clean   # remove the d4 files and the controller

It pairs once over the LAN link (`POST /api/connect/pair`, then the
pairing as a guest at the server; the controller's seed is kept in
~/.config/opendmd/d4-controller.seed, never printed) and keeps the key for
later runs. Each run sends a generated noise GIF (nothing third-party; with
--copy, the panel's own prepared copy of an earlier run's GIF is fetched
and sent along) and, while it goes, reads the panel's `/api/status` each
second over the LAN: the free heap and its floor, late frames (`misses`),
`transfer`. The report per run: KB/s, credits, the last credit (`done` or `err`), the
card file byte-equal (fetched back over the LAN), heap low, late frames
added. On the classic the panel must run a build with `connect-send-classic`
(sends are off there by default: `hello` says `send: false` until D4
passes). D4 passes on the classic when the heap stays over the session
gate's floor (20 KB outside LAN load) and no steady-state late frames are
added; then sends can be turned on for the classic in release builds. Never contacts anything but --lan and --server.
"""
from __future__ import annotations

import argparse
import asyncio
import io
import json
import os
import random
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import aiohttp  # noqa: E402
import nacl.signing  # noqa: E402

import odc  # noqa: E402
from roundtrip import Controller, guest_pair, send_args, stream  # noqa: E402

SEED = Path.home() / ".config" / "opendmd" / "d4-controller.seed"


def lan(base, method, path, body=None, key=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(base + path, method=method, data=data if data is not None else (b"" if method != "GET" else None))
    if body is not None:
        req.add_header("Content-Type", "application/json")
    if key:
        req.add_header("Authorization", "Bearer " + key)
    with urllib.request.urlopen(req, timeout=30) as r:
        raw = r.read()
        return json.loads(raw) if r.headers.get("Content-Type", "").startswith("application/json") else raw


def bucket(name: str) -> str:
    """`opendmd_core::layout`'s bucket: FNV-1a-32 of the name, low byte."""
    h = 0x811C9DC5
    for ch in name.encode():
        h = ((h ^ ch) * 0x01000193) & 0xFFFFFFFF
    return f"{h & 0xFF:02x}"


def noise_gif(kb: int, seed: int) -> bytes:
    from PIL import Image
    rng = random.Random(seed)
    pal = [c for i in range(64) for c in (i * 4, 255 - i * 4, (i * 37) % 256)]
    frames = []
    while True:
        fr = Image.frombytes("P", (128, 32), bytes(rng.randrange(64) for _ in range(128 * 32)))
        fr.putpalette(pal)
        frames.append(fr)
        buf = io.BytesIO()
        frames[0].save(buf, "GIF", save_all=True, append_images=frames[1:], duration=80, loop=0)
        if len(buf.getvalue()) >= kb * 1024 or len(frames) >= 400:
            return buf.getvalue()


async def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lan", required=True, help="the panel's LAN address, http://...")
    ap.add_argument("--server", required=True, help="the Connect server, https://...")
    ap.add_argument("--api-key", default=os.environ.get("OPENDMD_API_KEY"))
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--size-kb", type=int, default=200)
    ap.add_argument("--copy", action="store_true", help="send a prepared copy too (the panel's own, from an earlier run)")
    ap.add_argument("--clean", action="store_true")
    a = ap.parse_args()
    base = a.lan.rstrip("/")
    info = lan(base, "GET", "/api/connect", key=a.api_key)
    if not info.get("enabled") or info.get("state") != "open":
        sys.exit(f"D4: Remote control is not open on the panel ({info.get('state')!r}): switch it on in Settings first")
    async with aiohttp.ClientSession() as http:
        SEED.parent.mkdir(parents=True, exist_ok=True)
        fresh = not SEED.exists()
        if fresh:
            SEED.write_bytes(os.urandom(32))
            SEED.chmod(0o600)
        sk = nacl.signing.SigningKey(SEED.read_bytes())
        k = odc.key_id(bytes(sk.verify_key))
        listed = any(c.get("k") == k.hex() for c in info.get("controllers", []))
        if a.clean:
            for n in [e["name"] for e in lan(base, "GET", "/api/files?dir=/gifs/Sent", key=a.api_key).get("entries", []) if e["name"].startswith("d4-")]:
                lan(base, "DELETE", f"/api/files/gifs/Sent/{n}", key=a.api_key)
            if listed:
                lan(base, "DELETE", f"/api/connect/controllers/{k.hex()}", key=a.api_key)
            print("D4: cleaned")
            return
        if not listed:
            link = lan(base, "POST", "/api/connect/pair", key=a.api_key)["link"]
            secret = link.split("#", 1)[1]
            # The panel's id comes from the server's find.
            ctl = Controller(a.server, b"", "D4 test")
            ctl.sk, ctl.pub, ctl.k = sk, bytes(sk.verify_key), k
            async with http.post(f"{a.server}/v1/find", data=odc.dumps({"pair_id": odc.pair_id(secret).hex()})) as r:
                ctl.dev = odc.parse_dev((await r.json())["dev"])
            res = await guest_pair(http, a.server, ctl.dev, secret, ctl)
            if not (res.get("replies") or [{}])[0].get("k"):
                sys.exit(f"D4: pairing failed: {res}")
            print("D4: paired a test controller")
        st = lan(base, "GET", "/api/status", key=a.api_key)
        # The panel's Connect id is not on the LAN API: the pairing's find
        # gave it, kept beside the seed for later runs.
        dev_file = SEED.with_suffix(".dev")
        if not listed:
            dev_file.write_text(odc.dev_text(ctl.dev))
        dev = odc.parse_dev(dev_file.read_text().strip())
        c = Controller(a.server, dev, "D4 test")
        c.sk, c.pub, c.k = sk, bytes(sk.verify_key), k
        at = await c.attach(http)
        if not (at and at.get("online")):
            sys.exit(f"D4: the panel is not online at the server: {at}")
        report = []
        for run in range(a.runs):
            gif = noise_gif(a.size_kb, run)
            copy = b""
            name = f"d4-{int(time.time())}-{run}.gif"
            if a.copy and run > 0 and report[-1]["path"]:
                # The panel's own copy of the last run's GIF (its cache
                # builder makes one), sent with this run's GIF: the bytes
                # only matter for the write path, not for what plays.
                prev = report[-1]["path"].rsplit("/", 1)[1]
                try:
                    copy = lan(base, "GET", f"/cache/Sent/{bucket(prev)}/" + urllib.request.quote(prev), key=a.api_key)
                    gif = lan(base, "GET", "/gifs/Sent/" + urllib.request.quote(prev), key=a.api_key)
                except Exception:  # noqa: BLE001
                    copy = b""
            tid = random.randrange(1, 2**32)
            misses0 = (st.get("misses") or {}).get("steady")
            lows, stop = [], asyncio.Event()

            async def watch():
                while not stop.is_set():
                    s = await asyncio.to_thread(lan, base, "GET", "/api/status", None, a.api_key)
                    lows.append(((s.get("heap") or {}).get("free"), (s.get("heap") or {}).get("floor")))
                    await asyncio.sleep(1)
            w = asyncio.create_task(watch())
            f = c.frame("send", send_args(tid, name, gif, copy))
            await c.send(f)
            ack = await c.ack(f)
            t0 = time.time()
            credits, end, _ = await stream(c, tid, gif + copy, timeout=60) if ack and ack.get("ok") else ([], None, 0)
            wall = time.time() - t0
            stop.set()
            await w
            st = lan(base, "GET", "/api/status", key=a.api_key)
            back = lan(base, "GET", "/gifs/Sent/" + urllib.request.quote(end["name"]), key=a.api_key) if end and end.get("done") else b""
            r = {"run": run, "bytes": len(gif) + len(copy), "ack": ack, "end": end, "path": "/gifs/Sent/" + end["name"] if end and end.get("done") else None, "credits": len(credits),
                 "kb_s": round((len(gif) + len(copy)) / 1024 / wall, 1) if wall > 0 else None, "byte_equal": back == gif,
                 "heap_free_min": min((x[0] for x in lows if x[0]), default=None), "heap_floor": min((x[1] for x in lows if x[1]), default=None),
                 "late_frames_added": ((st.get("misses") or {}).get("steady") or 0) - (misses0 or 0)}
            print("D4", json.dumps(r))
            report.append(r)
            await asyncio.sleep(5)
        ok = all(r["byte_equal"] and (r["heap_free_min"] or 0) > 20 * 1024 and r["late_frames_added"] == 0 for r in report)
        print("D4_PASS" if ok else "D4_FAIL", json.dumps({"runs": len(report), "fw": st.get("version")}))


if __name__ == "__main__":
    asyncio.run(main())
