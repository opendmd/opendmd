# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow", "numpy"]
# ///
"""
ODM v1 prototype: pre-rendered delta-run-length frame format for OpenDMD.

Encodes every GIF under a source tree into a candidate binary format ("ODM v1"):
  - fully composite each frame (Pillow applies GIF disposal when frames are
    iterated in order via ImageSequence.Iterator + convert("RGB"))
  - build ONE global palette per file (<=256 colours; quantise via a stacked
    contact sheet if the file has more than 256 distinct colours)
  - encode each frame as a stream of ops relative to the previous frame
    (frame 0's "previous" is an all-black canvas):
      KEEP n        - next n pixels unchanged from previous frame
      FILL n, idx   - next n pixels all palette index idx
      LIT n, idx[n] - n literal palette indices
    op byte = tag<<6 | len_low6 (tag: KEEP=0, FILL=1, LIT=2); if the run is
    >=63, len_low6 is written as 63 and a little-endian u16 real length follows.
    Greedy: at each position, take the longest available KEEP/FILL run; if the
    best available run is <4, buffer literal pixels until reaching a position
    where a KEEP/FILL run of >=4 becomes available (or end of frame).

File layout: 32-byte header, palette (n_colors*3 bytes), then per frame:
    delay_ms:u16, data_len:u32, <data_len bytes of ops>

Also implements the decoder and verifies round-trip correctness, and measures
size/composition stats across the whole archive (see bottom of file / --help).
"""

from __future__ import annotations

import argparse
import struct
import sys
import time
import traceback
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

import numpy as np
from PIL import Image, ImageSequence

TAG_KEEP = 0
TAG_FILL = 1
TAG_LIT = 2

HEADER_STRUCT = "<4sBHHIH17s"  # magic, version, W, H, n_frames, n_colors, reserved
HEADER_SIZE = struct.calcsize(HEADER_STRUCT)
assert HEADER_SIZE == 32, HEADER_SIZE
MAGIC = b"ODM1"


# --------------------------------------------------------------------------
# Run-length helpers (vectorised)
# --------------------------------------------------------------------------

def compute_remaining_runs(arr: np.ndarray) -> np.ndarray:
    """remaining[i] = length of the run of equal consecutive values starting
    at i and extending to the right (i.e. how far a FILL/KEEP run beginning
    at i could reach)."""
    n = arr.shape[0]
    change = np.empty(n, dtype=bool)
    change[0] = True
    change[1:] = arr[1:] != arr[:-1]
    run_start_idx = np.flatnonzero(change)
    run_lengths = np.diff(np.append(run_start_idx, n))
    total_len_per_pos = np.repeat(run_lengths, run_lengths)
    start_per_pos = np.repeat(run_start_idx, run_lengths)
    pos = np.arange(n)
    remaining = total_len_per_pos - (pos - start_per_pos)
    return remaining.astype(np.int64)


def write_op_header(buf: bytearray, tag: int, length: int) -> None:
    if length < 63:
        buf.append((tag << 6) | length)
    else:
        buf.append((tag << 6) | 63)
        buf += struct.pack("<H", length)


def op_header_size(length: int) -> int:
    return 1 if length < 63 else 3


def encode_frame(cur_idx: np.ndarray, unchanged, build_bytes: bool):
    """cur_idx: (n,) uint8 palette indices for this frame, raster order.
    unchanged: (n,) bool array (pixel RGB == previous frame RGB) or None to
    disable KEEP entirely (used for the no-KEEP comparison scheme).
    Returns (ops_bytes_or_None, size, keep_px, fill_px, lit_px)."""
    n = cur_idx.shape[0]
    fill_remaining = compute_remaining_runs(cur_idx)
    if unchanged is not None:
        keep_remaining_raw = compute_remaining_runs(unchanged)
        keep_remaining = np.where(unchanged, keep_remaining_raw, 0)
    else:
        keep_remaining = np.zeros(n, dtype=np.int64)

    good = (fill_remaining >= 4) | (keep_remaining >= 4)
    good_positions = np.flatnonzero(good)
    gp_len = good_positions.shape[0]

    buf = bytearray() if build_bytes else None
    size = 0
    keep_px = fill_px = lit_px = 0
    p = 0
    while p < n:
        j = int(np.searchsorted(good_positions, p, side="left"))
        at_good = j < gp_len and good_positions[j] == p
        if at_good:
            klen = int(keep_remaining[p])
            flen = int(fill_remaining[p])
            if klen >= flen:
                run_len = klen
                if build_bytes:
                    write_op_header(buf, TAG_KEEP, run_len)
                size += op_header_size(run_len)
                keep_px += run_len
            else:
                run_len = flen
                val = int(cur_idx[p])
                if build_bytes:
                    write_op_header(buf, TAG_FILL, run_len)
                    buf.append(val)
                size += op_header_size(run_len) + 1
                fill_px += run_len
            p += run_len
        else:
            next_good = int(good_positions[j]) if j < gp_len else n
            lit_len = next_good - p
            if build_bytes:
                write_op_header(buf, TAG_LIT, lit_len)
                buf += cur_idx[p:next_good].tobytes()
            size += op_header_size(lit_len) + lit_len
            lit_px += lit_len
            p = next_good
    return (bytes(buf) if build_bytes else None), size, keep_px, fill_px, lit_px


