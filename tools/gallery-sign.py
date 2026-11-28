# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl==1.5.0"]
# ///
"""Sign and verify the project's gallery Formulas (the signed-gallery design;
opendmd_core::gallery_sig is the firmware's verifier).

    uv run --locked tools/gallery-sign.py keygen [--key K] [--pub P]
    uv run --locked tools/gallery-sign.py sign --formulas DIR [--key K] [--kid N] [--rev R] [--sigs FILE] [--pub P]
    uv run --locked tools/gallery-sign.py verify --formulas DIR [--sigs FILE] [--pub [ID=]P ...]
    uv run --locked tools/gallery-sign.py selftest     # the shared vectors, then sign/verify/tamper with throwaway keys

A remote controller may add a Formula to a panel only if the project signed
it (protocol 5.7). The signature is Ed25519, by the gallery key (never the
firmware's), over one fixed binary message:

    "opendmd-gallery-v1 formula\\0"  domain, 27 B
    kid   u8      the gallery key's id
    abi   u8      the compiled format (the .psb's fourth byte)
    rev   u32 LE  the signing revision (the firmware refuses one under its floor)
    stem  u8 length + bytes ([a-z0-9-], 1 to 40, a letter or digit first)
    src   u32 LE length + SHA-256 of the source as sent
    psb   u32 LE length + SHA-256 of the compiled file

DIR is what `opendmd-gallery check --trusted --out DIR` writes for the
project's Formulas (opendmd-web/formulas/*.psi): `<stem>.psi`, the source as
the gallery sends it (no gallery header), and `<stem>.psb`, its compiled
file (`make gallery-sign` and `make gallery-check` make it). `sign` writes
opendmd-web/formulas/signatures.json: for each stem a list of signatures,
one per gallery key (a rotation carries both); it replaces the entries of
the key it signs with, keeps other keys', and drops stems no longer in DIR.
`--rev` defaults to the firmware's floor (REV_FLOOR in
opendmd-core/src/gallery_sig.rs) or the highest this key signed at before,
whichever is higher; raise it to revoke (sign the rest again, raise the
floor in the next firmware).

The private key is the raw 32-byte Ed25519 seed, by default
~/.config/opendmd/gallery.key (mode 0600, never in a repo, never in CI);
the public half is firmware/keys/gallery.pub (key 0; gallery-<id>.pub for a
later id), committed, and compiled into the firmware's GALLERY_KEYS. `sign`
refuses a key file others can read, and a key whose public half is not the
committed one for its id (`--pub` names another, for tests). `keygen`
refuses to overwrite either file.

`verify` checks the signatures file against DIR and the committed keys in
Python (an independent check of `make gallery-check`'s, which uses the
firmware's own verifier). With no gallery key committed yet (no
firmware/keys/gallery*.pub and no signatures file) it says UNSIGNED and
exits 0: that state is never reported as signed, and a panel refuses every
add in it. Any mix of the two fails.
"""
import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
from pathlib import Path

from nacl.exceptions import BadSignatureError
from nacl.signing import SigningKey, VerifyKey

DOMAIN = b"opendmd-gallery-v1 formula\0"
MAX_STEM = 40
MAX_MESSAGE = len(DOMAIN) + 1 + 1 + 4 + 1 + MAX_STEM + 2 * (4 + 32)

REPO = Path(__file__).resolve().parent.parent
DEFAULT_KEY = Path.home() / ".config" / "opendmd" / "gallery.key"
KEYS_DIR = REPO / "firmware" / "keys"
DEFAULT_SIGS = REPO / "opendmd-web" / "formulas" / "signatures.json"
CORE_SRC = REPO / "opendmd-core" / "src" / "gallery_sig.rs"
VECTORS = REPO / "opendmd-core" / "tests" / "gallery-vectors.json"
# The group order L: S >= L is a non-canonical signature.
L = 2**252 + 27742317777372353535851937790883648493


def die(msg: str, code: int = 1) -> None:
    print(f"gallery-sign: {msg}", file=sys.stderr)
    sys.exit(code)


def stem_ok(s: str) -> bool:
    return (isinstance(s, str) and 1 <= len(s) <= MAX_STEM and s.isascii() and s[0].isalnum()
            and all(c in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in s))


