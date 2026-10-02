# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl>=1.5"]
# ///
"""Sign, verify and inspect OpenDMD firmware bundles (docs/ota.md).

    uv run tools/fw-sign.py keygen [--key K] [--pub P]
    uv run tools/fw-sign.py sign <app.bin> --version <v> --out <bundle.odfw> [--key K]
    uv run tools/fw-sign.py verify <bundle.odfw> [--pub P]
    uv run tools/fw-sign.py inspect <bundle.odfw>
    uv run tools/fw-sign.py selftest          # sign, verify, tamper each field -> refused

A bundle is a 140-byte header and then the app image (what `espflash
save-image` writes, starting with the ESP image magic 0xE9):

    magic     "ODFW1\\0\\0\\0"  8 B
    version   32 B, UTF-8, NUL-padded
    length    u32 LE, image bytes
    sha256    32 B, of the image
    sig       64 B, Ed25519 over magic|version|length|sha256 (the first 76 B)

The private key is the raw 32-byte Ed25519 seed, by default
~/.config/opendmd/opendmd.key (mode 0600, never in the repo); the public key
is the raw 32 bytes, by default firmware/keys/opendmd.pub (committed: the
firmware compiles it in). `keygen` refuses to overwrite either. The
firmware's verifier is `opendmd_core::ota`, whose tests hold a vector made
by this script.
"""
import argparse
import hashlib
import os
import struct
import sys
from pathlib import Path

from nacl.exceptions import BadSignatureError
from nacl.signing import SigningKey, VerifyKey

MAGIC = b"ODFW1\0\0\0"
VERSION_LEN = 32
SIGNED_LEN = 8 + VERSION_LEN + 4 + 32  # 76
HEADER_LEN = SIGNED_LEN + 64  # 140
ESP_IMAGE_MAGIC = 0xE9
# The v2 factory partition (firmware/partitions.csv).
FACTORY_BYTES = 0x240000

REPO = Path(__file__).resolve().parent.parent
DEFAULT_KEY = Path.home() / ".config" / "opendmd" / "opendmd.key"
DEFAULT_PUB = REPO / "firmware" / "keys" / "opendmd.pub"


def die(msg: str, code: int = 1) -> None:
    print(f"fw-sign: {msg}", file=sys.stderr)
    sys.exit(code)


def load_key(path: Path) -> SigningKey:
    try:
        seed = path.read_bytes()
    except OSError as e:
        die(f"cannot read the private key {path}: {e} (make one with `keygen`)")
    if len(seed) != 32:
        die(f"{path} is {len(seed)} B, a key is a 32-byte Ed25519 seed")
    return SigningKey(seed)


def load_pub(path: Path) -> VerifyKey:
    try:
        raw = path.read_bytes()
    except OSError as e:
        die(f"cannot read the public key {path}: {e}")
    if len(raw) != 32:
        die(f"{path} is {len(raw)} B, a public key is 32 B")
    return VerifyKey(raw)


def build_header(version: str, image: bytes, key: SigningKey) -> bytes:
    v = version.encode()
    if not v or len(v) > VERSION_LEN or b"\0" in v:
        die(f"version must be 1..{VERSION_LEN} bytes of UTF-8 without NUL: {version!r}")
    signed = (
        MAGIC
        + v.ljust(VERSION_LEN, b"\0")
        + struct.pack("<I", len(image))
        + hashlib.sha256(image).digest()
    )
    assert len(signed) == SIGNED_LEN
    sig = key.sign(signed).signature
    return signed + sig


def parse_header(data: bytes) -> dict:
    if len(data) < HEADER_LEN:
        raise ValueError(f"truncated: {len(data)} B, the header alone is {HEADER_LEN} B")
    if data[:8] != MAGIC:
        raise ValueError("not an OpenDMD firmware bundle (bad magic)")
    raw_v = data[8 : 8 + VERSION_LEN]
    v = raw_v.rstrip(b"\0")
    if not v or b"\0" in v:
        raise ValueError("bad version field")
    (length,) = struct.unpack_from("<I", data, 40)
    return {
        "version": v.decode(),
        "length": length,
        "sha256": data[44:76],
        "sig": data[76:140],
        "signed": data[:SIGNED_LEN],
    }


