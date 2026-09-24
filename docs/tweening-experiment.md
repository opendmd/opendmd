# Tweening experiment: smoother motion on a 128x32 panel

The question: many pack GIFs run at 10–25 fps. Can we add in-between frames
that make motion smoother on the panel (100 Hz refresh, frames down to 20 ms)
**without smears**, and for which content? I tried three things and compared
them through the LED look:

1. measure the archive,
2. a deterministic shift tweener,
3. RIFE (a learned interpolation model), with a phosphor-persistence model
   as the cheap baseline.

Scripts, sheets and the 30 GIFs are in
[`opendmd-web/experiments/tween/`](../opendmd-web/experiments/tween/README.md).
Every sheet goes through `art/process/led.py`, the numpy port of
`opendmd-render` that matches the wasm preview pixel for pixel.

## Summary

- **Most motion in the pack is already as smooth as integer pixels
  allow.** 76% of translation pairs move **1 px**. There is nothing to put
  between two frames 1 px apart without blending, and blending is a smear.
  The `*_SCROLL_*` files, the biggest translation users, are almost all
  1 px at 30–50 ms.
- **The panel's timing blocks much of the rest.** A pair can only be split
  if each part stays ≥ 20 ms, so anything under 40 ms can't be split at
  all. Frames of ≤ 29 ms are 24% of the pack, and the 30–49 ms bucket
  (44%) is mostly 30 ms, the pack's commonest delay.
- What's left is **2.7% of all frame pairs** (48,635 of 1.79 M) and **3.4%
  of play time**. It sits in 3,260 of 11,040 files, and 1,003 files have
  tweenable motion in at least 10% of their pairs. It is concentrated in
  2–8 px/frame scrolls at 40–200 ms: marquees, credits, "slide" pictures,
  sprites crossing the screen, and some game footage.
- **The shift tweener works on that content.** Every frame it makes is
  pixels of A or B moved by whole pixels, so there are no new colours and no
  blends. On pure scrolls it is exact. It needs two gates to stay clean: a
  strict fit (≥ 97% of changed pixels explained) and a consistency check.
  Without them it tears sprites that change pose while they move. With them
  it touched **0 cut/other pairs** in the 10 controls.
- **RIFE is the wrong tool for this pack.**
  - At 4x-nearest input it reproduces the shift tweener on rigid motion
    (median 0.3% of changed pixels differ after palette snapping), but at
    670 ms per frame on this CPU, not microseconds.
  - At native 128x32 it softens edges (11% of changed pixels differ).
  - With no cut detector, it cross-dissolves or morphs every cut and
    animation change: a third to two-fifths of the changed pixels become
    neither frame.
  - It never beat the shift method on anything I could find in the
    candidates.
- **Persistence (`max(new, 0.6·prev)` per 10 ms refresh) is not an
  alternative.** It adds no positions, only trails. On fast text and scrolls
  it is the worst of the four (see the Chase HQ sheet), and on flashes and
  cuts it leaves a fading ghost.
- **Recommendation:** offer "Smooth motion" as an **opt-in** Studio/prepare
  option that runs the shift tweener with the gates below. Keep it off by
  default. Recommend it only when the analysis finds tweenable motion in at
  least 10% of the pairs, and hide it otherwise. No model.

## 1. The archive

`measure.py` looked at all 11,047 GIFs, excluding the `.cap12`/`.cap10d`
variants another experiment is writing beside them. It took 25 min on 15
processes.

- **Decoded:** 11,040. Seven files fail in Pillow ("broken data stream" or
  "truncated"): `Arcade/ARCADE_Bagman_01_RattenJager`,
  `Computers/C64_Pharaohs_Return_09`, `Consoles/MD_Kid_Chameleon_02_raph`,
  `NEW 2021/13`, `Other/OTHER_SCROLL_SG1000_Console`,
  `Pinball_Short/PINBALL_SHORT_ACDC_Serum27` and
  `Pinball_Short/PINBALL_SHORT_Big_Bang_Bar_Serum09`. The brief says 8; the
  other one decodes in Pillow.
- **Frames and pairs:** 1,792,058 frames and 1,792,055 consecutive pairs,
  counting each looping file's last → first pair.
- **Play time:** 26.2 hours.
- **Size:** two files are not 128x32. They are centre-cropped, as at
  playback.

Delays are after the player's rules (0 → 100 ms, floor 20 ms).