def message(kid: int, abi: int, rev: int, stem: str, src_len: int, src_sha: bytes, psb_len: int, psb_sha: bytes) -> bytes:
    """The signed message (opendmd_core::gallery_sig::message); ValueError for a stem outside the rule."""
    if not stem_ok(stem):
        raise ValueError(f"stem {stem!r} is outside the rule ([a-z0-9-], 1 to {MAX_STEM}, a letter or digit first)")
    if not (0 <= kid <= 255 and 0 <= abi <= 255 and 0 <= rev < 2**32 and 0 <= src_len < 2**32 and 0 <= psb_len < 2**32):
        raise ValueError("a field is out of range")
    if len(src_sha) != 32 or len(psb_sha) != 32:
        raise ValueError("a hash is not 32 bytes")
    return (DOMAIN + bytes([kid, abi]) + rev.to_bytes(4, "little") + bytes([len(stem)]) + stem.encode()
            + src_len.to_bytes(4, "little") + src_sha + psb_len.to_bytes(4, "little") + psb_sha)


def verify_strict(pub: bytes, msg: bytes, sig: bytes) -> bool:
    """Ed25519 as the firmware checks it (verify_strict): libsodium refuses a
    small-order key or R and a non-canonical S as well; S is checked here too."""
    if len(sig) != 64 or len(pub) != 32 or int.from_bytes(sig[32:], "little") >= L:
        return False
    try:
        VerifyKey(pub).verify(msg, sig)
        return True
    except (BadSignatureError, ValueError, TypeError):
        return False
    except Exception:  # noqa: BLE001 (pynacl raises CryptoError on a bad key)
        return False


def rev_floor() -> int:
    m = re.search(r"pub const REV_FLOOR: u32 = (\d+);", CORE_SRC.read_text())
    if not m:
        die(f"cannot find REV_FLOOR in {CORE_SRC}")
    return int(m.group(1))


def load_key(path: Path) -> SigningKey:
    try:
        mode = path.stat().st_mode
    except OSError as e:
        die(f"cannot read the private key {path}: {e} (make one with `keygen`)")
    if mode & 0o077:
        die(f"{path} is readable by others (mode {mode & 0o777:o}): chmod 600 {path}")
    seed = path.read_bytes()
    if len(seed) != 32:
        die(f"{path} is {len(seed)} B, a key is a 32-byte Ed25519 seed")
    return SigningKey(seed)


def committed_pubs() -> dict[int, bytes]:
    """firmware/keys/gallery.pub (id 0) and gallery-<id>.pub."""
    out: dict[int, bytes] = {}
    for p in sorted(KEYS_DIR.glob("gallery*.pub")):
        rest = p.name[len("gallery"):-len(".pub")]
        if rest == "":
            kid = 0
        elif rest.startswith("-") and rest[1:].isdigit():
            kid = int(rest[1:])
        else:
            die(f"{p}: not gallery.pub or gallery-<id>.pub")
        raw = p.read_bytes()
        if len(raw) != 32:
            die(f"{p} is {len(raw)} B, a public key is 32 B")
        out[kid] = raw
    return out


def parse_pubs(specs: list[str] | None) -> dict[int, bytes]:
    if not specs:
        return committed_pubs()
    out: dict[int, bytes] = {}
    for spec in specs:
        sid, eq, path = spec.partition("=")
        kid, path = (int(sid), path) if eq and sid.isdigit() else (0, spec)
        raw = Path(path).read_bytes()
        if len(raw) != 32 or kid in out or not 0 <= kid <= 255:
            die(f"--pub {spec}: a 32-byte key, each id once, ids 0..255")
        out[kid] = raw
    return out


def published(d: Path) -> dict[str, tuple[bytes, bytes]]:
    """DIR's Formulas: stem -> (source as sent, compiled file)."""
    if not d.is_dir():
        die(f"{d}: no such directory (opendmd-gallery check --trusted --out {d} ...)")
    out = {}
    for p in sorted(d.glob("*.psi")):
        stem = p.name[:-4]
        psb = p.with_suffix(".psb")
        if not psb.is_file():
            die(f"{p}: no {psb.name} beside it")
        if not stem_ok(stem):
            die(f"{p}: stem outside the rule")
        out[stem] = (p.read_bytes(), psb.read_bytes())
    if not out:
        die(f"{d}: no Formulas")
    return out