def decode_odm(data: bytes):
    """Reference decoder. Returns list of (H,W,3) uint8 RGB frames.
    State kept: palette (n_colors*3 bytes) + integer read cursor(s). Nothing
    else - the canvas buffer is the only per-pixel state and is assumed to
    already exist on the target (it's the framebuffer)."""
    magic, version, w, h, n_frames, n_colors, _reserved = struct.unpack_from(
        HEADER_STRUCT, data, 0
    )
    if magic != MAGIC:
        raise ValueError("bad magic")
    off = HEADER_SIZE
    palette = np.frombuffer(data, dtype=np.uint8, count=n_colors * 3, offset=off).reshape(
        n_colors, 3
    )
    off += n_colors * 3
    n = w * h
    canvas = np.zeros((n, 3), dtype=np.uint8)
    frames_out = []
    for _ in range(n_frames):
        delay, data_len = struct.unpack_from("<HI", data, off)
        off += 6
        frame_end = off + data_len
        p = 0
        while p < n:
            b0 = data[off]
            off += 1
            tag = b0 >> 6
            length = b0 & 0x3F
            if length == 63:
                (length,) = struct.unpack_from("<H", data, off)
                off += 2
            if tag == TAG_KEEP:
                pass
            elif tag == TAG_FILL:
                val = data[off]
                off += 1
                canvas[p : p + length] = palette[val]
            elif tag == TAG_LIT:
                vals = np.frombuffer(data, dtype=np.uint8, count=length, offset=off)
                off += length
                canvas[p : p + length] = palette[vals]
            else:
                raise ValueError(f"bad tag {tag}")
            p += length
        if off != frame_end:
            raise ValueError(f"frame offset mismatch: {off} vs {frame_end}")
        frames_out.append(canvas.reshape(h, w, 3).copy())
    return frames_out


# --------------------------------------------------------------------------
# Per-file processing
# --------------------------------------------------------------------------

