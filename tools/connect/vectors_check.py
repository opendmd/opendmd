# /// script
# requires-python = ">=3.11"
# dependencies = ["pynacl==1.5.0"]
# ///
"""Check the Python implementation of Connect v1 (odc.py) against the
shared vectors the Rust core pins (opendmd-core/tests/connect-vectors.json):
every derived value recomputed, every command case through a fresh panel
session (`expect`) and the server's pre-check (`server`).

    uv run tools/connect/vectors_check.py [VECTORS.json]

Prints one line per group and `VECTORS PASS` at the end; exits non-zero on
the first mismatch. Local only: no network, no device.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import nacl.signing

import odc

DEFAULT = Path(__file__).resolve().parents[2] / "opendmd-core" / "tests" / "connect-vectors.json"


def check(cond, what):
    if not cond:
        sys.exit(f"FAIL: {what}")


def main():
    v = json.loads(Path(sys.argv[1] if len(sys.argv) > 1 else DEFAULT).read_text())
    check(v["version"] == odc.VERSION and v["min_version"] == odc.MIN_VERSION, "version")
    lim = v["limits"]
    for name, mine in [("max_text", odc.MAX_TEXT), ("max_body", odc.MAX_BODY), ("command_head", odc.COMMAND_HEAD), ("max_chunk", odc.MAX_CHUNK), ("max_file", odc.MAX_FILE), ("max_keys", odc.MAX_KEYS), ("max_label_bytes", odc.MAX_LABEL), ("max_n", odc.MAX_SAFE), ("rate_per_key", odc.RATE_PER_KEY), ("rate_per_panel", odc.RATE_PER_PANEL)]:
        check(lim[name] == mine, f"limit {name}")
    d = v["domains"]
    check(d["cmd"].encode() == odc.CMD_DOMAIN and d["attach"].encode() == odc.ATTACH_DOMAIN and d["dev_tag"].encode() == odc.DEV_TAG_DOMAIN, "domains")
    check(tuple(v["allowed"]) == odc.ALLOWED, "allow-list")
    for c in odc.ALLOWED:
        for word in v["never"]:
            check(word not in c, f"{c} contains {word}")
    print("limits, domains, allow-list: ok")

    for b in v["codec"]["base32"]:
        raw = bytes.fromhex(b["hex"])
        check(odc.b32(raw) == b["text"] and odc.unb32(b["text"], len(raw)) == raw, f"base32 {b}")
    for s in v["codec"]["base32_refused"]:
        check(odc.unb32(s, 6) is None, f"base32 refused {s}")
    for h in v["codec"]["hmac_sha256"]:
        check(odc.mac256(bytes.fromhex(h["key"]), bytes.fromhex(h["msg"])).hex() == h["mac"], "hmac")
    print("codec: ok")

    p = v["panel"]
    id_key = bytes.fromhex(p["id_key"])
    dev = odc.mint_dev(bytes.fromhex(p["dev_random"]), id_key)
    check(dev.hex() == p["dev_hex"] and odc.dev_text(dev) == p["dev"] and p["dev_subprotocol"] == "d." + p["dev"], "dev")
    check(odc.dev_tag_ok(dev, id_key) and not odc.dev_tag_ok(odc.parse_dev(p["forged_dev"]), id_key), "dev tag")
    secret = bytes.fromhex(p["secret"])
    check(json.loads(p["register_body"]) == {"v": 1, "secret_hash": odc.sha256(secret).hex()}, "register body")
    sess = bytes.fromhex(p["sess"])
    print("panel id: ok")

    keys: dict[bytes, odc.Key] = {}
    ctl = {}
    for c in v["controllers"]:
        sk = nacl.signing.SigningKey(bytes.fromhex(c["seed"]))
        pub = bytes(sk.verify_key)
        check(pub.hex() == c["pub"] and odc.key_id(pub).hex() == c["k"], f"controller {c['name']}")
        ctl[c["name"]] = sk
        if c["on_list"]:
            keys[odc.key_id(pub)] = odc.Key(pub, c["flags"], c["label"])
    km = json.loads(p["keys_message"])
    check(km["t"] == "keys" and [(e["k"], e["pub"], e["f"]) for e in km["keys"]] == [(k.k.hex(), k.pub.hex(), k.flags) for k in keys.values()], "keys message")
    sc = v["signature_check"]
    sk = nacl.signing.SigningKey(bytes.fromhex(sc["seed"]))
    check(sk.sign(bytes.fromhex(sc["msg"])).signature.hex() == sc["sig"], "signature check")
    print("controllers and keys: ok")

    for n in v["negotiation"]:
        check(odc.negotiate(n["offered"]) == n["result"], f"negotiate {n}")
    print(f"negotiation: {len(v['negotiation'])} ok")

    at = v["attach"]
    nonce = bytes.fromhex(at["nonce"])
    a_pub = bytes(ctl["A"].verify_key)
    check(odc.attach_signature(ctl["A"], dev, nonce).hex() == at["by_a"], "attach signature")
    check(odc.verify_attach(a_pub, dev, nonce, bytes.fromhex(at["by_a"])), "attach verifies")
    check(not odc.verify_attach(a_pub, dev, nonce, bytes.fromhex(at["by_stranger"])), "stranger's attach under A")
    check(json.loads(at["message"]) == {"t": "attach", "k": odc.key_id(a_pub).hex(), "sig": at["by_a"]}, "attach message")
    print("attach: ok")

    pr = v["pairing"]
    s = odc.b32(bytes.fromhex(pr["random"]), 100)
    check(s == pr["secret"] and odc.pair_id(s).hex() == pr["pair_id"], "pairing secret / id")
    gpub = bytes.fromhex(pr["pub"])
    check(odc.pair_mac(s, dev, gpub).hex() == pr["mac"] and odc.paired_mac(s, gpub).hex() == pr["mac2"], "pairing MACs")
    w = odc.Window(s, until=300)
    try:
        w.check(dev, a_pub, bytes.fromhex(pr["mac"]), 0)
        check(False, "a swapped key paired")
    except odc.Bad as e:
        check(str(e) == "bad-mac", "swapped key: bad-mac")
    check(w.check(dev, gpub, bytes.fromhex(pr["mac"]), 0).hex() == pr["mac2"], "window pairs")
    iv = v["invite"]
    s2 = odc.b32(bytes.fromhex(iv["random"]), 128)
    check(s2 == iv["secret"] and odc.invite_id(s2).hex() == iv["invite_id"], "invite id")
    check(odc.add_mac(s2, dev, gpub).hex() == iv["mac"] and odc.confirm_code(gpub) == iv["confirm"], "invite MAC / code")
    print("pairing and invites: ok")

    server_keys = {k: e.pub for k, e in keys.items()}
    for c in v["commands"]:
        sess_obj = odc.Session(dev, sess)
        for b in c["before"]:
            sess_obj.check(keys, p["max_brightness"], bytes.fromhex(b))
        frame = bytes.fromhex(c["frame"])
        try:
            sess_obj.check(keys, p["max_brightness"], frame)
            got = "ok"
        except odc.Refusal as r:
            got = r.code
            check((r.ack() is None) == (r.code == "bad-frame"), f"{c['name']}: ack")
        check(got == c["expect"], f"panel {c['name']}: {got} != {c['expect']}")
        try:
            odc.precheck(server_keys, frame)
            srv = "forward"
        except odc.Refusal as r:
            srv = r.code
        check(srv == c["server"], f"server {c['name']}: {srv} != {c['server']}")
    print(f"commands: {len(v['commands'])} cases, panel and server agree with the vectors")
    print("VECTORS PASS")


if __name__ == "__main__":
    main()
