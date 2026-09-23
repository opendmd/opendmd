# Art pipeline experiments (throwaway)

Prototypes behind `docs/art-pipeline.md`. Nothing here is part of the app, and
nothing here holds third-party game art. Inputs are downloaded into `$ART_TMP`
(default `/tmp/opendmd-art`) by a script. Outputs are PNGs (and GIFs, which the
repo's `*.gif` rule ignores). Every input is CC0, public domain or permissively
licensed (see `process/fetch_inputs.py`). No ROMs are stored here. The emulator
README links to the test ROMs.

| dir | what | run |
|---|---|---|
| `cors/` | Playwright probe: `fetch()`, `<img crossOrigin>`+canvas and plain `<img>`+canvas against ~30 art/metadata hosts, from a real http origin | `uv run opendmd-web/experiments/art/cors/cors_probe.py` (see `NOTES.txt`) |
| `emulator/` | EmulatorJS + a local ROM file, frames grabbed into PNGs, with timings and bytes | see `emulator/README.txt` |
| `process/` | the processing primitives (`art.py`), a numpy port of `opendmd-render` (`led.py`), checked against the real wasm `Preview` (`check_led.py`), and the five demos (`demos.py`) | below |
| `studio/` | a one-page browser Studio: Commons search / URL / file / drop / paste → native-pixel detection, auto window, pan/scroll, a palette that keeps accents → `gifenc` GIF → the real wasm `Preview` and `encodeGif` (prepared-copy check) | below |

```bash
export ART_TMP=/tmp/opendmd-art
make web                                                  # once: www/pkg for check_led.py and the studio
uv run opendmd-web/experiments/art/process/fetch_inputs.py
uv run opendmd-web/experiments/art/process/check_led.py   # led.py vs the wasm renderer: expect 0 diff
uv run opendmd-web/experiments/art/process/demos.py       # out/*.png sheets, out/*.gif
uv run opendmd-web/experiments/art/studio/drive.py        # headless Studio pass: studio/out/*
# or interactively:
cd opendmd-web && uv run python -m http.server 8090       # then http://localhost:8090/experiments/art/studio/
```

`demos.py`'s NeuQuant column shells out to `process/nq/`, a 20-line Rust
binary using the same `color_quant 1.1.0` as the core's ODM encoder. It is built
into `$ART_TMP/nq-target` on first use (`cargo build --offline`).

The studio page loads `gifenc` 1.0.3 (MIT, 3.9 KB gzipped) from jsDelivr, and
`/www/pkg` from the app build, so serve `opendmd-web/` as the web root.
