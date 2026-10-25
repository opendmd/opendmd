# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl>=1.5"]
# ///
"""Sign, verify and inspect OpenDMD firmware bundles (docs/ota.md).

    uv run tools/fw-sign.py keygen [--key K] [--pub P]
    uv run tools/fw-sign.py sign <app.bin> --version <v> [--variant 4mb|8mb] --out <bundle.odfw> [--key K]
    uv run tools/fw-sign.py verify <bundle.odfw> [--pub P]
    uv run tools/fw-sign.py inspect <bundle.odfw>
    uv run tools/fw-sign.py selftest          # sign, verify, tamper each field -> refused

A bundle is a 140-byte header and then the app image (what `espflash
save-image` writes, starting with the ESP image magic 0xE9):

    magic     "ODFW1" + the flash layout: "\\0\\0\\0" (4mb) or "8mb"   8 B
    version   32 B, UTF-8, NUL-padded
    length    u32 LE, image bytes
    sha256    32 B, of the image
    sig       64 B, Ed25519 over magic|version|length|sha256 (the first 76 B)

The magic names the flash layout (the variant) the image is built for: a
4mb bundle keeps the original magic, so the 4 MB players already in use take
it; an 8mb bundle's magic is one they refuse. The device refuses a bundle
for the other layout, and an image whose app descriptor (the version string
at 0x30, "<version> <variant>") is not a build for the bundle's layout:
`sign` and `verify` check the same.

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

# The flash layouts and their magics (opendmd-core/src/ota.rs).
MAGICS = {"4mb": b"ODFW1\0\0\0", "8mb": b"ODFW18mb"}
VERSION_LEN = 32
SIGNED_LEN = 8 + VERSION_LEN + 4 + 32  # 76
HEADER_LEN = SIGNED_LEN + 64  # 140
ESP_IMAGE_MAGIC = 0xE9
# The factory partition of each layout (firmware/partitions.csv, partitions-8mb.csv).
FACTORY_BYTES = {"4mb": 0x240000, "8mb": 0x300000}
# esp_app_desc_t in an app image: its magic word at 0x20, the version at 0x30.
APP_DESC_MAGIC = 0xABCD5432

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


def app_desc_version(image: bytes) -> str | None:
    """The app descriptor's version string, or None without its magic word."""
    if len(image) < 0x50 or struct.unpack_from("<I", image, 0x20)[0] != APP_DESC_MAGIC:
        return None
    return image[0x30:0x50].split(b"\0", 1)[0].decode("utf-8", "replace")


def check_desc(image: bytes, variant: str) -> None:
    """The image is an OpenDMD build for `variant` (opendmd_core::ota::check_app_desc)."""
    found = app_desc_version(image)
    if found is None or not found.endswith(" " + variant) or len(found) <= len(variant) + 1:
        raise ValueError(f"the image inside is not an OpenDMD {variant} build "
                         f"(its app descriptor says '{found if found is not None else 'nothing: no app descriptor'}')")


