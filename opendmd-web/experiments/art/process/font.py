"""Minimal BDF bitmap-font reader and text renderer."""
import numpy as np


class Bdf:
    def __init__(self, path):
        self.glyphs, self.ascent = {}, 0
        lines = open(path, encoding="latin-1").read().splitlines()
        i = 0
        while i < len(lines):
            ln = lines[i]
            if ln.startswith("FONT_ASCENT"):
                self.ascent = int(ln.split()[1])
            elif ln.startswith("FONT_DESCENT"):
                self.descent = int(ln.split()[1])
            elif ln.startswith("STARTCHAR"):
                enc = dw = bbx = None
                rows = []
                while not lines[i].startswith("ENDCHAR"):
                    t = lines[i].split()
                    if t[0] == "ENCODING":
                        enc = int(t[1])
                    elif t[0] == "DWIDTH":
                        dw = int(t[1])
                    elif t[0] == "BBX":
                        bbx = tuple(map(int, t[1:5]))
                    elif t[0] == "BITMAP":
                        i += 1
                        while not lines[i].startswith("ENDCHAR"):
                            rows.append(int(lines[i], 16))
                            i += 1
                        break
                    i += 1
                w, h, xo, yo = bbx
                nbits = ((w + 7) // 8) * 8
                bm = np.array([[(r >> (nbits - 1 - b)) & 1 for b in range(w)] for r in rows], bool).reshape(h, w)
                self.glyphs[enc] = (dw, bm, xo, yo)
            i += 1
        self.height = self.ascent + self.descent

    def width(self, text, spacing=0):
        return sum(self.glyphs.get(ord(c), self.glyphs[ord("?")])[0] + spacing for c in text) - spacing

    def mask(self, text, spacing=0):
        m = np.zeros((self.height, max(1, self.width(text, spacing))), bool)
        x = 0
        for c in text:
            dw, bm, xo, yo = self.glyphs.get(ord(c), self.glyphs[ord("?")])
            top = self.ascent - yo - bm.shape[0]
            for yy in range(bm.shape[0]):
                for xx in range(bm.shape[1]):
                    if bm[yy, xx] and 0 <= top + yy < m.shape[0] and 0 <= x + xo + xx < m.shape[1]:
                        m[top + yy, x + xo + xx] = True
            x += dw + spacing
        return m