def cmd_keygen(a: argparse.Namespace) -> None:
    key_path, pub_path = Path(a.key).expanduser(), Path(a.pub)
    for p in (key_path, pub_path):
        if p.exists():
            die(f"{p} exists; refusing to overwrite a key")
    sk = SigningKey.generate()
    key_path.parent.mkdir(parents=True, exist_ok=True)
    os.chmod(key_path.parent, 0o700)
    fd = os.open(key_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as f:
        f.write(bytes(sk))
    pub_path.parent.mkdir(parents=True, exist_ok=True)
    pub_path.write_bytes(bytes(sk.verify_key))
    print(f"private key: {key_path} (0600; keep it out of the repo, back it up)")
    print(f"public key:  {pub_path} ({bytes(sk.verify_key).hex()})")


def cmd_sign(a: argparse.Namespace) -> None:
    image = Path(a.image).read_bytes()
    if not image or image[0] != ESP_IMAGE_MAGIC:
        die(f"{a.image} is not an ESP app image (first byte is not 0xE9)")
    if len(image) > FACTORY_BYTES:
        die(f"{a.image} is {len(image)} B, the factory partition is {FACTORY_BYTES} B")
    header = build_header(a.version, image, load_key(Path(a.key).expanduser()))
    Path(a.out).write_bytes(header + image)
    print(f"{a.out}: version {a.version!r}, image {len(image)} B, sha256 {hashlib.sha256(image).hexdigest()}")


def check(data: bytes, pub: VerifyKey) -> dict:
    h = parse_header(data)
    image = data[HEADER_LEN:]
    if h["length"] != len(image):
        raise ValueError(f"length mismatch: the header says {h['length']} B, the image is {len(image)} B")
    if h["length"] > FACTORY_BYTES:
        raise ValueError(f"the image is {h['length']} B, the factory partition is {FACTORY_BYTES} B")
    if not image or image[0] != ESP_IMAGE_MAGIC:
        raise ValueError("the image does not start with the ESP image magic 0xE9")
    if hashlib.sha256(image).digest() != h["sha256"]:
        raise ValueError("SHA-256 mismatch")
    try:
        pub.verify(h["signed"], h["sig"])
    except BadSignatureError:
        raise ValueError("bad signature (wrong key, or the header was changed)") from None
    return h


def cmd_verify(a: argparse.Namespace) -> None:
    data = Path(a.bundle).read_bytes()
    try:
        h = check(data, load_pub(Path(a.pub)))
    except ValueError as e:
        die(f"{a.bundle}: {e}")
    print(f"{a.bundle}: ok, version {h['version']!r}, image {h['length']} B")


def cmd_inspect(a: argparse.Namespace) -> None:
    data = Path(a.bundle).read_bytes()
    try:
        h = parse_header(data)
    except ValueError as e:
        die(f"{a.bundle}: {e}")
    image = data[HEADER_LEN:]
    print(f"bundle   {a.bundle}, {len(data)} B")
    print(f"version  {h['version']}")
    print(f"length   {h['length']} B (image present: {len(image)} B)")
    print(f"sha256   {h['sha256'].hex()} ({'matches' if hashlib.sha256(image).digest() == h['sha256'] else 'DOES NOT match'})")
    print(f"sig      {h['sig'].hex()}")
    print(f"image[0] 0x{image[0]:02X}" if image else "image[0] (none)")


def cmd_selftest(_a: argparse.Namespace) -> None:
    sk = SigningKey.generate()
    image = bytes([ESP_IMAGE_MAGIC]) + os.urandom(4095)
    bundle = build_header("9.9.9-selftest", image, sk) + image
    check(bundle, sk.verify_key)
    cases = {
        "magic": 0, "version": 8, "length": 40, "sha256": 50, "sig": 100, "image": HEADER_LEN + 10,
    }
    for name, at in cases.items():
        b = bytearray(bundle)
        b[at] ^= 0x01
        try:
            check(bytes(b), sk.verify_key)
        except ValueError as e:
            print(f"tampered {name:8} -> refused: {e}")
        else:
            die(f"a bundle with its {name} tampered verified")
    try:
        check(bundle, SigningKey.generate().verify_key)
    except ValueError as e:
        print(f"wrong key        -> refused: {e}")
    else:
        die("a bundle verified under the wrong key")
    try:
        check(bundle[:-1], sk.verify_key)
    except ValueError as e:
        print(f"cut short        -> refused: {e}")
    else:
        die("a cut-short bundle verified")
    print("selftest ok")


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    k = sub.add_parser("keygen", help="make a key pair (refuses to overwrite)")
    k.add_argument("--key", default=str(DEFAULT_KEY))
    k.add_argument("--pub", default=str(DEFAULT_PUB))
    s = sub.add_parser("sign", help="app image -> signed bundle")
    s.add_argument("image")
    s.add_argument("--version", required=True)
    s.add_argument("--out", required=True)
    s.add_argument("--key", default=str(DEFAULT_KEY))
    v = sub.add_parser("verify", help="check a bundle as the device does")
    v.add_argument("bundle")
    v.add_argument("--pub", default=str(DEFAULT_PUB))
    i = sub.add_parser("inspect", help="print a bundle's header")
    i.add_argument("bundle")
    sub.add_parser("selftest", help="round trip and tamper checks with a throwaway key")
    a = p.parse_args()
    {"keygen": cmd_keygen, "sign": cmd_sign, "verify": cmd_verify, "inspect": cmd_inspect, "selftest": cmd_selftest}[a.cmd](a)


if __name__ == "__main__":
    main()
