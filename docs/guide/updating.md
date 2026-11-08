# Updating

OpenDMD has two parts that get updated: the firmware, which runs the
panel, and the web app you use at `opendmd.local`. Your settings, WiFi
details, playlists and GIFs stay as they are whichever way you update.

## Over WiFi

1. In the web app, open **Settings › Device › Firmware** and press
   **Check for updates**.
2. If there's a new version, the app says what's new and shows an
   **Update** button. Press it.
3. Keep the page open and the SD card in. The panel says **Updating**, then
   **Installing** for about a minute (it goes dark for a moment when the
   board restarts), then **Updated to** the new version, and the web app
   reloads. If the update doesn't go through, your GIFs carry on as before.

The phone or computer you're using needs to be online, because the app
downloads the update from this site before sending it to the display. If
anything goes wrong it tells you what, and nothing is left half-installed.

## Beta updates

Before each release there are betas: early versions you can try sooner.
They may have problems. To get them, switch on **Get
beta updates** under **Settings › Device › Firmware**, and **Check for
updates** then offers the newest beta as well as the releases. The switch
is kept on the display, so every phone or computer you use sees the same
choice.

Switching it off doesn't take the display back to an older version. It
keeps the beta it has, and gets the next regular release once it is newer.

The [installer](install.md) offers the latest release, and the latest
beta too when there is one newer than it.

## With the installer

If the display won't start or you can't reach the web app, the
[installer](install.md) updates both parts over USB.

1. Open the installer in Chrome, Edge, Opera or Firefox 151+ and plug the
   display's board into your computer by USB.
2. Click **Install** and pick the board's port. The installer shows the
   version the board runs.
3. Click **Update OpenDMD**. Leave **Erase device** unticked for an
   update, because ticking it wipes your WiFi details and settings.

The panel stays dark while the installer writes. The board then restarts
with the new version, shows **Updated to** it on the panel, and rejoins
your WiFi.

## From a file

You can also download the firmware update file, which ends in `.odfw`,
from the [installer page](install.md#firmware-update-file) and use
**Update from file…** next to Check for updates. This updates only the
firmware. If the file is older than what the display runs, the app asks
first.

## Finding your version

**Settings › Device › Firmware** shows the firmware version and the board
it is for, and **Settings › Device › About** shows the web app's.
