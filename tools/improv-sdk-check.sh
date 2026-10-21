#!/usr/bin/env bash
# Run the real Improv Serial host library against opendmd-core's framing.
#
#   tools/improv-sdk-check.sh            (normally via `make improv-check`)
#
# Builds opendmd-core's `improv_device` example (a pretend OpenDMD on
# stdin/stdout, answering through `opendmd_core::improv` the way
# firmware/src/improv.rs does), fetches improv-wifi-serial-sdk 2.8.0 — the
# version ESP Web Tools 10.4.0 depends on — with `npm pack` into
# target/improv-sdk/, and drives the example with its `ImprovSerial` class
# under node: detect, device info, scan, a wrong and a right password,
# current state, an RPC we do not implement. Fails on any difference from
# the expected answers. Needs node (18+) and npm, and the network once.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
sdk_version=2.8.0
work="$root/target/improv-sdk"
mkdir -p "$work"

(cd "$root/opendmd-core" && RUSTUP_TOOLCHAIN=stable cargo build -q --example improv_device)
device="$root/opendmd-core/target/debug/examples/improv_device"

if [ ! -f "$work/sdk/serial.js" ]; then
  (cd "$work" && npm pack -q "improv-wifi-serial-sdk@$sdk_version" >/dev/null)
  rm -rf "$work/pkg" && mkdir -p "$work/pkg" "$work/sdk/util"
  tar xzf "$work/improv-wifi-serial-sdk-$sdk_version.tgz" -C "$work/pkg"
  # The package's ES modules import without extensions (it is meant for a
  # bundler); node wants them.
  for f in "$work"/pkg/package/dist/*.js "$work"/pkg/package/dist/util/*.js; do
    rel=${f#"$work/pkg/package/dist/"}
    sed -E 's#from "(\.\.?/[A-Za-z0-9/_-]+)"#from "\1.js"#' "$f" > "$work/sdk/$rel"
  done
  echo '{"type":"module"}' > "$work/sdk/package.json"
fi

cat > "$work/run.mjs" <<'EOF'
import { spawn } from "node:child_process";
import { ImprovSerial } from "./sdk/serial.js";
const dev = spawn(process.argv[2], [], { stdio: ["pipe", "pipe", "inherit"] });
const port = {
  readable: new ReadableStream({
    start(c) {
      dev.stdout.on("data", (d) => c.enqueue(new Uint8Array(d)));
      dev.stdout.on("end", () => c.close());
    },
  }),
  writable: new WritableStream({ write(chunk) { dev.stdin.write(Buffer.from(chunk)); } }),
};
const logger = { log() {}, debug() {}, error: (...a) => console.log("sdk error:", ...a) };
const c = new ImprovSerial(port, logger);
const info = await c.initialize(1500);
console.log(`info ${info.firmware} ${info.version} ${info.chipFamily} ${info.name}; state ${c.state}`);
for (const s of await c.scan(5000)) console.log(`network ${s.name} ${s.rssi} ${s.secured}`);
try { await c.provision("Home", "wrong", 5000); console.log("wrong password: accepted"); }
catch (e) { console.log(`wrong password: ${e}; state ${c.state}`); }
await c.provision("Home", "right", 5000);
console.log(`right password: state ${c.state}, url ${c.nextUrl}`);
await c.requestCurrentState(3000);
console.log(`current state ${c.state}, url ${c.nextUrl}`);
try { await c.getHostname(3000); console.log("hostname: answered"); }
catch (e) { console.log(`hostname: ${e}`); }
dev.stdin.end();
process.exit(0);
EOF

expected='info OpenDMD 0.9.0 ESP32 opendmd; state 2
network a -1 false
network Café Ñ -71 false
network Home -48 true
wrong password: UNABLE_TO_CONNECT; state 2
right password: state 4, url http://192.168.1.69/
current state 4, url http://192.168.1.69/
hostname: UNKNOWN_RPC_COMMAND'

got=$(cd "$work" && timeout 60 node run.mjs "$device" 2>/dev/null)
if [ "$got" != "$expected" ]; then
  echo "improv-check: the SDK's view differs from what was expected" >&2
  diff <(echo "$expected") <(echo "$got") >&2 || true
  exit 1
fi
echo "$got"
echo "improv-check: improv-wifi-serial-sdk $sdk_version reads opendmd-core's frames correctly"
