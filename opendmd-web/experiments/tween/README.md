# Tweening experiment

Can we synthesise in-between frames that make motion smoother on the
128x32 panel without smears, and for which content? Write-up, numbers and
recommendation: [`docs/tweening-experiment.md`](../../../docs/tweening-experiment.md).

Python via `uv` (inline script metadata: Pillow, numpy, scipy). Sheets are
rendered through `../art/process/led.py`, the numpy port of
`opendmd-render`'s `LedRenderer` (checked pixel-identical against the wasm
preview by `../art/process/check_led.py`).

| script | step |
|---|---|
| `common.py` | GIF loading (player delay rules: 0 → 100 ms, floor 20 ms), the pair classifier, the shift tweener and its gate |
| `measure.py` | 1. archive statistics, parallel: `uv run measure.py [root] [--jobs 15]`, ~25 min for 11,047 files; `--report` reprints |
| `tween.py` | 2. picks 20 candidates + 10 controls from `out/files.jsonl`, writes `out/gifs/<name>.tween.gif`, `out/tl/*.npz`, `out/tween_table.md` |
| `rife.py` | 3. RIFE v4.6 through `rife-ncnn-vulkan` on lavapipe (CPU): `RIFE=<dir with the binary> uv run rife.py [--scale 4 --window-only]` |
| `compare.py` | 3. RIFE vs the exact shift in-betweens, off-palette pixels, "neither A nor B" on non-rigid pairs → `out/compare.md` |
| `bench_rife.py` | 3. RIFE ms per inference (native and 4x) with start-up subtracted |
| `sheets.py` | 2-4. one PNG per file in `out/sheets/`: original, persistence (step 4), shift tween, RIFE raw/snapped, native and 4x |

Order: `measure.py` → `tween.py` → `rife.py` (native, then `--scale 4
--window-only`) → `compare.py` → `sheets.py`.

`rife-ncnn-vulkan` is not vendored: the 20221029 Ubuntu release zip from
github.com/nihui/rife-ncnn-vulkan, unzipped anywhere; `rife.py` sets
`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` (Mesa lavapipe, CPU).
The Intel iGPU needs the user in the `render` group.

Outputs are under `out/` (the GIFs are ignored by the repo's `*.gif` rule;
the `.npz` timelines are scratch). The source card is only read. Other
experiments write `<name>.cap12.gif` / `.cap10d.gif` beside the pack's GIFs;
`measure.py` skips those (`DERIVED`).
