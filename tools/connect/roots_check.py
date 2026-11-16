# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Do the panel's pinned TLS roots still trust the Connect server's live chain?

Remote control's TLS client (firmware/src/connect/tls.rs) trusts only the
roots in its `ROOTS` list (DER files in firmware/connect-roots/), and offers
one cipher suite, ECDHE-ECDSA-AES128-GCM-SHA256 over TLS 1.2. If Cloudflare
moves connect.opendmd.dev to a chain that ends at none of them, every panel
in the field loses Remote control until it is updated. This check, run
weekly by .github/workflows/connect-roots.yml (`make connect-roots-check`
by hand), catches that, and a pinned root's expiry, ahead of time:

1. the pins: every file in firmware/connect-roots/ is in `ROOTS` and every
   entry has its file; each parses; its expiry;
2. the live chain (unless --pins-only): a TLS 1.2 handshake with the
   panel's one suite (and groups P-256, P-384), then `GET /v1/health` on
   it (the only request made); the chain verified by `openssl verify`
   against the pinned roots only (no system store), for the host name;
   the first certificate from the leaf whose issuer is a pinned root, as
   the firmware's trusted-certificate callback finds it;
   the Certificate message (the chain's DER and its framing) must fit
   the panel's TLS input cap (the Makefile's CONNECT_TLS_IN), with a
   warning within 512 B of it;
3. expiry: the root the live chain ends at must have more than
   --fail-days (365) left, and a warning names any pinned root with less
   than --warn-days (730).

    uv run tools/connect/roots_check.py [--host connect.opendmd.dev] [--pins-only]

Needs the `openssl` command (3.x). No secrets; contacts nothing but --host.
Exit status 0 when every check passed (warnings allowed), 1 otherwise.
"""
from __future__ import annotations

import argparse
import datetime as dt
import os
import re
import ssl
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
TLS_RS = REPO / "firmware" / "src" / "connect" / "tls.rs"
ROOTS_DIR = REPO / "firmware" / "connect-roots"
SUITE = "ECDHE-ECDSA-AES128-GCM-SHA256"
GH = os.environ.get("GITHUB_ACTIONS") == "true"

FIX = """
What to do (the panels in the field depend on this):
  - Find the live chain's root: openssl s_client -connect {host}:443 -servername {host} \\
      -tls1_2 -cipher {suite} -showcerts </dev/null
  - If Cloudflare now issues from another CA, either pin that CA's root (its DER into
    firmware/connect-roots/, a line in ROOTS in firmware/src/connect/tls.rs, a firmware
    release, and time for panels to update before the old chain goes), or set the
    zone's edge certificate back to a CA that is pinned (Cloudflare dashboard,
    SSL/TLS > Edge Certificates: the certificate authority of the advanced certificate).
  - If a pinned root is near its expiry, pin its successor the same way, well ahead.
  - Run this again: uv run tools/connect/roots_check.py --host {host}
