"""The API lane (`run.py api`, `make emu-api`): the firmware's HTTP API driven from the host.

Two boots of the real full image, each with WiFi credentials seeded into its NVS (a copy of
the image: `seeded_image`), on the virtual access point with the guest's port 80 published on
127.0.0.1. The whole lane runs inside a private network namespace (`unshare -rn`, set up by
run.py): the emulator's NAT has nowhere to go but the loopback, so nothing reaches a real
network or device (mDNS, SNTP, DNS, RePlayOS all fail quietly), and the fixed port never
collides with another run.

Boot A (`--no-reboot`: the emulator stops at the first reset, and writes the card out):
  basics      GET /api/status and /api/config; PUT a setting, read back; a bad PUT is a 400
  long        GET /api/wifi/scan past 0.3 s: a 202 + X-Job, status reads answered meanwhile, a
              second long request a 503, the answer from /api/job, 404 after (LONG_REQUEST)
  stalled     a PUT /api/config whose body stops part way: a 503 + Retry-After after one receive
              timeout, the connection dropped, net.body_stalled counted, nothing saved, and a
              status read sent meanwhile answered (STALLED_BODY)
  files       uploads (PUT /api/files/..., ?new=1 and its 409, a replace), read back byte for
              byte, listings (?names=1 and a walk), moves (and their 409/404), deletes (404 after)
  play        /api/play of an uploaded GIF, a Formula (`@formula?psb=`), the same saved to
              the card (.psi + .psb) by its path; skip, pause and resume
  look        (--look, --look-psb) a card Formula as look_formula: drawn; its .psb written
              anew (a bad one, then the real one) rebuilds the look at once (formula_look.error
              comes, then goes); its .psi deleted clears look_formula and the look
  sent        (--long-psb) Show on panel of a long Formula (Endless road V2, past any path):
              POST /api/play/formula, played as `@formula:~sent` by its name line, live
              edits ~200 ms apart in place (the same start), off Recently played; 413 past
              16 KB, 415 not octet-stream, 409 a live edit once the panel moved on
  www         PUT /api/www with the web app's pack; GET / and a wasm at its ?v= address:
              no-cache, the pack's CRC as ETag, 304 for If-None-Match naming it, 200 for another
  events      GET /api/events (events.rs): two listeners get events (the heap with 0, 1 and 2),
              a third is a 503 + Retry-After; a keep-alive at least every 3 s of device time, a
              skip arrives as an event; both listeners keep getting events through rounds of
              2 and 5 simultaneous 300 KB uploads, status reads beside them answered; a
              listener that stops reading never holds the device up; closed listeners leave
              the list (net.events.listeners back to 0)
  bursts      (--connections 2 or 5, the default 5) a test board's wedge recipe: rounds of 2, 2, 2, 5,
              5, 5 uploads of 300 KB at once beside status reads; each 200, or 503 + Retry-After
              and 200 when asked again; never a 408; the network answering; nothing held after;
              the least free heap of each round (the census image's low watch) over BURST_HEAP_MIN
  load x3     uploads read back, settings saves, saves into a big folder, listings and status
              reads (display-stress's loads), each on its own connection at once by default
              (--connections 2: status polls beside the rest; 1: one at a time), then 60 s quiet:
              the heap census (`emu-census`) of the last quiet spell against the first's
  cache       a recursive delete under /cache while the FAT index builds (the corruption
              hazard fixed in fatidx-heap): the build dropped, rebuilt, not stale
  ota         GET /api/firmware (layout v3); bundles for table v2, with a bad signature, a bad image: 422
  start-up    (boot A's NVS seeded with boot_fx = a bundled Formula, BOOT_GOOD) the start-up show
              was that Formula (`boot show: formula @formula:…`, its frames drawn, then the
              playlist; status boot_formula played); a bundled name that is not there, a
              broken inline one and an 8.3 alias are 400s, nothing saved; the card Formula
              saved in `play` set as boot_formula for the next boot
  end         every file read back again; POST /api/reboot (boot A ends, the card written)
Then the card boot A wrote: fatcheck (no cross-links or lost clusters), the settings file
(/opendmd.json) holds the last setting saved, the files byte for byte, nothing left behind.
Boot B (from that card, rebooting allowed; NVS seeded with a boot_fx that names no Formula,
BOOT_BAD): the setting and the files survived the power cycle; the start-up show fell back
to the wordmark for NVS's bad value, then played the card's Formula once the boot task read
it (the card's settings win); deleting that Formula clears boot_formula (config and status);
PUT /api/firmware with the signed bundle: verified, staged, the updater installs it, the
firmware is back with the bundle's version, and that boot's start-up show is the wordmark.

Fails at once on a panic, Guru Meditation, stack overflow, abort() or a reset not asked for;
prints EMU_API_OK or EMU_API_FAIL with the reason. Correctness only, never timing.
"""
import base64, contextlib, hashlib, json, os, random, re, shutil, signal, socket, struct, subprocess, sys, threading, time, zlib
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
# TCP, ping or ARP answered; a test board showed it too). The netdiag walk (`netdiag:` lines,
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
# BURST_HEAP (net-wedge2): a test board's heap went to 8 KB (ESP-IDF's minimum) under five 300 KB
# uploads at once beside a status read a second, and with it the WiFi driver, which allocates its
# frame buffers from the heap, stopped delivering: refusals 10.5 s late, status reads timing out.
# The emu-census image's low watch (a task reading the free heap every ms, `lowwatch:` every 5 s
# with what was held at the low) put the lows of the bursts in a status read during an upload:
# the whole answer and every part of it at once (~5 KB of Rust strings), its ~3.7 KB of segments,
# and the upload's received frames (up to the WiFi driver's six buffers, ~10 KB). The answer is
# written out in pieces now (http.rs), a status read below 10 KB free is a 503, and a client that
# gave up has its unacknowledged reply aborted, not retransmitted to nobody (netstat.rs).
# Two bounds, per round:
# - BURST_HEAP_MIN, the least free heap: 20 KB. The census image plays at 53-58 KB at its quiet
#   lows (~26 KB above that test board); the bursts' lows were 29.7-45 KB, the player's share moving them
#   by up to ~15 KB (a dissolve's buffers at a switch, the read-ahead ring), so this alone is a
#   coarse check: under 20 KB here a test board would be at nothing.
# - BURST_NET_MAX, what the C side (lwIP, the WiFi driver, FatFs: everything but the Rust
#   allocator, which the player's GIFs dominate) holds at the low above its quiet level in the same
#   rounds: 22 KB. By construction one upload and one status read can hold ~20 KB there (six
#   receive buffers ~10 KB, one reply's segments ~4.4 KB, a few pcbs and sockets, the upload's FIL);
#   measured 11-18 KB. More means something grew per connection: a second reply in flight, refused
#   uploads' frames kept, replies piling up behind a client that left.
BURST_HEAP_MIN = 20 * 1024
BURST_NET_MAX = 22 * 1024
# The heap census: the least of the last quiet spell against the first's (as emu-leak).
LEAK_BLOCKS, LEAK_BYTES = 24, 2048

ANSI = re.compile(r"\x1b\[[0-9;]*m")
GIF_START = re.compile(r"\] (/gifs/\S+\.gif) <- (prepared|original)")
CENSUS = re.compile(r"census: (\d+) blocks in use, (\d+) B allocated")
CENSUS_TOTAL = re.compile(r"census: \d+ blocks in use, (\d+) B allocated, (\d+) B free")
BUILT = re.compile(r"fat index: built (\d+) records .*stale (true|false)")
BUILD_STARTED = re.compile(r"fat index: build started")
FATIDX_WARN = re.compile(r"^W \(\d+\) \S+: fat index: (?!.*no raw reads)|raw read of .* differs")
DELETE_LINE = re.compile(r"DELETE (/cache/\S+) \(recursive\): (\d+) entries removed, done (true|false)")
HEARTBEAT = re.compile(r"heartbeat: heap (\d+) B free \(floor (\d+) B.*main stack (\d+) B unused.*drain stack (\d+) B unused")
HTTPD_STACK = re.compile(r"httpd task: stack high-water (\d+) B unused")
NETDIAG_SUM = re.compile(r"netdiag: sum: (\d+) sockets open, \d+ TIME_WAIT pcbs, (\d+) rx pbufs held")
LOWWATCH = re.compile(r"lowwatch: window low (\d+) B free \(rust (-?\d+) B, sockets (\d+), uploads (\d+)\); now (\d+) B free, rust (-?\d+) B")
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


