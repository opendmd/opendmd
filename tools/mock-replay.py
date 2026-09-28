# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""A fake RePlayOS, for testing an OpenDMD ESP32 (and the local mock device)
against the real HTTP API without a Raspberry Pi.

RePlayOS is a closed libretro frontend that exposes a tiny HTTP API on port
55356: `get_version` (open), `get_status` (needs a bearer-ish token in an
`X-RePlay-Token` header). This plays back a scripted timeline of system/game
states over wall-clock time, so a client polling `get_status` sees the same
sequence of menu / game_play / pause-menu transitions a real box would.

    uv run tools/mock-replay.py [--host 0.0.0.0] [--port 55356] [--token 1234] \
        [--timeline FILE.json] [--loop | --no-loop] [--speed 1.0] \
        [--start-step 0] [--quiet] [--any-token] [--pad BYTES]

Real API mimicked (base `http://<host>:<port>/api/v1/`, all GET, JSON, HTTP
only):

  - `GET /api/v1/get_version` -- no token needed.
  - `GET /api/v1/get_status`  -- needs header `X-RePlay-Token: <token>`;
    missing/wrong token -> 401 `{"error":"unauthorized"}`. Correct token ->
    the 9 fields in the exact order the real device sends them: system,
    game_file, game_name, paused, halted, view, view_id, core_file,
    core_info. A "menu" step (no system running) has system/game_file/
    game_name/core_file/core_info as empty strings and paused/halted false.
    `view` is one of system_list(0) system_options(1) game_play(2)
    game_options(3) achievements(4) leaderboards(5); `view_id` is the number.
  - Anything else under `/api/v1/` -> 404 JSON. Other paths -> 404 JSON.

Built-in default timeline (loops, total 5 min 26 s):

    step  start   secs  view            system / game
    0     0:00     20   system_list     (menu)
    1     0:20     30   game_play       sega_smd: Sonic the Hedgehog 3 (USA)
    2     0:50      6   game_options    (pause menu, same game)
    3     0:56     30   game_play       sega_smd: Sonic the Hedgehog 3 (USA)
    4     1:26     40   system_list     (menu)
    5     2:06     60   game_play       nintendo_snes: Super Mario World (USA)
    6     3:06     40   system_list     (menu)
    7     3:46     60   game_play       sega_smd: Sonic the Hedgehog 3 (USA)
    8     4:46     40   system_list     (menu)
                  ----
                  326 s = 5:26, then loops back to step 0 (unless --no-loop,
                  in which case it parks on step 8 forever)

A custom `--timeline FILE.json` is a JSON list of steps, each:

    {"view": "system_list"|"system_options"|"game_play"|"game_options"|
             "achievements"|"leaderboards",
     "secs": N,
     "system": "...", "game_name": "...", "game_file": "...",
     "core_file": "...", "core_info": "...", "paused": bool}

Any field besides "view"/"secs" may be omitted (defaults to "" / false); a
step with an empty/omitted "system" is treated as a menu step and reports
all game fields as "" regardless of what else is set.

Control endpoints for tests (no token required):

  - `GET /mock/state`        -> {"step", "steps", "elapsed_in_step",
                                  "remaining", "status": {...get_status...}}
  - `POST /mock/goto?step=N` -> jump to step N now, restarting its clock.
  - `POST /mock/token?value=X` -> change the accepted token at runtime.

`--any-token` accepts any `X-RePlay-Token` (a device whose saved code you
do not know can be pointed here without touching it); `--pad BYTES` makes
every `get_status` reply that much bigger (a `padding` field of that many
characters before the real ones, then a nested object and an array holding
decoy `system`/`view` keys) — for testing a client that must read a large
reply in bounded memory.

