// The echo Worker under Miniflare (local workerd; no Cloudflare account,
// no network): `npm test` in connect/s0-echo.
import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
// Miniflare 5 takes its own config shape; convertV4MiniflareOptions takes
// the long-standing one.
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { fill, fnv, FNV_START, hex8, payloadState } from "../src/payload.js";

const root = fileURLToPath(new URL("..", import.meta.url));
const SECRET = "test-secret-not-real";

function worker(bindings) {
  return new Miniflare(
    convertV4MiniflareOptions({
      modulesRoot: root,
      modules: [
        { type: "ESModule", path: `${root}src/index.js` },
        { type: "ESModule", path: `${root}src/payload.js` },
      ],
      compatibilityDate: "2026-10-20",
      durableObjects: { ECHO: { className: "EchoS0", useSQLite: true } },
      bindings,
    }),
  );
}

let mf;
before(async () => {
  mf = worker({ S0_SECRET: SECRET });
  await mf.ready;
});
after(async () => {
  await mf?.dispose();
});

/** Open the socket; messages queue until taken with next(). */
async function open(k = SECRET) {
  const res = await mf.dispatchFetch(`http://connect-s0.test/ws?k=${encodeURIComponent(k)}`, {
    headers: { Upgrade: "websocket" },
  });
  assert.equal(res.status, 101);
  const ws = res.webSocket;
  const queue = [];
  const waiters = [];
  ws.addEventListener("message", (e) => {
    const w = waiters.shift();
    if (w) w(e.data);
    else queue.push(e.data);
  });
  ws.accept();
  const next = (ms = 5000) =>
    queue.length
      ? Promise.resolve(queue.shift())
      : new Promise((resolve, reject) => {
          const t = setTimeout(() => reject(new Error("no message")), ms);
          waiters.push((d) => {
            clearTimeout(t);
            resolve(d);
          });
        });
  return { ws, next, queued: () => queue.length };
}

test("the payload is opendmd-core's (connect::tests::payload_matches_the_worker)", () => {
  const st = payloadState(1);
  assert.deepEqual([...fill(st, new Uint8Array(8))], [0x21, 0x20, 0x04, 0x00, 0x01, 0x06, 0x08, 0x04]);
  // In uneven pieces: the same bytes as one fill.
  const a = fill(payloadState(1), new Uint8Array(524288));
  const st2 = payloadState(1);
  let h = FNV_START;
  let off = 0;
  for (const n of [1, 3, 1000, 7, 4096, 999]) {
    const p = fill(st2, new Uint8Array(n));
    assert.deepEqual(p, a.subarray(off, off + n));
    off += n;
  }
  h = fnv(FNV_START, a);
  assert.equal(hex8(h), "2c1355c2");
  assert.notDeepEqual([...fill(payloadState(0), new Uint8Array(4))], [0, 0, 0, 0]);
});

test("only /ws, only with the secret, only an upgrade", async () => {
  assert.equal((await mf.dispatchFetch("http://connect-s0.test/")).status, 404);
  assert.equal((await mf.dispatchFetch("http://connect-s0.test/ws")).status, 403);
  assert.equal((await mf.dispatchFetch("http://connect-s0.test/ws?k=wrong")).status, 403);
  assert.equal((await mf.dispatchFetch(`http://connect-s0.test/ws?k=${SECRET}x`)).status, 403);
  assert.equal((await mf.dispatchFetch(`http://connect-s0.test/ws?k=${SECRET}`)).status, 426);
});

test("no secret set: everything refused", async () => {
  const bare = worker({});
  await bare.ready;
  try {
    const res = await bare.dispatchFetch("http://connect-s0.test/ws?k=", { headers: { Upgrade: "websocket" } });
    assert.equal(res.status, 403);
  } finally {
    await bare.dispose();
  }
});

test("ping, echo, errors", async () => {
  const { ws, next } = await open();
  ws.send("ping");
  assert.equal(await next(), "pong");
  ws.send("echo 41");
  assert.equal(await next(), "echo 41");
  ws.send("stream 0 1000 4 1");
  assert.equal(await next(), "error bad stream");
  ws.send("stream 100 99999 4 1");
  assert.equal(await next(), "error bad stream");
  ws.send("what");
  assert.equal(await next(), "error unknown");
  ws.close(1000, "");
});

test("M10: 512 KB in credited chunks, never more than the window in flight", async () => {
  const { ws, next, queued } = await open();
  const total = 524288;
  const chunk = 1000;
  const window = 4;
  ws.send(`stream ${total} ${chunk} ${window} 1`);
  const want = fill(payloadState(1), new Uint8Array(total));
  let got = 0;
  let h = FNV_START;
  let unacked = 0;
  let end = null;
  while (end === null) {
    const m = await next();
    if (typeof m === "string") {
      end = m;
      break;
    }
    const b = new Uint8Array(m);
    assert.ok(b.length <= chunk);
    assert.deepEqual(b, want.subarray(got, got + b.length));
    got += b.length;
    h = fnv(h, b);
    unacked += 1;
    assert.ok(unacked <= window, `${unacked} frames in flight`);
    // Credit two at a time, as the probe does (window / 2).
    if (unacked >= 2 && queued() === 0) {
      ws.send(`credit ${unacked}`);
      unacked = 0;
    }
  }
  assert.equal(got, total);
  assert.equal(end, `end ${total} ${hex8(h)}`);
  assert.equal(hex8(h), "2c1355c2");
  ws.close(1000, "");
});

test("a stream's place survives between credits on a fresh message", async () => {
  const { ws, next } = await open();
  ws.send("stream 2500 1000 1 7");
  const want = fill(payloadState(7), new Uint8Array(2500));
  const a = new Uint8Array(await next());
  assert.deepEqual(a, want.subarray(0, 1000));
  ws.send("credit 1");
  const b = new Uint8Array(await next());
  assert.deepEqual(b, want.subarray(1000, 2000));
  ws.send("credit 5");
  const c = new Uint8Array(await next());
  assert.deepEqual(c, want.subarray(2000, 2500));
  assert.equal(await next(), `end 2500 ${hex8(fnv(FNV_START, want))}`);
  ws.close(1000, "");
});
