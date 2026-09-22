# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Push a local directory onto the player's SD card over the HTTP API.

This is how the web app gets onto a card: every file under <dir> is PUT to
`/api/files/www/<relative path>`, which the firmware streams to the card in
4 KB chunks and renames into place. `pkg/` (the wasm-pack output) is included —
it is the whole point.

    uv run tools/card-sync.py <host> opendmd-web/www [--prefix /www] [--dry-run]

Prints each file's size and transfer rate, then a total. Nothing is skipped on
the basis of timestamps: the device has no clock we trust and the card is the
only copy that matters.
"""
import argparse
import http.client
import os
import sys
import time

def put(host, path, data, timeout):
    conn = http.client.HTTPConnection(host, timeout=timeout)
    try:
        conn.request("PUT", path, body=data,
                     headers={"Content-Type": "application/octet-stream",
                              "Content-Length": str(len(data))})
        resp = conn.getresponse()
        body = resp.read().decode("utf-8", "replace")
        return resp.status, body
    finally:
        conn.close()

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("host", help="device IP or hostname, e.g. 192.168.1.50 or opendmd.local")
    ap.add_argument("dir", help="local directory to upload")
    ap.add_argument("--prefix", default="/www", help="card-absolute destination prefix")
    ap.add_argument("--timeout", type=float, default=60.0)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    root = os.path.abspath(args.dir)
    if not os.path.isdir(root):
        sys.exit(f"{root}: not a directory")

    files = []
    for dirpath, _dirnames, filenames in os.walk(root):
        for name in sorted(filenames):
            if name.startswith("."):
                continue
            full = os.path.join(dirpath, name)
            rel = os.path.relpath(full, root).replace(os.sep, "/")
            files.append((full, rel))
    files.sort(key=lambda f: f[1])
    if not files:
        sys.exit(f"{root}: nothing to upload")

    prefix = args.prefix.strip("/")
    total_bytes = 0
    failures = 0
    started = time.monotonic()

    for full, rel in files:
        data = open(full, "rb").read()
        dest = f"/api/files/{prefix}/{rel}" if prefix else f"/api/files/{rel}"
        if args.dry_run:
            print(f"{rel:<44} {len(data):>9,} B  (dry run -> {dest})")
            continue
        t0 = time.monotonic()
        try:
            status, body = put(args.host, dest, data, args.timeout)
        except OSError as e:
            print(f"{rel:<44} {len(data):>9,} B  FAILED: {e}")
            failures += 1
            continue
        dt = time.monotonic() - t0
        rate = len(data) / dt / 1024 if dt > 0 else 0
        if status == 200:
            print(f"{rel:<44} {len(data):>9,} B  {dt:6.2f} s  {rate:8.1f} KB/s")
            total_bytes += len(data)
        else:
            print(f"{rel:<44} {len(data):>9,} B  HTTP {status}: {body.strip()[:120]}")
            failures += 1

    if args.dry_run:
        return
    elapsed = time.monotonic() - started
    rate = total_bytes / elapsed / 1024 if elapsed > 0 else 0
    print(f"\n{len(files) - failures}/{len(files)} files, {total_bytes:,} B "
          f"in {elapsed:.1f} s ({rate:.1f} KB/s overall)")
    if failures:
        sys.exit(1)

if __name__ == "__main__":
    main()
