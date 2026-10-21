# Troubleshooting

## The installer can't find or connect to the board

- Use Chrome, Edge or Opera on a computer, not Firefox, Safari or a phone.
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
  password), it opens the **OpenDMD-Setup** network again so you can enter
  the new details, as on [first start](install.md#first-start).

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

## Still stuck?

- **Building the display, the hardware or GIFs:** the DMDoS community's
  [Telegram group](https://t.me/DMD0S) is where people help each other.
  The [Assembly section of dmdos.net](https://dmdos.net/) shows each step
  of the build with a photo.
- **OpenDMD itself:** open an issue on
  [GitHub](https://github.com/opendmd/opendmd/issues). OpenDMD is an
  independent project, so please don't ask the DMDoS team to fix it.