def app_partition(image: bytes, subtype: int) -> tuple[int, int]:
    """An app partition's offset and size from the image's table at 0x8000 (0x10: ota_0)."""
    for off in range(0x8000, 0x8000 + 0xC00, 32):
        e = image[off:off + 32]
        if e[:2] != b"\xaa\x50":
            break
        typ, sub, start, size = struct.unpack_from("<BBII", e, 2)
        if typ == 0 and sub == subtype:
            return start, size
    raise LaneFail(f"no app partition 0x{subtype:02x} in the image's partition table")


def with_updater(image: Path, updater: Path, out: Path) -> Path:
    """A copy of `image` with `updater` in its ota_0 (the rest of the slot erased)."""
    data = bytearray(image.read_bytes())
    start, size = app_partition(bytes(data), 0x10)
    up = updater.read_bytes()
    if len(up) > size:
        raise LaneFail(f"{updater} ({len(up)} B) does not fit ota_0 ({size} B)")
    data[start:start + size] = up + b"\xff" * (size - len(up))
    out.write_bytes(data)
    return out


def nvs_gen_python() -> Path:
    found = sorted(Path.home().glob(".espressif/python_env/idf5.3_py3*_env/bin/python"))
    if not found:
        raise LaneFail("no ESP-IDF v5.3 python env under ~/.espressif/python_env (it has nvs_partition_gen)")
    return found[-1]


def seeded_image(image: Path, out_dir: Path, ssid="demo", password="demo-password", extra: dict | None = None) -> Path:
    """A copy of `image` whose NVS holds WiFi credentials (namespace `opendmd`, `wifi_ssid`/
    `wifi_pass`) and the `extra` string keys, written with IDF v5.3's nvs_partition_gen.
    Cached by the image's bytes and the keys."""
    data = image.read_bytes()
    extra = extra or {}
    digest = hashlib.sha256(data + repr(sorted(extra.items())).encode()).hexdigest()[:16]
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
    rows = "".join(f"{k},data,string,{v}\n" for k, v in extra.items())
    csv.write_text(f"key,type,encoding,value\nopendmd,namespace,,\nwifi_ssid,data,string,{ssid}\nwifi_pass,data,string,{password}\n{rows}")
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


