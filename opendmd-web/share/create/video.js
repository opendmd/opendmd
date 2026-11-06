// Video in (opendmd-notes docs/research/tenor.md §6 A): an MP4 or a WebM, from a file, a
// link or the relay, as frames for the Studio's timeline. Site-only and
// loaded only when a video arrives (create.js: the Studio's `o.video`).
//
// The video plays muted from a blob: address of our own (so the canvas is
// never tainted) and each frame the browser presents
// (requestVideoFrameCallback, Baseline 2024) is drawn at the working size
// and handed on as RGB with how long it shows: at most FPS a second, for
// the first MAX_S seconds. A 128x32 panel needs no more, and the timeline
// trims, deletes and changes the speed after. Without the callback (an
// older browser) the video is stepped by seeking instead.

export const FPS = 20; // at most 20 frames a second (50 ms each)
export const MAX_S = 10; // the first 10 seconds
const LONG = 480; // the working size's longer side at most (Fit then shrinks it to the panel)
const BUDGET = 40e6; // RGB bytes at most (the art module keeps 64 MB)

const ms = (s) => Math.max(20, Math.round(s * 100) * 10);
const once = (v, ev, why) => new Promise((res, rej) => {
  const t = setTimeout(() => rej(new Error(why)), 15000);
  v.addEventListener(ev, () => { clearTimeout(t); res(); }, { once: true });
  v.addEventListener('error', () => { clearTimeout(t); rej(new Error(why)); }, { once: true });
});

// `put(rgb, ms, {w, h, ow, oh})` for each frame (the first call starts the
// animation); `alive()` false stops it (null); `progress(t, of)` in seconds.
// {ow, oh, cut, secs}, or throws an Error saying why in plain words.
export async function frames(blob, { put, alive = () => true, progress }) {
  const v = document.createElement('video');
  Object.assign(v, { muted: true, playsInline: true, preload: 'auto' });
  const src = URL.createObjectURL(blob);
  const cant = 'This browser can’t play that video';
  try {
    v.src = src;
    await once(v, 'loadedmetadata', cant);
    let dur = v.duration;
    if (!Number.isFinite(dur)) { // (a WebM may not say until its end is seen)
      v.currentTime = 1e7;
      await once(v, 'seeked', cant);
      dur = v.duration;
      v.currentTime = 0;
      await once(v, 'seeked', cant);
    }
    const ow = v.videoWidth, oh = v.videoHeight;
    if (!ow || !oh) throw new Error('That video has no picture');
    const len = Math.min(dur > 0 && Number.isFinite(dur) ? dur : MAX_S, MAX_S);
    const k = Math.min(1, LONG / Math.max(ow, oh), Math.sqrt(BUDGET / (Math.ceil(len * FPS) * 3 * ow * oh)));
    const w = Math.max(1, Math.round(ow * k)), h = Math.max(1, Math.round(oh * k));
    const info = { w, h, ow, oh };
    const c = Object.assign(document.createElement('canvas'), { width: w, height: h });
    const g = c.getContext('2d', { willReadFrequently: true });
    g.imageSmoothingQuality = 'high';
    // A frame is handed on when the next one arrives (its delay is the gap).
    let held = null, at = 0, next = 0, seen = 0;
    const grab = (t) => {
      g.drawImage(v, 0, 0, w, h);
      const d = g.getImageData(0, 0, w, h).data, rgb = new Uint8Array(w * h * 3);
      for (let i = 0, j = 0; i < d.length; i += 4, j += 3) { rgb[j] = d[i]; rgb[j + 1] = d[i + 1]; rgb[j + 2] = d[i + 2]; }
      if (held) put(held, ms(t - at), info);
      held = rgb; at = t; seen = t;
      next += 1 / FPS;
      if (next <= t) next = t + 1 / FPS;
      progress?.(t, len);
    };
    if ('requestVideoFrameCallback' in v) {
      await new Promise((res, rej) => {
        let last = performance.now();
        const done = () => { clearInterval(watch); v.pause(); res(); };
        // Stopped (another picture), or no frame for 5 s (a stall, a hidden tab): what came is kept.
        const watch = setInterval(() => { if (!alive() || performance.now() - last > 5000) done(); }, 250);
        const on = (_now, m) => {
          last = performance.now();
          if (!alive() || m.mediaTime >= len) return done();
          if (m.mediaTime + 0.002 >= next) grab(m.mediaTime);
          v.requestVideoFrameCallback(on);
        };
        v.requestVideoFrameCallback(on);
        v.addEventListener('ended', done, { once: true });
        v.play().catch(() => { clearInterval(watch); rej(new Error(cant)); });
      });
    } else {
      for (let t = 0; t < len && alive(); t += 1 / FPS) {
        v.currentTime = t;
        await once(v, 'seeked', cant);
        grab(t);
      }
    }
    if (!alive()) return null;
    if (!held) throw new Error('No pictures could be read from that video');
    const end = Math.min(len, dur > 0 && Number.isFinite(dur) ? dur : seen + 1 / FPS);
    const stalled = seen + 1 < end;
    put(held, ms(stalled ? 1 / FPS : Math.max(1 / FPS, end - at)), info);
    return { ow, oh, cut: dur > len + 0.05 || stalled, secs: stalled ? seen : end };
  } finally {
    v.removeAttribute('src');
    v.load();
    URL.revokeObjectURL(src);
  }
}
