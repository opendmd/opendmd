# What you need

OpenDMD runs on the **DMDoS display**: two LED panels, an ESP32 board and
a microSD card, built into a 128x32 dot-matrix display. The display is
designed by the DMDoS community project, and **[dmdos.net](https://dmdos.net/)**
is where to find everything for it: the parts list with shop links in its
**Hardware** section, step-by-step build instructions with photos in
**Assembly**, and frames in **Frame**.

This page sums up the parts and what OpenDMD expects of them. Already have
a working DMDoS display? Then you have everything:
[install OpenDMD](install.md) on it.

<!-- site:cards -->

### 1 · ESP32 DevKit V1 (USB-C or micro-USB)

The brain of the display: a common ESP32 development board with a USB-C or micro-USB
socket. It runs the firmware, drives the panels and serves the web app over
WiFi. OpenDMD installs onto it straight from your browser.

**Where:** the Hardware section of [dmdos.net](https://dmdos.net/) links
to it on AliExpress.

### 2 · DMDoS Board V3

The carrier board the ESP32 plugs into, designed and made by
**[MORTACA](https://mortaca.com)**. It sits on the panels' input
connector, holds the microSD card slot and feeds power to the panels.
OpenDMD is built for this board's wiring.

**Where:** [mortaca.com](https://mortaca.com), linked from the Hardware
section of [dmdos.net](https://dmdos.net/).

### 3 · A microSD card

Where your GIFs live. DMDoS suggests a 32 GB card, but any card you have
will do. OpenDMD reads FAT32 cards; a bigger card that comes as exFAT can
be formatted from the web app. See [The SD card](card.md).

**Where:** any shop; the Hardware section of [dmdos.net](https://dmdos.net/)
has a suggestion.

### 4 · Two P4 64x32 LED panels

Two HUB75 panels with a 4 mm pitch, 256x128 mm each, joined side by side
into one 512x128 mm display of 128x32 LEDs.

**Where:** the Hardware section of [dmdos.net](https://dmdos.net/) links
to the panels it uses. Check the size and pitch if you buy elsewhere.

### 5 · A USB-C cable and power

A USB **data** cable to install OpenDMD from a computer (some cables only
carry power), and a USB-C power supply to run the display. The panels draw
a lot of current when bright, and a weak supply makes the display reset.

**Where:** you probably have both already.

### 6 · A frame (optional)

A 3D-printable frame by **Janibol of
[Retromojones](https://www.youtube.com/@retromojones)** is on
[Thingiverse](https://www.thingiverse.com/thing:6704880). It needs a large
printer (300x300x400 mm). A printed kit can be ordered from dmdos.net, and
[@jac72](https://t.me/jac72) makes custom wooden frames.

**Where:** the Frame section of [dmdos.net](https://dmdos.net/).

<!-- site:cards-end -->

## Building it

Follow the **Assembly** section of [dmdos.net](https://dmdos.net/): it
shows each step with a photo, and takes a few minutes even without any DIY
experience.
It covers joining the panels, which connector the board goes on, the power
leads and the ribbon cable.

You don't need to install the DMDoS firmware before OpenDMD. If the board
already runs it, OpenDMD replaces it, and your SD card stays as it is.

## GIFs

OpenDMD comes with no GIFs. The DMDoS GIF pack, **DMDos GUI v1 + GIFs**,
is in the Software section of [dmdos.net](https://dmdos.net/): a card set
up with it plays on OpenDMD as it is. You can also make your own with
[Create Art](create-art.md), or add any 128x32 GIF from the
[web app](web-app.md).

## Next

- [Install OpenDMD](install.md) on the ESP32.

OpenDMD is an independent project, not made by or affiliated with DMDoS,
MORTACA or Retromojones. Thanks to all three for the display, the board and
the frame.
