# /// script
# dependencies = []
# ///
"""Play each Formula on a device and record its now_playing.formula timing."""
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
    samples, beams = [], []
    for _ in range(3):
        s = json.loads(req("GET", "/api/status")[1])
        f = (s.get("now_playing") or {}).get("formula") or {}
        samples.append({k: f.get(k) for k in ("frame_us", "frame_ms", "asked_ms", "beam_us", "compile_us", "still", "jit")} | {"heap": s["heap"]["free"], "floor": s["heap"]["floor"], "misses": s.get("deadline_misses")} | {k: s.get(k) for k in ("look", "glow", "formula_look")})
        beams.append(((s.get("scan") or {}).get("beam")) or {})
        time.sleep(1)
    # Racing the beam over the samples: per raced frame, the µs spent
    # writing pairs and in the pair hook (writes included), late frames
    # (the counters wrap: a negative difference is dropped).
    a, b = beams[0], beams[-1]
    n = b.get("frames", 0) - a.get("frames", 0)
    per = lambda k: round((b.get(k, 0) - a.get(k, 0)) / n) if n > 0 and b.get(k, 0) >= a.get(k, 0) else None
    beam = {"frames": n, "late_frames": b.get("late_frames", 0) - a.get("late_frames", 0), "write_us_per_frame": per("write_us"), "pair_us_per_frame": per("pair_us"), "first_us": b.get("first_us"), "last_us": b.get("last_us")}
    r = {"name": name, "status": st, **samples[-1], "frame_us_all": [x["frame_us"] for x in samples], "beam": beam}
    rows.append(r)
    print(f"{name:40} us={r['frame_us_all']} ms={r['frame_ms']} beam={r['beam_us']} jit={r['jit']} heap={r['heap']} write/frame={beam['write_us_per_frame']} pair/frame={beam['pair_us_per_frame']} look={r['look']} glow={r['glow']}")
pathlib.Path(OUT).write_text(json.dumps(rows, indent=1))
