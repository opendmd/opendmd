// opendmd-connect-s0: the scratch echo server for OpenDMD Connect's spike S0
// (firmware/src/connect_probe.rs is its only client). Throwaway: deleted
// after S0 (CONNECT.md section 13, answer 2). No protocol, no pairing, no
// storage: one Durable Object, hibernating WebSockets, and four messages.
//
//   GET /ws?k=<S0_SECRET>   upgrade (anything else: 404; a wrong or missing
//                           k, or no S0_SECRET set: 403, before the object
//                           is woken, so nobody else burns its duration)
//
// On the socket (text frames from the panel):
//   ping                  -> pong      (the runtime's auto-response: the
//                                       object is not woken, nothing billed)
//   (WebSocket ping frames are answered by the runtime as well)
//   echo <anything>       -> echo <anything>   (wakes the object)
//   stream <bytes> <chunk> <window> <seed>
//                         -> <window> binary frames of <chunk> bytes of
//                            the seed's stream (src/payload.js), then
//   credit <n>            -> n more frames each; after the last byte,
//                         -> end <bytes> <fnv-1a hex>
//
// Nothing is logged: no console.log, and invocation logs are off
// (wrangler.toml).

import { DurableObject } from "cloudflare:workers";
import { fill, fnv, FNV_START, hex8, payloadState } from "./payload.js";

const MAX_BYTES = 16 * 1024 * 1024;
const MAX_CHUNK = 16384;
const MAX_WINDOW = 64;

/** Constant-time string compare (both short). */
function same(a, b) {
  if (typeof a !== "string" || typeof b !== "string" || a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return d === 0;
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname !== "/ws") return new Response("not found\n", { status: 404 });
    if (!env.S0_SECRET || !same(url.searchParams.get("k"), env.S0_SECRET)) {
      return new Response("forbidden\n", { status: 403 });
    }
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("expected a WebSocket upgrade\n", { status: 426 });
    }
    // One object for the spike: there is one panel.
    const stub = env.ECHO.get(env.ECHO.idFromName("s0"));
    return stub.fetch(request);
  },
};

export class EchoS0 extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    // Answered by the runtime while the object sleeps.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
  }

  async fetch(request) {
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    this.ctx.acceptWebSocket(server, ["panel"]);
    server.serializeAttachment(null);
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws, message) {
    if (typeof message !== "string") return; // the panel sends no binary
    if (message.startsWith("echo ")) {
      ws.send(message);
      return;
    }
    const w = message.split(" ");
    if (w[0] === "stream" && w.length === 5) {
      const [bytes, chunk, window, seed] = w.slice(1).map((v) => Number.parseInt(v, 10));
      if (!(bytes > 0 && bytes <= MAX_BYTES && chunk > 0 && chunk <= MAX_CHUNK && window > 0 && window <= MAX_WINDOW && seed >= 0)) {
        ws.send("error bad stream");
        return;
      }
      const s = { total: bytes, left: bytes, chunk, st: payloadState(seed), h: FNV_START };
      this.send(ws, s, window);
      return;
    }
    if (w[0] === "credit" && w.length === 2) {
      const n = Number.parseInt(w[1], 10);
      const s = ws.deserializeAttachment();
      if (s && n > 0 && n <= MAX_WINDOW) this.send(ws, s, n);
      return;
    }
    ws.send("error unknown");
  }

  /** Send up to `n` chunks of stream `s`, then keep its place. */
  send(ws, s, n) {
    for (let k = 0; k < n && s.left > 0; k++) {
      const out = fill(s.st, new Uint8Array(Math.min(s.chunk, s.left)));
      s.h = fnv(s.h, out);
      s.left -= out.length;
      ws.send(out);
    }
    if (s.left === 0) {
      ws.send(`end ${s.total} ${hex8(s.h)}`);
      ws.serializeAttachment(null);
    } else {
      ws.serializeAttachment(s);
    }
  }

  async webSocketClose(ws, code) {
    // Answer the panel's close (1005/1006 cannot be sent back).
    try {
      ws.close(code === 1005 || code === 1006 ? 1000 : code, "");
    } catch {
      // already closed
    }
  }

  async webSocketError() {}
}
