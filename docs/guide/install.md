# Install OpenDMD

Put OpenDMD on your display from this page. It takes about a minute, and
the installer gets the board onto your WiFi before you unplug it. OpenDMD
replaces the DMDoS software on the same ESP32 board, with nothing to rewire
or swap, and it doesn't change your [SD card](card.md).

> Works in Chrome, Edge, Opera or Firefox 151+ on a computer running
> Windows, macOS, Linux or ChromeOS. Safari, phones and tablets can't talk
> to USB boards from a web page.

<!-- site:install-button -->

## Before you start

- A computer with one of those browsers.
- A USB data cable. Some cables only carry power; if the installer can't
  find the board, try another cable.
- The display's ESP32 board, the ESP32 DevKit V1. It is the small board
  with the USB socket that sits on the DMDoS Board, and it can stay in the
  display. See [What you need](hardware.md) if you don't have a display
  yet.

## Step by step

<!-- site:steps -->

1. **Plug in the board.** Connect the ESP32 to your computer with the USB
   cable.
2. **Click the Install button** above and pick its port from the list. It
   is often called "USB Serial", "CH340", "CH343" or "CP210x".
3. **Choose whether to erase.** On a first install, when the board still
   runs DMDoS or anything else, the installer asks whether to **Erase
   device**. Tick it: this clears the board completely before installing.
4. **Wait for it to finish.** It takes about a minute. The panel stays
   dark while the installer writes over USB, and lights up when the board
   restarts.
5. **Connect it to your WiFi.** When it says **Installation complete**,
   click **Next**, pick your network, type its password and click
   **Connect**. **Visit device** then opens the web app. OpenDMD joins
   2.4 GHz networks that are open or take a WPA2 or WPA3 password; a
   [work or school login](#work-or-school-wifi) is set up another way.

> **If it won't connect**
>
> - If the installer can't connect, hold down the board's **BOOT** button
>   while it says "Connecting", and let go once it starts writing.
>   Unplugging the board and trying again helps too.
> - If the board doesn't show up in the list at all, try another cable,
>   then install the driver for the board's USB chip.
>   [Troubleshooting](troubleshooting.md#the-installer-cant-find-or-connect-to-the-board)
>   has the links.

## A board that already runs OpenDMD

Plug it in and click the **Install** button. The installer recognises it
and shows its name and the version it runs. From there:

- **Update OpenDMD** puts this page's version on it, keeping your WiFi and
  settings. It only shows up when the versions differ.
- **Change Wi-Fi** moves it to another network, or **Connect to Wi-Fi** if
  it isn't on one.
- **Visit device** opens its web app.
- **Logs & Console** shows what the board is doing, which helps when
  asking for help.
- **Erase User Data** reinstalls from scratch and forgets every setting.

## WiFi without the installer

If you skipped the WiFi step, or the installer couldn't reach the board
afterwards, the display makes its own WiFi network for you to join instead.
While it waits, the panel says **Join WiFi  OpenDMD-Setup**.

<!-- site:steps -->

1. **Join the setup network.** On your phone or computer, join the WiFi
   network **OpenDMD-Setup**. After a fresh install it can take a minute
   or two to appear.
2. **Enter your WiFi details.** Open <http://192.168.71.1/setup> in a
   browser. Pick your WiFi network or type its name, enter its password,
   then press **Connect**.

    **Display name** is the display's address on your network, so
    `opendmd` becomes <http://opendmd.local/>. If you have more than one
    display, give each its own name, such as `opendmd-cab` and
    `opendmd-desk`. A name is up to 32 lowercase letters, digits and
    hyphens.

    If the OpenDMD web app pops up when you join, you can do the same
    there under **Settings › Device › Network**. The board restarts every
    ten minutes while it waits; if the setup network vanishes, wait a
    moment and join it again.

3. **Open the web app.** Rejoin your usual WiFi and open
   **<http://opendmd.local/>**, or the name you gave it. You manage the
   display from there from now on.

With your card in, the panel starts playing straight away.

## Work or school WiFi

A network that asks for a username as well as a password (eduroam,
WPA-Enterprise) can't be set from the installer. Skip the installer's WiFi
step, join the **OpenDMD-Setup** network as above, and tick **Work or
school login** on its setup page. The display doesn't check the network's
server certificate, so a fake network with the same name could capture
that login. If your network offers a password made for one device, use
that.

## Setting up a card

If your card has no GIFs yet, format it FAT32 and copy your DMDoS card, or
the DMDoS GIF pack, onto it as it is. [The SD card](card.md) has the
details.

## Firmware update file

Once you're set up, **Check for updates** in the web app keeps OpenDMD up
to date over WiFi ([Updating](updating.md)). If you'd rather install an
update from a file, here it is:

<!-- site:firmware-bundle -->

## Going back to DMDoS

To put the original DMDoS software back, use the browser installer in the
Software section of [dmdos.net](https://dmdos.net/) the same way.

## Next

- [The web app](web-app.md): what each part of it does.
- [Updating](updating.md): getting newer versions later.
- [Troubleshooting](troubleshooting.md): if something doesn't work.