# The start-up show's Formula seeded into NVS (`boot_fx`, nvs_config.rs): boot A a bundled
# one (Starfield: light, no card needed), boot B one that names nothing (the wordmark plays).
BOOT_GOOD = "@formula:starfield"
BOOT_BAD = "@formula:no-such-formula"
BOOT_CARD = "/formulas/emu-api.psi"
BOOT_SHOW = re.compile(r"boot show: (.*)")


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

    def raw(self, method, path, body=None, timeout=60.0, extra=None, follow=True):
        """One request; a long one's `202` + `X-Job` is followed through `GET /api/job` to its
        answer, as the web app does (`follow=False`: the 202 itself)."""
        code, headers, got = self._raw(method, path, body, timeout, extra)
        end = time.time() + 120
        while follow and code == 202 and headers.get("X-Job") and time.time() < end:
            time.sleep(0.5)
            code, headers, got = self._raw("GET", f"/api/job?id={headers['X-Job']}", None, timeout, None)
        return code, headers, got

    def _raw(self, method, path, body=None, timeout=60.0, extra=None):
        data, headers = None, {}
        if isinstance(body, (bytes, bytearray)):
            data, headers = bytes(body), {"Content-Type": "application/octet-stream"}
        elif body is not None:
            data, headers = json.dumps(body).encode(), {"Content-Type": "application/json"}
        headers.update(extra or {})
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
        self.card_gif = "/gifs/Other/" + sorted(root.glob("*.gif"))[0].name if self.gifs else None
        if len(self.gifs) < 6:
            raise LaneFail(f"fewer than 6 GIFs in {root} (run mkcard.py)")

    def ok(self, name, detail):
        line = f"pass  {name}: {detail}"
        self.passes.append(line)
        print(line, flush=True)

    def expect(self, cond, name, detail):
        if not cond:
            raise LaneFail(f"{name}: {detail}")

    def boot(self, label, card: Path, no_reboot: bool, card_out: Path | None, image: Path | None = None):
        image = image or self.a.seeded
        env = {**os.environ, "ESP32SIM_SD_IMG": str(card)}
        env.pop("ESP32SIM_SD_OUT", None)
        if card_out:
            card_out.unlink(missing_ok=True)
            env["ESP32SIM_SD_OUT"] = str(card_out)
        cmd = [str(self.sim), "--chip", "esp32", "--board", "esp32dev-sdspi", "--boot", "rom", "--rom", str(self.rom),
               "--flash-image", str(image), "--flash-mb", str(max(4, image.stat().st_size >> 20)),
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
            if os.environ.get("EMU_API_ONLY") == "sent":
                # (A diagnostic shortcut: Show on panel alone, then stop.)
                self.files(api)
                self.sent(emu, api)
                return
            if os.environ.get("EMU_API_ONLY") == "look":
                # (A diagnostic shortcut: the look's Formula alone, then stop.)
                self.look(emu, api)
                return
            if os.environ.get("EMU_API_ONLY") == "events":
                # (A diagnostic shortcut: the event stream alone, then stop.)
                self.events(emu, api)
                return
            if os.environ.get("EMU_API_ONLY") == "bursts":
                # (A diagnostic shortcut: the bursts alone, then stop.)
                self.bursts(emu, api)
                return
            if os.environ.get("EMU_API_ONLY") == "stalled":
                # (A diagnostic shortcut: the stalled body alone, then stop.)
                self.dwell_set = int(api.json("GET", "/api/config")[1]["dwell_max"])
                self.stalled_body(api)
                return
            self.basics(api)
            self.stalled_body(api)
            self.long_request(api)
            self.files(api)
            self.play(emu, api)
            if getattr(self.a, "look_psb", None):
                self.look(emu, api)
            if getattr(self.a, "long_psb", None):
                self.sent(emu, api)
            self.www(api)
            self.events(emu, api)
            if self.a.connections > 1:
                self.bursts(emu, api)
            self.load_rounds(emu, api)
            self.cache_delete(emu, api)
            self.ota_refusals(api)
            self.startup_a(emu, api)
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
        # The location: a pair, rounded to 0.01°, kept by the device (never the card's file,
        # never /api/status); one half alone, or off the globe, is a 400 with nothing saved.
        self.expect(cfg.get("lat", 0) is None and cfg.get("lon", 0) is None, "location", f"no location yet, read lat {cfg.get('lat')} lon {cfg.get('lon')}")
        api.json("PUT", "/api/config", {"lat": 51.5074, "lon": -0.1278})
        _, c = api.json("GET", "/api/config")
        self.expect((c.get("lat"), c.get("lon")) == (51.51, -0.13), "location", f"read back {c.get('lat')}, {c.get('lon')}; set 51.5074, -0.1278")
        for bad in ({"lat": 10}, {"lon": 10}, {"lat": 91, "lon": 0}, {"lat": 0, "lon": -180.5}, {"lat": None, "lon": 3}):
            api.call("PUT", "/api/config", bad, want=(400,))
        _, c = api.json("GET", "/api/config")
        self.expect((c.get("lat"), c.get("lon")) == (51.51, -0.13), "location", f"a refused PUT changed it: {c.get('lat')}, {c.get('lon')}")
        self.expect("51.51" not in json.dumps(api.status()), "location", "the location is in /api/status")
        api.json("PUT", "/api/config", {"lat": None, "lon": None})
        _, c = api.json("GET", "/api/config")
        self.expect(c.get("lat", 0) is None and c.get("lon", 0) is None, "location", f"cleared, read {c.get('lat')}, {c.get('lon')}")
        api.json("PUT", "/api/config", {"lat": -33.8688, "lon": 151.2093})
        _, c = api.json("GET", "/api/config")
        self.expect((c.get("lat"), c.get("lon")) == (-33.87, 151.21), "location", f"read back {c.get('lat')}, {c.get('lon')}")
        code, _, slim = api.raw("GET", "/api/status?now=1")
        sj = json.loads(slim) if code == 200 else {}
        self.expect(code == 200 and len(slim) < 600 and {"t_ms", "now_playing", "paused", "transfer"} <= set(sj) and "heap" not in sj,
                    "status", f"the slim status (?now=1): HTTP {code}, {len(slim)} B, keys {sorted(sj)}")
        self.expect("transfer" in st, "status", "no `transfer` in /api/status")
        self.ok("status, config", f"GET both; PUT dwell_max {self.dwell0} -> {self.dwell_set} read back; a bad value 400, nothing changed; lat/lon set, rounded, refused alone or off the globe, cleared, set again")

    def stalled_body(self, api):
        """STALLED_BODY (net-stall): a JSON body that stops part way. The httpd task reads it, and
        serves no one else meanwhile: it waited 3 x 10 s for the rest, then 10 s more purging it,
        so under simultaneous uploads (their frames holding the WiFi driver's receive buffers) one
        live edit's stalled body held every client, status polls and the refusals that would
        have freed those buffers included. Now one receive timeout (2 s) ends it: a 503 with
        Retry-After (the web app asks again), the connection dropped, `net.body_stalled`
        counted; and a status read sent meanwhile is answered, after that wait, not 30-40 s
        later. (The wait itself is not checked: emulated time is not wall time.)"""
        before = ((api.status() or {}).get("net") or {}).get("body_stalled")
        self.expect(before is not None, "stalled body", "no net.body_stalled in /api/status")
        body = json.dumps({"dwell_max": self.dwell_set}).encode()
        head = (f"PUT /api/config HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\nContent-Type: application/json\r\n"
                f"Content-Length: {len(body) + 64}\r\n\r\n").encode()
        with socket.create_connection(("127.0.0.1", PORT), timeout=30) as s:
            s.sendall(head + body[:6])
            time.sleep(0.5)
            t0 = time.time()
            code, _, _ = api.raw("GET", "/api/status", timeout=20)
            status_s = time.time() - t0
            self.expect(code == 200, "stalled body", f"a status read beside a stalled body: HTTP {code}")
            got = b""
            try:
                while chunk := s.recv(4096):
                    got += chunk
            except (ConnectionResetError, TimeoutError, socket.timeout):
                pass
        line = got.split(b"\r\n", 1)[0]
        self.expect(b" 503 " in line + b" ", "stalled body", f"the stalled PUT was answered {line[:80]!r} (want 503), {len(got)} B")
        self.expect(re.search(rb"(?im)^retry-after:\s*\d", got), "stalled body", f"its 503 has no Retry-After: {got[:300]!r}")
        net = (api.status() or {}).get("net") or {}
        self.expect((net.get("body_stalled") or 0) > before, "stalled body", f"net.body_stalled {before} -> {net.get('body_stalled')}")
        _, cfg = api.json("GET", "/api/config")
        self.expect(cfg.get("dwell_max") == self.dwell_set, "stalled body", f"the stalled PUT changed dwell_max to {cfg.get('dwell_max')}")
        self.ok("stalled body", f"a PUT whose body stopped: {line.decode(errors='replace')} with Retry-After, dropped, net.body_stalled {before} -> {net.get('body_stalled')}; a status read beside it answered in {status_s:.1f} s wall")

    def long_request(self, api):
        """LONG_REQUEST (net-stall): a request the httpd task used to wait out for another task
        (here the WiFi scan, up to 12 s on the network worker; a rebuild was up to 60 s) holds
        no one else up: it is a 202 + X-Job once it takes over 0.3 s, status reads answer
        while it is pending, a second long request meanwhile is a 503 + Retry-After, and
        GET /api/job gives its own answer."""
        code, headers, body = api.raw("GET", "/api/wifi/scan", timeout=20, follow=False)
        if code == 200:
            self.ok("long request", "the scan answered within 0.3 s: no pending state to check")
            return
        job = headers.get("X-Job")
        self.expect(code == 202 and job and json.loads(body).get("pending") is True, "long request", f"GET /api/wifi/scan: HTTP {code}, X-Job {job}, {body[:120]!r}")
        reads, second, end = 0, None, time.time() + 60
        while time.time() < end:
            c, _, _ = api.raw("GET", "/api/status", timeout=10)
            self.expect(c == 200, "long request", f"a status read while the scan was pending: HTTP {c}")
            reads += 1
            if second is None:
                c2, h2, _ = api.raw("POST", "/api/reindex", timeout=10, follow=False)
                second = (c2, h2.get("Retry-After"))
            code, headers, body = api.raw("GET", f"/api/job?id={job}", timeout=10, follow=False)
            if code != 202:
                break
            time.sleep(0.3)
        self.expect(code == 200 and isinstance(json.loads(body), (list, dict)), "long request", f"GET /api/job?id={job}: HTTP {code}, {body[:120]!r}")
        self.expect(second[0] in (503, 200) and (second[0] == 200 or second[1]), "long request", f"a reindex while the scan was pending: {second}")
        code, _, _ = api.raw("GET", f"/api/job?id={job}", timeout=10, follow=False)
        self.expect(code == 404, "long request", f"the job asked again after its answer: HTTP {code}, wanted 404")
        self.ok("long request", f"the scan: 202 (job {job}), {reads} status reads answered meanwhile, a reindex beside it {second[0]}, then its answer from /api/job (200), 404 after")

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
        # A card file is never a page (docs/api.md "Security"): a GIF as image/gif, anything
        # else (an .html written to the card) a sandboxed download.
        put(f"{d}/emu-page.html", b"<script>alert(1)</script>")
        self.expected[f"{d}/emu-page.html"] = b"<script>alert(1)</script>"
        heads = {}
        for p in (f"{d}/emu-a.gif", f"{d}/emu-page.html"):
            code, h, _ = api.raw("GET", q(p))
            heads[p] = (code, {k.lower(): v for k, v in h.items()})
        ca, ha = heads[f"{d}/emu-a.gif"]
        cp, hp = heads[f"{d}/emu-page.html"]
        self.expect(ca == 200 and ha.get("content-type") == "image/gif" and ha.get("x-content-type-options") == "nosniff"
                    and ha.get("content-security-policy", "").startswith("sandbox") and "content-disposition" not in ha,
                    "card headers", f"a GIF: {ha.get('content-type')}, {ha.get('content-security-policy')}")
        self.expect(cp == 200 and hp.get("content-type") == "application/octet-stream" and hp.get("content-disposition") == "attachment"
                    and hp.get("content-security-policy", "").startswith("sandbox"), "card headers",
                    f"an .html on the card: {hp.get('content-type')}, {hp.get('content-disposition')}")
        self.ok("card headers", "a GIF goes out as image/gif, an .html on the card as a sandboxed download (octet-stream, attachment)")

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
        self.listing(api, d, {"emu-a.gif", "emu-b.gif", "emu-big.bin", "emu-page.html"})
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

    def look(self, emu, api):
        """The look's own Formula saved anew and deleted (formula_look::file_changed)."""
        psi, psb = "/formulas/emu-look.psi", "/formulas/emu-look.psb"
        good = self.a.look_psb.read_bytes()
        api.call("PUT", "/api/files" + psi, self.a.look.read_bytes())
        api.call("PUT", "/api/files" + psb, good)
        api.json("PUT", "/api/config", {"look_formula": psi})
        fl = lambda s: s.get("formula_look") or {}
        self.wait_status(api, lambda s: fl(s).get("source") == "setting" and not fl(s).get("error"), 60, "the card's look drawn")
        # Compiled anew (garbage: does not build): rebuilt at once, so the status says why.
        mark = emu.mark()
        api.call("PUT", "/api/files" + psb, b"not a compiled Formula")
        st = self.wait_status(api, lambda s: fl(s).get("error"), 60, "the look rebuilt from a bad .psb (formula_look.error)")
        why = fl(st)["error"]
        api.call("PUT", "/api/files" + psb, good)
        self.wait_status(api, lambda s: fl(s).get("source") == "setting" and not fl(s).get("error"), 60, "the look rebuilt from the real .psb")
        self.expect(api.json("GET", "/api/config")[1].get("look_formula") == psi, "look", "look_formula changed by a .psb write")
        # By an 8.3 alias: refused (a write path is the long name), the look kept.
        api.call("DELETE", "/api/files/formulas/EMU-LO~1.PSI", want=(400,))
        self.expect(api.json("GET", "/api/config")[1].get("look_formula") == psi, "look", "look_formula changed by a refused alias delete")
        # The Formula deleted: the look goes with it.
        api.call("DELETE", "/api/files" + psi)
        _, cfg = api.json("GET", "/api/config")
        self.expect(cfg.get("look_formula") == "", "look", f"look_formula {cfg.get('look_formula')!r} after its Formula was deleted")
        self.wait_status(api, lambda s: s.get("formula_look") is None, 60, "no look once its Formula was deleted")
        emu.wait_for(re.compile(r"formula look: its Formula was deleted"), 30, mark, "the look's end on the console")
        api.call("DELETE", "/api/files" + psb)
        self.ok("look", f"a card Formula as the look; its .psb written anew rebuilt it at once (a bad one: {why!r}; then drawn again); an 8.3 alias delete refused (400); its .psi deleted cleared look_formula and the look")

    def sent(self, emu, api):
        """Show on panel as the web app sends it: the compiled bytes as the body."""
        long = self.a.long_psb.read_bytes()
        b64 = self.a.psb.read_text().strip()
        short = base64.urlsafe_b64decode(b64 + "=" * (-len(b64) % 4))
        name = lambda b: b[6:6 + b[5]].decode()
        self.expect(len(long) * 4 // 3 > 6144, "sent", f"--long-psb is {len(long)} B: it fits a path")
        # (Kept until changed: the JIT's waits below outlast a turn of the dwell.)
        api.json("PUT", "/api/config", {"formula_keep": "hold"})
        jit = lambda s: (s["now_playing"].get("formula") or {}).get("jit") or {}
        def jit_off(what):
            # (The status says the last kernel's word until the new
            # Formula's is made: first it goes, then it comes back.)
            # (Missed when the new kernel is drawing by the next read: fine.)
            with contextlib.suppress(LaneFail):
                self.wait_status(api, lambda s: jit(s).get("on") is not True, 10, f"the JIT off {what}")
        def jit_on(what):
            # (None: no kernel within the wait. The kernel's build asks for a
            # contiguous executable block; a fragmented heap holds it off.)
            t0 = time.time()
            try:
                # (Road V2's kernel, ~4 KB: not the last Formula's, 550 B.)
                st = self.wait_status(api, lambda s: jit(s).get("on") is True and jit(s).get("bytes", 0) > 2000,
                                      120, f"the JIT drawing {what}")
            except LaneFail:
                return None
            return time.time() - t0, jit(st)
        api.json("POST", "/api/play", {"path": "/gifs/EmuApi/emu-a.gif"})
        self.wait_status(api, lambda s: s["now_playing"]["path"] == "/gifs/EmuApi/emu-a.gif", 30, "the GIF before")
        _, r = api.json("POST", "/api/play/formula?a=0.5&key=secret", long)
        self.expect(r.get("path") == "@formula:~sent?a=0.5", "sent", f"answered {r} (the sliders kept, never the key)")
        st = self.wait_status(api, lambda s: s["now_playing"]["path"].startswith("@formula:~sent")
                              and (s["now_playing"].get("formula") or {}).get("frame_ms", 0) > 0, 60, "the long Formula playing")
        np = st["now_playing"]
        self.expect(np["title"] == name(long), "sent", f"title {np['title']!r}, wanted {name(long)!r}")
        ordinal = np["ordinal"]
        first = jit_on("the long Formula")
        # Live edits as the editor sends them (~200 ms apart): the other Formula, then back.
        mark = emu.mark()
        for k in range(10):
            body = short if k % 2 == 0 else long
            api.json("POST", f"/api/play/formula?live=1&a={k / 10}", body)
            time.sleep(0.2)
        st = self.wait_status(api, lambda s: s["now_playing"]["path"] == "@formula:~sent?a=0.9", 30, "the last live edit")
        np = st["now_playing"]
        self.expect(np["ordinal"] == ordinal and np["title"] == name(long), "sent",
                    f"after 10 live edits: ordinal {np['ordinal']} (was {ordinal}), title {np['title']!r}")
        # The JIT comes back after the edits pause, with one kernel made for
        # the burst, not one per edit (Jit::settling).
        jit_off("after the live edits")
        after = jit_on("after the live edits")
        made = len(emu.find(re.compile(r"formula: kernel of \d+ B at .*to be checked"), mark))
        self.expect(made <= 2, "sent", f"{made} kernels made for a burst of 10 live edits")
        # A slot the Formula doesn't declare, sent out of its 0..1 (`a=30`):
        # held to it, so the kernel is checked and draws (it waited forever).
        mark = emu.mark()
        api.json("POST", "/api/play/formula?live=1&a=30", long)
        jit_off("after a=30")
        wide = jit_on("after a=30")
        if first and after:
            self.expect(wide is not None, "sent", f"after a=30 the JIT never drew (now {jit(api.status())})")
        else:
            self.notes.append(f"sent: no kernel on this heap ({jit(api.status())}): the JIT's checks were not made")
            self.expect(not emu.find(re.compile(r"kernel of \d+ B at .*to be checked"), mark)
                        or emu.find(re.compile(r"checked against the VM"), mark), "sent", "a kernel made after a=30 was never checked")
        _, rec = api.json("GET", "/api/status?recent=1")
        self.expect(not any("~sent" in str(x) for x in rec.get("recent") or []), "sent", f"in Recently played: {rec.get('recent')}")
        api.call("POST", "/api/play/formula", b"\0" * (16 * 1024 + 1), want=(413,))
        api.call("POST", "/api/play/formula", {"path": "x"}, want=(415,))
        api.call("POST", "/api/play/formula", b"not a formula", want=(400,))
        # The Formula refused, the one on the panel plays on.
        self.expect(api.status()["now_playing"]["ordinal"] == ordinal, "sent", "a refused one moved the panel on")
        api.json("POST", "/api/skip")
        self.wait_status(api, lambda s: s["now_playing"]["ordinal"] != ordinal, 60, "a skip off the sent Formula")
        api.call("POST", "/api/play/formula?live=1", long, want=(409,))
        api.json("PUT", "/api/config", {"formula_keep": "turn"})
        self.ok("sent", f"a {len(long)} B compiled Formula ({len(long) * 4 // 3} B as a path) shown by its bytes as {name(long)!r}, "
                f"10 live edits in place (the same start), not in Recently played; 413, 415, 400; 409 once moved on; "
                f"the JIT on after the show, the edits ({made} kernel made) and a=30: {first}, {after}, {wide}")

    def www(self, api):
        pack = self.a.www_pack.read_bytes()
        lower = lambda h: {k.lower(): v for k, v in h.items()}
        # Only a signed pack (docs/www-pack.md "Signature"): an unsigned one is a 422 at its
        # header, the same pack with one byte changed (its CRC made to match) a 422 once read.
        if pack[19] & 1:
            unsigned = bytearray(pack[:-72])
            unsigned[19] &= ~1
            unsigned[8:12] = struct.pack("<I", len(unsigned))
            unsigned[12:16] = struct.pack("<I", zlib.crc32(bytes(unsigned[64:])))
            code, _, body = api.raw("PUT", "/api/www", bytes(unsigned), timeout=300)
            self.expect(code == 422 and b"not signed" in body, "www", f"an unsigned pack: HTTP {code} {body[:120]!r}")
            forged = bytearray(pack)
            forged[200] ^= 1
            forged[12:16] = struct.pack("<I", zlib.crc32(bytes(forged[64:])))
            code, _, body = api.raw("PUT", "/api/www", bytes(forged), timeout=300)
            self.expect(code == 422 and b"signature" in body, "www", f"an altered pack: HTTP {code} {body[:120]!r}")
            _, info = api.json("GET", "/api/www")
            self.expect(info.get("ok") is True, "www", f"after the refusals the web app in flash is as it was: {info}")
        # The Host gate (docs/api.md "Security"): a name that is not the panel's is a 421 for
        # every route, GETs included; 127.0.0.1 and localhost (this port forward) answer.
        for path in ("/", "/api/status"):
            code, _, body = api.raw("GET", path, extra={"Host": f"evil.example:{PORT}"})
            self.expect(code == 421 and b"answers only to" in body, "www", f"GET {path} with Host evil.example: HTTP {code} {body[:100]!r}")
        code, _, _ = api.raw("GET", "/api/status", extra={"Host": f"localhost:{PORT}"})
        self.expect(code == 200, "www", f"GET /api/status with Host localhost: HTTP {code}")
        _, r = api.json("PUT", "/api/www", pack, timeout=300)
        self.expect(r.get("ok") is True and r.get("error") is None, "www", f"PUT /api/www answered {r}")
        _, info = api.json("GET", "/api/www")
        code, headers, page = api.raw("GET", "/")
        ctype = {k.lower(): v for k, v in headers.items()}.get("content-type", "")
        self.expect(code == 200 and ctype.startswith("text/html") and len(page) > 200, "www", f"GET /: HTTP {code}, {ctype}, {len(page)} B")
        # Caching (www.rs "Caching"): no-cache, the pack's CRC as every file's ETag, a 304 for it
        # (answered on the httpd task), and the wasm at its build's address (?v=, js/wasm.js).
        tag = f'"{struct.unpack_from("<I", pack, 12)[0]:08x}"'
        h = lower(headers)
        self.expect("frame-ancestors 'none'" in h.get("content-security-policy", "") and h.get("x-content-type-options") == "nosniff",
                    "www", f"GET /: the security headers {h.get('content-security-policy', '')[:50]!r}")
        self.expect(h.get("etag") == tag and h.get("cache-control") == "no-cache", "www",
                    f"GET /: ETag {h.get('etag')} (the pack's CRC is {tag}), Cache-Control {h.get('cache-control')}")
        code, h304, body = api.raw("GET", "/", extra={"If-None-Match": tag})
        self.expect(code == 304 and not body and lower(h304).get("etag") == tag, "www", f"GET / with its ETag: HTTP {code}, {len(body)} B")
        wasm = "/pkg-psi/opendmd_psi_web_bg.wasm?v=0123456789abcdef"
        code, hw, body = api.raw("GET", wasm)
        hw = lower(hw)
        self.expect(code == 200 and hw.get("etag") == tag and hw.get("cache-control") == "no-cache" and hw.get("content-encoding") == "gzip"
                    and hw.get("content-type") == "application/wasm" and len(body) > 10000, "www",
                    f"GET {wasm}: HTTP {code}, {hw.get('content-type')}, {hw.get('etag')}, {hw.get('cache-control')}, {len(body)} B")
        code304, _, body304 = api.raw("GET", wasm, extra={"If-None-Match": f'W/"00000000", {tag}'})
        code_other, _, _ = api.raw("GET", wasm, extra={"If-None-Match": '"00000000"'})
        self.expect(code304 == 304 and not body304 and code_other == 200, "www",
                    f"the wasm with its ETag in a list: HTTP {code304}; with another: HTTP {code_other}")
        self.ok("www pack", f"an unsigned and an altered pack refused (422); Host evil.example 421; "
                f"PUT {len(pack)} B: {info.get('version')}, {info.get('entries')} entries, ok; GET / serves it with its CSP; "
                f"no-cache, ETag {tag}, 304 for it (/ and the wasm at ?v=), 200 for another")

    # -- simultaneous uploads (a test board's wedge recipe) -------------------------

    def events(self, emu, api):
        """`GET /api/events` (firmware events.rs): the Now view's state pushed on change and every
        3 s. Device times (`t_ms`) are emulated; wall times are not the device's."""
        mark = emu.mark()
        api.json("POST", "/api/play", {"path": self.card_gif})
        emu.wait_for(re.compile(re.escape(f"] {self.card_gif} <- ")), 60, mark, f"`{self.card_gif} <- `")
        time.sleep(5)

        def heap_now():
            vals = []
            for _ in range(5):
                vals.append(api.status()["heap"]["free"])
                time.sleep(0.5)
            return sorted(vals)[2]

        def listeners():
            return ((api.status().get("net") or {}).get("events") or {}).get("listeners")

        class Listener:
            def __init__(self, read=True):
                self.events, self.closed, self.bad = [], False, 0
                self.s = socket.create_connection(("127.0.0.1", PORT), timeout=30)
                if not read:
                    self.s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096)
                self.s.sendall(b"GET /api/events HTTP/1.1\r\nHost: opendmd\r\nAccept: text/event-stream\r\n\r\n")
                self.head = b""
                while b"\r\n\r\n" not in self.head:
                    d = self.s.recv(1024)
                    if not d:
                        break
                    self.head += d
                self.code = int(self.head.split(b" ", 2)[1]) if self.head.startswith(b"HTTP/") else 0
                self.buf = self.head.split(b"\r\n\r\n", 1)[1] if b"\r\n\r\n" in self.head else b""
                if self.code == 200 and read:
                    threading.Thread(target=self.loop, daemon=True).start()

            def loop(self):
                try:
                    while True:
                        d = self.s.recv(8192)
                        if not d:
                            break
                        self.buf += d
                        while b"\n\n" in self.buf:
                            ev, self.buf = self.buf.split(b"\n\n", 1)
                            if not ev.startswith(b"data: "):
                                continue
                            try:
                                j = json.loads(ev[6:])
                                self.events.append((time.time(), j["t_ms"], j["now_playing"]["ordinal"], len(ev) + 2, j))
                            except (ValueError, KeyError):
                                self.bad += 1
                except OSError:
                    pass
                self.closed = True

            def close(self):
                with contextlib.suppress(OSError):
                    self.s.close()

        def gaps(evs):
            return sorted(y[1] - x[1] for x, y in zip(evs, evs[1:]))

        h0 = heap_now()
        self.expect(listeners() == 0, "events", f"net.events.listeners {listeners()} before any listener")
        a = Listener()
        self.expect(a.code == 200 and b"text/event-stream" in a.head, "events", f"GET /api/events answered {a.head[:200]!r}")
        end = time.time() + 20
        while not a.events and time.time() < end:
            time.sleep(0.2)
        self.expect(a.events, "events", "no first event in 20 s wall")
        first = a.events[0][4]
        for k in ("t_ms", "now_playing", "paused", "entries", "manifest", "index_state", "brightness", "sd", "mode", "schedule", "context"):
            self.expect(k in first, "events", f"the event has no `{k}`: {sorted(first)}")
        # What the panel plays, as the status says (it may move on meanwhile: one of the next events then).
        st = api.status()
        end = time.time() + 15
        while not any(e[4]["now_playing"]["path"] == st["now_playing"]["path"] for e in a.events) and time.time() < end:
            time.sleep(0.2)
        self.expect(any(e[4]["now_playing"]["path"] == st["now_playing"]["path"] for e in a.events), "events",
                    f"no event names what the status says plays ({st['now_playing']['path']}): {[e[4]['now_playing']['path'] for e in a.events]}")
        time.sleep(6)
        h1 = heap_now()
        b = Listener()
        self.expect(b.code == 200, "events", f"a second listener answered {b.code}")
        time.sleep(6)
        h2 = heap_now()
        c = Listener()
        self.expect(c.code == 503 and re.search(rb"(?im)^retry-after:", c.head), "events", f"a third listener answered {c.head[:120]!r}, wanted 503 + Retry-After")
        c.close()
        self.expect(listeners() == 2, "events", f"net.events.listeners {listeners()} with two listening")
        # Idle (the GIF looping, nothing changing): a keep-alive at least every 3 s.
        n0 = len(a.events)
        time.sleep(20)
        idle = a.events[n0:]
        ig = gaps(idle)
        self.expect(len(idle) >= 3 and ig and ig[-1] <= 4000, "events idle",
                    f"{len(idle)} events in 20 s wall while idle, device gaps {ig}: a keep-alive every 3 s wanted")
        self.expect(a.bad == 0 and b.bad == 0, "events", f"{a.bad + b.bad} events that were not the JSON wanted")
        ev_len = max(e[3] for e in a.events)

        def stable(j):
            np = {k: v for k, v in j["now_playing"].items() if k not in ("elapsed_ms", "frame", "formula")}
            return {**j, "t_ms": 0, "now_playing": np}
        changed: dict[str, int] = {}
        for x, y in zip(idle, idle[1:]):
            sx, sy = stable(x[4]), stable(y[4])
            for k in sorted(set(sx) | set(sy)):
                if sx.get(k) != sy.get(k):
                    changed[k] = changed.get(k, 0) + 1
        # A skip: an event with the next file, without waiting for the keep-alive.
        st = api.status()
        ordinal, t_skip = st["now_playing"]["ordinal"], st["t_ms"]
        api.json("POST", "/api/skip")
        end = time.time() + 30
        hit = None
        while hit is None and time.time() < end:
            hit = next((e for e in a.events if e[2] != ordinal and e[1] >= t_skip), None)
            time.sleep(0.1)
        self.expect(hit is not None, "events", f"no event with the next file within 30 s wall of a skip (ordinal {ordinal})")
        self.ok("events", f"heap free (median of 5 status reads) {h0} B with 0 listeners, {h1} with 1, {h2} with 2 "
                f"(per listener {h0 - h1}, {h1 - h2} B); a third 503 + Retry-After; idle: {len(idle)} events in 20 s wall, "
                f"device gaps max {ig[-1]} ms (changes between them: {changed or 'none, keep-alives'}); events up to {ev_len} B; a skip's event {hit[1] - t_skip} ms of device time after the status read before it")
        self.notes.append(f"events heap: 0/1/2 listeners {h0}/{h1}/{h2} B free, events up to {ev_len} B")

        # Rounds of simultaneous 300 KB uploads, both listeners and a status read beside them.
        work = self.logs / f"events-{self.stamp}"
        work.mkdir(parents=True, exist_ok=True)
        src = work / "blob.bin"
        src.write_bytes(blob(78, 300 * 1024))
        n_a0, n_b0 = len(a.events), len(b.events)
        codes: dict[int, int] = {}
        lock = threading.Lock()
        polls: list[float] = []
        for rnd, n in enumerate((2, 5), 1):
            def one(i):
                url = f"http://127.0.0.1:{PORT}/api/files/gifs/_events/r{rnd}u{i}.bin"
                t0 = time.time()
                while True:
                    r = subprocess.run(["curl", "-s", "-o", "/dev/null", "-D", str(work / f"h{i}"), "-w", "%{http_code}", "-m", "120",
                                        "-T", str(src), url], capture_output=True, text=True)
                    code = int(r.stdout.strip() or 0)
                    with lock:
                        codes[code] = codes.get(code, 0) + 1
                    head = (work / f"h{i}").read_text(errors="replace") if (work / f"h{i}").exists() else ""
                    if code == 503 and re.search(r"(?im)^retry-after:", head) and time.time() - t0 < 300:
                        time.sleep(2)
                        continue
                    return
            ts = [threading.Thread(target=one, args=(i,), daemon=True) for i in range(1, n + 1)]
            for t in ts:
                t.start()
            while any(t.is_alive() for t in ts):
                emu.check()
                t_s = time.time()
                code, _, _ = api.raw("GET", "/api/status", timeout=20)
                polls.append(time.time() - t_s)
                self.expect(code in (200, 503), "events burst", f"a status read beside the uploads answered {code}")
                time.sleep(1)
            for t in ts:
                t.join(300)
        self.expect(codes.get(200, 0) == 7 and set(codes) <= {200, 503}, "events burst", f"the uploads answered {codes}: 7 x 200 wanted in the end")
        self.expect(not a.closed and not b.closed, "events burst", "a listener was closed during the uploads")
        burst_a, burst_b = a.events[n_a0:], b.events[n_b0:]
        bg = gaps(burst_a)
        self.expect(len(burst_a) >= 3 and len(burst_b) >= 3 and bg and bg[-1] <= 6000, "events burst",
                    f"{len(burst_a)}/{len(burst_b)} events to the listeners during the uploads, device gaps {bg[-12:]}")
        self.expect(max(polls) < 10, "events burst", f"a status read beside the uploads took {max(polls):.1f} s wall")
        self.ok("events burst", f"uploads {dict(sorted(codes.items()))}; {len(burst_a)} and {len(burst_b)} events to the two listeners, "
                f"device gaps p50 {bg[len(bg) // 2]} ms max {bg[-1]} ms; {len(polls)} status reads beside, wall max {max(polls):.2f} s (not a timing check)")
        for rnd, n in enumerate((2, 5), 1):
            for i in range(1, n + 1):
                api.call("DELETE", "/api/files" + q(f"/gifs/_events/r{rnd}u{i}.bin"), want=(200, 404))
        api.call("DELETE", "/api/files" + q("/gifs/_events"), want=(200, 404))
        shutil.rmtree(work, ignore_errors=True)

        # Closed listeners leave the list (close_fn), and a place is free again.
        a.close()
        b.close()
        end = time.time() + 30
        while listeners() != 0 and time.time() < end:
            time.sleep(0.5)
        self.expect(listeners() == 0, "events", f"net.events.listeners {listeners()} 30 s wall after both closed")
        # A listener that never reads: the device never waits for it (the emulator's NAT takes
        # every byte into host buffers, so the 5 s drop itself is a device check).
        stall = Listener(read=False)
        self.expect(stall.code == 200, "events stall", f"answered {stall.code}")
        t_s, worst = time.time(), 0.0
        while time.time() - t_s < 20:
            t1 = time.time()
            api.status()
            worst = max(worst, time.time() - t1)
            time.sleep(1)
        stall.close()
        end = time.time() + 30
        while listeners() != 0 and time.time() < end:
            time.sleep(0.5)
        ev = (api.status().get("net") or {}).get("events")
        self.expect(ev and ev["listeners"] == 0, "events stall", f"net.events {ev} after the listener that never read closed")
        self.ok("events stall", f"a listener that never reads: status reads beside it max {worst:.2f} s wall over 20 s; "
                f"closed, it left the list (net.events {ev})")

    def bursts(self, emu, api):
        """The spare's wedge recipe (wedge.sh) in the emulator: rounds of 2, 2, 2, 5, 5, 5 uploads of
        300 KB at once (`curl -T`, as there: curl stops sending at an early reply) to
        /gifs/_wedge/bN.bin, with a status read every second beside them. Each upload is answered
        200, or 503 with Retry-After and then 200 when asked again (as the web app does): never a
        408, never no answer; every status read answers; the network answers after every round (the
        wedge: no TCP, ping or ARP answered again until a reset); and, 20 s after the last round,
        lwIP holds no received frame (`netdiag: sum`, emu-census builds)."""
        # A card GIF on the panel first: the heap bound is about the network, and a Formula
        # left on from `play` (kept) holds ~20 KB more than a GIF.
        mark = emu.mark()
        api.json("POST", "/api/play", {"path": self.card_gif})
        emu.wait_for(re.compile(re.escape(f"] {self.card_gif} <- ")), 60, mark, f"`{self.card_gif} <- `")
        work = self.logs / f"burst-{self.stamp}"
        work.mkdir(parents=True, exist_ok=True)
        src = work / "blob.bin"
        src.write_bytes(blob(77, 300 * 1024))
        log, codes = [], {}
        burst_lows: list[int] = []
        net_at_low: list[int] = []
        net_quiet: list[int] = []
        status_wall: list[float] = []
        dwell = getattr(self, "dwell_set", None)
        saves = 0
        for rnd, n in enumerate(BURST_ROUNDS, 1):
            round_mark = emu.mark()
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
            beat = 0
            while any(t.is_alive() for t in threads):
                emu.check()
                try:
                    t_s = time.time()
                    code, _, _ = api.raw("GET", "/api/status", timeout=15)
                    status_wall.append(time.time() - t_s)
                    if code != 200:
                        errors.append(f"status beside round {rnd}: HTTP {code}")
                    # A settings save (a JSON body, the heavy door: 200, or 503 + Retry-After
                    # and 200 when asked again, as the web app does) every third read.
                    beat += 1
                    if beat % 3 == 0 and dwell is not None:
                        api.call("PUT", "/api/config", {"dwell_max": dwell})
                        saves += 1
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
            lows = [(int(m[1]), int(m[2]), int(m[3])) for _, m in emu.find(LOWWATCH, round_mark)]
            censuses = emu.find(CENSUS_TOTAL)
            total = (int(censuses[-1][1][1]) + int(censuses[-1][1][2])) if censuses else None
            if lows and total:
                # The C side at each window's low, and at its end (mostly quiet: a sample).
                for _, m in emu.find(LOWWATCH, round_mark):
                    net_at_low.append(total - int(m[1]) - int(m[2]))
                    net_quiet.append(total - int(m[5]) - int(m[6]))
            if lows:
                low = min(lows)
                burst_lows.append(low[0])
                print(f"api: burst round {rnd} ({n} at once): heap low {low[0]} B (rust {low[1]} B, {low[2]} sockets) over {len(lows)} windows", flush=True)
                if low[0] < BURST_HEAP_MIN:
                    errors.append(f"the heap went to {low[0]} B free (BURST_HEAP_MIN {BURST_HEAP_MIN} B): "
                                  + "; ".join(m.string.split("lowwatch: ")[1][:220] for _, m in emu.find(LOWWATCH, round_mark) if int(m[1]) == low[0])[:400])
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
        self.expect(burst_lows and net_at_low, "bursts", "no `lowwatch:`/`census:` lines beside the bursts: is this an emu-census image?")
        net_grew = max(net_at_low) - min(net_quiet)
        self.expect(net_grew <= BURST_NET_MAX, "burst heap",
                    f"the C side (lwIP, WiFi, FatFs) held {net_grew} B more at a low than when quiet (BURST_NET_MAX {BURST_NET_MAX} B)")
        self.ok("burst heap", f"least free heap per round {burst_lows} B (low watch, every ms), none under BURST_HEAP_MIN {BURST_HEAP_MIN} B; the C side {net_grew} B over its quiet level at the worst (BURST_NET_MAX {BURST_NET_MAX} B); {len(status_wall)} status reads beside them, the slowest {max(status_wall or [0]):.2f} s wall (not a timing check)")
        net = (api.status() or {}).get("net") or {}
        self.expect("heavy_refused" in net, "bursts", f"no net.heavy_refused in /api/status: {sorted(net)}")
        self.ok("heavy door", f"{saves} settings saves beside the uploads, each 200 in the end; net.heavy_refused {net.get('heavy_refused')}, status_refused {net.get('status_refused')}")
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
        self.expect(info.get("variant") == "4mb" and info.get("layout") == 3, "firmware", f"GET /api/firmware: {info}")
        st = api.status()
        self.expect(st.get("layout") == 3, "firmware", f"/api/status layout: {st.get('layout')}")
        b = bytearray(self.a.bundle.read_bytes())
        self.expect(b[:8] == b"ODFW34mb", "firmware", f"the bundle is not for the 4mb table v3: {bytes(b[:8])!r}")
        # The same bundle labelled for the DevKit's table v2 (what 1.0.0-beta.15
        # and earlier signed): refused at the header, before the signature, in words.
        v2 = bytearray(b)
        v2[:8] = b"ODFW1\0\0\0"
        _, why = api.call("PUT", "/api/firmware", v2, want=(422,), timeout=300)
        self.expect(b"older flash layout v2" in why and b"has layout v3" in why, "firmware", f"a table v2 bundle: {why[:200]!r}")
        sig = bytearray(b)
        sig[100] ^= 0x01  # in the Ed25519 signature (bytes 76..140)
        mark = self.emu.mark()
        api.call("PUT", "/api/firmware", sig, want=(422,), timeout=300)
        # Refused at the header: the panel never said "Updating".
        self.expect(not self.emu.find(re.compile(r"update screen: updating"), mark), "ota panel",
                    "a bundle refused at its header put the update message up")
        img = bytearray(b)
        img[140 + 4096] ^= 0x01  # in the app image: its SHA-256 no longer matches
        mark = self.emu.mark()
        api.call("PUT", "/api/firmware", img, want=(422,), timeout=300)
        # Refused after its upload: the message was up while it came in, and
        # went with the refusal; the playlist plays on (a GIF starts after).
        up, _ = self.emu.wait_for(re.compile(r"update screen: updating"), 30, mark, "the update message going up")
        down, _ = self.emu.wait_for(re.compile(r"update screen: off .*playback resumes"), 30, up, "the update message coming down")
        self.emu.wait_for(re.compile(r"opendmd_firmware: \[\d+\] /"), 120, down, "a GIF after the refused update")
        self.ok("ota panel", "the message up while the bad-image bundle came in, down at its 422, a GIF after; none for the one refused at its header")
        _, info2 = api.json("GET", "/api/firmware")
        self.expect(info2.get("pending") is None, "firmware", f"a refused bundle left something pending: {info2}")
        self.ok("ota refusals", f"GET /api/firmware {info.get('version')} (layout v3); a bundle for table v2 422 at the header ('{why[:60].decode(errors='replace')}…'), one with a flipped signature bit 422 at the header, one with a flipped image bit 422 after its upload; nothing pending")

    def startup_a(self, emu, api):
        """The start-up show this boot: NVS's bundled Formula (BOOT_GOOD), drawn until the
        playlist; boot_formula's checks; the card's Formula (saved in `play`) for boot B."""
        said = [m[1] for _, m in emu.find(BOOT_SHOW)]
        played = f"formula {BOOT_GOOD}" in said and any(re.fullmatch(r"formula at \d+ ms", x) for x in said)
        to_list = [s for s in said if s.startswith("show -> playlist")]
        self.expect(played and to_list and not any("does not build" in s for s in said), "start-up",
                    f"boot A's show was not {BOOT_GOOD} through to the playlist: {said[:12]}")
        st = api.status()
        self.expect(st.get("boot_formula") == {"played": True}, "start-up", f"status boot_formula {st.get('boot_formula')}")
        _, cfg = api.json("GET", "/api/config")
        self.expect(cfg.get("boot_formula") == BOOT_GOOD, "start-up", f"config boot_formula {cfg.get('boot_formula')!r}")
        for bad in ("@formula:no-such-formula", "@formula?psb=AAAA", "/formulas/no-such.psi", "starfield"):
            api.call("PUT", "/api/config", {"boot_formula": bad}, want=(400,))
        api.call("PUT", "/api/config", {"boot_formula": "@formula?psb=" + "A" * 2100}, want=(413,))
        self.expect(api.json("GET", "/api/config")[1].get("boot_formula") == BOOT_GOOD, "start-up", "a refused boot_formula changed it")
        _, r = api.json("PUT", "/api/config", {"boot_formula": BOOT_CARD})
        self.expect(r.get("ok") is True and api.json("GET", "/api/config")[1].get("boot_formula") == BOOT_CARD, "start-up",
                    f"the card's Formula as boot_formula: {r}")
        self.ok("start-up", f"the show was {BOOT_GOOD} (NVS) through to the playlist ({to_list[0]}), status played; "
                f"4 bad values 400 (no such bundled one, a broken inline one, a card file that is not there, not a Formula), a long inline one 413, nothing saved; {BOOT_CARD} set for the next boot")

    def startup_b(self, emu, api):
        """Boot B: NVS's BOOT_BAD fell back to the wordmark, then the card's Formula (its
        settings win) played once the boot task read it; deleting it clears boot_formula."""
        said = [m[1] for _, m in emu.find(BOOT_SHOW)]
        fell = any(s.startswith(f"formula {BOOT_BAD} does not build") and s.endswith("the built-in show") for s in said)
        read = any(s.startswith("the card's formula read at") for s in said)
        card = f"formula {BOOT_CARD}" in said
        self.expect(fell and read and card and any(s.startswith("show -> playlist") for s in said), "start-up",
                    f"boot B's show: fell back {fell}, the card's read {read}, played {card}: {said[:14]}")
        self.expect(api.status().get("boot_formula") == {"played": True}, "start-up", f"status boot_formula {api.status().get('boot_formula')}")
        _, cfg = api.json("GET", "/api/config")
        self.expect(cfg.get("boot_formula") == BOOT_CARD, "start-up", f"config boot_formula {cfg.get('boot_formula')!r} (the card's)")
        mark = emu.mark()
        api.call("DELETE", "/api/files" + BOOT_CARD)
        _, cfg = api.json("GET", "/api/config")
        self.expect(cfg.get("boot_formula") == "", "start-up", f"boot_formula {cfg.get('boot_formula')!r} after its Formula was deleted")
        emu.wait_for(re.compile(r"start-up show: its Formula was deleted"), 30, mark, "the start-up Formula's end on the console")
        self.expected.pop(BOOT_CARD, None)
        self.ok("start-up B", f"NVS's {BOOT_BAD} fell back to the wordmark, then the card's {BOOT_CARD} played once read; deleting it cleared boot_formula")

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
        self.expect("lat" not in js and "lon" not in js, "card", "/opendmd.json holds the location (the device keeps it itself)")
        self.expect(js.get("boot_formula") == BOOT_CARD, "card", f"/opendmd.json has boot_formula {js.get('boot_formula')!r}, the last saved was {BOOT_CARD}")
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
        self.ota_boot("B", self.a.seeded_bad, lit=True)

    def run_c(self):
        """Boot C: the same update with an updater from before the panel in ota_0
        (`--old-updater`, a release's): /api/firmware says so, the firmware's
        Installing screen says the panel goes dark, and that updater installs it."""
        image = with_updater(self.a.seeded, self.a.old_updater, self.logs / f"api-{self.stamp}-old-updater.bin")
        try:
            self.ota_boot("C", image, lit=False)
        finally:
            image.unlink(missing_ok=True)

    def ota_boot(self, label, image: Path, lit: bool):
        emu, api = self.boot(label, self.card_out, False, None, image)
        self.emu = emu
        try:
            st = api.wait_up(240)
            ran = st.get("version")
            _, cfg = api.json("GET", "/api/config")
            self.expect(cfg.get("dwell_max") == self.dwell_set, "boot B", f"dwell_max {cfg.get('dwell_max')} after the power cycle, saved {self.dwell_set}")
            # (The look's Formula was deleted in boot A: cleared for good.)
            self.expect(cfg.get("look_formula") == "", "boot B", f"look_formula {cfg.get('look_formula')!r} after the power cycle")
            for p, data in self.expected.items():
                code, back = api.call("GET", q(p), want=(200, 404))
                self.expect(code == 200 and back == data, "boot B", f"GET {p}: HTTP {code}, {'same' if back == data else 'different'} bytes")
            self.ok(f"boot {label}", f"from boot A's card: dwell_max {self.dwell_set} kept, {len(self.expected)} files the same")
            if lit:
                self.startup_b(emu, api)
            # Which updater ota_0 holds, and whether it lights the panel.
            _, fw = api.json("GET", "/api/firmware")
            upd = fw.get("updater") or {}
            self.expect(upd.get("panel") is lit and (upd.get("version") or "").startswith("updater "), "ota",
                        f"GET /api/firmware updater {upd}, wanted panel {lit}")
            said = emu.find(re.compile(r"updater in ota_0: (updater \d+) \(lights the panel while it installs: (true|false)\)"))
            self.expect(said and said[-1][1].group(1) == upd.get("version") and said[-1][1].group(2) == str(lit).lower(), "ota",
                        f"the boot's `updater in ota_0` line: {[m.group(0) for _, m in said]}")
            want = self.a.bundle.read_bytes()[8:40].rstrip(b"\0").decode()
            mark = emu.mark()
            with emu.cond:
                emu.resets_allowed = emu.resets + 2  # into the updater, back into the firmware
            t0 = time.time()
            _, r = api.json("PUT", "/api/firmware", self.a.bundle.read_bytes(), timeout=600)
            took = time.time() - t0
            self.expect(r.get("ok") is True and r.get("rebooting") is True, "ota", f"PUT /api/firmware answered {r}")
            emu.wait_for(re.compile(r"signature ok( \(key \d+\))?; staging it on the card"), 5, mark, "`signature ok`")
            emu.wait_for(re.compile(r"staged and verified .* restarting into the updater"), 60, mark, "`restarting into the updater`")
            # The panel said so all the way: up while it came in, "Installing"
            # once staged, never taken down before the restart.
            emu.wait_for(re.compile(r"update screen: updating"), 5, mark, "the update message")
            screen = r"update screen: installing(?! \(dark)" if lit else r"update screen: installing \(dark updater\)"
            emu.wait_for(re.compile(screen), 5, mark, "the installing message" + ("" if lit else ", dark updater's"))
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
            # The updater's picture (docs/ota.md "The updater's panel"): the
            # driver comes up after the firmware's restart (I2S TX is not
            # modelled, so its refresh is not judged here), and is blanked
            # before the restart into the firmware.
            panel = emu.find(re.compile(r"updater: (panel up in \d+ ms: .*|panel skipped: .*)"), mark)
            if lit:
                self.expect(len(panel) == 1 and panel[0][1].group(1).startswith("panel up"), "ota",
                            f"the updater's panel: {[m.group(1) for _, m in panel]}")
            else:
                self.expect(not panel and emu.find(re.compile(r"updater: OpenDMD updater 1 started"), mark), "ota",
                            f"not the old updater's run: {[m.group(1) for _, m in panel]}")
            refresh = emu.find(re.compile(r"updater: (panel refresh: .*)"), mark)
            st = api.status()
            self.expect(emu.resets == 3, "ota", f"{emu.resets} resets, wanted 3 (power-on, updater, firmware)")
            self.expect(not emu.find(re.compile(r"update screen: off"), mark), "ota", "the update message came down before the restart")
            # The first boot after it: "Updated to" only for another version
            # than the one that ran (the lane's own bundle is the same build).
            said = emu.find(re.compile(r"boot show: \w+, says 'Updated to' this boot"), mark)
            self.expect(bool(said) == (ran != want), "ota",
                        f"'Updated to' on the boot after the update: {bool(said)}, the versions {ran} -> {want}")
            if lit:
                # Its start-up Formula deleted before the update: the wordmark this boot.
                shows = [m[1] for _, m in emu.find(BOOT_SHOW, mark)]
                self.expect(any(s.startswith("power-on at") for s in shows) and not any(s.startswith("formula") for s in shows), "start-up",
                            f"the boot after the update was not the wordmark: {shows[:10]}")
                self.ok("start-up after", "the boot after the update (its Formula deleted) showed the wordmark")
            self.ok("ota" if lit else "ota old updater", f"{upd.get('version')} in ota_0, panel {upd.get('panel')}: PUT {self.a.bundle.stat().st_size} B in {took:.0f} s wall: signature ok, staged, verified; the updater ({len(ulog)} log lines, '{ulog[-2] if len(ulog) > 1 else ''}') installed it; back as {info['version']} ({st.get('reset_reason')}) in {time.time() - back:.0f} s wall")
            if lit:
                self.ok("ota panel up", f"the updater's '{panel[0][1].group(1)}'; '{refresh[0][1].group(1) if refresh else 'no refresh line'}'")
            else:
                self.ok("ota dark updater", "the Installing screen said 'Dark while it installs'; the old updater ran with no panel")
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
        if os.environ.get("EMU_API_ONLY") not in ("bursts", "sent", "stalled", "events"):
            lane.card_checks()
            lane.run_b()
            if getattr(a, "old_updater", None):
                lane.run_c()
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
