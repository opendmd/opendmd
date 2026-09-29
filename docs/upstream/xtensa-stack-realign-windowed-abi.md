# Upstream: Xtensa LLVM realigns the stack without MOVSP (windowed ABI)

**Status (2026-09-29): found in OpenDMD, worked around, not yet reported
upstream.** This page is written so it can move to the project wiki as is,
and its last section is a draft issue ready to file on llvm/llvm-project
when we decide to.

## Summary

For a function with a stack local aligned to more than the stack alignment,
the Xtensa backend's prologue (`XtensaFrameLowering::emitPrologue`, windowed
ABI) allocates the frame with `entry` and then realigns `a1` with ordinary
arithmetic:

```text
entry   a1, 352
movi.n  a8, 63
movi.n  a9, 64
and     a8, a1, a8
sub     a8, a9, a8
add.n   a1, a1, a8        ; a1 moved without MOVSP
```

On the windowed ABI that is unsafe. The window-overflow handler saves a
frame's caller's `a0`-`a3` in the 16 bytes **below that frame's `a1`**, and
`retw`/the underflow handler reloads them from below the `a1` the function
has **when it returns**. If an interrupt (or any window overflow) happens
between `entry` and the `add`, the caller's registers are spilled below the
old `a1`; the function then moves `a1`, and on return they are reloaded from
below the new one: stale stack. The caller resumes with garbage in `a0`-`a3`
(return address, stack pointer, first arguments). The architecture's answer
is `MOVSP`, which exists precisely to move `a1` while keeping the base save
area consistent (it copies the save area and raises an exception if the
caller's window is live).

## How it showed up in OpenDMD

- **Trigger:** `std::sync::mpsc::channel()`. std's mpmc channel places a
  64-byte-aligned `CachePadded` on the stack, so every `channel::<T>`
  instantiation gets the prologue above.
- **Symptom:** rare `LoadProhibited` / `Double exception` panics on CPU0 with
  corrupted backtraces, e.g. in the `/api/play` handler right after it
  created a reply channel: `A0 0x0000000a`, `A2 0x0000000a`, and the
  backtrace's second frame reading `0x6c656e6e` ("nnel", part of the request
  body `@fx:tunnel`) where a saved stack pointer should be. The registers
  were a stale `String { cap: 10, ptr, len: 10 }`.
- **Why it is rare:** the window is a few instructions long; the function
  runs from flash, so a cache miss there stretches it; an interrupt must land
  in it while enough windows are live to force a spill.

## Evidence

- **Symbolised crash:** PC `0x400d8195` is `l32i.n a4, a7, 0` (the handler's
  `Arc<Ctx>` load), executed immediately after returning from
  `std::sync::mpmc::channel::<Result<(), PlayError>>`, with `a7` (the closure
  environment) 0.
- **Deterministic torture test** (the `diag-heap` build's
  `GET /api/status?diag_chan=100000&impl=std|ours`, 100,000 channels created
  on the httpd task): with std's channel, a panic in 2 of 3 runs, and
  comprehensive heap poisoning caught the spill of `a4`-`a7` into a freed
  block 32 B below a garbage stack pointer (the torture loop's frame had
  `a1` inside the heap). With a channel that needs no over-alignment: 0 of
  3.
- **Static scan:** of 8,755 windowed functions in the firmware ELF, exactly
  the 7 std channel instantiations had a realigning prologue.

## Toolchain

| component | version |
|---|---|
| Rust | `esp` toolchain, rustc 1.97.0-nightly (8ea53bcd7 2026-07-08) |
| LLVM (rustc) | 21.1.3 |
| esp-clang (bindgen) | esp-20.1.1_20250829 |
| GCC (IDF C code) | xtensa-esp-elf esp-15.2.0_20250920 |
| espup | 0.17.1 |
| target | xtensa-esp32-espidf, ESP-IDF v5.3, esp-idf-sys 0.36 |