def build_header(version: str, image: bytes, key: SigningKey, variant: str = "4mb") -> bytes:
    v = version.encode()
    if not v or len(v) > VERSION_LEN or b"\0" in v:
        die(f"version must be 1..{VERSION_LEN} bytes of UTF-8 without NUL: {version!r}")
    if variant not in MAGICS:
        die(f"unknown variant {variant!r}: one of {', '.join(MAGICS)}")
    signed = (
        MAGICS[variant]
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
    variant = next((k for k, m in MAGICS.items() if data[:8] == m), None)
    if variant is None:
        raise ValueError("not an OpenDMD firmware bundle (bad magic)")
    raw_v = data[8 : 8 + VERSION_LEN]
    v = raw_v.rstrip(b"\0")
    if not v or b"\0" in v:
        raise ValueError("bad version field")
    (length,) = struct.unpack_from("<I", data, 40)
    return {
        "variant": variant,
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
    if len(image) > FACTORY_BYTES[a.variant]:
        die(f"{a.image} is {len(image)} B, the {a.variant} factory partition is {FACTORY_BYTES[a.variant]} B")
    try:
        check_desc(image, a.variant)
    except ValueError as e:
        die(f"{a.image}: {e}; refusing to sign it as {a.variant}")
    header = build_header(a.version, image, load_key(Path(a.key).expanduser()), a.variant)
    Path(a.out).write_bytes(header + image)
    print(f"{a.out}: {a.variant}, version {a.version!r}, image {len(image)} B, sha256 {hashlib.sha256(image).hexdigest()}")


def check(data: bytes, pub: VerifyKey) -> dict:
    h = parse_header(data)
    image = data[HEADER_LEN:]
    if h["length"] != len(image):
        raise ValueError(f"length mismatch: the header says {h['length']} B, the image is {len(image)} B")
    if h["length"] > FACTORY_BYTES[h["variant"]]:
        raise ValueError(f"the image is {h['length']} B, the {h['variant']} factory partition is {FACTORY_BYTES[h['variant']]} B")
    if not image or image[0] != ESP_IMAGE_MAGIC:
        raise ValueError("the image does not start with the ESP image magic 0xE9")
    if hashlib.sha256(image).digest() != h["sha256"]:
        raise ValueError("SHA-256 mismatch")
    try:
        pub.verify(h["signed"], h["sig"])
    except BadSignatureError:
        raise ValueError("bad signature (wrong key, or the header was changed)") from None
    check_desc(image, h["variant"])
    return h


def cmd_verify(a: argparse.Namespace) -> None:
    data = Path(a.bundle).read_bytes()
    try:
        h = check(data, load_pub(Path(a.pub)))
    except ValueError as e:
        die(f"{a.bundle}: {e}")
    print(f"{a.bundle}: ok, {h['variant']}, version {h['version']!r}, image {h['length']} B")


def cmd_inspect(a: argparse.Namespace) -> None:
    data = Path(a.bundle).read_bytes()
    try:
        h = parse_header(data)
    except ValueError as e:
        die(f"{a.bundle}: {e}")
    image = data[HEADER_LEN:]
    print(f"bundle   {a.bundle}, {len(data)} B")
    print(f"variant  {h['variant']}")
    print(f"version  {h['version']}")
    print(f"app desc {app_desc_version(image)!r}")
    print(f"length   {h['length']} B (image present: {len(image)} B)")
    print(f"sha256   {h['sha256'].hex()} ({'matches' if hashlib.sha256(image).digest() == h['sha256'] else 'DOES NOT match'})")
    print(f"sig      {h['sig'].hex()}")
    print(f"image[0] 0x{image[0]:02X}" if image else "image[0] (none)")


def fake_image(variant: str) -> bytes:
    """4 KB that pass for an app image of `variant`: 0xE9 first, an app
    descriptor at 0x20 with "9.9.9-selftest <variant>" at 0x30."""
    image = bytearray(bytes([ESP_IMAGE_MAGIC]) + os.urandom(4095))
    struct.pack_into("<I", image, 0x20, APP_DESC_MAGIC)
    image[0x30:0x50] = f"9.9.9-selftest {variant}".encode().ljust(32, b"\0")
    return bytes(image)


def cmd_selftest(_a: argparse.Namespace) -> None:
    sk = SigningKey.generate()
    image = fake_image("4mb")
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
    image8 = fake_image("8mb")
    bundle8 = build_header("9.9.9-selftest", image8, sk, "8mb") + image8
    if check(bundle8, sk.verify_key)["variant"] != "8mb":
        die("an 8mb bundle did not read back as 8mb")
    print("8mb round trip   -> ok")
    # The layout is signed: an 8mb bundle relabelled 4mb (or the reverse) fails.
    for name, b in (("8mb as 4mb", MAGICS["4mb"] + bundle8[8:]), ("4mb as 8mb", MAGICS["8mb"] + bundle[8:])):
        try:
            check(b, sk.verify_key)
        except ValueError as e:
            print(f"{name:16} -> refused: {e}")
        else:
            die(f"a bundle relabelled {name} verified")
    # A 4mb image signed as 8mb: the descriptor gives it away.
    mixed = build_header("9.9.9-selftest", image, sk, "8mb") + image
    try:
        check(mixed, sk.verify_key)
    except ValueError as e:
        print(f"4mb image, 8mb   -> refused: {e}")
    else:
        die("a 4mb image in an 8mb bundle verified")
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
    s.add_argument("--variant", default="4mb", choices=sorted(MAGICS),
                   help="the flash layout the image is built for (its app descriptor must agree)")
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
