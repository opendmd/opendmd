<h1 align="center"><img src="docs/images/wordmark.png" width="440" alt="OpenDMD"></h1>

<p align="center">New open firmware and a web app for the DMDoS LED dot-matrix display.</p>

<p align="center">
  <a href="https://opendmd.dev/guide/hardware"><img src="https://img.shields.io/badge/for_the-DMDoS_display-33d1ff?labelColor=15171b" alt="For the DMDoS display"></a>
  <a href="LICENSE-APACHE"><img src="https://img.shields.io/badge/license-MIT_%2F_Apache--2.0-5ef09a?labelColor=15171b" alt="License: MIT or Apache 2.0"></a>
</p>

<p align="center">
  <a href="https://opendmd.dev/demo/"><b>Try the demo in your browser →</b></a>
  &nbsp;·&nbsp; <a href="https://opendmd.dev/install/">Install on your display</a>
  &nbsp;·&nbsp; <a href="https://opendmd.dev/guide/hardware">What you need</a>
  &nbsp;·&nbsp; <a href="https://opendmd.dev/">Documentation</a>
</p>


OpenDMD is open-source firmware and a web app for the **DMDoS display**: a 128x32 LED
dot-matrix display, two chained 64x32 HUB75 panels driven by an ESP32, that plays animated GIFs
from a microSD card, like the "DMD" in a pinball machine. DMDoS is a community project from
Spain, and **[dmdos.net](https://dmdos.net/)** is where to get the hardware, build the display
and find GIFs. OpenDMD is an alternative firmware for the same board: it plays a DMDoS card as
it is, and adds a web app to manage it.

- **The firmware** runs on the display's own ESP32 board. It plays the card and serves the web
  app over WiFi.
- **The web app** at `http://opendmd.local/` works in any browser, on a phone or a computer.
  Previews and GIF preparation run right in the browser, using the same engine as the display.
- **The simulator** plays a card in a desktop window, drawn as LEDs rather than flat pixels
  (for developers: [build it from source](docs/simulator.md); everyone else can try the
  [web demo](https://opendmd.dev/demo/)).


## OpenDMD and DMDoS

OpenDMD only replaces the software on the ESP32. The hardware, the build and the GIFs come
from DMDoS:

<!-- site:features -->

- **The hardware:** the parts list, with where to buy each one, is in the Hardware section of
  [dmdos.net](https://dmdos.net/). The DMDoS Board V3 is made by
  [MORTACA](https://mortaca.com). [What you need](docs/guide/hardware.md) sums it up.
- **Building it:** the Assembly section of [dmdos.net](https://dmdos.net/) walks through it
  step by step, with a photo for each.
- **A frame:** a 3D-printable frame by Janibol of
  [Retromojones](https://www.youtube.com/@retromojones) is on
  [Thingiverse](https://www.thingiverse.com/thing:6704880). The Frame section of dmdos.net has
  printed kits and wooden frames.
- **GIFs:** the DMDoS GIF pack is in the Software section of [dmdos.net](https://dmdos.net/).
  A card set up with it plays on OpenDMD as it is.

OpenDMD is an independent project, not made by or affiliated with DMDoS or MORTACA. The
original DMDoS firmware can be put back at any time from the Software section of dmdos.net.

## Install

Open the **[installer](https://opendmd.dev/install/)** in Chrome, Edge, Opera or
Firefox on a computer, plug the
display's ESP32 board into your computer by USB, and click **Install**. When it's done, the
installer asks for your WiFi and puts the board on it; the web app is then at
`http://opendmd.local/`. (Skipped that? The display opens a WiFi network called
`OpenDMD-Setup` to set it up from instead.) [Install](docs/guide/install.md) has the details.

From source: `make fw-release && make flasher` (see [For developers](docs/developers.md)).

## What it does

<!-- site:features -->

- **Library:** search, browse by category and platform, preview each GIF with the panel look,
  send it to the panel, and add, rename or delete GIFs.
- **Playlists:** build them by platform, category and title words, then reorder and switch
  between them live. A GIF can be a favourite in one playlist and play less often in another.
- **Create Art:** find game screenshots, boxes and logos, or GeoCities GIFs, or use your own
  picture. Frame it, recolour it, add text, trim frames, and save it as a GIF ready for the panel.
- **Smooth motion:** adds the in-between frames to GIFs that move in jerky steps, without
  changing the file.
- **Formulas:** a few lines of maths that draw the panel's pixels. Ten come with the panel (a big
  clock, Matrix, a starfield, an endless synthwave road, a time gate, and five looks to draw over
  any GIF: scanlines, an old TV, a vignette, an old photo, glitter); the online [gallery](https://opendmd.dev/formulas/) has the
  rest (plasma, fire, rain, a tunnel, Mode 7 rotozoom, a wireframe cube, …) and the ones people share.
  Remix one, drag a number and watch, submit it to the gallery, or save it as a GIF.
- **Looks:** full colour or a single amber, red, green, blue or white, with optional phosphor glow.
- **Schedules:** a playlist or brightness for parts of the week or of the year (Halloween and
  Christmas playlists that come on by themselves), and a night mode that turns the panel off
  or dims it, between set times or from sunset to sunrise.
- **Smooth playback:** the display gets each GIF ready in the background, so GIFs start
  quickly and play smoothly.

## Screenshots

| Library | Create Art |
| --- | --- |
| ![The Library: two GIFs ticked, and the menu that adds them to a playlist](docs/images/library.png) | ![Create Art with a picture loaded: Frame, Look, Text, Preview and Save](docs/images/create-art.png) |
| **Now playing** | **Formulas** |
| ![The Now view, previewing what the panel plays](docs/images/now.png) | ![Formulas: the ones that come with OpenDMD, and one playing in its details](docs/images/formulas.png) |
| **Playlists** | **On a phone** |
| ![A playlist open in the editor](docs/images/playlists.png) | <img src="docs/images/phone-library.png" width="300" alt="The Library on a phone, two GIFs ticked to add to a playlist"> |

The demo card in these screenshots holds OpenDMD's own Formulas and the
[Kenney Pixel Platformer](https://kenney.nl/assets/pixel-platformer) sprites (CC0).
Game title screens from [libretro-thumbnails](https://github.com/libretro-thumbnails),
made into panel GIFs with Create Art. `make screenshots` takes them again
(`tools/screenshots.py`); the GIFs themselves are made on your machine and
never published.

## Documentation

The documentation is also at **[opendmd.dev](https://opendmd.dev/)**.

- [What you need](docs/guide/hardware.md), [Install](docs/guide/install.md) and
  [Updating](docs/guide/updating.md)
- [The web app](docs/guide/web-app.md), [Create Art](docs/guide/create-art.md)
  and [Formulas](docs/guide/formulas.md)
- [The SD card](docs/guide/card.md): where GIFs go and how their names are read
- [Troubleshooting](docs/guide/troubleshooting.md)
- [For developers](docs/developers.md): building from source, the crates, the
  [HTTP API](docs/api.md)

## Development

Everything builds from this repository with `make`; [For developers](docs/developers.md) lists
the toolchains, the common targets and the crates.

## License

Either [Apache 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. Contributions are
dual licensed the same way.

The web app's headings use [Silkscreen](https://github.com/googlefonts/silkscreen) by Jason
Kottke, a pixel font under the [SIL Open Font License 1.1](opendmd-web/www/fonts/OFL.txt). A
small subset of it is bundled with the web app and the setup page, so they work without the
internet, and published with the site. The site's own headings use
[Fraunces](https://github.com/undercasetype/Fraunces) by the Fraunces Project Authors, also
under the [SIL Open Font License 1.1](site/fonts/OFL-Fraunces.txt); one cut of it is served
from the site itself.

The firmware and the web app are built on other open-source software (ESP-IDF, FreeRTOS,
lwIP, Mbed TLS and the Rust crates among it), listed with each licence's full text in
[the firmware's](licenses/THIRD-PARTY-LICENSES-firmware.txt) and
[the web app's](licenses/THIRD-PARTY-LICENSES-web.txt) third-party notices. Each release and
the web app itself carry them (Settings › Device › About › Open-source licences).

OpenDMD comes with no GIFs. The animations on a DMDoS card are third-party content that this
project can't relicense or redistribute: get the DMDoS GIF pack from
[dmdos.net](https://dmdos.net/), or make your own with Create Art. The animations on the
OpenDMD site are OpenDMD's own Formulas, recorded from the simulator, and the
CC0 [Kenney Pixel Platformer](https://kenney.nl/assets/pixel-platformer) sprites.
