# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""The gallery key in a firmware image is the committed one, never a test key.

    uv run tools/gallery-key-check.py <app.bin>
    uv run tools/gallery-key-check.py --selftest

An image built with `connect-formula-add` (it carries "formula-add: ") must
hold the committed gallery key, firmware/keys/gallery.pub, whose hex is
pinned in opendmd-core/src/gallery_sig_set.rs (PINNED): the two must
agree, and the key's 32 bytes must be in the image. Any image (with the
feature or without) must hold none of the test keys: the shared vectors'
(opendmd-core/tests/gallery-vectors.json) and the throwaway keys the tests
make from their seeds. An image without the feature must hold no gallery
key at all (nothing of gallery_sig is in a release).

Run by make fw-release-variant on every image it builds (releases and the
emulator's); `--selftest` by make test. Prints one `pass:` line, or exits 1
with the reason.
"""
import hashlib
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MARKER = b"formula-add: "


class Refused(Exception):
    pass


def die(msg: str) -> None:
    raise Refused(msg)


def pinned() -> dict[int, bytes]:
    src = (REPO / "opendmd-core" / "src" / "gallery_sig_set.rs").read_text()
    m = re.search(r"^const PINNED: &\[\(u8, &str\)\] = &\[(.*)\];$", src, re.M)
    if not m:
        die("cannot find PINNED in opendmd-core/src/gallery_sig_set.rs")
    return {int(i): bytes.fromhex(h) for i, h in re.findall(r'\((\d+), "([0-9a-f]{64})"\)', m.group(1))}


def committed() -> dict[int, bytes]:
    out = {}
    for p in sorted((REPO / "firmware" / "keys").glob("gallery*.pub")):
        rest = p.name[len("gallery"):-len(".pub")]
        out[0 if rest == "" else int(rest.lstrip("-"))] = p.read_bytes()
    return out


def test_keys() -> list[bytes]:
    """The vectors' public keys (the throwaway seeds' among them)."""
    v = json.loads((REPO / "opendmd-core" / "tests" / "gallery-vectors.json").read_text())
    keys = [bytes.fromhex(k["pub"]) for k in v["keys"]]
    return [k for k in keys if k != bytes([1] + [0] * 31)]  # (not the identity point: 0x01 then zeros is common data)


def check(image: bytes, name: str, keys: dict[int, bytes], tests: list[bytes]) -> str:
    for k in tests:
        if k in image:
            die(f"{name} holds a test key ({k.hex()[:16]}...): never in a firmware image")
    with_add = MARKER in image
    if with_add:
        if not keys:
            die(f"{name} is built with connect-formula-add but no gallery key is pinned")
        for kid, k in keys.items():
            if k not in image:
                die(f"{name} is built with connect-formula-add but does not hold the committed gallery key {kid} ({k.hex()[:16]}...)")
        return f"pass: {name}: with formula-add, holds the committed gallery key(s) {sorted(keys)} (pinned), no test key"
    for kid, k in keys.items():
        if k in image:
            die(f"{name} has no formula-add but holds gallery key {kid}: nothing of gallery_sig belongs in it")
    return f"pass: {name}: no formula-add, no gallery key, no test key"


def keys_agree() -> dict[int, bytes]:
    p, c = pinned(), committed()
    if p != c:
        die(f"PINNED ({ {i: k.hex()[:16] for i, k in p.items()} }) and firmware/keys/gallery*.pub ({ {i: k.hex()[:16] for i, k in c.items()} }) disagree")
    return p


def selftest() -> None:
    keys, tests = keys_agree(), test_keys()
    real = next(iter(keys.values()), hashlib.sha256(b"no key pinned").digest())
    cases = [
        (b"x" * 100, True, "a release: nothing"),
        (b"x" + MARKER + real + b"y", True, "with the feature and the key"),
        (b"x" + MARKER + b"y", False, "with the feature, no key"),
        (b"x" + MARKER + real + tests[0] + b"y", False, "with the feature, the key and a test key"),
        (b"x" + tests[-1] + b"y", False, "a release with a test key"),
        (b"x" + real + b"y", False, "a release with the gallery key"),
    ]
    for img, want, what in cases:
        try:
            check(img, what, keys, tests)
            got = True
        except Refused:
            got = False
        if got != want:
            print(f"FAIL: gallery-key-check selftest: {what}: {'passed' if got else 'failed'}", file=sys.stderr)
            sys.exit(1)
    print(f"pass: gallery-key-check selftest: {len(cases)} images, each judged right ({len(tests)} test keys refused)")


def main() -> None:
    if len(sys.argv) == 2 and sys.argv[1] == "--selftest":
        return selftest()
    if len(sys.argv) != 2:
        die("usage: gallery-key-check.py <app.bin> | --selftest")
    p = Path(sys.argv[1])
    print(check(p.read_bytes(), p.name, keys_agree(), test_keys()))


if __name__ == "__main__":
    try:
        main()
    except Refused as e:
        print(f"FAIL: gallery-key-check: {e}", file=sys.stderr)
        sys.exit(1)
