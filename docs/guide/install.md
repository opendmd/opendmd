# Install OpenDMD

Put OpenDMD on your display straight from this page. It takes about a
minute, your SD card and its GIFs are left alone, and the installer gets
the board onto your WiFi before you unplug it.

OpenDMD replaces the DMDoS firmware on the same ESP32 board: nothing to
rewire or swap. Want the original firmware back later? Install it again
from the Software section of [dmdos.net](https://dmdos.net/).

> **Works in:** Chrome, Edge, Opera or Firefox (version 151 or later) on a
> computer: Windows, macOS, Linux or ChromeOS. **Not** Safari, and not
> phones or tablets: they can't talk to USB boards from a web page.

<!-- site:install-button -->

## Before you start

- **A computer with one of those browsers.**
- **A USB data cable.** Some cables only carry power; if the installer
  can't find the board, try another cable.
- **The display's ESP32 board**: the ESP32 DevKit V1 (the small board
  with the USB socket that sits on the DMDoS Board).
  It can stay in the display. Don't have a display yet? See
  [What you need](hardware.md).

## Step by step

<!-- site:steps -->

1. **Plug in the board.** Connect the ESP32 to your computer with the USB
   cable.
2. **Click the Install button** above and pick its port
   from the list. It is often called "USB Serial", "CH340", "CH343" or
   "CP210x".
3. **Choose whether to erase.** On a first install (the board still runs
   DMDoS or anything else) the installer asks whether to **Erase
   device**: **tick it.** This clears the board completely before
   installing.
4. **Wait for it to finish.** It takes about a minute, and the board
   restarts by itself. The panel stays dark while the installer writes
   over USB, and lights up when the board restarts.
5. **Connect it to your WiFi.** When it says **Installation complete**,
   click **Next**. The installer lists the WiFi networks the board can
   see: pick yours, type its password and click **Connect**. Once the
   board is on, **Visit device** opens the web app. That's it.
   (OpenDMD joins 2.4 GHz networks that are open or take a password,
   WPA2 or WPA3. A work or school network that asks for a username too
   (eduroam, WPA2-Enterprise) can't be set from the installer: skip this
   step, join the open **OpenDMD-Setup** network the board raises, and tick
   **Work or school login** on its setup page.)

    No GIFs on your card yet? Format it FAT32 and copy your DMDoS card, or
    the DMDoS GIF pack, onto it as it is: see [The SD card](card.md).

> **Connection problems?**
>
> - If the installer can't connect, hold down the board's **BOOT** button
>   while it says "Connecting", and let go once it starts writing.
>   Unplugging the board and trying again helps too.
> - If the board doesn't show up in the list at all, try another cable,
>   then install the driver for the board's USB chip (its name is printed
>   on the small chip next to the USB socket) and plug the board in again:
>   [CH340/CH341](http://www.wch-ic.com/downloads/CH341SER_ZIP.html),
>   [CP210x](https://www.silabs.com/developers/usb-to-uart-bridge-vcp-drivers)
>   or [FTDI](https://ftdichip.com/drivers/). Windows and older versions
>   of macOS are the ones that need it.

## A board that already runs OpenDMD

Plug it in and click the **Install** button: the installer
recognises it and shows its name and the version it runs. From there:

- **Update OpenDMD** puts this page's version on it. Your WiFi and
  settings stay. It only shows up when the versions differ.
- **Change Wi-Fi** moves it to another network (or **Connect to Wi-Fi**
  if it isn't on one).
- **Visit device** opens its web app.
- **Logs & Console** shows what the board is up to, handy when asking
  for help.
- **Erase User Data** reinstalls from scratch and forgets every setting.

Boards on older OpenDMD versions don't introduce themselves yet, so the
installer just offers **Install** and asks about erasing: **leave Erase
unticked** and your WiFi and settings stay.

## WiFi without the installer

If you skipped the WiFi step, or the installer couldn't reach the board
afterwards, the player makes its own network for you to join instead:

<!-- site:steps -->

1. **Join the setup network.** On your phone or computer, join the WiFi
   network **OpenDMD-Setup**. After a fresh install it can take a minute
   or two to appear.
2. **Enter your WiFi details.** Open <http://192.168.71.1/setup> in a
   browser. Pick your WiFi network (or type its name), enter its password,
   then press **Connect**.

    **Display name** is the display's address on your network:
    `opendmd` becomes <http://opendmd.local/>. If you have more than one
    display, give each its own name (`opendmd-cab`, `opendmd-desk`):
    lowercase letters, digits and hyphens, up to 32.

    If the OpenDMD web app pops up by itself when you join, you can do the
    same there under **Settings → Device → Network**. The board restarts
    every ten minutes while it waits; if the setup network vanishes, wait a
    moment and join it again.

3. **Open the web app.** Rejoin your usual WiFi and open
   **<http://opendmd.local/>** (or the name you gave it). That's the web
   app, where you manage everything from now on.

With your card in, the panel starts playing straight away. A card that
worked with DMDoS works as it is. See [The SD card](card.md) for setting
up a new one.

## Firmware update file

Once you're set up, **Check for updates** in the web app keeps the player
current over WiFi ([Updating](updating.md)). If you'd rather install the
firmware from a file, here it is:

<!-- site:firmware-bundle -->

## Going back to DMDoS

OpenDMD doesn't lock you in. To put the original DMDoS firmware back, use
the browser installer in the Software section of
[dmdos.net](https://dmdos.net/) the same way. Your SD card works with
either.

## Next

- [The web app](web-app.md): what each part of it does.
- [Updating](updating.md): getting newer versions later.
- [Troubleshooting](troubleshooting.md): if something doesn't work.
