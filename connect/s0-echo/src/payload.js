// M10's test stream: the bytes opendmd-core's `connect::Payload` makes from
// the same seed (xorshift32, 0 taken as 0x9E3779B9, each step's word sent
// little-endian), generated as they are sent and never stored. FNV-1a
// (32 bits) over them is what `end` reports.
//
// The state is two numbers, so a hibernating object keeps a stream's place
// in its socket's attachment between credits.

export function payloadState(seed) {
  return { x: seed >>> 0 || 0x9e3779b9, k: 0 };
}

/** Fill `out` (a Uint8Array) from `st`, advancing it. */
export function fill(st, out) {
  let { x, k } = st;
  for (let i = 0; i < out.length; i++) {
    if (k === 0) {
      x ^= x << 13;
      x >>>= 0;
      x ^= x >>> 17;
      x ^= x << 5;
      x >>>= 0;
    }
    out[i] = (x >>> (8 * k)) & 0xff;
    k = (k + 1) & 3;
  }
  st.x = x;
  st.k = k;
  return out;
}

export const FNV_START = 0x811c9dc5;

export function fnv(h, bytes) {
  for (let i = 0; i < bytes.length; i++) {
    h = Math.imul(h ^ bytes[i], 0x01000193) >>> 0;
  }
  return h >>> 0;
}

export function hex8(n) {
  return (n >>> 0).toString(16).padStart(8, "0");
}