def process_file(path_str: str, gif_root_str: str, out_root_str: str, verify: bool):
    path = Path(path_str)
    gif_root = Path(gif_root_str)
    out_root = Path(out_root_str)
    try:
        im = Image.open(path)
        frames_rgb = []
        delays = []
        for frame in ImageSequence.Iterator(im):
            rgb = frame.convert("RGB")
            frames_rgb.append(np.asarray(rgb, dtype=np.uint8))
            delays.append(int(frame.info.get("duration", 100)) & 0xFFFF)
        if not frames_rgb:
            return {"path": str(path), "error": "no frames"}
    except Exception as e:  # noqa: BLE001
        return {"path": str(path), "error": f"{type(e).__name__}: {e}"}

    h, w = frames_rgb[0].shape[0], frames_rgb[0].shape[1]
    for f in frames_rgb:
        if f.shape[0] != h or f.shape[1] != w:
            return {"path": str(path), "error": "inconsistent frame size within file"}

    n_frames = len(frames_rgb)
    try:
        stack = np.stack(frames_rgb, axis=0)  # (n_frames,h,w,3)
        flat = stack.reshape(-1, 3)
        colors, inverse = np.unique(flat, axis=0, return_inverse=True)
        n_colors_raw = colors.shape[0]
        needed_quant = False
        if n_colors_raw <= 256:
            palette = colors.astype(np.uint8)
            indices = inverse.reshape(n_frames, h, w).astype(np.uint8)
            n_colors = n_colors_raw
        else:
            needed_quant = True
            contact = Image.fromarray(stack.reshape(n_frames * h, w, 3), "RGB")
            try:
                dither_none = Image.Dither.NONE
            except AttributeError:  # older Pillow
                dither_none = Image.NONE
            quantized = contact.quantize(colors=256, dither=dither_none)
            qarr = np.asarray(quantized, dtype=np.uint8)
            qpal_flat = quantized.getpalette()
            max_idx = int(qarr.max())
            n_colors = max_idx + 1
            palette = np.array(qpal_flat[: n_colors * 3], dtype=np.uint8).reshape(n_colors, 3)
            indices = qarr.reshape(n_frames, h, w)

        prev_rgb_flat = np.zeros((h * w, 3), dtype=np.uint8)
        frame_ops = []  # (delay, ops_bytes)
        ops_bytes_nokeep_total = 0
        keep_px_total = fill_px_total = lit_px_total = 0
        for i in range(n_frames):
            idx_flat = indices[i].reshape(-1)
            rgb_flat = palette[idx_flat]
            unchanged = np.all(rgb_flat == prev_rgb_flat, axis=1)
            ops_bytes, _size, kp, fp, lp = encode_frame(idx_flat, unchanged, build_bytes=True)
            _, nokeep_size, _, _, _ = encode_frame(idx_flat, None, build_bytes=False)
            frame_ops.append((delays[i], ops_bytes))
            ops_bytes_nokeep_total += nokeep_size
            keep_px_total += kp
            fill_px_total += fp
            lit_px_total += lp
            prev_rgb_flat = rgb_flat

        header = struct.pack(HEADER_STRUCT, MAGIC, 1, w, h, n_frames, n_colors, b"\x00" * 17)
        out = bytearray()
        out += header
        out += palette.tobytes()
        overhead_bytes = len(out) + n_frames * 6
        ops_bytes_keep_total = 0
        for delay, ops_bytes in frame_ops:
            out += struct.pack("<HI", delay, len(ops_bytes))
            out += ops_bytes
            ops_bytes_keep_total += len(ops_bytes)
        odm_bytes = bytes(out)

        rel = path.relative_to(gif_root)
        out_path = out_root / rel.with_suffix(".odm")
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_bytes(odm_bytes)

        verify_ok = None
        verify_error = None
        if verify:
            try:
                decoded = decode_odm(odm_bytes)
                verify_ok = True
                for i in range(n_frames):
                    truth = palette[indices[i].reshape(-1)].reshape(h, w, 3)
                    if not np.array_equal(decoded[i], truth):
                        verify_ok = False
                        verify_error = f"frame {i} mismatch"
                        break
            except Exception as e:  # noqa: BLE001
                verify_ok = False
                verify_error = f"{type(e).__name__}: {e}"

        return {
            "path": str(path),
            "rel": str(rel),
            "gif_size": path.stat().st_size,
            "odm_size": len(odm_bytes),
            "overhead_bytes": overhead_bytes,
            "ops_bytes_keep": ops_bytes_keep_total,
            "ops_bytes_nokeep": ops_bytes_nokeep_total,
            "n_frames": n_frames,
            "n_colors": n_colors,
            "needed_quant": needed_quant,
            "w": w,
            "h": h,
            "keep_px": keep_px_total,
            "fill_px": fill_px_total,
            "lit_px": lit_px_total,
            "total_px": n_frames * h * w,
            "verify_ok": verify_ok,
            "verify_error": verify_error,
        }
    except Exception as e:  # noqa: BLE001
        return {"path": str(path), "error": f"{type(e).__name__}: {e}\n{traceback.format_exc()}"}


# --------------------------------------------------------------------------
# Main / reporting
# --------------------------------------------------------------------------