def entry(sk: SigningKey, kid: int, rev: int, stem: str, src: bytes, psb: bytes) -> dict:
    if len(psb) < 4 or psb[:3] != b"PSB":
        die(f"{stem}.psb is not a compiled Formula")
    abi = psb[3]
    ss, ps = hashlib.sha256(src).digest(), hashlib.sha256(psb).digest()
    sig = sk.sign(message(kid, abi, rev, stem, len(src), ss, len(psb), ps)).signature
    return {"kid": kid, "abi": abi, "rev": rev, "src_len": len(src), "src_sha256": ss.hex(),
            "psb_len": len(psb), "psb_sha256": ps.hex(), "sig": sig.hex()}


def write_json(path: Path, v: dict) -> None:
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(v, indent=1, sort_keys=True) + "\n")
    tmp.replace(path)


def sign_dir(sk: SigningKey, kid: int, rev: int | None, formulas: dict, sigs_path: Path) -> dict:
    old = json.loads(sigs_path.read_text()) if sigs_path.exists() else {}
    table = old.get("formulas", {})
    if rev is None:
        before = [e.get("rev", 0) for lst in table.values() for e in lst if e.get("kid") == kid]
        rev = max([rev_floor(), *before])
    out = {}
    for stem, (src, psb) in formulas.items():
        keep = [e for e in table.get(stem, []) if e.get("kid") != kid]
        out[stem] = sorted(keep + [entry(sk, kid, rev, stem, src, psb)], key=lambda e: e["kid"])
    v = {"about": "OpenDMD's gallery signatures (tools/gallery-sign.py; checked by make gallery-check with the firmware's verifier). "
                  "Per Formula, one per gallery key: kid, abi, rev, the lengths and SHA-256s of the source as sent and its compiled file, "
                  "and the Ed25519 signature over the message in opendmd_core::gallery_sig.",
         "domain": DOMAIN.decode().rstrip("\0"), "formulas": out}
    write_json(sigs_path, v)
    return v


def check_dir(formulas: dict, sigs: dict | None, pubs: dict[int, bytes], floor: int) -> tuple[str, list[str]]:
    """('unsigned'|'signed', problems)."""
    if not pubs:
        if sigs is not None:
            return "mixed", ["signatures.json is there but no firmware/keys/gallery*.pub is committed"]
        return "unsigned", []
    if sigs is None:
        return "mixed", ["a gallery key is committed but there is no signatures.json: make gallery-sign"]
    bad = []
    table = sigs.get("formulas", {})
    for stem in table:
        if stem not in formulas:
            bad.append(f"{stem}: signed, but not a project Formula (stale)")
    for stem, (src, psb) in formulas.items():
        by = set()
        for e in table.get(stem, []):
            try:
                kid = e["kid"]
                ok_fields = (e["src_len"] == len(src) and e["src_sha256"] == hashlib.sha256(src).hexdigest()
                             and e["psb_len"] == len(psb) and e["psb_sha256"] == hashlib.sha256(psb).hexdigest())
                if not ok_fields:
                    bad.append(f"{stem}: key {kid}'s signature is for another source or compiled file (stale)")
                    continue
                if kid not in pubs:
                    bad.append(f"{stem}: signed by key {kid}, which is not committed")
                    continue
                if e["abi"] != psb[3]:
                    bad.append(f"{stem}: signed for compiled format {e['abi']}, the file is {psb[3]}")
                    continue
                if e["rev"] < floor:
                    bad.append(f"{stem}: signed at rev {e['rev']}, under the firmware's floor {floor}")
                    continue
                msg = message(kid, e["abi"], e["rev"], stem, e["src_len"], bytes.fromhex(e["src_sha256"]), e["psb_len"], bytes.fromhex(e["psb_sha256"]))
                if not verify_strict(pubs[kid], msg, bytes.fromhex(e["sig"])):
                    bad.append(f"{stem}: key {kid}'s signature does not verify")
                    continue
                by.add(kid)
            except (KeyError, TypeError, ValueError) as x:
                bad.append(f"{stem}: a malformed entry ({x})")
        for kid in pubs:
            if kid not in by:
                bad.append(f"{stem}: no current signature by gallery key {kid} (make gallery-sign)")
    return "signed", bad


def cmd_keygen(a) -> None:
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