`--speed` multiplies wall-clock time for quick local runs (e.g. `--speed 10`
makes the whole default timeline play out in ~33 s). `--start-step` picks
which step the timeline is at when the server starts.
"""
import argparse
import json
import signal
import sys
import threading
import time
from urllib.parse import parse_qs, urlsplit

VIEW_IDS = {
    "system_list": 0,
    "system_options": 1,
    "game_play": 2,
    "game_options": 3,
    "achievements": 4,
    "leaderboards": 5,
}

DEFAULT_TIMELINE = [
    {"view": "system_list", "secs": 20},
    {"view": "game_play", "secs": 30, "system": "sega_smd",
     "game_name": "Sonic the Hedgehog 3 (USA)",
     "game_file": "/media/nvme/roms/sega_smd/Sonic the Hedgehog 3 (USA).md",
     "core_file": "/opt/replay/cores/genesis_plus_gx_libretro.so",
     "core_info": "Genesis Plus GX"},
    {"view": "game_options", "secs": 6, "system": "sega_smd",
     "game_name": "Sonic the Hedgehog 3 (USA)",
     "game_file": "/media/nvme/roms/sega_smd/Sonic the Hedgehog 3 (USA).md",
     "core_file": "/opt/replay/cores/genesis_plus_gx_libretro.so",
     "core_info": "Genesis Plus GX", "paused": True},
    {"view": "game_play", "secs": 30, "system": "sega_smd",
     "game_name": "Sonic the Hedgehog 3 (USA)",
     "game_file": "/media/nvme/roms/sega_smd/Sonic the Hedgehog 3 (USA).md",
     "core_file": "/opt/replay/cores/genesis_plus_gx_libretro.so",
     "core_info": "Genesis Plus GX"},
    {"view": "system_list", "secs": 40},
    {"view": "game_play", "secs": 60, "system": "nintendo_snes",
     "game_name": "Super Mario World (USA)",
     "game_file": "/media/nvme/roms/nintendo_snes/Super Mario World (USA).sfc",
     "core_file": "/opt/replay/cores/snes9x_libretro.so",
     "core_info": "Snes9x"},
    {"view": "system_list", "secs": 40},
    {"view": "game_play", "secs": 60, "system": "sega_smd",
     "game_name": "Sonic the Hedgehog 3 (USA)",
     "game_file": "/media/nvme/roms/sega_smd/Sonic the Hedgehog 3 (USA).md",
     "core_file": "/opt/replay/cores/genesis_plus_gx_libretro.so",
     "core_info": "Genesis Plus GX"},
    {"view": "system_list", "secs": 40},
]


def timestamp():
    t = time.time()
    ms = int((t - int(t)) * 1000)
    return time.strftime("%H:%M:%S", time.localtime(t)) + f".{ms:03d}"


def log(msg):
    print(f"{timestamp()} {msg}", flush=True)


def normalize_step(raw, idx):
    if not isinstance(raw, dict):
        sys.exit(f"mock-replay: timeline step {idx}: not an object")
    view = raw.get("view")
    if view not in VIEW_IDS:
        sys.exit(f"mock-replay: timeline step {idx}: bad or missing \"view\" {view!r} "
                  f"(must be one of {', '.join(VIEW_IDS)})")
    secs = raw.get("secs")
    try:
        secs = float(secs)
    except (TypeError, ValueError):
        secs = None
    if secs is None or secs <= 0:
        sys.exit(f"mock-replay: timeline step {idx}: bad or missing \"secs\" {raw.get('secs')!r}")
    return {
        "view": view,
        "secs": secs,
        "system": str(raw.get("system") or ""),
        "game_name": str(raw.get("game_name") or ""),
        "game_file": str(raw.get("game_file") or ""),
        "core_file": str(raw.get("core_file") or ""),
        "core_info": str(raw.get("core_info") or ""),
        "paused": bool(raw.get("paused", False)),
    }


def load_timeline(path):
    try:
        with open(path, "r", encoding="utf-8") as f:
            raw = json.load(f)
    except (OSError, json.JSONDecodeError) as e:
        sys.exit(f"mock-replay: {path}: {e}")
    if not isinstance(raw, list) or not raw:
        sys.exit(f"mock-replay: {path}: expected a non-empty JSON list of steps")
    return [normalize_step(step, i) for i, step in enumerate(raw)]


def step_desc(step):
    if step["system"]:
        desc = f'{step["view"]} {step["system"]} "{step["game_name"]}"'
        if step["paused"]:
            desc += " [paused]"
        return desc
    return step["view"]


class State:
    """Owns the timeline clock. Thread-safe: HTTP handlers and the ticker
    thread all go through here."""

    def __init__(self, timeline, speed, loop, start_step, token):
        self.timeline = timeline
        self.speed = speed
        self.loop = loop
        self.token = token
        self.lock = threading.Lock()
        self.cum_start = []
        acc = 0.0
        for step in timeline:
            self.cum_start.append(acc)
            acc += step["secs"]
        self.total = acc
        self.origin_wall = time.monotonic()
        self.origin_virtual = self.cum_start[start_step]

    def _virtual_elapsed_locked(self):
        return self.origin_virtual + (time.monotonic() - self.origin_wall) * self.speed

    def _locate_locked(self, ve):
        n = len(self.timeline)
        if self.loop and self.total > 0:
            t = ve % self.total
        else:
            t = min(ve, self.total)
        acc = 0.0
        for i, step in enumerate(self.timeline):
            secs = step["secs"]
            if i == n - 1 or t < acc + secs:
                elapsed = min(max(t - acc, 0.0), secs)
                return i, elapsed
            acc += secs
        return n - 1, self.timeline[-1]["secs"]  # unreachable in practice

    def snapshot(self):
        """Returns (idx, elapsed_in_step, remaining, step)."""
        with self.lock:
            ve = self._virtual_elapsed_locked()
            idx, elapsed = self._locate_locked(ve)
            step = self.timeline[idx]
            remaining = max(step["secs"] - elapsed, 0.0)
            return idx, elapsed, remaining, step

    def goto(self, idx):
        with self.lock:
            idx = max(0, min(idx, len(self.timeline) - 1))
            self.origin_wall = time.monotonic()
            self.origin_virtual = self.cum_start[idx]
            return idx

    def get_token(self):
        with self.lock:
            return self.token

    def set_token(self, token):
        with self.lock:
            self.token = token


def status_body(step):
    if step["system"]:
        return {
            "system": step["system"],
            "game_file": step["game_file"],
            "game_name": step["game_name"],
            "paused": step["paused"],
            "halted": False,
            "view": step["view"],
            "view_id": VIEW_IDS[step["view"]],
            "core_file": step["core_file"],
            "core_info": step["core_info"],
        }
    return {
        "system": "",
        "game_file": "",
        "game_name": "",
        "paused": False,
        "halted": False,
        "view": step["view"],
        "view_id": VIEW_IDS[step["view"]],
        "core_file": "",
        "core_info": "",
    }


def ticker_loop(state, stop_event):
    last_idx = None
    n = len(state.timeline)
    while not stop_event.is_set():
        idx, _elapsed, _remaining, step = state.snapshot()
        if idx != last_idx:
            log(f"step {idx + 1}/{n}: {step_desc(step)} for {step['secs']:g} s")
            last_idx = idx
        stop_event.wait(0.02)


def padded(body, pad):
    """`body` with `pad` bytes of decoys in front (see `--pad`)."""
    if pad <= 0:
        return body
    decoys = {"padding": "x" * pad,
              "nested": {"system": "decoy", "view": "game_play", "list": [1, {"view": "decoy"}]}}
    return {**decoys, **body}


def make_handler(state, quiet, any_token=False, pad=0):
    import http.server

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        server_version = "OpenDMD-mock-replay/0.1"

        def log_message(self, fmt, *args):
            pass

        def _send_json(self, status, obj):
            body = json.dumps(obj).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def _log_request(self, method, path, status, token_ok):
            if status == 401 or not quiet:
                tok = "n/a" if token_ok is None else ("ok" if token_ok else "bad")
                log(f"{self.client_address[0]:15s} {method:5s} {path:40s} {status} token={tok}")

        def do_GET(self):
            self._dispatch("GET")

        def do_POST(self):
            self._dispatch("POST")

        def _dispatch(self, method):
            parsed = urlsplit(self.path)
            path = parsed.path
            qs = parse_qs(parsed.query)
            token_ok = None
            status = 500
            try:
                if method == "GET" and path == "/api/v1/get_version":
                    status = 200
                    self._send_json(status, {"version": "1.8.0", "name": "RePlayOS"})
                elif method == "GET" and path == "/api/v1/get_status":
                    supplied = self.headers.get("X-RePlay-Token")
                    token_ok = supplied is not None and (any_token or supplied == state.get_token())
                    if not token_ok:
                        status = 401
                        self._send_json(status, {"error": "unauthorized"})
                    else:
                        status = 200
                        _idx, _e, _r, step = state.snapshot()
                        self._send_json(status, padded(status_body(step), pad))
                elif method == "GET" and path.startswith("/api/v1/"):
                    status = 404
                    self._send_json(status, {"error": "not found"})
                elif method == "GET" and path == "/mock/state":
                    status = 200
                    idx, elapsed, remaining, step = state.snapshot()
                    self._send_json(status, {
                        "step": idx,
                        "steps": len(state.timeline),
                        "elapsed_in_step": round(elapsed, 3),
                        "remaining": round(remaining, 3),
                        "status": status_body(step),
                    })
                elif method == "POST" and path == "/mock/goto":
                    raw = (qs.get("step") or [None])[0]
                    try:
                        want = int(raw)
                    except (TypeError, ValueError):
                        want = None
                    if want is None:
                        status = 400
                        self._send_json(status, {"error": "bad or missing ?step="})
                    else:
                        idx = state.goto(want)
                        status = 200
                        log(f"control: POST /mock/goto?step={raw} -> now at step {idx + 1}/{len(state.timeline)}")
                        self._send_json(status, {"ok": True, "step": idx})
                elif method == "POST" and path == "/mock/token":
                    value = (qs.get("value") or [None])[0]
                    if value is None:
                        status = 400
                        self._send_json(status, {"error": "bad or missing ?value="})
                    else:
                        state.set_token(value)
                        status = 200
                        log(f"control: POST /mock/token?value={value} -> token changed")
                        self._send_json(status, {"ok": True})
                else:
                    status = 404
                    self._send_json(status, {"error": "not found"})
            except (BrokenPipeError, ConnectionResetError):
                pass
            finally:
                self._log_request(method, self.path, status, token_ok)

    return Handler


def parse_args():
    ap = argparse.ArgumentParser(
        description="A fake RePlayOS HTTP API for testing an OpenDMD ESP32.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument("--host", default="0.0.0.0")
    ap.add_argument("--port", type=int, default=55356)
    ap.add_argument("--token", default="1234", help="token get_status requires (default: 1234)")
    ap.add_argument("--timeline", help="JSON file overriding the built-in default timeline")
    ap.add_argument("--loop", action=argparse.BooleanOptionalAction, default=True,
                     help="loop the timeline forever (default: on)")
    ap.add_argument("--speed", type=float, default=1.0, help="time multiplier (default: 1.0)")
    ap.add_argument("--start-step", type=int, default=0, help="step index to start at (default: 0)")
    ap.add_argument("--quiet", action="store_true", help="suppress the one-line-per-request log")
    ap.add_argument("--any-token", action="store_true", help="accept any X-RePlay-Token")
    ap.add_argument("--pad", type=int, default=0, metavar="BYTES",
                    help="make every get_status reply this many bytes bigger (decoy fields first)")
    return ap.parse_args()


def main():
    args = parse_args()
    import http.server

    if args.timeline:
        timeline = load_timeline(args.timeline)
    else:
        timeline = [normalize_step(step, i) for i, step in enumerate(DEFAULT_TIMELINE)]
    if not (0 <= args.start_step < len(timeline)):
        sys.exit(f"mock-replay: --start-step {args.start_step} out of range "
                  f"(0..{len(timeline) - 1})")
    if args.speed <= 0:
        sys.exit(f"mock-replay: --speed must be > 0, got {args.speed}")

    state = State(timeline, args.speed, args.loop, args.start_step, args.token)

    stop_event = threading.Event()
    ticker = threading.Thread(target=ticker_loop, args=(state, stop_event), daemon=True)
    ticker.start()

    handler_cls = make_handler(state, args.quiet, args.any_token, args.pad)
    server = http.server.ThreadingHTTPServer((args.host, args.port), handler_cls)
    server.daemon_threads = True

    def handle_sigterm(_signum, _frame):
        raise KeyboardInterrupt()

    signal.signal(signal.SIGTERM, handle_sigterm)

    total_m, total_s = divmod(state.total, 60)
    log(f"mock-replay: http://{args.host}:{args.port}  token={args.token!r}  "
        f"{len(timeline)} steps, {int(total_m)}:{total_s:04.1f} total, "
        f"loop={args.loop}, speed={args.speed:g}x, start-step={args.start_step}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        stop_event.set()
        server.server_close()


if __name__ == "__main__":
    main()