### Frame delays

| delay | frames | share |
|---|---:|---:|
| ≥ 80 ms | 276,957 | 15.5% |
| 50–79 ms | 296,252 | 16.5% |
| 30–49 ms | 784,554 | 43.8% |
| ≤ 29 ms | 434,295 | 24.2% |

### Motion per pair

Classes are applied in order, on colour-exact frames (equality on the
indexed image).

- **static:** ≤ 0.5% of pixels change.
- **translation:** a global integer shift within ±8 px explains ≥ 90% of the
  changed pixels. "Explains" means the pixel arrived (B[p] = A[p−s]) or left
  (A[p] = B[p+s]). The shift must also hold on ≥ 90% of the *whole* frame,
  so a sprite over a static textured background is not a global
  translation.
- **regional:** after bridging 2 px gaps, one connected changed region holds
  ≥ 90% of the changes, and a shift explains ≥ 80% of it.
- **cut:** more than 60% of pixels change and there is no fit.
- **other:** everything else.

| class | pairs | share of pairs | share of changed pixels | share of play time |
|---|---:|---:|---:|---:|
| static | 249,524 | 13.9% | 0.1% | 11.6% |
| translation | 449,756 | 25.1% | 30.0% | 19.2% |
| regional | 219,048 | 12.2% | 11.2% | 11.6% |
| cut | 59,441 | 3.3% | 15.2% | 6.1% |
| other | 814,286 | 45.4% | 43.5% | 51.5% |

"Other" is animation: sprites changing pose, palette flashes, fades, text
drawn on, particles. It is the largest class, and no shift-based method
touches it.

### Shift size by delay (translation + regional pairs)

| shift | ≤ 29 ms | 30–49 ms | 50–79 ms | ≥ 80 ms |
|---|---:|---:|---:|---:|
| translation, 1 px | 54,992 | 226,700 | 52,221 | 7,298 |
| translation, ≥ 2 px | 23,456 | 45,842 | 24,304 | 14,943 |
| regional, 1 px | 25,352 | 33,980 | 11,590 | 8,042 |
| regional, ≥ 2 px | 32,009 | 60,771 | 22,553 | 24,751 |

