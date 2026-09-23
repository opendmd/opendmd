"""A numpy port of opendmd-render's LedRenderer (opendmd-render/src/lib.rs):
same kernel (0.42-pitch emitter, 1.10 aspect, 0.75-pitch bloom at 0.22),
same mask/package backdrop, same linear-light accumulation. Used to render
the comparison sheets; check_led.py compares it against the real wasm
Preview."""
import numpy as np

DOT_DIAMETER, DOT_ASPECT, BLOOM_RADIUS, BLOOM_STRENGTH = 0.42, 1.10, 0.75, 0.22
MASK_LEVEL, UNLIT_LEVEL, PACKAGE_SIZE, EDGE_SOFTNESS = 5 / 255, 10 / 255, 0.5, 1.0
ENCODE_STEPS = 4096
W, H = 128, 32


class Led:
    def __init__(self, scale=8, gamma=2.2, gain=1.0, bloom=BLOOM_STRENGTH):
        s = scale
        self.s, self.gamma, self.gain = s, gamma, gain
        k = 2 * s
        self.origin = s // 2 - s
        centre = s / 2 - self.origin
        rx = DOT_DIAMETER * s / 2
        ry = rx * DOT_ASPECT
        br = BLOOM_RADIUS * s
        j, i = np.mgrid[0:k, 0:k]
        dy, dx = j + 0.5 - centre, i + 0.5 - centre
        norm = np.sqrt((dx / rx) ** 2 + (dy / ry) ** 2)
        r = np.sqrt(dx * dx + dy * dy)
        with np.errstate(divide="ignore", invalid="ignore"):
            edge = np.where(norm > 1e-7, r / norm, 0)
        core = np.where(norm <= 1e-7, 1.0, np.clip(0.5 + (edge - r) / EDGE_SOFTNESS, 0, 1))
        t = r / br
        halo = np.where(r < br, bloom * (1 - t) ** 2, 0.0)
        self.kernel = (core + halo * (1 - core)).astype(np.float32)
        mask = MASK_LEVEL ** gamma
        extra = UNLIT_LEVEL ** gamma - mask
        bd = np.full((H * s, W * s, 3), mask, np.float32)
        pkg = max(1, round(PACKAGE_SIZE * s))
        pad = (s - min(pkg, s)) // 2
        for jj in range(min(pkg, s)):
            for ii in range(min(pkg, s)):
                bd[pad + jj :: s, pad + ii :: s] += extra
        self.backdrop = bd
        lut = np.arange(ENCODE_STEPS + 1) / ENCODE_STEPS
        self.encode = np.clip(lut ** (1 / gamma) * 255 + 0.5, 0, 255).astype(np.uint8)

    def render(self, rgb):
        """rgb: (32,128,3) uint8 -> (32*s, 128*s, 3) uint8."""
        s, k = self.s, 2 * self.s
        lin = (rgb.astype(np.float32) / 255) ** self.gamma * self.gain
        P = s  # padding so the kernel can spill a pitch past each edge
        acc = np.zeros((H * s + 2 * P, W * s + 2 * P, 3), np.float32)
        for j in range(k):
            for i in range(k):
                kv = self.kernel[j, i]
                if kv == 0:
                    continue
                y0, x0 = P + self.origin + j, P + self.origin + i
                acc[y0 : y0 + H * s : s, x0 : x0 + W * s : s] += lin * kv
        acc = acc[P : P + H * s, P : P + W * s] + self.backdrop
        idx = (np.clip(acc, 0, 1) * ENCODE_STEPS).astype(np.int32)
        return self.encode[idx]
