# Kenney "Pixel Platformer" 1.2 (CC0), the files the art demos use

Vendored from https://kenney.nl/assets/pixel-platformer (Created/distributed by
Kenney, www.kenney.nl, 2023-10-07). Licence: Creative Commons Zero (CC0 1.0),
see `License.txt` (Kenney's own, unmodified). CC0 allows copying and
redistribution with no conditions; crediting Kenney is courteous, not required.

Only the five files `demos.py` reads are here (about 80 KB), byte for byte as
in the pack's zip:

| file | used for |
|---|---|
| `Tilemap/tilemap_packed.png` | `b_pan.gif` |
| `Tilemap/tilemap-characters.png`, `Tilemap/tilemap-characters_packed.png` | `b_walk.gif` |
| `SampleA.png`, `SampleB.png` | `d_scroll.gif` |

kenney.nl moved the pack's zip behind a download page (2026-10-29), so there
is no stable direct URL to fetch it from; `fetch_inputs.py` copies these
instead. Nothing here is from the DMDoS pack.
