# The home page's scenes

The home page's hero has a scene behind it. There are seven, all published;
everyone gets `shelf`, and a viewer can ask for another. All of it is our
own pixel art, drawn in code; every machine in it is made up.

| Scene | What it is | Address | Drawn by |
|---|---|---|---|
| `shelf` | A collector's wall of made-up consoles, by day | `/` (or `/?scene=shelf`) | `tools/site-scene-alt.py` |
| `synth` | Synthwave: a banded sun, a grid, ridges and palms | `/?scene=synth` | `tools/site-scene-alt.py --style synth` |
| `night` | The first scene: the games den at night, dithered | `/?scene=night` | `tools/site-scene.py` |
| `day` | The same den by daylight | `/?scene=day` | `tools/site-scene-alt.py --style day` |
| `chunky` | A 16-bit playroom, bold outlines | `/?scene=chunky` | `tools/site-scene-alt.py --style chunky` |
| `neon` | The same wall at night under neon strips | `/?scene=neon` | `tools/site-scene-alt.py --style neon` |
| `arcade` | An arcade corner on a confetti carpet | `/?scene=arcade` | `tools/site-scene-alt.py --style arcade` |

## Switching (one viewer, no rebuild)

`/?scene=NAME` shows that scene and remembers it in the browser
(`localStorage`, key `opendmd-scene`), so `/` shows it from then on.
`/?scene=shelf` goes back to everyone's and forgets. A name that isn't a
scene shows `shelf` and changes nothing. With no storage (a private window)
the address still works and nothing is remembered. There is no control on
the page for it: the address is the only way in.

How: `scene.js` (a small classic script in the home page's head, so it runs
before the first paint) puts the name on `<html data-scene="NAME">`;
`site.css` holds each scene as one rule of variables (`[data-scene=NAME]`:
the picture, the wall colour, the ink, the shade behind the words, the top
bar, the room under the words), which only `.hero` and the home page's top
bar read. So only the scene in use is fetched, and a scene's colours cannot
reach another's.

`shelf` and `day` have a light wall and dark ink, and keep them in the dark
theme: a scene looks the same in both themes, by design.

## The pieces

- `site/theme/den.webp`, `den-phone.webp`: everyone's scene (1920x1488, laid
  `cover` behind the hero; 780x720, the foot of the scene under the words on
  a narrow screen). `den-NAME.webp`, `den-NAME-phone.webp`: the others.
  `make site-scene` redraws them all.
- `site/theme/site.css`: "The home page's scene" near the top.
- `site/theme/scene.js`, and `SCENES` in `tools/site.py` (what is
  published), `tools/site-smoke.py` (what is checked: `scenes`) and
  `tools/site-scene-shots.py`.
- `uv run tools/site-scene-shots.py` (after `make site`) photographs the
  hero in every scene at 1280, 1920, 820 and 390, light and dark, into
  `target/hero-art/`, with a contact sheet.

## To change everyone's scene

1. In `tools/site-scene-alt.py` set `LIVE = "NAME"` and run
   `make site-scene`: NAME is written to `den.webp` and `den-phone.webp`
   and the scene that was everyone's to `den-<its name>*.webp`. (`night` is
   drawn by `tools/site-scene.py`: change its file names there.)
2. In `site/theme/site.css` swap the two scenes' variables: NAME's rule
   becomes the `:root` one (with `url(den.webp)`), the old `:root` values
   become `[data-scene=<old>]` (with `url(den-<old>.webp)`). The
   `[data-scene]` rule holds what every scene but everyone's starts from, so
   it must undo whatever of `:root` the others do not want (today: shelf's
   dark ink and light shade).
3. Put NAME first in the list in `scene.js` and in the three `SCENES`.
4. `make site`, the screenshots, then `tools/site-smoke.py` (`--fn scenes`
   for these checks alone).

## To add a scene

Draw it (`STYLES` in `tools/site-scene-alt.py`), give it a
`[data-scene=NAME]` rule, add the name to `scene.js` and the three `SCENES`.
Keep the upper middle of the picture bare (the panel and the words lie over
it) and the top row of the phone picture the flat colour of `--den`.
