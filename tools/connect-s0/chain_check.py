# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Does the live chain of a hostname end at one of the probe's pinned roots
(firmware/connect-roots/*.der)? CONNECT.md 5.2's check, for the maintainer
to run (it connects to the host; nothing in the repo runs it for you):

    uv run tools/connect-s0/chain_check.py connect-s0.opendmd.dev

Offers only the probe's suite (ECDHE-ECDSA-AES128-GCM-SHA256, TLS 1.2), as
the panel does, prints each certificate's subject, issuer, key and expiry,
and the pinned root whose subject is the top issuer (or FAIL). Needs the
`openssl` command.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOTS = Path(__file__).resolve().parents[2] / "firmware" / "connect-roots"


def openssl(*args, data=None):
    return subprocess.run(["openssl", *args], input=data, capture_output=True, check=False).stdout


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    host = sys.argv[1]
    out = openssl("s_client", "-connect", f"{host}:443", "-servername", host, "-tls1_2",
                  "-cipher", "ECDHE-ECDSA-AES128-GCM-SHA256", "-showcerts", data=b"").decode(errors="replace")
    pems = re.findall(r"-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----\n?", out, re.S)
    if not pems:
        sys.exit(f"FAIL: no certificates from {host} with the probe's suite:\n{out[-800:]}")
    size = 0
    issuers = []
    for k, pem in enumerate(pems):
        der = openssl("x509", "-outform", "der", data=pem.encode())
        size += len(der)
        info = openssl("x509", "-noout", "-subject", "-issuer", "-enddate", "-nameopt", "RFC2253", data=pem.encode()).decode()
        key = openssl("x509", "-noout", "-text", data=pem.encode()).decode()
        alg = re.search(r"Public Key Algorithm: (\S+)", key)
        curve = re.search(r"NIST CURVE: (\S+)|ASN1 OID: (\S+)", key)
        print(f"[{k}] {len(der)} B, {alg[1] if alg else '?'} {(curve[1] or curve[2]) if curve else ''}")
        for line in info.splitlines():
            print(f"    {line}")
            if line.startswith("issuer="):
                issuers.append(line[len("issuer="):])
    print(f"chain: {len(pems)} certificates, {size} B of DER")
    # mbedTLS looks for each certificate's parent among the trusted roots
    # first, so the chain is trusted at the first certificate (from the
    # leaf) whose issuer is a pinned root (a cross-signed root sent above
    # it does not matter).
    pinned = {}
    for der in sorted(ROOTS.glob("*.der")):
        subj = openssl("x509", "-inform", "der", "-in", str(der), "-noout", "-subject", "-nameopt", "RFC2253").decode().strip()
        pinned[subj[len("subject="):]] = der.name
    for k, issuer in enumerate(issuers):
        if issuer in pinned:
            print(f"PASS: certificate [{k}] is issued by a pinned root: {pinned[issuer]} ({issuer})")
            return 0
    print(f"FAIL: no certificate's issuer is a pinned root ({issuers}; pinned: {list(pinned)})")
    return 1


if __name__ == "__main__":
    sys.exit(main())
