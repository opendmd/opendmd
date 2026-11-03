"""The API lane (`run.py api`, `make emu-api`): the firmware's HTTP API driven from the host.

Two boots of the real full image, each with WiFi credentials seeded into its NVS (a copy of
the image: `seeded_image`), on the virtual access point with the guest's port 80 published on
127.0.0.1. The whole lane runs inside a private network namespace (`unshare -rn`, set up by
run.py): the emulator's NAT has nowhere to go but the loopback, so nothing reaches a real
network or device (mDNS, SNTP, DNS, RePlayOS all fail quietly), and the fixed port never
collides with another run.

Boot A (`--no-reboot`: the emulator stops at the first reset, and writes the card out):
  basics      GET /api/status and /api/config; PUT a setting, read back; a bad PUT is a 400
  files       uploads (PUT /api/files/..., ?new=1 and its 409, a replace), read back byte for
              byte, listings (?names=1 and a walk), moves (and their 409/404), deletes (404 after)
  play        /api/play of an uploaded GIF, a Formula (`@formula?psb=`), the same saved to
              the card (.psi + .psb) by its path; skip, pause and resume
  www         PUT /api/www with the web app's pack
  bursts      (--connections 2 or 5, the default 5) the spare's wedge recipe: rounds of 2, 2, 2, 5,
              5, 5 uploads of 300 KB at once beside status reads; each 200, or 503 + Retry-After
              and 200 when asked again; never a 408; the network answering; nothing held after
  load x3     uploads read back, settings saves, saves into a big folder, listings and status
              reads (display-stress's loads), each on its own connection at once by default
              (--connections 2: status polls beside the rest; 1: one at a time), then 60 s quiet:
              the heap census (`emu-census`) of the last quiet spell against the first's
  cache       a recursive delete under /cache while the FAT index builds (the corruption
              hazard fixed in fatidx-heap): the build dropped, rebuilt, not stale
  ota         GET /api/firmware; bundles with a bad signature and a bad image: 422
  end         every file read back again; POST /api/reboot (boot A ends, the card written)
Then the card boot A wrote: fatcheck (no cross-links or lost clusters), the settings file
(/opendmd.json) holds the last setting saved, the files byte for byte, nothing left behind.
Boot B (from that card, rebooting allowed): the setting and the files survived the power
cycle; PUT /api/firmware with the signed bundle: verified, staged, the updater installs it,
the firmware is back with the bundle's version.

Fails at once on a panic, Guru Meditation, stack overflow, abort() or a reset not asked for;
prints EMU_API_OK or EMU_API_FAIL with the reason. Correctness only, never timing.
"""
import base64, contextlib, hashlib, json, os, random, re, shutil, signal, struct, subprocess, sys, threading, time
import http.client, urllib.error, urllib.parse, urllib.request
from pathlib import Path

PORT = 8080  # in the lane's own network namespace: never taken by another run
WIFI = "ssid=demo,chan=6,psk=demo-password"
# A quiet spell's census meets the playing GIF's phases (its levels step by 5-10 KB: ~196, 201,
# 209, 211, 221 KB in the census image), and its least is compared: 30 s (~4 samples) missed the
# lowest phase in one spell of three and failed a flat heap by 5 KB; 60 s takes ~8.
LOAD_S, QUIET_S = 30, 60
BURST_ROUNDS = [2, 2, 2, 5, 5, 5]
ROUNDS = max(2, int(os.environ.get("EMU_API_ROUNDS", "3")))
# RX_BUFFERS (found by this lane, 2026-11-03; fixed in net-wedge): two uploads at once got the
# one being received a 408 (`no data for 5 s`), and a few rounds wedged the network for good (no
# TCP, ping or ARP answered; the spare showed it too). The netdiag walk (`netdiag:` lines,
# emu-census builds) showed why: every frame lwIP holds stays in one of the WiFi driver's six
# dynamic receive buffers. The upload being received held four out-of-order segments after a lost
# one and the waiting upload's unread data the other two, so its retransmission had no buffer;
# then the 408'd uploads, closed into FIN_WAIT_1 with no socket, kept their out-of-order segments
# (4 + 2): the ACK of their FIN could never come in, the queue's expiry (6 RTOs) receded as the
# RTO backed off, and no frame was received again. Fixed: the expiry is one RTO
# (CONFIG_LWIP_TCP_OOSEQ_TIMEOUT=1: between two retransmissions the silence always passes it),
# and one upload at a time, taken only with the stream task idle, a second one 503 + Retry-After
# at once, keeping its place in line (stream.rs: a waiting upload held five buffers for 2.6 KB of
# small segments, and starved a listing ahead of it). The windows stay 5760 (the 1440 asked for was below Kconfig's range and
# silently 5760 all along; 2880 made uploads ~3.5x slower here and the firmware bundle hit the
# 120 s cap; no out-of-order queue at all, the same for the www pack). See "The receive side"
# in sdkconfig.defaults. The bursts and the five-connection load are the gate.
# EMPTY_FOLDER (found by this lane; fixed in net-wedge): `DELETE /api/files/gifs/<folder>` of an
# emptied folder went through `std::fs::remove_file`, which FatFs's f_unlink does for an empty
# directory too, so the branch that calls `fatidx::dir_changed()` was never taken. The firmware
# now tells a directory apart first; `final_a` checks the index is stale at once after.
# The heap census: the least of the last quiet spell against the first's (as emu-leak).
LEAK_BLOCKS, LEAK_BYTES = 24, 2048

ANSI = re.compile(r"\x1b\[[0-9;]*m")
GIF_START = re.compile(r"\] (/gifs/\S+\.gif) <- (prepared|original)")
CENSUS = re.compile(r"census: (\d+) blocks in use, (\d+) B allocated")
BUILT = re.compile(r"fat index: built (\d+) records .*stale (true|false)")
BUILD_STARTED = re.compile(r"fat index: build started")
FATIDX_WARN = re.compile(r"^W \(\d+\) \S+: fat index: (?!.*no raw reads)|raw read of .* differs")
DELETE_LINE = re.compile(r"DELETE (/cache/\S+) \(recursive\): (\d+) entries removed, done (true|false)")
HEARTBEAT = re.compile(r"heartbeat: heap (\d+) B free \(floor (\d+) B.*main stack (\d+) B unused.*drain stack (\d+) B unused")
HTTPD_STACK = re.compile(r"httpd task: stack high-water (\d+) B unused")
NETDIAG_SUM = re.compile(r"netdiag: sum: (\d+) sockets open, \d+ TIME_WAIT pcbs, (\d+) rx pbufs held")
WORKER_STACK = re.compile(r"fat index: built .* (\d+) B of the worker's stack unused")


class LaneFail(Exception):
    pass


# --- the seeded image ----------------------------------------------------------

