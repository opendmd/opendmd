# Vendored `gif` 0.13.3 (patched)

This is the `gif` crate, version 0.13.3, exactly as published on crates.io
(MIT OR Apache-2.0 — see LICENSE-MIT / LICENSE-APACHE), with one change in
`src/reader/decoder.rs`, `LzwReader::reset`:

```rust
self.decoder = None;   // added
self.decoder = Some(LzwDecoder::new(BitOrder::Lsb, min_code_size));
```

Upstream builds the replacement LZW decoder while the previous one is still
alive whenever a frame's minimum code size changes. A `weezl` decoder owns
~29 KB of tables regardless of code size, so the swap transiently doubles
the decoder's footprint — on the ESP32 that took the free-heap floor from
~50 KB to ~20 KB on GIFs with per-frame palettes (423 of the 11,047 in the
DMDoS pack).

The same fix against upstream `master` lives on a fork:
https://github.com/duggan/image-gif/tree/lzw-reset-drop-before-realloc
(commit 7cb6e5c). It has not been submitted upstream yet. When it lands in a
release, delete this directory and the `[patch.crates.io]` entries in
`opendmd-core`, `opendmd-sim` and `firmware`.

Do not edit anything else here; keep it diffable against 0.13.3.
