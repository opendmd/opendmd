# /// script
# dependencies = []
# ///
"""Play each Formula on the spare and record its now_playing.formula timing."""
import json, sys, time, urllib.request, pathlib
HOST = sys.argv[1]; OUT = sys.argv[2]; PATHS = sys.argv[3:]
def req(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(f"http://{HOST}{path}", data=data, method=method, headers={"Content-Type": "application/json"} if data else {})
    with urllib.request.urlopen(r, timeout=10) as f:
        return f.status, f.read().decode()
rows = []
for p in PATHS:
    if p.startswith("@file:"):
        name, path = p[6:].split("=", 1)
    else:
        name, path = p, p
    try:
        st, body = req("POST", "/api/play", {"path": path})
    except Exception as e:
        rows.append({"name": name, "error": str(e)}); print(name, "ERR", e); continue
    time.sleep(5)
    samples = []
    for _ in range(3):
        s = json.loads(req("GET", "/api/status")[1])
        f = (s.get("now_playing") or {}).get("formula") or {}
        samples.append({k: f.get(k) for k in ("frame_us", "frame_ms", "asked_ms", "beam_us", "compile_us", "still", "jit")} | {"heap": s["heap"]["free"], "floor": s["heap"]["floor"], "misses": s.get("deadline_misses")})
        time.sleep(1)
    r = {"name": name, "status": st, **samples[-1], "frame_us_all": [x["frame_us"] for x in samples]}
    rows.append(r)
    print(f"{name:40} us={r['frame_us_all']} ms={r['frame_ms']} beam={r['beam_us']} jit={r['jit']} heap={r['heap']}")
pathlib.Path(OUT).write_text(json.dumps(rows, indent=1))