def nvs_partition(image: bytes) -> tuple[int, int]:
    """The `nvs` data partition's offset and size from the image's table at 0x8000."""
    for off in range(0x8000, 0x8000 + 0xC00, 32):
        e = image[off:off + 32]
        if e[:2] != b"\xaa\x50":
            break
        typ, sub, start, size = struct.unpack_from("<BBII", e, 2)
        if typ == 1 and sub == 2:
            return start, size
    raise LaneFail("no nvs partition in the image's partition table")


def nvs_gen_python() -> Path:
    found = sorted(Path.home().glob(".espressif/python_env/idf5.3_py3*_env/bin/python"))
    if not found:
        raise LaneFail("no ESP-IDF v5.3 python env under ~/.espressif/python_env (it has nvs_partition_gen)")
    return found[-1]


def seeded_image(image: Path, out_dir: Path, ssid="demo", password="demo-password") -> Path:
    """A copy of `image` whose NVS holds WiFi credentials (namespace `opendmd`, `wifi_ssid`/
    `wifi_pass`), written with IDF v5.3's nvs_partition_gen. Cached by the image's bytes."""
    data = image.read_bytes()
    digest = hashlib.sha256(data).hexdigest()[:16]
    out = out_dir / f"{image.stem}-wifi-{digest}.bin"
    if out.exists():
        return out
    out_dir.mkdir(parents=True, exist_ok=True)
    # Older seeds go once a day old (another run may be booting one now).
    for old in out_dir.glob("*-wifi-*.bin"):
        if time.time() - old.stat().st_mtime > 86400:
            old.unlink(missing_ok=True)
    start, size = nvs_partition(data)
    csv, binf = out_dir / f"nvs-{digest}.csv", out_dir / f"nvs-{digest}.bin"
    csv.write_text(f"key,type,encoding,value\nopendmd,namespace,,\nwifi_ssid,data,string,{ssid}\nwifi_pass,data,string,{password}\n")
    subprocess.run([str(nvs_gen_python()), "-m", "esp_idf_nvs_partition_gen", "generate", str(csv), str(binf), hex(size)],
                   check=True, stdout=subprocess.DEVNULL)
    nvs = binf.read_bytes()
    if len(nvs) != size:
        raise LaneFail(f"nvs_partition_gen wrote {len(nvs)} B, the partition is {size} B")
    tmp = out.with_suffix(f".{os.getpid()}.tmp")
    tmp.write_bytes(data[:start] + nvs + data[start + size:])
    os.replace(tmp, out)
    csv.unlink()
    binf.unlink()
    print(f"api: seeded WiFi credentials into {out} (nvs at {start:#x})", flush=True)
    return out


# --- the emulator and its console ------------------------------------------------

