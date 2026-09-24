# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Push a local directory onto the player's SD card over the HTTP API.

This is how the web app gets onto a card: every file under <dir> is PUT to
`/api/files/www/<relative path>`, which the firmware receives on its stream
task (one transfer at a time, downloads included), writes to the card in
2 KB chunks and renames into place. `pkg/` (the wasm-pack output) is
included — it is the whole point. A `503` (the device's transfer queue was
full, or its heap was short) is retried after the `Retry-After` it gives,
up to three times.

    uv run tools/card-sync.py <host> opendmd-web/www [--prefix /www] [--dry-run]

Or a list of card-absolute paths, each read from the same path under a local
copy of the card — how prepared copies (`opendmd-pack convert`'s
`cache/…` files) get onto a card:

    uv run tools/card-sync.py <host> --from ~/scratch/dmdos-card --paths copies.txt

The list may be a manifest: blank lines are skipped. Paths are percent-encoded
(the card has `/gifs/NEW 2021/`).

Prints each file's size and transfer rate, then a total. Nothing is skipped on
the basis of timestamps: the device has no clock we trust and the card is the
only copy that matters.
"""
import argparse
import http.client
import os
import sys
import time
import urllib.parse

def put(host, path, data, timeout, retries=3):
    for attempt in range(retries + 1):
        conn = http.client.HTTPConnection(host, timeout=timeout)
        try:
            conn.request("PUT", path, body=data,
                         headers={"Content-Type": "application/octet-stream",
                                  "Content-Length": str(len(data))})
            resp = conn.getresponse()
            body = resp.read().decode("utf-8", "replace")
            wait = resp.getheader("Retry-After")
        finally:
            conn.close()
        if resp.status != 503 or attempt == retries:
            return resp.status, body
        time.sleep(float(wait) if wait and wait.isdigit() else 1.0)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("host", help="device IP or hostname, e.g. 192.168.1.50 or opendmd.local")
    ap.add_argument("dir", nargs="?", help="local directory to upload")
    ap.add_argument("--from", dest="src", help="local card copy that --paths are read from")
    ap.add_argument("--paths", help="file of card-absolute paths to upload from --from")
    ap.add_argument("--prefix", default="/www", help="card-absolute destination prefix")
    ap.add_argument("--timeout", type=float, default=60.0)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    files = []
    if args.paths:
        if not args.src or args.dir:
            sys.exit("--paths needs --from <card copy> and no <dir>")
        root = os.path.abspath(args.src)
        args.prefix = ""
        with open(args.paths, encoding="utf-8") as f:
            for line in f:
                rel = line.strip().lstrip("/")
                if rel:
                    files.append((os.path.join(root, rel), rel))
        if not files:
            sys.exit(f"{args.paths}: no paths")
    elif not args.dir:
        sys.exit("need <dir>, or --from and --paths")
    else:
        root = os.path.abspath(args.dir)
        if not os.path.isdir(root):
            sys.exit(f"{root}: not a directory")
    for dirpath, _dirnames, filenames in (os.walk(root) if not args.paths else []):
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
        quoted = urllib.parse.quote(rel)
        dest = f"/api/files/{prefix}/{quoted}" if prefix else f"/api/files/{quoted}"
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
