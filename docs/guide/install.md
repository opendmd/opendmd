# Install OpenDMD

Put OpenDMD on your display from this page. It takes about four minutes, and
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
2. **Click the Install button** above and pick its port from the list. Its
   name depends on the board's USB chip: "USB Serial" (CH340), "USB Single
   Serial" (CH343) or "CP2102 USB to UART Bridge Controller" (CP210x).
3. **Choose whether to erase.** On a first install, when the board still
   runs DMDoS or anything else, the installer asks whether to **Erase
   device**. Tick it: this clears the board completely before installing.
4. **Wait for it to finish.** It takes about four minutes. The panel stays
   dark while the installer writes over USB, and lights up when the board
   restarts.
5. **Connect it to your WiFi.** When it says **Installation complete**,
   click **Next**, pick your network, type its password and click
   **Connect**. **Visit device** then opens the web app. OpenDMD joins
   2.4 GHz networks that are open or take a WPA2 or WPA3 password; a
   [work or school login](#work-or-school-wifi) is set up another way.
   With **Play from the card only, no WiFi** ticked there is no WiFi
   step: the display starts playing its card ([A display without
   WiFi](#a-display-without-wifi)).

> **If it won't connect**
>
> - If the installer can't connect, hold down the board's **BOOT** button
>   as you click **Install**, and let go once it starts writing.
>   Unplugging the board and trying again helps too.
> - If the board doesn't show up in the list at all, try another cable,
>   then install the driver for the board's USB chip.
>   [Troubleshooting](troubleshooting.md#the-installer-cant-find-or-connect-to-the-board)
>   has the links.

## A board that already runs OpenDMD

Plug it in and click the **Install** button. The installer recognises it
and shows its name and the version it runs. From there:

- **Update OpenDMD** puts this page's version on it, keeping your WiFi and
  settings. It only shows up when the versions differ. This is also the
  way to take a display on 1.0.0 or 1.0.1 to a later version, once, when
  its web app says the update needs a one-time USB reinstall
  ([Updating](updating.md#when-an-update-needs-the-installer-once)).
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
    there under **Settings › Device › Network**. While a display has no
    WiFi to join, it restarts every ten minutes to try again; if the setup
    network vanishes, wait a moment and join it again. A display set to
    [play from the card only](#a-display-without-wifi) makes no setup
    network at all.

3. **Open the web app.** Rejoin your usual WiFi and open
   **<http://opendmd.local/>**, or the name you gave it. You manage the
   display from there from now on.

With your card in, the panel starts playing straight away.

## A display without WiFi

A display can also do without WiFi altogether: it plays its card, and never
starts WiFi, the setup network or the web app. You change what it plays by
changing the card on a computer, as with DMDoS.

- **On the install page:** tick **Play from the card only, no WiFi** under
  the Install button before you click it. This erases the display's saved
  settings (WiFi, name, brightness and the rest) whether or not you tick
  **Erase device**, and the installer asks for no WiFi at the end.
- **Or on the card:** copy the one-line file `opendmd.json` offered beside
  the box to the top folder of the card (or write it yourself: it holds
  `{"network":"off"}`). A card that already has an `opendmd.json` from an
  OpenDMD display: open it in a text editor and change `"network":"on"` to
  `"network":"off"`. The display reads it when it starts with the card in.

Leave the box unticked for anything else you do with the installer later:
only then does it recognise the display and offer **Update OpenDMD** and
**Connect to Wi-Fi**. **Update OpenDMD** keeps the display without WiFi.

### Turning WiFi back on

- **With the installer:** plug the display in, click **Install** with the
  box unticked, then **Connect to Wi-Fi**, and pick your network. The
  display joins it, turns WiFi on for good and restarts.
- **On the card:** open the card's `opendmd.json` on a computer and change
  `"off"` to `"on"` (`"network":"on"`). Deleting the line does not turn
  WiFi back on: the display keeps what it had. Check the file is still
  valid (the quotes, the braces, a comma between settings): the display
  replaces a file it can't read. With no WiFi saved, it then makes its
  [setup network](#wifi-without-the-installer).

The card has the last word. A fresh install with **Erase device** ticked
turns WiFi back on, but if the card still says `"network":"off"`, the
display goes back to playing without WiFi as soon as it starts with that
card. Change the card's file too.

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