class Emulator:
    """esp32sim with the console read on a thread: every line kept and logged, the fail
    patterns and resets watched (`resets_allowed`: the power-on one, plus any asked for)."""

    def __init__(self, cmd, env, log: Path, fail_rx, label: str):
        self.cmd, self.env, self.log, self.fail_rx, self.label = cmd, env, log, fail_rx, label
        self.lines: list[str] = []
        self.cond = threading.Condition()
        self.failure = None
        self.resets = 0
        self.resets_allowed = 1
        self.done = False

    def start(self):
        print(f"api: boot {self.label}: " + " ".join(self.cmd), flush=True)
        print(f"api: boot {self.label}: log {self.log}", flush=True)
        self.p = subprocess.Popen(self.cmd, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                  text=True, errors="replace", start_new_session=True)
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        with open(self.log, "w") as lf:
            for raw in self.p.stdout:
                lf.write(raw)
                lf.flush()
                line = ANSI.sub("", raw.rstrip("\n"))
                with self.cond:
                    self.lines.append(line)
                    if line.startswith("rst:0x"):
                        self.resets += 1
                        if self.resets > self.resets_allowed and not self.failure:
                            self.failure = f"unexpected reset: {line}"
                    for rx, name in self.fail_rx:
                        if rx.search(line) and not self.failure:
                            self.failure = f"{name}: {line[:200]}"
                    self.cond.notify_all()
        self.p.wait()
        with self.cond:
            self.done = True
            self.cond.notify_all()

    def mark(self) -> int:
        with self.cond:
            return len(self.lines)

    def check(self):
        if self.failure:
            raise LaneFail(f"boot {self.label}: {self.failure}")

    def wait_for(self, rx, timeout, start=0, what=None):
        """The first line from `start` on that matches `rx`: (index, match). Raises on a
        console failure, the emulator's end, or the timeout."""
        end = time.time() + timeout
        i = start
        with self.cond:
            while True:
                while i < len(self.lines):
                    m = rx.search(self.lines[i])
                    if m:
                        return i, m
                    i += 1
                if self.failure:
                    raise LaneFail(f"boot {self.label}: {self.failure}")
                if self.done:
                    raise LaneFail(f"boot {self.label}: the emulator stopped (exit {self.p.returncode}) before {what or rx.pattern!r}")
                left = end - time.time()
                if left <= 0:
                    raise LaneFail(f"boot {self.label}: no {what or rx.pattern!r} within {timeout:.0f} s wall")
                self.cond.wait(min(left, 1.0))

    def find(self, rx, start=0, end=None):
        with self.cond:
            return [(i, m) for i, l in enumerate(self.lines[start:end], start) if (m := rx.search(l))]

    def wait_exit(self, timeout) -> bool:
        try:
            self.p.wait(timeout)
            return True
        except subprocess.TimeoutExpired:
            return False

    def kill(self):
        try:
            os.killpg(self.p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.p.wait()

    def tail(self, n=15):
        with self.cond:
            return "\n".join(l[:200] for l in self.lines[-n:])


# --- HTTP ------------------------------------------------------------------------

class Api:
    """The device's API on 127.0.0.1:PORT. A 503 with Retry-After is the firmware's "busy,
    ask again" (a full stream queue, the heap short for a moment): retried, and counted."""

    def __init__(self, emu: Emulator):
        self.emu = emu
        self.base = f"http://127.0.0.1:{PORT}"
        self.lock = threading.Lock()
        self.stats = {"requests": 0, "busy_retries": 0}

    def raw(self, method, path, body=None, timeout=60.0):
        data, headers = None, {}
        if isinstance(body, (bytes, bytearray)):
            data, headers = bytes(body), {"Content-Type": "application/octet-stream"}
        elif body is not None:
            data, headers = json.dumps(body).encode(), {"Content-Type": "application/json"}
        r = urllib.request.Request(self.base + path, data=data, method=method, headers=headers)
        with self.lock:
            self.stats["requests"] += 1
        try:
            with urllib.request.urlopen(r, timeout=timeout) as f:
                return f.status, dict(f.headers), f.read()
        except urllib.error.HTTPError as e:
            return e.code, dict(e.headers or {}), e.read()
        except (urllib.error.URLError, ConnectionResetError, BrokenPipeError) as e:
            # An upload refused while another is received: 503 + Retry-After, then a reset under
            # the rest of the body, which urllib (it sends the whole body before it reads) sees
            # alone. curl and browsers read the 503 (the bursts check curl does). The firmware
            # never resets an upload it took, so the web app asks again (`sent` in source.js);
            # so does `call`.
            reason = getattr(e, "reason", e)
            if data is not None and isinstance(reason, (ConnectionResetError, BrokenPipeError)):
                return 0, {"Retry-After": "2"}, b"reset under the body"
            self.emu.check()
            raise LaneFail(f"{method} {path[:120]}: no answer ({e!r})")
        except (http.client.HTTPException, OSError) as e:
            self.emu.check()
            raise LaneFail(f"{method} {path[:120]}: no answer ({e!r})")

    def call(self, method, path, body=None, want=(200,), timeout=60.0, tries=12):
        for _ in range(tries):
            code, headers, data = self.raw(method, path, body, timeout)
            if code in (0, 503) and 503 not in want and "Retry-After" in headers:
                with self.lock:
                    self.stats["busy_retries"] += 1
                time.sleep(min(float(headers.get("Retry-After") or 1), 3))
                continue
            break
        if code not in want:
            self.emu.check()
            raise LaneFail(f"{method} {path[:120]}: HTTP {code}, wanted {'/'.join(map(str, want))}: {data[:200]!r}")
        return code, data

    def json(self, method, path, body=None, want=(200,), timeout=60.0):
        code, data = self.call(method, path, body, want, timeout)
        try:
            return code, json.loads(data) if data else None
        except ValueError:
            raise LaneFail(f"{method} {path[:120]}: HTTP {code} but not JSON: {data[:200]!r}")

    def status(self):
        return self.json("GET", "/api/status")[1]

    def wait_up(self, timeout):
        end = time.time() + timeout
        while time.time() < end:
            self.emu.check()
            if self.emu.done:
                raise LaneFail(f"boot {self.emu.label}: the emulator stopped before the API answered")
            try:
                code, _, data = self.raw("GET", "/api/status", timeout=5)
                if code == 200:
                    return json.loads(data)
            except LaneFail:
                pass
            time.sleep(1)
        raise LaneFail(f"boot {self.emu.label}: GET /api/status did not answer within {timeout} s wall")


def q(path: str) -> str:
    return urllib.parse.quote(path)


def blob(seed: int, n: int) -> bytes:
    return random.Random(seed).randbytes(n)


# --- the lane --------------------------------------------------------------------

class Lane:
    def __init__(self, a, sim, rom, fail_rx):
        self.a, self.sim, self.rom, self.fail_rx = a, sim, rom, fail_rx
        self.passes: list[str] = []
        self.notes: list[str] = []
        self.stamp = time.strftime("%Y%m%d-%H%M%S") + f"-{os.getpid()}"
        self.logs = a.log.parent if a.log else a.emu_dir / "logs"
        self.logs.mkdir(parents=True, exist_ok=True)
        # The card boot A writes out (256 MB): removed after a pass, kept after a failure.
        self.card_out = self.logs / f"api-{self.stamp}-card.img"
        self.heap_free_min = None
        root = a.emu_dir / "card" / "root" / "gifs" / "Other"
        self.gifs = [p.read_bytes() for p in sorted(root.glob("*.gif"))[:6]]
        if len(self.gifs) < 6:
            raise LaneFail(f"fewer than 6 GIFs in {root} (run mkcard.py)")

    def ok(self, name, detail):
        line = f"pass  {name}: {detail}"
        self.passes.append(line)
        print(line, flush=True)

    def expect(self, cond, name, detail):
        if not cond:
            raise LaneFail(f"{name}: {detail}")

    def boot(self, label, card: Path, no_reboot: bool, card_out: Path | None):
        env = {**os.environ, "ESP32SIM_SD_IMG": str(card)}
        env.pop("ESP32SIM_SD_OUT", None)
        if card_out:
            card_out.unlink(missing_ok=True)
            env["ESP32SIM_SD_OUT"] = str(card_out)
        cmd = [str(self.sim), "--chip", "esp32", "--board", "esp32dev-sdspi", "--boot", "rom", "--rom", str(self.rom),
               "--flash-image", str(self.a.seeded), "--flash-mb", str(max(4, self.a.seeded.stat().st_size >> 20)),
               "--max-seconds", str(self.a.seconds or 7200), "--console", "uart0",
               "--wifi", WIFI, "--net", "nat", "--hostfwd", f"tcp:127.0.0.1:{PORT}-:80", "--no-dump"]
        if no_reboot:
            cmd.append("--no-reboot")
        emu = Emulator(cmd, env, self.logs / f"api-{self.stamp}-{label}.log", self.fail_rx, label)
        emu.start()
        return emu, Api(emu)

    # -- boot A ----------------------------------------------------------------

    def run_a(self):
        emu, api = self.boot("A", self.a.card, True, self.card_out)
        self.emu = emu
        try:
            self.boot_checks(emu, api)
            self.basics(api)
            self.files(api)
            self.play(emu, api)
            self.www(api)
            if self.a.connections > 1:
                self.bursts(emu, api)
            self.load_rounds(emu, api)
            self.cache_delete(emu, api)
            self.ota_refusals(api)
            self.final_a(emu, api)
        finally:
            if not emu.done:
                emu.kill()
        self.report_console(emu)

    def boot_checks(self, emu, api):
        st = api.wait_up(240)
        sha = emu.find(re.compile(r"ELF file SHA256:\s+([0-9a-f]+)"))
        self.expect(sha and self.a.want_sha.startswith(sha[0][1][1]), "boot", f"the booted app is not {self.a.image}")
        self.expect(st.get("wifi") == "sta" and st.get("ip") == "10.0.2.15", "boot", f"not a station on the virtual AP: wifi {st.get('wifi')}, ip {st.get('ip')}")
        self.expect((st.get("sd") or {}).get("mounted") is True, "boot", "no card mounted")
        self.ok("boot", f"{st.get('version')} ({st.get('variant')}) answers on 127.0.0.1:{PORT} as {st.get('ip')}, card mounted, settings on the {st['sd'].get('settings_store')}")
        # The first FAT index build (~20 s after boot, a few s on the synthetic card).
        i, m = emu.wait_for(BUILT, 240, what="the first `fat index: built`")
        self.expect(m[2] == "false", "boot", f"the first index build left it stale: {emu.lines[i][:200]}")
        self.ok("first index build", f"{m[1]} records, stale false")

    def basics(self, api):
        st = api.status()
        for key in ("heap", "now_playing", "sd", "www"):
            self.expect(key in st, "status", f"no `{key}` in /api/status")
        self.expect(st["www"].get("ok") is True, "status", f"the web app in flash is not ok: {st['www']}")
        _, cfg = api.json("GET", "/api/config")
        self.dwell0 = int(cfg["dwell_max"])
        self.dwell_set = self.dwell0 + 7
        _, r = api.json("PUT", "/api/config", {"dwell_max": self.dwell_set})
        self.expect(r.get("ok") is True, "config", f"PUT /api/config answered {r}")
        _, cfg2 = api.json("GET", "/api/config")
        self.expect(cfg2.get("dwell_max") == self.dwell_set, "config", f"dwell_max read back {cfg2.get('dwell_max')}, set {self.dwell_set}")
        api.call("PUT", "/api/config", {"dwell_max": "seven"}, want=(400,))
        _, cfg3 = api.json("GET", "/api/config")
        self.expect(cfg3.get("dwell_max") == self.dwell_set, "config", "a refused PUT changed dwell_max")
        self.ok("status, config", f"GET both; PUT dwell_max {self.dwell0} -> {self.dwell_set} read back; a bad value 400, nothing changed")

    def files(self, api):
        g = self.gifs
        d = "/gifs/EmuApi"
        self.expected = {}  # card path -> bytes, for the read-backs and the card check
        self.absent = set()

        def put(path, data, new=False, want=(200,)):
            code, body = api.call("PUT", "/api/files" + q(path) + ("?new=1" if new else ""), data, want=want, timeout=120)
            if code == 200:
                r = json.loads(body)
                self.expect(r.get("ok") is True and r.get("bytes") == len(data), "upload", f"PUT {path}: {r}")
            return code

        def get(path):
            return api.call("GET", q(path), want=(200, 404))

        put(f"{d}/emu-a.gif", g[0])
        put(f"{d}/emu-b.gif", g[1], new=True)
        put(f"{d}/emu-b.gif", g[2], new=True, want=(409,))
        put(f"{d}/emu-a.gif", g[2])  # a replace
        put(f"{d}/emu-c.gif", g[3])
        put(f"{d}/emu-d.gif", g[4], new=True)
        big = blob(1, 300 * 1024)
        put(f"{d}/emu-big.bin", big)
        self.expected |= {f"{d}/emu-a.gif": g[2], f"{d}/emu-b.gif": g[1], f"{d}/emu-big.bin": big}
        for p, want in [(f"{d}/emu-a.gif", g[2]), (f"{d}/emu-b.gif", g[1]), (f"{d}/emu-c.gif", g[3]), (f"{d}/emu-big.bin", big)]:
            code, data = get(p)
            self.expect(code == 200 and data == want, "read back", f"GET {p}: HTTP {code}, {len(data)} B, {'same' if data == want else 'DIFFERENT'} bytes")
        self.ok("uploads", f"PUT 5 files (a 300 KB one), ?new=1 created one and answered 409 over an existing one (left as it was), a replace; read back byte for byte")

        # Moves: to another folder, onto an existing name (409), from nowhere (404).
        _, r = api.json("POST", "/api/files/move", {"from": f"{d}/emu-c.gif", "to": "/gifs/EmuApi2/emu-c2.gif"})
        self.expect(r.get("ok") is True, "move", f"{r}")
        self.expect(get(f"{d}/emu-c.gif")[0] == 404, "move", "the old name still answers")
        code, data = get("/gifs/EmuApi2/emu-c2.gif")
        self.expect(code == 200 and data == g[3], "move", "the moved file does not read back the same")
        api.call("POST", "/api/files/move", {"from": f"{d}/emu-d.gif", "to": f"{d}/emu-a.gif"}, want=(409,))
        api.call("POST", "/api/files/move", {"from": f"{d}/nope.gif", "to": f"{d}/nope2.gif"}, want=(404,))
        self.expected["/gifs/EmuApi2/emu-c2.gif"] = g[3]
        self.absent.add(f"{d}/emu-c.gif")
        # Deletes: gone after, a second one 404.
        api.call("DELETE", "/api/files" + q(f"{d}/emu-d.gif"))
        self.expect(get(f"{d}/emu-d.gif")[0] == 404, "delete", "the deleted file still answers")
        api.call("DELETE", "/api/files" + q(f"{d}/emu-d.gif"), want=(404,))
        self.absent.add(f"{d}/emu-d.gif")
        # An emptied folder, for EMPTY_FOLDER at the end of boot A.
        put("/gifs/EmuApiEmpty/e.gif", g[0])
        api.call("DELETE", "/api/files" + q("/gifs/EmuApiEmpty/e.gif"))
        self.absent.add("/gifs/EmuApiEmpty/e.gif")
        self.ok("move, delete", "moved to a new folder (read back the same, the old name 404); onto an existing name 409, from nowhere 404; deleted (404 after, a second delete 404)")
        self.listing(api, d, {"emu-a.gif", "emu-b.gif", "emu-big.bin"})
        self.listing(api, "/gifs/EmuApi2", {"emu-c2.gif"})

    def listing(self, api, d, want: set):
        _, walk = api.json("GET", f"/api/files?dir={q(d)}&sizes=1")
        got = {e["name"]: e.get("size") for e in walk["entries"] if not e.get("dir")}
        self.expect(set(got) == want, "listing", f"{d} lists {sorted(got)}, wanted {sorted(want)}")
        for n, size in got.items():
            exp = self.expected.get(f"{d}/{n}")
            self.expect(exp is None or size == len(exp), "listing", f"{d}/{n}: size {size}, wrote {len(exp)}")
        _, names = api.json("GET", f"/api/files?dir={q(d)}&names=1")
        gifs = {e["name"] for e in names["entries"] if not e.get("dir")}
        want_gifs = {n for n in want if n.endswith(".gif")}
        self.expect(want_gifs <= gifs and gifs <= want, "listing", f"{d} ?names=1 lists {sorted(gifs)}, wanted {sorted(want_gifs)}")
        self.ok("listing", f"{d}: {sorted(want)} with their sizes; ?names=1 the same GIFs ({'names file' if names.get('names') else 'a walk'})")

    def wait_status(self, api, cond, timeout, what):
        end = time.time() + timeout
        st = None
        while time.time() < end:
            st = api.status()
            if cond(st):
                return st
            time.sleep(0.5)
        raise LaneFail(f"{what} not seen in /api/status within {timeout} s wall (now playing {((st or {}).get('now_playing') or {}).get('path')!r})")

    def play(self, emu, api):
        p = "/gifs/EmuApi/emu-a.gif"
        mark = emu.mark()
        api.json("POST", "/api/play", {"path": p})
        emu.wait_for(re.compile(re.escape(f"] {p} <- original")), 60, mark, f"`{p} <- original` on the console")
        self.wait_status(api, lambda s: s["now_playing"]["path"] == p, 30, f"now_playing {p}")
        formula = "@formula?psb=" + self.a.psb.read_text().strip()
        api.json("POST", "/api/play", {"path": formula})
        st = self.wait_status(api, lambda s: s["now_playing"]["path"].startswith("@formula") and (s["now_playing"].get("formula") or {}).get("frame_ms", 0) > 0,
                              60, "the Formula playing (now_playing.formula.frame_ms)")
        fms = st["now_playing"]["formula"]
        # A Formula on the card, as the web app saves one: its source and the compiled .psb beside it.
        b64 = self.a.psb.read_text().strip()
        psb = base64.urlsafe_b64decode(b64 + "=" * (-len(b64) % 4))
        src = (self.a.formula_src.read_bytes() if getattr(self.a, "formula_src", None) else b"# compiled by the API lane\n")
        for path, data in (("/formulas/emu-api.psi", src), ("/formulas/emu-api.psb", psb)):
            api.call("PUT", "/api/files" + path, data)
            self.expect(api.call("GET", path)[1] == data, "play", f"{path} does not read back the same")
            self.expected[path] = data
        api.json("POST", "/api/play", {"path": "/formulas/emu-api.psi"})
        self.wait_status(api, lambda s: s["now_playing"]["path"].startswith("/formulas/emu-api.psi"), 60, "the card's Formula playing")
        api.call("POST", "/api/play", {"path": "/gifs/EmuApi/nope.gif"}, want=(404,))
        api.call("POST", "/api/play", {"path": "gifs/relative.gif"}, want=(400,))
        before = api.status()["now_playing"]["ordinal"]
        api.json("POST", "/api/skip")
        self.wait_status(api, lambda s: s["now_playing"]["ordinal"] != before, 60, "a skip")
        _, r = api.json("POST", "/api/pause")
        self.expect(r.get("paused") is True, "pause", f"{r}")
        # (/api/status is answered from its last answer for a moment: STATUS_CACHE.)
        self.wait_status(api, lambda s: s.get("paused") is True, 30, "paused")
        _, r = api.json("POST", "/api/pause")
        self.expect(r.get("paused") is False, "pause", f"resume answered {r}")
        self.wait_status(api, lambda s: s.get("paused") is False, 30, "playing again")
        self.ok("play", f"an uploaded GIF (from the original), a Formula by psb (frame {fms.get('frame_ms')} ms, jit {fms.get('jit')}), a Formula saved to the card (.psi and .psb, read back) by its path; a missing file 404, a relative path 400; skip; pause and resume")

    def www(self, api):
        pack = self.a.www_pack.read_bytes()
        _, r = api.json("PUT", "/api/www", pack, timeout=300)
        self.expect(r.get("ok") is True and r.get("error") is None, "www", f"PUT /api/www answered {r}")
        _, info = api.json("GET", "/api/www")
        code, headers, page = api.raw("GET", "/")
        ctype = {k.lower(): v for k, v in headers.items()}.get("content-type", "")
        self.expect(code == 200 and ctype.startswith("text/html") and len(page) > 200, "www", f"GET /: HTTP {code}, {ctype}, {len(page)} B")
        self.ok("www pack", f"PUT {len(pack)} B: {info.get('version')}, {info.get('entries')} entries, ok; GET / serves it")

    # -- simultaneous uploads (the spare's wedge recipe) -------------------------

    def bursts(self, emu, api):
        """The spare's wedge recipe (wedge.sh) in the emulator: rounds of 2, 2, 2, 5, 5, 5 uploads of
        300 KB at once (`curl -T`, as there: curl stops sending at an early reply) to
        /gifs/_wedge/bN.bin, with a status read every second beside them. Each upload is answered
        200, or 503 with Retry-After and then 200 when asked again (as the web app does): never a
        408, never no answer; every status read answers; the network answers after every round (the
        wedge: no TCP, ping or ARP answered again until a reset); and, 20 s after the last round,
        lwIP holds no received frame (`netdiag: sum`, emu-census builds)."""
        work = self.logs / f"burst-{self.stamp}"
        work.mkdir(parents=True, exist_ok=True)
        src = work / "blob.bin"
        src.write_bytes(blob(77, 300 * 1024))
        log, codes = [], {}
        for rnd, n in enumerate(BURST_ROUNDS, 1):
            errors: list[str] = []
            lock = threading.Lock()

            def one(i):
                url = f"http://127.0.0.1:{PORT}/api/files/gifs/_wedge/b{i}.bin"
                hdr = work / f"h{i}.txt"
                t0, tries = time.time(), 0
                while True:
                    tries += 1
                    r = subprocess.run(["curl", "-s", "-o", "/dev/null", "-D", str(hdr), "-w", "%{http_code}", "-m", "90",
                                        "-T", str(src), url], capture_output=True, text=True)
                    code = int(r.stdout.strip() or 0)
                    with lock:
                        codes[code] = codes.get(code, 0) + 1
                    head = hdr.read_text(errors="replace") if hdr.exists() else ""
                    after = re.search(r"(?im)^retry-after:\s*(\d+)", head)
                    if code == 503 and after and time.time() - t0 < 120:
                        time.sleep(min(int(after[1]), 5))
                        continue
                    with lock:
                        log.append(f"r{rnd} up{i} {code} {time.time() - t0:.1f}s x{tries}")
                        if code != 200:
                            errors.append(f"up{i}: HTTP {code} (curl exit {r.returncode}) after {time.time() - t0:.1f} s")
                    return

            threads = [threading.Thread(target=one, args=(i,), daemon=True) for i in range(1, n + 1)]
            for t in threads:
                t.start()
            while any(t.is_alive() for t in threads):
                emu.check()
                try:
                    code, _, _ = api.raw("GET", "/api/status", timeout=15)
                    if code != 200:
                        errors.append(f"status beside round {rnd}: HTTP {code}")
                except LaneFail as e:
                    errors.append(f"status beside round {rnd}: {e}")
                    break
                time.sleep(1)
            for t in threads:
                t.join(200)
            try:
                api.raw("GET", "/api/status", timeout=15)
            except LaneFail as e:
                errors.append(f"after round {rnd}: {e}")
            if errors:
                raise LaneFail(f"burst round {rnd} ({n} uploads at once): " + "; ".join(errors[:4]) + f" (so far: {', '.join(log[-12:])})")
        for i in range(1, max(BURST_ROUNDS) + 1):
            api.call("DELETE", "/api/files" + q(f"/gifs/_wedge/b{i}.bin"), want=(200, 404))
        api.call("DELETE", "/api/files" + q("/gifs/_wedge"))
        shutil.rmtree(work, ignore_errors=True)
        # Nothing left holding the WiFi driver's receive buffers once it is quiet.
        mark = emu.mark()
        emu.wait_for(NETDIAG_SUM, 60, mark, "a `netdiag: sum` line after the bursts")
        time.sleep(25)
        sums = [(int(m[1]), int(m[2])) for _, m in emu.find(NETDIAG_SUM, mark)]
        self.expect(sums and sums[-1][1] == 0, "bursts", f"lwIP still holds received frames after the bursts (sockets, frames per walk): {sums}")
        self.ok("bursts", f"rounds of {'/'.join(map(str, BURST_ROUNDS))} 300 KB uploads at once beside status reads: every one 200 in the end (answers {dict(sorted(codes.items()))}), the network answering after each; quiet after: {sums[-1][0]} sockets, {sums[-1][1]} frames held")

    # -- the load -------------------------------------------------------------

    def load_rounds(self, emu, api):
        quiet = []
        for k in range(ROUNDS):
            counts = self.load(api, k)
            self.ok(f"load {k + 1}/{ROUNDS}", ", ".join(f"{n} {v}" for n, v in counts.items()))
            start = emu.mark()
            end = time.time() + QUIET_S
            while time.time() < end:
                emu.check()
                time.sleep(1)
            quiet.append([(int(m[1]), int(m[2])) for _, m in emu.find(CENSUS, start)])
        self.expect(all(quiet), "census", f"census lines per quiet spell {[len(c) for c in quiet]}: is this an emu-census image?")
        b0, y0 = min(c[0] for c in quiet[0]), min(c[1] for c in quiet[0])
        b1, y1 = min(c[0] for c in quiet[-1]), min(c[1] for c in quiet[-1])
        mids = ", ".join(f"{min(c[0] for c in s)} blocks / {min(c[1] for c in s)} B" for s in quiet)
        self.expect(b1 - b0 <= LEAK_BLOCKS and y1 - y0 <= LEAK_BYTES, "census",
                    f"the heap grew across the load rounds: least blocks {b0} -> {b1}, allocated {y0} -> {y1} B (each quiet spell: {mids})")
        self.ok("census", f"flat across {ROUNDS} load rounds: least blocks {b0} -> {b1}, allocated {y0} -> {y1} B (each quiet spell: {mids})")

    def load(self, api, k):
        """display-stress's loads at once for LOAD_S: status polls, settings saves, uploads
        read back, saves into a big folder, listings. Every answer as expected."""
        stop = threading.Event()
        errors: list[str] = []
        counts = {"status": 0, "saves": 0, "uploads": 0, "card saves": 0, "listings": 0}
        lock = threading.Lock()
        values = [self.dwell0 + 7, self.dwell0 + 8]

        def loop(name, fn, gap):
            n = 0
            while not stop.is_set():
                try:
                    fn(n)
                    with lock:
                        counts[name] += 1
                except LaneFail as e:
                    with lock:
                        errors.append(f"{name}: {e}")
                    stop.set()
                except Exception as e:  # noqa: BLE001
                    with lock:
                        errors.append(f"{name}: {e!r}")
                    stop.set()
                n += 1
                stop.wait(gap)

        def status(_):
            st = api.status()
            free = (st.get("heap") or {}).get("free")
            if free is not None:
                self.heap_free_min = free if self.heap_free_min is None else min(self.heap_free_min, free)

        def save(n):
            v = values[n % 2]
            _, r = api.json("PUT", "/api/config", {"dwell_max": v})
            if r.get("ok") is not True:
                raise LaneFail(f"PUT /api/config answered {r}")
            self.dwell_set = v

        def upload(n):
            data = blob(1000 * k + n, 64 * 1024)
            p = f"/gifs/EmuApi/up-{n % 3}.bin"
            api.call("PUT", "/api/files" + q(p), data, timeout=120)
            code, back = api.call("GET", q(p))
            if back != data:
                raise LaneFail(f"GET {p}: {len(back)} B, not the bytes written")
            api.call("DELETE", "/api/files" + q(p))

        def card_save(n):
            p = f"/gifs/Consoles/zz-emu-api-{k}-{n}.tmp"
            api.call("PUT", "/api/files" + q(p) + "?new=1", blob(7 * n + k, 24 * 1024), timeout=120)
            api.call("DELETE", "/api/files" + q(p))
            self.absent.add(p)

        def listing(_):
            _, r = api.json("GET", "/api/files?dir=/gifs/Consoles&names=1")
            if r.get("count", len(r.get("entries", []))) < 500:
                raise LaneFail(f"/gifs/Consoles lists {r.get('count')} entries")

        def card_work(n):
            # One connection for the rest, in turn: an upload read back; every second turn a
            # settings save, every third a save into the big folder, every fourth a listing.
            upload(n)
            if n % 2 == 1:
                save(n // 2)
                counts["saves"] += 1
            if n % 3 == 2:
                card_save(n // 3)
                counts["card saves"] += 1
            if n % 4 == 3:
                listing(n)
                counts["listings"] += 1

        def serial_work(n):
            # One connection at a time: a status read between the card transfers.
            status(n)
            counts["status"] += 1
            card_work(n)

        if self.a.connections == 5:
            loads = [("status", status, 0.25), ("saves", save, 2), ("uploads", upload, 0), ("card saves", card_save, 3), ("listings", listing, 4)]
        elif self.a.connections == 2:
            loads = [("status", status, 0.5), ("uploads", card_work, 0)]
        else:
            loads = [("uploads", serial_work, 0)]
        threads = [threading.Thread(target=loop, args=a, daemon=True) for a in loads]
        for t in threads:
            t.start()
        stop.wait(LOAD_S)
        stop.set()
        for t in threads:
            t.join(180)
        self.emu.check()
        if errors:
            raise LaneFail(f"load round {k + 1}: " + "; ".join(errors[:3]))
        # The last setting saved is what the card file must hold.
        _, cfg = api.json("GET", "/api/config")
        self.expect(cfg.get("dwell_max") == self.dwell_set, "load", f"dwell_max {cfg.get('dwell_max')} after the saves, last saved {self.dwell_set}")
        return counts

    # -- a recursive delete under /cache while the index builds ------------------

    def cache_delete(self, emu, api):
        st0 = api.status()["sd"]["fat_index"]
        for attempt, cat in enumerate(["Consoles", "Other", "Arcade"], 1):
            # A write at /cache's root drops the index (`release_for_write`: `dir_changed`):
            # rebuilt after 30 s quiet. (A GIF written or deleted is only noted in the
            # index's delta.)
            mark = emu.mark()
            t = f"/cache/emu-api-touch-{attempt}.tmp"
            api.call("PUT", "/api/files" + q(t), b"x" * 512)
            api.call("DELETE", "/api/files" + q(t))
            self.absent.add(t)
            emu.wait_for(BUILD_STARTED, 180, mark, "a `fat index: build started` after the writes")
            builds0 = api.status()["sd"]["fat_index"]["builds"]
            started = emu.find(BUILD_STARTED, mark)[-1][0]
            rounds, removed, done = 0, 0, False
            first_del = emu.mark()
            while not done:
                _, r = api.json("DELETE", f"/api/files/cache/{cat}?recursive=1", timeout=120)
                rounds += 1
                removed += r["removed"]
                done = r["done"] is True
                self.expect(rounds < 40, "cache delete", f"/cache/{cat} not done after {rounds} rounds")
            # (The line may reach the console just after the answer.)
            emu.wait_for(re.compile(DELETE_LINE.pattern.replace(r"(true|false)", "true")), 30, first_del, "the delete's `done true` line")
            dels = emu.find(DELETE_LINE, first_del)
            self.absent.add(f"/cache/{cat}")
            built_between = emu.find(BUILT, started, dels[0][0])
            if not built_between:
                break
            self.notes.append(f"attempt {attempt}: the build finished before the delete of /cache/{cat} reached it; again")
        else:
            raise LaneFail("cache delete: three builds finished before the delete reached them")
        self.ok("cache delete", f"/cache/{cat} removed in {rounds} round(s) ({removed} entries) while the index built (`build started` at line {started}, no `built` before the delete)")
        i, m = emu.wait_for(BUILT, 240, dels[-1][0], "the index rebuilt after the delete")
        self.expect(m[2] == "false", "cache delete", f"the rebuild left the index stale: {emu.lines[i][:200]}")
        warns = emu.find(FATIDX_WARN, started)
        self.expect(not warns, "cache delete", f"fat index warnings: {warns[0][1].string[:200] if warns else ''}")
        st = self.wait_status(api, lambda s: not s["sd"]["fat_index"]["building"], 60, "the build over")
        fi = st["sd"]["fat_index"]
        self.expect(fi["stale"] is False and fi["files"] == int(m[1]), "cache delete", f"fat_index after the rebuild: {fi}")
        self.expect(fi["builds"] >= builds0 + 2, "cache delete", f"builds {builds0} -> {fi['builds']}: the build under way was not dropped and run again")
        self.expect(fi["files"] < st0["files"], "cache delete", f"records {st0['files']} -> {fi['files']}: the copies deleted are still indexed")
        api.call("GET", f"/api/files?dir=/cache/{cat}", want=(404,))
        # One of its GIFs plays from the original now.
        _, names = api.json("GET", f"/api/files?dir=/gifs/{cat}&names=1")
        gif = f"/gifs/{cat}/" + sorted(e["name"] for e in names["entries"] if e["name"].endswith(".gif"))[0]
        mark = emu.mark()
        api.json("POST", "/api/play", {"path": gif})
        emu.wait_for(re.compile(re.escape(f"] {gif} <- original")), 60, mark, f"`{gif} <- original`")
        self.ok("index rebuilt", f"{st0['files']} -> {fi['files']} records, builds {builds0} -> {fi['builds']} (the one under way dropped), stale false, no fat index warnings; /cache/{cat} 404; {gif} plays from the original")

    # -- OTA refusals and the end of boot A -----------------------------------

    def ota_refusals(self, api):
        _, info = api.json("GET", "/api/firmware")
        self.expect(info.get("variant") == "4mb", "firmware", f"GET /api/firmware: {info}")
        b = bytearray(self.a.bundle.read_bytes())
        sig = bytearray(b)
        sig[100] ^= 0x01  # in the Ed25519 signature (bytes 76..140)
        api.call("PUT", "/api/firmware", sig, want=(422,), timeout=300)
        img = bytearray(b)
        img[140 + 4096] ^= 0x01  # in the app image: its SHA-256 no longer matches
        api.call("PUT", "/api/firmware", img, want=(422,), timeout=300)
        _, info2 = api.json("GET", "/api/firmware")
        self.expect(info2.get("pending") is None, "firmware", f"a refused bundle left something pending: {info2}")
        self.ok("ota refusals", f"GET /api/firmware {info.get('version')}; a bundle with a flipped signature bit 422 at the header, one with a flipped image bit 422 after its upload; nothing pending")

    def final_a(self, emu, api):
        for p, data in self.expected.items():
            code, back = api.call("GET", q(p), want=(200, 404))
            self.expect(code == 200 and back == data, "read back", f"GET {p} at the end: HTTP {code}, {'same' if back == data else 'different'} bytes")
        for p in self.absent:
            if p.startswith("/gifs/"):
                self.expect(api.call("GET", q(p), want=(200, 404))[0] == 404, "read back", f"{p} is back")
        # The index ends current: rebuilt (30 s after the last write) if anything made it stale.
        st = self.wait_status(api, lambda s: s["sd"]["fat_index"]["stale"] is False and not s["sd"]["fat_index"]["building"], 120,
                              "the FAT index current (stale false, not building)")
        # EMPTY_FOLDER: deleting an emptied /gifs folder marks the index stale at once (FatFs's
        # unlink removes an empty directory too, so the firmware tells a directory apart first),
        # and the index is rebuilt current without it.
        api.call("DELETE", "/api/files" + q("/gifs/EmuApiEmpty"))
        fi = api.status()["sd"]["fat_index"]
        self.expect(fi["stale"] is True, "empty folder", f"DELETE /gifs/EmuApiEmpty (emptied): the index not stale after: {fi}")
        self.expect(api.call("GET", "/api/files?dir=" + q("/gifs/EmuApiEmpty"), want=(200, 404))[0] == 404, "empty folder", "the folder still lists")
        st = self.wait_status(api, lambda s: s["sd"]["fat_index"]["stale"] is False and not s["sd"]["fat_index"]["building"], 180,
                              "the FAT index current again after the emptied folder went")
        self.ok("empty folder", f"DELETE of an emptied /gifs folder: gone, the index stale at once, rebuilt current ({st['sd']['fat_index']['files']} records)")
        fi = st["sd"]["fat_index"]
        self.final_status = st
        _, r = api.json("POST", "/api/reboot")
        self.expect(r.get("rebooting") is True, "reboot", f"{r}")
        if not emu.wait_exit(120):
            raise LaneFail("boot A: the emulator did not stop at the reboot (--no-reboot)")
        time.sleep(0.5)
        self.expect(self.card_out.exists(), "card", f"the emulator wrote no card to {self.card_out}")
        h = st["heap"]
        self.ok("end of boot A", f"{len(self.expected)} files read back the same, the gone ones 404; index stale {fi['stale']}, {fi['files']} records; heap {h['free']} B free (floor {h['floor']} B); rebooted, the card written out")

    def report_console(self, emu):
        hb = emu.find(HEARTBEAT)
        httpd = [int(m[1]) for _, m in emu.find(HTTPD_STACK)]
        worker = [int(m[1]) for _, m in emu.find(WORKER_STACK)]
        st = getattr(self, "final_status", None) or {}
        parts = []
        if hb:
            parts.append(f"heartbeat heap floor {min(int(m[2]) for _, m in hb)} B (least free {min(int(m[1]) for _, m in hb)} B over {len(hb)} heartbeats)")
            parts.append(f"main stack {min(int(m[3]) for _, m in hb)} B unused, log drain {min(int(m[4]) for _, m in hb)} B")
        if self.heap_free_min is not None:
            parts.append(f"least heap free in a status poll under load {self.heap_free_min} B")
        if httpd:
            parts.append(f"httpd stack {min(httpd)} B unused")
        if st.get("stream_stack") is not None:
            parts.append(f"stream task stack {st['stream_stack']} B unused")
        if worker:
            parts.append(f"network worker {min(worker)} B unused (at the index builds)")
        self.ok("heap and stacks (boot A)", "; ".join(parts) or "none reported")

    # -- the card boot A left ------------------------------------------------------

    def card_checks(self):
        img = self.card_out
        env = {**os.environ, "MTOOLS_SKIP_CHECK": "1"}
        r = subprocess.run([sys.executable, str(Path(__file__).with_name("fatcheck.py")), str(img)], capture_output=True, text=True)
        self.expect(r.returncode == 0, "fatcheck", r.stdout.strip()[:600] + r.stderr[-300:])
        self.ok("fatcheck", r.stdout.strip().removeprefix("fatcheck: "))

        def read(path):
            p = subprocess.run(["mcopy", "-n", "-i", str(img), f"::{path}", "-"], capture_output=True, env=env)
            return p.stdout if p.returncode == 0 else None

        def exists(path):
            return subprocess.run(["mdir", "-b", "-i", str(img), f"::{path}"], capture_output=True, env=env).returncode == 0

        settings = read("/opendmd.json")
        self.expect(settings is not None, "card", "no /opendmd.json on the card")
        js = json.loads(settings)
        self.expect(js.get("dwell_max") == self.dwell_set, "card", f"/opendmd.json has dwell_max {js.get('dwell_max')}, the last saved was {self.dwell_set}")
        for p, data in self.expected.items():
            self.expect(read(p) == data, "card", f"{p} on the card is not the bytes uploaded")
        for p in sorted(self.absent):
            self.expect(not exists(p), "card", f"{p} is on the card (deleted or moved over the API)")
        left = [p for p in ("/update/firmware.part", "/update/bundle.part", "/gifs/EmuApi/up-0.bin", "/gifs/EmuApi/up-1.bin", "/gifs/EmuApi/up-2.bin") if exists(p)]
        listing = subprocess.run(["mdir", "-/", "-b", "-i", str(img), "::/"], capture_output=True, text=True, env=env).stdout
        left += [l for l in listing.splitlines() if l.endswith(".part") or l.endswith(".tmp")]
        self.expect(not left, "card", f"left behind: {left[:6]}")
        self.ok("the card after boot A", f"/opendmd.json dwell_max {js['dwell_max']} (the last saved); {len(self.expected)} files the bytes uploaded; {len(self.absent)} deleted or moved paths gone; no .part/.tmp left")

    # -- boot B: the card again, the real firmware update ----------------------------

    def run_b(self):
        emu, api = self.boot("B", self.card_out, False, None)
        self.emu = emu
        try:
            st = api.wait_up(240)
            _, cfg = api.json("GET", "/api/config")
            self.expect(cfg.get("dwell_max") == self.dwell_set, "boot B", f"dwell_max {cfg.get('dwell_max')} after the power cycle, saved {self.dwell_set}")
            for p, data in self.expected.items():
                code, back = api.call("GET", q(p), want=(200, 404))
                self.expect(code == 200 and back == data, "boot B", f"GET {p}: HTTP {code}, {'same' if back == data else 'different'} bytes")
            self.ok("boot B", f"from boot A's card: dwell_max {self.dwell_set} kept, {len(self.expected)} files the same")
            want = self.a.bundle.read_bytes()[8:40].rstrip(b"\0").decode()
            mark = emu.mark()
            with emu.cond:
                emu.resets_allowed = emu.resets + 2  # into the updater, back into the firmware
            t0 = time.time()
            _, r = api.json("PUT", "/api/firmware", self.a.bundle.read_bytes(), timeout=600)
            took = time.time() - t0
            self.expect(r.get("ok") is True and r.get("rebooting") is True, "ota", f"PUT /api/firmware answered {r}")
            emu.wait_for(re.compile(r"signature ok; staging it on the card"), 5, mark, "`signature ok`")
            emu.wait_for(re.compile(r"staged and verified .* restarting into the updater"), 60, mark, "`restarting into the updater`")
            emu.wait_for(re.compile(r"rst:0x"), 60, mark, "the restart into the updater")
            back = time.time()
            end = back + 300
            info = None
            while time.time() < end:
                emu.check()
                try:
                    code, _, data = api.raw("GET", "/api/firmware", timeout=5)
                    if code == 200:
                        info = json.loads(data)
                        if info.get("last") and "booted after the updater installed" in info["last"]:
                            break
                except LaneFail:
                    pass
                time.sleep(2)
            self.expect(info and info.get("version") == want and f"installed {want}" in (info.get("last") or ""), "ota",
                        f"the firmware did not come back from the updater with {want}: {info}")
            ulog = [l for l in (info.get("log") or "").splitlines() if l.startswith("updater")]
            self.expect(any("image verified" in l for l in ulog), "ota", f"the updater's log: {ulog}")
            st = api.status()
            self.expect(emu.resets == 3, "ota", f"{emu.resets} resets, wanted 3 (power-on, updater, firmware)")
            self.ok("ota", f"PUT {self.a.bundle.stat().st_size} B in {took:.0f} s wall: signature ok, staged, verified; the updater ({len(ulog)} log lines, '{ulog[-2] if len(ulog) > 1 else ''}') installed it; back as {info['version']} ({st.get('reset_reason')}) in {time.time() - back:.0f} s wall")
        finally:
            emu.kill()


def network_wedged(lane) -> bool:
    """After a failure: the emulator runs, the API does not answer, GIFs still start."""
    emu = getattr(lane, "emu", None)
    if not emu or emu.done or emu.failure:
        return False
    try:
        Api(emu).raw("GET", "/api/status", timeout=10)
        return False
    except LaneFail:
        pass
    try:
        emu.wait_for(GIF_START, 30, emu.mark())
        return True
    except LaneFail:
        return False


def run(a, sim, rom, fail_rx) -> int:
    t0 = time.time()
    lane = Lane(a, sim, rom, fail_rx)
    wall = a.wall or 5400

    def watchdog():
        emu = getattr(lane, "emu", None)
        print(f"api: the {wall} s wall limit: stopping the emulator", flush=True)
        if emu and not emu.done:
            emu.kill()
    timer = threading.Timer(wall, watchdog)
    timer.daemon = True
    timer.start()
    try:
        lane.run_a()
        lane.card_checks()
        lane.run_b()
    except LaneFail as e:
        if network_wedged(lane):
            e = LaneFail(f"{e} -- the network wedged: no TCP answer while the player plays on (RX_BUFFERS in api_lane.py)")
        print(f"EMU_API_FAIL: {e}")
        if lane.card_out.exists():
            print(f"(the card boot A wrote is kept: {lane.card_out})")
        emu = getattr(lane, "emu", None)
        if emu:
            print(f"--- last console lines of {emu.log}:")
            print(emu.tail())
        return 1
    for n in lane.notes:
        print(f"note  {n}")
    lane.card_out.unlink(missing_ok=True)
    print(f"EMU_API_OK ({time.time() - t0:.0f} s wall)")
    return 0