- **Can have an integer in-between:** 248,629 pairs move ≥ 2 px.
- **Timing allows it:** 106,748 of those can be split into parts of at
  least 20 ms and aren't a hold. A hold is a delay above
  max(2 × the file's median, 200 ms): a pause, not slow motion.
- **Passes the tweener's gates:** 48,635 pairs (2.71% of all pairs). They
  gain 74,151 new frames (+4.1% frames overall) and cover 3.4% of play
  time.

The raw "regional ≥ 5 px" count is inflated: many of those are
coincidental fits on animation, and the gates reject them.

### Top 20 by "translation frames × delay" (the literal ranking)

This is the sum of the delays of translation pairs. The last column is how
many of those pairs the tweener can actually use.

| file | frames | translation pairs | Σ delay (ms) | tweenable |
|---|---:|---:|---:|---:|
| `Consoles/MD_Demons_Of_Asteborg_01_sephirot68` | 4,489 | 2,414 | 89,610 | 4 |
| `Arcade/ARCADE_NEOGEO_KOF_96_02_sephirot68` | 1,841 | 1,771 | 88,820 | 0 |
| `Arcade/ARCADE_NEOGEO_KOF_96_03_sephirot68` | 969 | 965 | 48,250 | 0 |
| `Arcade/ARCADE_NEOGEO_SCROLL_Super Dodge Ball01` | 1,214 | 1,146 | 45,030 | 5 |
| `Consoles/SATURN_Dragon_Ball_Z_legends_04_sephirot68` | 1,419 | 1,412 | 42,360 | 0 |
| `Arcade/ARCADE_NEOGEO_SCROLL_Super Dodge Ball02` | 1,001 | 933 | 40,350 | 5 |
| `Test_Suite/32p_Test_Suite_By_RpiTeaM_V1` | 1,854 | 1,785 | 37,940 | 6 |
| `Test_Suite/32p_Test_Suite_V1_RpiTeam` | 1,854 | 1,785 | 37,940 | 6 |
| `Arcade/ARCADE_NEOGEO_SCROLL_Real_Bout_Fatal_Fury01` | 850 | 832 | 35,460 | 0 |
| `Arcade/ARCADE_MoonPatrol01_Wolfsoft` | 3,453 | 1,132 | 33,960 | 0 |
| `Arcade/ARCADE_MoonPatrol_01_Wolfsoft` | 3,453 | 1,132 | 33,960 | 0 |
| `Arcade/ARCADE_SCROLL_Mystic_Warriors_01` | 1,017 | 992 | 32,300 | 0 |
| `Arcade/ARCADE_SCROLL_Mystic_Warriors_02` | 1,013 | 989 | 32,180 | 0 |
| `XMAS/XMAS_ARCADE_BoogieWings02_RattenJager` | 1,770 | 1,146 | 32,020 | 1 |
| `Pinball_Story/PINBALL_STORY_LethalWeapon` | 1,407 | 561 | 31,780 | 126 |
| `Arcade/ARCADE_NEOGEO_SCROLL_Real_Bout_Fatal_Fury02` | 832 | 825 | 31,340 | 0 |
| `Consoles/PS1_SCROLL_Metal_Slug_X_23` | 637 | 625 | 31,250 | 0 |
| `Other/OTHER_SCROLL_Marvels05` | 802 | 774 | 30,960 | 15 |
| `The_80s_90s/The_80s_90s_SCROLL_Marvels_05` | 802 | 774 | 30,960 | 15 |
| `Consoles/PS1_SCROLL_Metal_Slug_X_22` | 631 | 618 | 30,900 | 0 |

This is the most important table. The files with the most translation are
long 1 px scrolls at 30–50 ms, and they have **nothing to gain**. So the
candidates below are the top 20 by *tweenable* motion time instead: the
delay summed over pairs that pass the gates.

- **Duplicates dropped:** byte-identical copies (the pack has `Other/` and
  `The_80s_90s/` copies of the same scrolls).
- **Series capped:** at most two per numbered series (Xmen01–05,
  Marvels01–05).

## 2. The shift tweener

Implemented in `common.py`, applied by `tween.py`.

### Rules

- **When a pair gets in-betweens:**
  - It is translation or regional, and has fit ≥ 0.97.
  - It passes the consistency gate: inconsistency ≤ 0.25. At every integer
    step, A moved forward and B moved back must agree wherever the pair
    changes or where the moved object lands. Disagreements are counted as a
    share of the pair's changed pixels.
  - Its delay is not a hold.
- **How many:** k = |shift| − 1 in-betweens (one per extra pixel), reduced
  until the delay splits into k+1 parts of ≥ 20 ms on the GIF's 10 ms grid.
  If no split is possible, the pair is left alone. Total duration is
  unchanged in all 30 files (asserted).
- **Global translation in-betweens:** frame A moved by round(t·shift). The
  strip entering the frame comes from B moved back. Where A and B disagree
  (a HUD, a flicker), a pixel that is the same in both keeps its value;
  otherwise the nearer end in time shows.
- **Regional in-betweens (a sprite over a background):** built in layers.
  - The object is the region's pixels that move with the shift.
  - The background comes from A where the object isn't in A, else from B.
  - The object is pasted at the integer step.
  - No pixel is ever blended.
- **Cuts, "other" and static pairs:** left untouched.

The consistency gate came from looking at the first sheets:

- **Makaimura's** man-to-monster morph during a scroll fit a shift at
  80–90%. The in-betweens mixed the two poses.
- **Bomb Jack's** text wipe, and M.U.L.E.'s jumping figure changing pose
  mid-jump, did the same.
- **Rigid scrolls** score exactly 0. Those failures scored 0.5–6.

### Results

Delays are shown as ms × count, the three commonest.

| role | file | frames | after | delays | after | synthesised | pairs tweened (translation/regional) | shift sizes |
|---|---|---:|---:|---|---|---:|---|---|
| candidate | `Consoles/PCE_CD_Jantei_Monogatari_III_Saver_Angels` | 223 | 512 | 100x223 | 50x314, 30x132, 40x66 | 56% | 222/1 | 2px x157, 3px x66 |
| candidate | `Other/OTHER_SCROLL_Xmen01` | 417 | 819 | 50x405, 200x12 | 30x402, 20x402, 200x12 | 49% | 402/0 | 2px x402 |
| candidate | `Other/OTHER_SCROLL_Xmen03` | 411 | 811 | 50x403, 100x6, 1000x2 | 30x400, 20x400, 100x6 | 49% | 400/0 | 2px x400 |
| candidate | `NEW 2021/14` | 129 | 257 | 150x128, 300x1 | 80x127, 70x127, 150x3 | 50% | 128/0 | 2px x128 |
| candidate | `Other/OTHER_SCROLL_Marvels02` | 393 | 775 | 50x384, 100x7, 1000x2 | 30x382, 20x382, 100x7 | 49% | 382/0 | 2px x382 |
| candidate | `Other/OTHER_SCROLL_Marvels03` | 393 | 775 | 50x384, 100x7, 1000x2 | 30x382, 20x382, 100x7 | 49% | 382/0 | 2px x382 |
| candidate | `Other/OTHER_Goldorak02` | 374 | 745 | 50x374 | 30x371, 20x371, 50x3 | 50% | 371/0 | 2px x371 |
| candidate | `Other/OTHER_Goldorak01` | 372 | 740 | 50x372 | 30x368, 20x368, 50x4 | 50% | 368/0 | 2px x368 |
| candidate | `Consoles/MD_NbaJam` | 440 | 898 | 60x254, 30x140, 40x21 | 20x638, 30x225, 1000x12 | 51% | 255/0 | 2px x42, 4px x178, 6px x34, 8px x1 |
| candidate | `Consoles/PICO-8_Honeycam_01` | 368 | 736 | 70x367, 120x1 | 20x316, 30x210, 70x157 | 50% | 208/2 | 2–8 px |
| candidate | `Other/OTHER_125` | 53 | 184 | 1000x18, 250x16, 500x7 | 140x30, 40x28, 70x26 | 71% | 8/20 | 4px x4, 6px x8, 7px x16 |
| candidate | `Other/OTHER_SCROLL_StarWars03` | 282 | 560 | 50x281, 1500x1 | 30x278, 20x278, 50x3 | 50% | 278/0 | 2px x278 |
| candidate | `Other/OTHER_SCROLL_StarWars02` | 280 | 557 | 50x279, 1500x1 | 30x277, 20x277, 50x2 | 50% | 277/0 | 2px x277 |
| candidate | `Consoles/NES_AboboLeft` | 75 | 152 | 180x75 | 90x146, 60x6 | 51% | 75/0 | 2px x73, 3px x2 |
| candidate | `Consoles/NES_AboboRight` | 75 | 152 | 180x75 | 90x146, 60x6 | 51% | 75/0 | 2px x73, 3px x2 |
| candidate | `Arcade/ARCADE_SFII_RattenJager` | 209 | 670 | 100x178, 60x24, 500x5 | 20x445, 30x150, 100x44 | 69% | 105/29 | 4px x75, 5px x59 |
| candidate | `Consoles/MD_Chase_HQ_II_01_Clivefrog` | 908 | 1175 | 50x815, 20x40, 30x36 | 50x548, 20x307, 30x303 | 23% | 265/2 | 2–8 px |
| candidate | `Consoles/MD_Ghostbusters_13_sephirot68` | 380 | 698 | 40x380 | 20x636, 40x62 | 46% | 318/0 | 2px x318 |
| candidate | `Other/OTHER_NINTENDO_MetroidScrollSamus` | 114 | 222 | 100x112, 1500x1, 1000x1 | 50x216, 100x4, 1500x1 | 49% | 108/0 | 2px x108 |
| candidate | `Consoles/SNES_MarioKart12_RattenJager` | 346 | 684 | 60x258, 50x64, 70x15 | 20x488, 60x110, 50x41 | 49% | 179/0 | 3–7 px |
| control | `Computers/PC_Leilanis Island_22` | 112 | 112 | 20x110 | = | 0% | 0/0 | |
| control | `Pinball_Short/PINBALL_SHORT_Elvis-Grease_Serum_23` | 35 | 35 | 90x22, 120x9 | = | 0% | 0/0 | |
| control | `Arcade/ARCADE_NEOGEO_Metal_Slug_X_03` | 36 | 56 | 100x36 | 100x30, 20x19, 30x6 | 36% | 6/0 | 3–7 px |
| control | `Consoles/NES_The_Legend_of_Zelda_03` | 32 | 32 | 200x32 | = | 0% | 0/0 | |
| control | `Computers/AMSTRAD_CPC_Bomb_Jack_02` | 186 | 186 | 30x98, 60x70 | = | 0% | 0/0 | |
| control | `Other/OTHER_Pixiv_Guru_01` | 16 | 16 | 200x16 | = | 0% | 0/0 | |
| control | `Other/OTHER_DBZ01` | 62 | 62 | 80x39, 40x11 | = | 0% | 0/0 | |
| control | `Other/OTHER_NINTENDO_Div118` | 50 | 54 | 80x38, 40x10 | 80x38, 20x8, 40x6 | 7% | 4/0 | 5px x4 |
| control | `Pinball_Short/PINBALL_SHORT_StarWars04` | 46 | 48 | 120x10, 60x9 | 120x10, 60x8 | 4% | 0/1 | 5px x1 |
| control | `Halloween/HALLOWEEN_OTHER_Div03` | 48 | 48 | 100x48 | = | 0% | 0/0 | |

Controls are 10 random files (seed 1) where cut + other is ≥ 60% of the
pairs.

### Where the detector was right or wrong (the controls)

- **Correctly tweened:** `Metal_Slug_X_03` (6 pairs) and `Div118` (4 pairs)
  are real rigid motion: a sprite sliding in on black and a sprite dropping
  into frame. The in-betweens are clean.
- **Ambiguous but harmless:** `StarWars04`, 1 pair. Two laser beams fly
  up-left while new ones fire. A diagonal line shifted along itself fits
  many shifts (the aperture problem), and the fit picked (0, −5). The
  in-between shows the old beams part-way out and no new beams yet, which is
  plausible at 30 ms.
- **Wrong before the consistency gate, rejected after it:** 3 pairs in
  `Bomb_Jack_02` were a row-by-row text wipe taken for a 2 px scroll. The
  in-between had holes.
- **Left alone, as they should be:** no cut or "other" pair was tweened in
  any control. Seven of the 10 controls come out byte-identical in frames
  and delays.

### What the candidates look like

- **Scrolls** (Xmen, Marvels, Star Wars, Goldorak, Ghostbusters, Metroid,
  `NEW 2021/14`): exact. Every in-between is the true intermediate scroll
  position.
  - The 50 ms → 30 + 20 ms split makes a 3-refresh/2-refresh cadence at
    100 Hz, i.e. 1 px per 25 ms on average.
  - It reads clearly smoother than 2 px jumps; the unevenness is not
    noticeable.
  - An ODM-side tweener could store 25/25, but the panel quantises to
    10 ms refreshes anyway.
- **Chase HQ II's marquee** (4 px per 50 ms): clean. This is the biggest
  visual win.
- **SFII:** Ryu rising behind the logo; the logo stays put. Mostly clean.
- **Honeycam:** an isometric scroll with a ball that animates. Small
  one-pixel glitches where the ball's pose changes during the move.
- **Mario Kart:**
  - Clean on the sideways slides.
  - Perspective road motion isn't a shift, so those pairs are left as they
    were.
- **OTHER_125:**
  - Eyes in the dark move 4–7 px per 250–1000 ms and become 70–140 ms steps.
    It is plausible, though it is a stylistic change: steps become glides.
  - Holds are protected by the hold rule, but a "slow" file is itself long
    delays.

### GIF size

The tweened GIFs are 2.8–7.4x the originals. Pillow's writer doesn't diff
frames, and it makes even untouched controls 1.8–2.7x bigger. The fair
figure is the frame count: +100% on a pure 2 px scroll, and +4% across the
archive if applied everywhere it's allowed. The ODM's run coding should
store an in-between of a scroll at about the cost of a scroll frame; I
didn't measure this.

## 3. RIFE

- **Model:** `rife-ncnn-vulkan` 20221029 (the only Linux release), model
  **rife-v4.6**, which supports arbitrary timesteps.
- **Hardware:** it runs on the CPU through Mesa's **lavapipe** Vulkan
  driver. The Intel iGPU is not usable here: `/dev/dri/renderD128` needs
  the `render` group, and fixing that needs sudo.
- **Inputs:**
  - Native 128x32: the binary pads internally.
  - 4x nearest (512x128), with the output sampled back at block centres.
- **Pairs:** every pair the shift tweener tweened got the same k at the same
  fractions. Every other non-static, splittable, non-hold pair got one
  in-between at 0.5, which is what a model-only "double the frame rate"
  pipeline would do.
- **Coverage:**
  - Native: all pairs in the sheet window plus 30 spread through each file,
    1,176 in-betweens.
  - 4x: the sheet window only, 230 in-betweens.
- **Snapping:** each in-between is also snapped to the nearest source
  colour.

### Timing on this CPU

i5-12600H, lavapipe, a shared machine at load average ~28 on 16 threads.
Measured with `bench_rife.py` in directory mode, start-up subtracted.

| input | per inference | start-up (model load + Vulkan) |
|---|---:|---:|
| 128x32 native | **130 ms** | 1.2 s |
| 512x128 (4x) | **670 ms** | 2.1 s |

- **During the runs:** at higher load they were 200–400 ms (native) and
  0.7–2.3 s (4x) per in-between, including start-up and wasted gaps.
- **One file** of 400 frames with a 2 px scroll at 4x: ~4.5 min.
- **The shift tweener** needs a few ms of numpy per pair, classification
  included. A Rust or JS port would be well under that, in the browser or
  in the prepare step.

### Palette breakage and accuracy

Numbers are from `compare.py`; per-file rows are in `out/compare.md`.
Medians are over files.

| | native | 4x |
|---|---:|---:|
| distinct colours per in-between that aren't in the source (median of file means) | 700–1,100 on scrolls, up to 2,661 | 1–80 on scrolls, up to 976 |
| pixels visibly off-palette (> 24 RGB distance from every source colour) | 2.1% (controls up to 46%) | 0.0% (controls up to 26%) |
| rigid pairs: changed pixels where RIFE-snapped ≠ the exact shift in-between | **10.6%** | **0.3%** |
| non-rigid pairs (cut/other/pose): changed pixels that are neither A's nor B's | **41%** | **33%** |

- **Native 128x32:**
  - The thousands of "new colours" are mostly ±1–2 drift, which snapping
    removes.
  - The 10.6% is real, though: the flow is estimated on 1-px features and
    edges come out half-lit. After snapping they flicker between the
    neighbours' colours (Xmen, Chase HQ sheets).
  - Honeycam (46%), the Abobo walk (47%) and Chase HQ's 4 px marquee
    (66%) are the worst.
- **4x nearest:** on rigid scrolls it matches the shift tweener to the
  pixel, after snapping. It is correct, but only a far more expensive route
  to the same frames.
- **Where it smears:** every non-rigid pair.
  - `Elvis-Grease` (a pinball flash between two screens) becomes a warped
    morph of both.
  - Makaimura's scene change becomes a dissolve with the monster ghosted
    over the next background.
  - `Div03` and `DBZ01` animation frames turn into blends.
  - Snapping turns blends into speckle rather than removing them.
  - RIFE has no idea of a cut. A cut detector would have to run first, and
    then RIFE is only being used on the pairs where the shift tweener is
    already exact.
- **Where it might beat the shift method:** motion that isn't a shift, like
  perspective roads (Mario Kart), rotation, zoom or parallax layers. The
  shift tweener leaves those pairs alone. This run measured too few of them
  to judge:
  - The 4x windows held none.
  - Native RIFE made 53% "neither" pixels on Mario Kart's non-shift pairs.

  I found no candidate where RIFE was clearly better.

## 4. Persistence baseline

The original frames through a phosphor model: every 10 ms refresh,
`out = max(new, prev · 0.6)` in linear light (γ 2.2). A lit LED falls to 36% of
its linear value after 2 refreshes (20 ms).

- **Scrolls:** it gives a trailing ghost 1–2 px long behind every moving
  edge. On the Chase HQ marquee (4 px per 50 ms) the text becomes a smear.
  It never shows an intermediate position, so it doesn't make motion
  smoother. It makes it blurrier.
- **Cuts and flashes:** the previous screen fades out over ~30 ms (the
  Elvis flash, the Makaimura scene change). It is harmless but pointless.
- **On-device cost:** a max and a multiply per LED per refresh, so it is
  cheap. But it answers a different question (LED afterglow) and is a
  "look" setting at best. **Don't ship it as "smooth motion".**

## Sheets

Each PNG is `out/sheets/<Category>__<name>.png`. Time runs left to right in
20 ms slices, showing what the panel shows at that instant. Rows are:
original, persistence, shift tween, RIFE native (raw, then snapped) and
RIFE 4x (raw, then snapped). Synthesised frames are marked above the cell:
orange for shift, blue for RIFE. Each sheet covers the window with the most
tweening, cropped to 48 px around the motion.

The ones worth opening first:

- **`Consoles__MD_Chase_HQ_II_01_Clivefrog.png`**: the fast marquee.
  - The shift tween is clean.
  - Persistence smears.
  - RIFE native is soft.
- **`Other__OTHER_SCROLL_Xmen01.png`**: a 2 px scroll at 50 ms. The shift
  tween is exact.
- **`Arcade__ARCADE_SFII_RattenJager.png`**: a regional move behind a static
  logo.
- **`Pinball_Short__PINBALL_SHORT_Elvis-Grease_Serum_23.png`** (control):
  - The shift tween leaves the flash alone.
  - RIFE morphs it at native and at 4x.
  - Persistence ghosts it.
- **`Consoles__PICO-8_Honeycam_01.png`**: the shift tween's worst candidate
  (small pose glitches).
- **`Pinball_Short__PINBALL_SHORT_StarWars04.png`** and
  **`Arcade__ARCADE_NEOGEO_Metal_Slug_X_03.png`**: the controls that did
  get tweens.

The tweened GIFs are `out/gifs/<name>.tween.gif`, 30 files: 20 candidates
and 10 controls. They are written there rather than beside the source,
because the card is shared and read-only for this experiment.

## Recommendation for the Studio / prepare pipeline

**"Smooth motion": the shift tweener, opt-in, per file. No model.**

- **Analysis:** it runs in the prepare step, or when the Studio exports. It
  is the classifier from this experiment, which is O(pairs × 288 shifts ×
  ≤ 512 sampled pixels), ~2 ms per pair in numpy.
- **Per file, the UI says:** "N% of this animation moves in steps of 2 px
  or more. Smooth motion adds M frames."
  - Offer the toggle only when N ≥ 10% (1,003 pack files, 9%).
  - Otherwise hide it: there's nothing to smooth.
- **Defaults:**

  | setting | default |
  |---|---|
  | shift search | ±8 px |
  | shift fit | ≥ 0.97 of changed pixels, with the whole-frame fit ≥ 0.9 for global translation |
  | consistency gate | ≤ 0.25 |
  | in-betweens per pair | \|shift\| − 1, one per pixel |
  | shortest frame | 20 ms, i.e. 2 panel refreshes |
  | hold rule | pairs slower than max(2 × median delay, 200 ms) are pauses and stay |

  Cuts, "other" and static pairs are never touched. Total duration is
  always preserved.
- **Output:** the ODM copy gets the extra frames. The user's GIF stays the
  GIF, and the product model is unchanged. The prepared copy's `delay_ms`
  is in ms, so an ODM-side tweener can split 50 ms as 30/20 or 25/25. The
  panel's 10 ms refresh makes those the same.
- **Where it pays off:** 2–8 px/frame marquees, credits and picture scrolls
  at 40–200 ms, and sprites crossing the screen. Title cards the Studio
  makes itself don't need it: they are generated at 1 px per 40 ms (25 px/s)
  already. For Studio-generated pans and scrolls the better "smooth motion"
  is simply **generate at 1 px per step** with a shorter delay, down to
  20 ms, rather than tween afterwards.
- **Not worth it:** the 1 px scrolls that dominate the pack, anything at
  ≤ 30 ms, and animation.

**What would be model-worthy.** Not frame interpolation for this pack:

- **Rigid motion is solved exactly.** Integer shifts on indexed pixels
  leave nothing for a model to add.
- **Everything else is animation or cuts.** There, any interpolation is a
  smear or a morph. On 5 mm LEDs that reads as a glitch, not smoothness.

The only open niche is **non-rigid camera motion**: perspective roads,
zooms, rotation and parallax. There a flow model at 4x plus palette
snapping could produce integer-pixel in-betweens. That would need a model
trained or fine-tuned on pixel art that outputs *index maps* (a warp field
applied to palette indices) rather than RGB. It is a research item, it
only helps a sliver of the "other"/regional pairs, and it runs at 0.7 s per
frame on this CPU, not on the device. I wouldn't pursue it before the
opt-in shift tweener has shipped and users ask for more.

## Reproducing

See [`opendmd-web/experiments/tween/README.md`](../opendmd-web/experiments/tween/README.md):
`measure.py` (25 min) → `tween.py` → `rife.py` (native, then
`--scale 4 --window-only`) → `compare.py` → `sheets.py`.