def cmd_sign(a) -> None:
    sk = load_key(Path(a.key).expanduser())
    pubs = parse_pubs(a.pub)
    if a.kid not in pubs:
        die(f"no committed public key for gallery key {a.kid} (firmware/keys/gallery{'' if a.kid == 0 else f'-{a.kid}'}.pub): "
            "keygen first, or --pub for a test")
    if bytes(sk.verify_key) != pubs[a.kid]:
        die(f"{a.key} is not gallery key {a.kid}: its public half is not the committed one")
    formulas = published(Path(a.formulas))
    v = sign_dir(sk, a.kid, a.rev, formulas, Path(a.sigs))
    revs = {e["rev"] for lst in v["formulas"].values() for e in lst if e["kid"] == a.kid}
    print(f"signed {len(formulas)} Formulas with gallery key {a.kid} at rev {sorted(revs)}: {a.sigs}")


def cmd_verify(a) -> None:
    formulas = published(Path(a.formulas))
    sp = Path(a.sigs)
    sigs = json.loads(sp.read_text()) if sp.exists() else None
    state, bad = check_dir(formulas, sigs, parse_pubs(a.pub), rev_floor())
    if bad:
        die("\n  ".join(["signatures:"] + bad))
    if state == "unsigned":
        print(f"GALLERY UNSIGNED: no gallery key committed yet (no firmware/keys/gallery*.pub, no {sp.name}): "
              f"{len(formulas)} project Formulas, none signed; a panel refuses every add")
    else:
        n = sum(len(x) for x in sigs["formulas"].values())
        print(f"GALLERY SIGNED: {len(formulas)} project Formulas, {n} signatures, each current and verified")


def check_vectors(path: Path) -> int:
    """The shared vectors (tests/gallery-vectors.json): messages byte for
    byte, the blue cases re-signed to the same bytes, every case's verdict."""
    v = json.loads(path.read_text())
    if bytes.fromhex(v["domain"]) != DOMAIN or v["max_message"] != MAX_MESSAGE:
        die("vectors: the domain or the message bound differ")
    keys = {k["kid"]: bytes.fromhex(k["pub"]) for k in v["keys"]}
    seeds = {n: SigningKey(bytes.fromhex(s)) for n, s in v["seeds"].items()}
    signer = {int(k): seeds[n] for k, n in v["signers"].items()}
    for kid, sk in signer.items():
        if bytes(sk.verify_key) != keys[kid]:
            die(f"vectors: key {kid}'s seed does not make its public key")
    floor = v["floor"]
    n = 0
    for c in v["cases"]:
        args = (c["kid"], c["abi"], c["rev"], c["stem"], c["src_len"], bytes.fromhex(c["src_sha256"]), c["psb_len"], bytes.fromhex(c["psb_sha256"]))
        try:
            msg = message(*args)
        except ValueError:
            msg = None
        if (msg.hex() if msg else None) != c["message"]:
            die(f"vectors {c['name']}: the message differs")
        sig = bytes.fromhex(c["sig"])
        if msg is None:
            got = "bad-stem"
        elif c["kid"] not in keys:
            got = "unknown-key"
        elif c["abi"] != c["player_abi"]:
            got = "wrong-abi"
        elif c["rev"] < floor:
            got = "old-rev"
        else:
            got = "ok" if verify_strict(keys[c["kid"]], msg, sig) else "bad-signature"
        if got != c["expect"]:
            die(f"vectors {c['name']}: {got}, the core says {c['expect']}")
        if c["expect"] == "ok" and signer[c["kid"]].sign(msg).signature != sig:
            die(f"vectors {c['name']}: this tool's signature is not the core's")
        n += 1
    return n


def refuses(f) -> bool:
    """Whether `f` stops with a non-zero exit (this tool's `die`)."""
    try:
        f()
    except SystemExit as e:
        return e.code not in (0, None)
    return False


