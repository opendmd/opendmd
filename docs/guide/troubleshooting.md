# Troubleshooting

## The installer can't find or connect to the board

- Use Chrome, Edge, Opera or Firefox (151 or later) on a computer, not
  Safari or a phone.
- Try a different USB cable: many only carry power.
- On Windows or older macOS, install the driver for the board's USB chip,
  then plug the board in again:
  [CH340/CH341](http://www.wch-ic.com/downloads/CH341SER_ZIP.html),
  [CP210x](https://www.silabs.com/developers/usb-to-uart-bridge-vcp-drivers)
  or [FTDI](https://ftdichip.com/drivers/). The chip's name is printed on
  it, next to the USB socket.
- Hold the board's **BOOT** button while the installer says "Connecting".

## I can't open opendmd.local

- Make sure your phone or computer is on the same WiFi as the player.
- Some devices and networks don't support `.local` names. Use the player's
  address instead: find it in your router's list of connected devices, or
  take the SD card out and power the display on, and the panel shows its
  address.
- If the player can't join its saved WiFi (a new router, a changed
  password), plug it into a computer, open the [installer](install.md),
  click **Install** and choose **Change Wi-Fi**. Or join the
  **OpenDMD-Setup** network it opens and enter the new details there, as
  in [WiFi without the installer](install.md#wifi-without-the-installer).
- **Two displays with the same name.** Every display is called `opendmd`
  until you rename it. When a second one joins your network with the same
  name, one of them moves to `opendmd-2.local` (and it may be the one you
  had first). Open `opendmd-2.local` or the display's address,
  then give each display its own name in **Settings → Device → Network**
  (a name another display already has is refused). **Settings → Device →
  About** shows the address a display really has.

## The panel says there's no card, or the card can't be read

- Check the card is pushed fully into its slot.
- The player needs a **FAT32** card. If the web app's **Now** view says the
  card needs FAT32, use **Format card…** there (this erases it), then add
  your GIFs again. See [The SD card](card.md).
- The player keeps checking for a card, so you don’t need to
  restart it after swapping cards.

## The display resets or flickers when it's bright

The power supply can't keep up at high brightness, especially on mostly
white pictures. Lower the brightness, or lower **Maximum brightness** in
**Settings → Power** if you raised it.

## Some LEDs show the wrong colours on certain pictures

If a few dots come out the wrong colour on some pictures but not others,
the panel may be fussy about timing. Open **Settings → Device → Panel
timing** and press **Test panel timing**. The panel shows some test
pictures, switching between **A** and **B** (the letter is in the bottom
corner): pick the one that looks clean and it is saved. If both have dots,
it tries a second setting the same way. Stop puts everything back, and so
does leaving the page for two minutes.

If neither helps, change the settings by hand, one at a time: **Clock
edge**, then **Latch blanking**, and only then **Driver chip**. Each change
is tried for a minute: press **Keep** if it looks better, or let it go back
by itself. The driver chip needs a restart to try, and if the panel goes
dark, just wait and it comes back on its own. **Reset to defaults** puts
everything back. After trying a different driver chip, switch the display
off and on before judging the result. If nothing helps, the panel itself
may be faulty.

## Settings says "Card running at reduced speed" or shows card errors

The card is wearing out. The player keeps going as best it can, but replace
the card soon and copy your GIFs over.

## The web app won't load, or looks broken

The player always has a simple backup page at
**<http://opendmd.local/setup>**: WiFi, basic playback settings and a
restart button. To put the web app right, run the
[installer](install.md) again with **Erase device** unticked.

## A WiFi update didn't finish

If the board's light blinks fast after a firmware update, it is waiting to
finish installing from the SD card: make sure the card is in and leave it.
If the player still doesn't come back, run the [installer](install.md)
again with **Erase device** unticked.

## Starting over

Running the [installer](install.md) with **Erase device** ticked wipes the
board back to a fresh install: WiFi details, settings and a forgotten
security key all go. Your SD card, GIFs and playlists are kept.

## Building the display

The [Assembly section of dmdos.net](https://dmdos.net/) shows each step of
the build with a photo.