def find_gifs(root: Path):
    return sorted(p for p in root.rglob("*") if p.is_file() and p.suffix.lower() == ".gif")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--gif-root", default="~/scratch/dmdos-card/gifs")
    ap.add_argument("--out-root", default="~/scratch/odm-proto")
    ap.add_argument("--workers", type=int, default=None)
    ap.add_argument("--limit", type=int, default=None)
    ap.add_argument("--no-verify", action="store_true")
    ap.add_argument("--verify-limit", type=int, default=None, help="verify only first N files")
    args = ap.parse_args()

    gif_root = Path(args.gif_root)
    out_root = Path(args.out_root)
    out_root.mkdir(parents=True, exist_ok=True)

    files = find_gifs(gif_root)
    if args.limit:
        files = files[: args.limit]
    print(f"found {len(files)} .gif files under {gif_root}", flush=True)

    t0 = time.time()
    results = []
    errors = []
    done = 0
    with ProcessPoolExecutor(max_workers=args.workers) as ex:
        futs = {}
        for i, p in enumerate(files):
            do_verify = not args.no_verify and (
                args.verify_limit is None or i < args.verify_limit
            )
            fut = ex.submit(process_file, str(p), str(gif_root), str(out_root), do_verify)
            futs[fut] = p
        for fut in as_completed(futs):
            r = fut.result()
            done += 1
            if "error" in r:
                errors.append(r)
            else:
                results.append(r)
            if done % 500 == 0:
                elapsed = time.time() - t0
                print(f"  {done}/{len(files)} done, {elapsed:.1f}s elapsed", flush=True)
    elapsed = time.time() - t0
    print(f"encode+verify pass done: {len(results)} ok, {len(errors)} errors, {elapsed:.1f}s", flush=True)

    print("\n=== ERRORS (skipped files) ===")
    for e in errors:
        print(f"  {e['path']}: {e['error'].splitlines()[0]}")

    if not results:
        print("no successful results, aborting report")
        return

    total_gif = sum(r["gif_size"] for r in results)
    total_odm = sum(r["odm_size"] for r in results)
    ratios = np.array([r["odm_size"] / r["gif_size"] for r in results])
    order = np.argsort(-ratios)
    worst = [results[i] for i in order[:10]]

    keep_px = sum(r["keep_px"] for r in results)
    fill_px = sum(r["fill_px"] for r in results)
    lit_px = sum(r["lit_px"] for r in results)
    total_px = sum(r["total_px"] for r in results)

    n_quant = sum(1 for r in results if r["needed_quant"])
    n_single_color = sum(1 for r in results if r["n_colors"] == 1)
    n_le4_colors = sum(1 for r in results if r["n_colors"] <= 4)

    total_ops_keep = sum(r["ops_bytes_keep"] for r in results)
    total_ops_nokeep = sum(r["ops_bytes_nokeep"] for r in results)

    verified = [r for r in results if r["verify_ok"] is not None]
    verify_pass = sum(1 for r in verified if r["verify_ok"])
    verify_fail = [r for r in verified if not r["verify_ok"]]

    strays = [r for r in results if (r["w"], r["h"]) != (128, 32)]

    max_palette_bytes = max(r["n_colors"] * 3 for r in results)
    mean_palette_bytes = sum(r["n_colors"] * 3 for r in results) / len(results)

    print("\n=== SIZE ===")
    print(f"total gif bytes: {total_gif} ({total_gif/1e6:.1f} MB)")
    print(f"total odm bytes: {total_odm} ({total_odm/1e6:.1f} MB)")
    print(f"overall ratio odm/gif: {total_odm/total_gif:.4f}")
    print(f"per-file ratio median: {np.median(ratios):.4f}  p95: {np.percentile(ratios,95):.4f}")
    print("worst 10 files (name, gif_size, odm_size, frames, ratio):")
    for r in worst:
        print(f"  {r['rel']}  gif={r['gif_size']}  odm={r['odm_size']}  frames={r['n_frames']}  ratio={r['odm_size']/r['gif_size']:.3f}")

    print("\n=== OP USAGE (archive-wide pixel share) ===")
    print(f"total pixels (sum over all frames): {total_px}")
    print(f"KEEP: {keep_px} ({100*keep_px/total_px:.2f}%)")
    print(f"FILL: {fill_px} ({100*fill_px/total_px:.2f}%)")
    print(f"LIT:  {lit_px} ({100*lit_px/total_px:.2f}%)")

    print("\n=== PALETTE ===")
    print(f"files needing quantisation (>256 colours): {n_quant} / {len(results)}")
    print(f"files with a single-colour palette: {n_single_color}")
    print(f"files with <=4 colours: {n_le4_colors}")
    print(f"palette bytes per file: max={max_palette_bytes} mean={mean_palette_bytes:.1f}")

    print("\n=== NO-KEEP COMPARISON (ops-stream bytes only, headers excluded, same on both sides) ===")
    print(f"ops bytes WITH keep:    {total_ops_keep} ({total_ops_keep/1e6:.1f} MB)")
    print(f"ops bytes WITHOUT keep: {total_ops_nokeep} ({total_ops_nokeep/1e6:.1f} MB)")
    if total_ops_nokeep:
        print(f"KEEP saves: {100*(1 - total_ops_keep/total_ops_nokeep):.1f}%")

    print("\n=== VERIFICATION ===")
    print(f"files verified: {len(verified)} / {len(results)}")
    print(f"pass: {verify_pass}  fail: {len(verify_fail)}")
    for r in verify_fail[:20]:
        print(f"  FAIL {r['rel']}: {r['verify_error']}")

    print("\n=== STRAYS (not 128x32) ===")
    for r in strays:
        print(f"  {r['rel']}: {r['w']}x{r['h']}")

    print("\n=== TIMING ===")
    print(f"wall clock for encode+verify of {len(files)} files with {args.workers or 'default'} workers: {elapsed:.1f}s")

    print("\n=== DECODER STATE (ESP32) ===")
    print("Python reference decoder keeps only: palette (n_colors*3 bytes) + integer read")
    print("offset(s) into the ops stream. No per-pixel/per-frame lookup tables, no extra")
    print(f"canvas copies. Max palette size seen: {max_palette_bytes} bytes; mean {mean_palette_bytes:.1f} bytes.")
    print("Plus a nominal 2KB read buffer for streaming ops bytes off flash/SD on-device.")


if __name__ == "__main__":
    main()