def cmd_selftest(_a) -> None:
    n = check_vectors(VECTORS)
    print(f"vectors: {n} cases agree with the core ({VECTORS.name})")
    with tempfile.TemporaryDirectory() as t:
        t = Path(t)
        key, pub = t / "k" / "gallery.key", t / "gallery.pub"
        cmd_keygen(argparse.Namespace(key=str(key), pub=str(pub)))
        if key.stat().st_mode & 0o777 != 0o600:
            die("keygen: the key is not 0600")
        if not refuses(lambda: cmd_keygen(argparse.Namespace(key=str(key), pub=str(t / "other.pub")))):
            die("keygen overwrote a key")
        print("keygen          -> 0600, refuses to overwrite")
        os.chmod(key, 0o644)
        if not refuses(lambda: load_key(key)):
            die("a key others can read was taken")
        os.chmod(key, 0o600)
        print("key mode 0644   -> refused")
        sk = load_key(key)
        d = t / "out"
        d.mkdir()
        for stem, body in (("plasma", b"gray(x)\n"), ("fire", b"rgb(x, y, t)\n")):
            (d / f"{stem}.psi").write_bytes(body)
            (d / f"{stem}.psb").write_bytes(b"PSB\x03" + hashlib.sha256(body).digest())
        sigs = t / "signatures.json"
        floor = rev_floor()
        sign_dir(sk, 0, None, published(d), sigs)
        pubs = {0: pub.read_bytes()}
        state, bad = check_dir(published(d), json.loads(sigs.read_text()), pubs, floor)
        if state != "signed" or bad:
            die(f"a fresh signature set did not verify: {bad}")
        print("sign, verify    -> ok")
        (d / "plasma.psi").write_bytes(b"gray(y)\n")
        if not check_dir(published(d), json.loads(sigs.read_text()), pubs, floor)[1]:
            die("a changed source verified")
        print("source changed  -> refused (stale)")
        (d / "plasma.psi").write_bytes(b"gray(x)\n")
        (d / "rain.psi").write_bytes(b"gray(t)\n")
        (d / "rain.psb").write_bytes(b"PSB\x03")
        if not check_dir(published(d), json.loads(sigs.read_text()), pubs, floor)[1]:
            die("a Formula with no signature verified")
        print("one unsigned    -> refused (missing)")
        (d / "rain.psi").unlink()
        (d / "rain.psb").unlink()
        other = SigningKey.generate()
        if not check_dir(published(d), json.loads(sigs.read_text()), {0: bytes(other.verify_key)}, floor)[1]:
            die("signatures verified under another key")
        print("another key     -> refused (a throwaway key's set never passes the committed key)")
        v = json.loads(sigs.read_text())
        v["formulas"]["fire"][0]["sig"] = v["formulas"]["fire"][0]["sig"][:-2] + "00"
        if not check_dir(published(d), v, pubs, floor)[1]:
            die("a tampered signature verified")
        print("tampered sig    -> refused")
        if check_dir(published(d), None, {}, floor) != ("unsigned", []):
            die("the no-key state was not reported as unsigned")
        if check_dir(published(d), json.loads(sigs.read_text()), {}, floor)[0] != "mixed":
            die("signatures with no committed key passed")
        if check_dir(published(d), None, pubs, floor)[0] != "mixed":
            die("a committed key with no signatures passed")
        print("no key          -> UNSIGNED only when neither key nor signatures are there")
        (t / "other.pub").write_bytes(bytes(other.verify_key))
        if not refuses(lambda: cmd_sign(argparse.Namespace(key=str(key), kid=0, rev=None, formulas=str(d), sigs=str(sigs), pub=[str(t / "other.pub")]))):
            die("signed with a key that is not the committed one")
        print("sign, wrong pub -> refused")
    print("selftest ok")


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    k = sub.add_parser("keygen", help="make the gallery key pair (refuses to overwrite)")
    k.add_argument("--key", default=str(DEFAULT_KEY))
    k.add_argument("--pub", default=str(KEYS_DIR / "gallery.pub"))
    s = sub.add_parser("sign", help="sign DIR's Formulas into the signatures file")
    s.add_argument("--formulas", required=True, help="opendmd-gallery check --trusted --out's directory")
    s.add_argument("--key", default=str(DEFAULT_KEY))
    s.add_argument("--kid", type=int, default=0)
    s.add_argument("--rev", type=int, default=None)
    s.add_argument("--sigs", default=str(DEFAULT_SIGS))
    s.add_argument("--pub", action="append", help="the public key the signing key must match, ID=P (default: the committed ones; tests only)")
    v = sub.add_parser("verify", help="check the signatures file against DIR and the committed keys")
    v.add_argument("--formulas", required=True)
    v.add_argument("--sigs", default=str(DEFAULT_SIGS))
    v.add_argument("--pub", action="append", help="a public key to check against, ID=P (default: the committed ones)")
    sub.add_parser("selftest", help="the shared vectors, then round trip and tamper checks with throwaway keys")
    a = p.parse_args()
    {"keygen": cmd_keygen, "sign": cmd_sign, "verify": cmd_verify, "selftest": cmd_selftest}[a.cmd](a)


if __name__ == "__main__":
    main()