"""


class Report:
    def __init__(self):
        self.failed = 0
        self.warned = 0

    def ok(self, msg: str):
        print(f"PASS: {msg}", flush=True)

    def warn(self, msg: str):
        self.warned += 1
        print(f"WARN: {msg}", flush=True)
        if GH:
            print(f"::warning title=Connect roots::{msg}", flush=True)

    def fail(self, msg: str):
        self.failed += 1
        print(f"FAIL: {msg}", flush=True)
        if GH:
            print(f"::error title=Connect roots::{msg}", flush=True)

    def info(self, msg: str):
        print(f"      {msg}", flush=True)


def openssl(*args: str, data: bytes | None = None, timeout: float = 30) -> subprocess.CompletedProcess:
    return subprocess.run(["openssl", *args], input=data, capture_output=True, check=False, timeout=timeout)


def pinned() -> list[tuple[str, Path]]:
    """`ROOTS` in tls.rs: (name, file), the build-time test root left out."""
    src = TLS_RS.read_text()
    block = re.search(r"pub static ROOTS: &\[\(&str, &\[u8\]\)\] = &\[(.*?)\n\];", src, re.S)
    if not block:
        sys.exit(f"cannot find `ROOTS` in {TLS_RS}")
    out = re.findall(r'\(\s*"([^"]+)",\s*include_bytes!\("\.\./\.\./connect-roots/([^"]+)"\)\s*\)', block[1])
    return [(name, ROOTS_DIR / f) for name, f in out]


def _tlv(b: bytes, at: int) -> tuple[int, int, int, int]:
    """One DER element at `at`: (tag, start of the whole element, start of its
    contents, end). Raises ValueError on a malformed one."""
    if at + 2 > len(b):
        raise ValueError("short")
    tag, n = b[at], b[at + 1]
    k = at + 2
    if n & 0x80:
        m = n & 0x7F
        if not 1 <= m <= 4 or k + m > len(b):
            raise ValueError("bad length")
        n = int.from_bytes(b[k:k + m], "big")
        k += m
    if k + n > len(b):
        raise ValueError("overrun")
    return tag, at, k, k + n


def _time(tag: int, v: bytes) -> dt.datetime:
    t = v.decode("ascii")
    if tag == 0x17:  # UTCTime, YYMMDDHHMMSSZ (RFC 5280: 50-99 -> 19xx)
        y = int(t[:2])
        t = ("19" if y >= 50 else "20") + t
    elif tag != 0x18:  # GeneralizedTime
        raise ValueError("not a time")
    return dt.datetime.strptime(t, "%Y%m%d%H%M%SZ").replace(tzinfo=dt.timezone.utc)


def cert_fields(der: bytes) -> dict:
    """A certificate's issuer and subject as their DER bytes (what the
    firmware's callback compares, `cx::cert_subject`), notAfter, and both
    names as text (for the log only). {} if it does not parse."""
    try:
        _, _, c, _ = _tlv(der, 0)                 # Certificate
        _, _, t, t_end = _tlv(der, c)             # tbsCertificate
        at = t
        tag, _, _, end = _tlv(der, at)
        if tag == 0xA0:                           # [0] version
            at = end
        at = _tlv(der, at)[3]                     # serialNumber
        at = _tlv(der, at)[3]                     # signature
        _, i0, _, i1 = _tlv(der, at)              # issuer
        _, _, v, v_end = _tlv(der, i1)            # validity
        nb = _tlv(der, v)
        na = _tlv(der, nb[3])
        _, s0, _, s1 = _tlv(der, v_end)           # subject
        f = {"issuer_der": der[i0:i1], "subject_der": der[s0:s1],
             "not_after": _time(na[0], der[na[2]:na[3]])}
    except (ValueError, UnicodeDecodeError, IndexError):
        return {}
    r = openssl("x509", "-inform", "der", "-noout", "-subject", "-issuer", "-nameopt", "RFC2253", data=der)
    for line in r.stdout.decode(errors="replace").splitlines():
        k, _, v = line.partition("=")
        f[k.strip()] = v.strip()
    f.setdefault("subject", f["subject_der"].hex()[:32])
    f.setdefault("issuer", f["issuer_der"].hex()[:32])
    return f


def days_left(f: dict) -> int:
    return (f["not_after"] - dt.datetime.now(dt.timezone.utc)).days


def tls_in() -> int:
    """The panel's TLS input cap: the Makefile's CONNECT_TLS_IN (what it
    writes as CONFIG_MBEDTLS_SSL_IN_CONTENT_LEN beside sdkconfig.connect)."""
    m = re.search(r"^CONNECT_TLS_IN \?=\s*(\d+)", (REPO / "Makefile").read_text(), re.M)
    if not m:
        sys.exit("cannot find CONNECT_TLS_IN in the Makefile")
    return int(m[1])


def check_pins(rep: Report, warn_days: int) -> dict[str, tuple[str, dict, bytes]]:
    pins = pinned()
    if not pins:
        rep.fail(f"`ROOTS` in {TLS_RS.relative_to(REPO)} names no root")
        return {}
    files = {p.name for _, p in pins}
    on_disk = {p.name for p in ROOTS_DIR.glob("*.der")}
    for extra in sorted(on_disk - files):
        rep.fail(f"firmware/connect-roots/{extra} is not in ROOTS (tls.rs): it is not pinned, so it trusts nothing")
    by_subject = {}
    for name, path in pins:
        if not path.exists():
            rep.fail(f"ROOTS names {name} ({path.name}), but firmware/connect-roots/{path.name} is missing")
            continue
        der = path.read_bytes()
        f = cert_fields(der)
        if not f:
            rep.fail(f"{path.name} does not parse as a DER certificate")
            continue
        left = days_left(f)
        rep.info(f"pinned {name}: {f['subject']}, expires {f['not_after']:%Y-%m-%d} ({left} days)")
        if left < 0:
            rep.fail(f"pinned root {name} expired on {f['not_after']:%Y-%m-%d}")
        elif left < warn_days:
            rep.warn(f"pinned root {name} expires on {f['not_after']:%Y-%m-%d}, in {left} days: pin its successor well before")
        by_subject[f["subject_der"]] = (name, f, der)
    if not rep.failed:
        rep.ok(f"{len(pins)} pinned roots in tls.rs, each with its file in firmware/connect-roots/, each parses")
    return by_subject


def check_live(rep: Report, host: str, by_subject: dict, fail_days: int):
    req = f"GET /v1/health HTTP/1.1\r\nHost: {host}\r\nUser-Agent: opendmd-roots-check\r\nConnection: close\r\n\r\n".encode()
    try:
        r = openssl("s_client", "-connect", f"{host}:443", "-servername", host, "-tls1_2", "-cipher", SUITE,
                    "-groups", "P-256:P-384", "-showcerts", "-ign_eof", data=req, timeout=45)
    except subprocess.TimeoutExpired:
        rep.fail(f"{host}: no answer within 45 s")
        return
    out = r.stdout.decode(errors="replace")
    pems = re.findall(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----\n?", out, re.S)
    if not pems:
        rep.fail(f"{host}: no TLS 1.2 handshake with the panel's suite {SUITE} "
                 f"(the panel could not connect at all): {(r.stderr.decode(errors='replace') or out)[-600:]}")
        return
    rep.ok(f"{host}: a TLS 1.2 handshake with {SUITE}, {len(pems)} certificates sent")
    status = re.search(r"^HTTP/1\.[01] (\d{3})", out, re.M)
    if status and status[1] == "200":
        rep.ok(f"{host}: GET /v1/health answers 200 on that session")
    else:
        rep.warn(f"{host}: GET /v1/health answered {status[1] if status else 'nothing'} (the chain is what this checks; the server may be down)")
    ders = [ssl.PEM_cert_to_DER_cert(p) for p in pems]
    chain = [cert_fields(d) for d in ders]
    for k, f in enumerate(chain):
        if f:
            rep.info(f"[{k}] {f['subject']} <- {f['issuer']}, {len(ders[k])} B, expires {f['not_after']:%Y-%m-%d}")
        else:
            rep.fail(f"{host}: certificate [{k}] of the live chain does not parse")
    # The Certificate handshake message must fit the panel's TLS input
    # buffer (CONNECT_TLS_IN: a larger record ends the session): its 4-byte
    # header, the list's 3-byte length, 3 bytes before each certificate.
    msg = 4 + 3 + sum(3 + len(d) for d in ders)
    cap = tls_in()
    if msg > cap:
        rep.fail(f"{host}: the Certificate message is {msg} B, over the panel's TLS input cap of {cap} B (CONNECT_TLS_IN)")
    elif msg > cap - 512:
        rep.warn(f"{host}: the Certificate message is {msg} B, within 512 B of the panel's TLS input cap of {cap} B")
    else:
        rep.ok(f"{host}: the Certificate message is {msg} B, under the panel's TLS input cap of {cap} B")
    # The firmware's trusted-certificate callback: the first certificate
    # from the leaf whose issuer (its DER bytes) is a pinned root's subject
    # ends the chain there.
    used = next(((k, by_subject[f["issuer_der"]]) for k, f in enumerate(chain) if f and f["issuer_der"] in by_subject), None)
    if used is None:
        rep.fail(f"{host}: no certificate in the live chain is issued by a pinned root "
                 f"(issuers: {[f.get('issuer') for f in chain]})")
    else:
        k, (name, f, _) = used
        rep.ok(f"{host}: certificate [{k}] is issued by the pinned root {name}")
        left = days_left(f)
        if left < fail_days:
            rep.fail(f"the live chain's root {name} expires on {f['not_after']:%Y-%m-%d}, in {left} days (under {fail_days})")
        else:
            rep.ok(f"the live chain's root {name} has {left} days left (over {fail_days})")
    # The chain verified as a client would, against the pinned roots only.
    with tempfile.TemporaryDirectory() as d:
        roots = Path(d, "roots.pem")
        with roots.open("wb") as w:
            for _, _, der in by_subject.values():
                w.write(openssl("x509", "-inform", "der", data=der).stdout)
        Path(d, "leaf.pem").write_text(pems[0])
        Path(d, "chain.pem").write_text("".join(pems[1:]))
        args = ["verify", "-no-CApath", "-no-CAstore", "-CAfile", str(roots), "-purpose", "sslserver",
                "-verify_hostname", host]
        if len(pems) > 1:
            args += ["-untrusted", str(Path(d, "chain.pem"))]
        v = openssl(*args, str(Path(d, "leaf.pem")))
        if v.returncode == 0:
            rep.ok(f"{host}: openssl verify, pinned roots only, for the name {host}: OK")
        else:
            rep.fail(f"{host}: openssl verify against the pinned roots fails: {(v.stdout + v.stderr).decode(errors='replace').strip()[-400:]}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--host", default="connect.opendmd.dev")
    ap.add_argument("--pins-only", action="store_true", help="check the pinned roots only (no network)")
    ap.add_argument("--fail-days", type=int, default=365)
    ap.add_argument("--warn-days", type=int, default=730)
    a = ap.parse_args()
    rep = Report()
    by_subject = check_pins(rep, a.warn_days)
    if not a.pins_only and by_subject:
        check_live(rep, a.host, by_subject, a.fail_days)
    verdict = "FAILED" if rep.failed else "passed"
    print(f"\nconnect roots check {verdict}: {rep.failed} failed, {rep.warned} warnings", flush=True)
    if rep.failed and not a.pins_only:
        print(FIX.format(host=a.host, suite=SUITE), flush=True)
    return 1 if rep.failed else 0


if __name__ == "__main__":
    sys.exit(main())