The same prologue code is present in upstream LLVM main and in Espressif's
`esp-20.1.1_20250829`, `esp-21.1.3_20260408` and `esp-22.1.4_20260825` tags
(`MOVSP` appears in the file only on the >32 KB large-frame path). Upgrading
does not help.

## Related upstream reports

- [llvm/llvm-project#208946](https://github.com/llvm/llvm-project/issues/208946),
  open: "[Xtensa] Incoming stack arguments read at the wrong address when the
  frame is dynamically realigned (windowed ABI)". Same prologue block,
  different defect (stack-passed arguments addressed from the realigned SP;
  the pad computed from an argument register with dynamic allocas). Does not
  mention spills, interrupts or `retw`.
- [llvm/llvm-project#208947](https://github.com/llvm/llvm-project/pull/208947),
  open, unmerged: the fix for #208946. Keeps the entry SP in a slot for fixed
  objects; still moves `a1` without `MOVSP`, so it would **not** fix this.
- Nothing in espressif/llvm-project or the esp-rs repositories matched
  (searched: movsp, realign, window spill, CachePadded, mpsc, LoadProhibited).
  Not searched: the esp-rs Matrix room and the esp32.com forum.

## Workaround in OpenDMD

- `firmware/src/chan.rs`: an MPSC channel on `Mutex` + `Condvar` +
  `VecDeque` with std's error types and disconnect semantics; every call site
  uses it (`use crate::chan as mpsc`). No `std::sync::mpsc` on the device.
- `tools/check-stack-realign.py`, run by `make fw-build` and
  `make fw-release`: fails the build if any function's prologue moves `a1`
  other than by `entry`/`movsp`, so a new dependency, a
  `#[repr(align(32))]` local or a toolchain change cannot bring it back
  unnoticed.
- Bonus: std's channel code was 58 KB of image, and its unbounded queue kept
  a 31-slot block allocated.
- The same exposure exists in any esp-idf Rust firmware that uses
  `std::sync::mpsc` on the device (stripe-dashboard was patched the same
  way).

## Possible fixes (for the upstream discussion)

1. Realign with `MOVSP`: compute the aligned value into a scratch register
   after `entry`, then `movsp a1, aX`. This is what `MOVSP` is for; it
   costs a few cycles and the alloca-exception path if the caller's window
   is live.
2. Or avoid moving `a1` at all: allocate `size + align - stack_align` with
   `entry`, and address over-aligned objects from an aligned base register
   (a frame-pointer-like base) instead of from `a1`.

Either must be reconciled with #208947's handling of fixed objects.

## Draft upstream issue (not filed)

> **[Xtensa] Dynamic stack realignment after `entry` moves a1 without MOVSP,
> corrupting spilled caller registers (windowed ABI)**
>
> With the windowed ABI, a function that has a stack object aligned beyond
> the stack alignment gets `entry a1, N` followed by
> `and`/`sub`/`add a1, a1, pad` (`XtensaFrameLowering::emitPrologue`).
> Moving `a1` after `entry` without `MOVSP` breaks the window save area
> invariant: if a window overflow (e.g. an interrupt) occurs between `entry`
> and the `add`, the caller's `a0`-`a3` are saved below the old `a1`, but
> are restored on return from below the new `a1`, so the caller resumes with
> stale registers.
>
> Seen in practice with Rust's `std::sync::mpsc::channel()` (64-byte-aligned
> `CachePadded` local) on ESP32 (LLVM 21.1.3, esp Rust toolchain): rare
> `LoadProhibited`/`Double exception` panics in the caller with corrupted
> backtraces; a loop creating 100,000 channels reproduces it within a few
> runs, and heap poisoning shows the spill landing 32 B below a garbage SP.
> Present in llvm main and Espressif's esp-22.1.4 release.
>
> Related: #208946 / #208947 (same prologue block, stack-argument
> addressing); that PR does not address the spill issue. Suggested fix:
> perform the realignment with `movsp`, or keep `a1` fixed and address
> over-aligned objects from an aligned base register.
>
> (Attach: the prologue disassembly, a minimal `#[repr(align(64))]` Rust
> repro, and the register dump.)
