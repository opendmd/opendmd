# Install OpenDMD

Put OpenDMD on your display straight from this page. It takes about a
minute, and your SD card and its GIFs are left alone.

<!-- site:install-button -->

## What you need

- **Chrome or Edge on a computer** (Windows, macOS, Linux or ChromeOS).
  Firefox, Safari and phones can't talk to USB boards from a web page.
- **A USB data cable.** Some cables only carry power; if the installer
  can't find the board, try another cable.
- **The display's controller board**, an ESP32 DevKit V1: the small board
  with a USB socket that the panels are wired to.

On Windows and older versions of macOS, the board's USB chip (a CH340) may
need its driver installed before the computer can see it.

## Install

1. Plug the board into your computer with the USB cable.
2. Click **Install** and pick the board's port from the list. It is often
   called "USB Serial" or "CH340".
3. The installer asks whether to **Erase device**:
    - **First install** (the board still runs DMDoS or anything else):
      **tick Erase.** This clears the board completely before installing.
    - **Updating** a board that already runs OpenDMD: **leave Erase
      unticked.** Your WiFi details and settings stay as they are.
4. Wait for it to finish. The board restarts by itself.

If the installer can't connect, hold down the board's **BOOT** button while
it says "Connecting", and let go once it starts writing. Unplugging the
board and trying again helps too.

## First start

A freshly installed player doesn't know your WiFi yet, so it makes its own
network for you to join:

1. On your phone or computer, join the WiFi network **OpenDMD-Setup**.
2. Open <http://192.168.71.1/setup> in a browser. Type your WiFi
   network's name (**Scan** lists the ones nearby) and its password, then
   press **Save & reboot**.

    If the OpenDMD web app pops up by itself when you join, you can do the
    same there under **Settings → Advanced → Network**. The board restarts
    every ten minutes while it waits; if the setup network vanishes, wait a
    moment and join it again.

3. Rejoin your usual WiFi and open **<http://opendmd.local/>**. That's the
   web app, where you manage everything from now on.

With your card in, the panel starts playing straight away. A card that
worked with DMDoS works as it is. See [The SD card](card.md) for setting
up a new one.

## Firmware update file

To update the firmware over WiFi instead of USB, download this file and
follow [Updating](updating.md#over-wifi-firmware-only):

<!-- site:firmware-bundle -->

## Next

- [The web app](web-app.md): what each part of it does.
- [Updating](updating.md): getting newer versions later.
- [Troubleshooting](troubleshooting.md): if something doesn't work.
