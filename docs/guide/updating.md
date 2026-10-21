# Updating

OpenDMD has two parts that get updated: the **firmware**, which runs the
panel, and the **web app** you use at `opendmd.local`. Your settings, WiFi
details, playlists and GIFs stay as they are whichever way you update.

## Over WiFi (the easy way)

1. In the web app, open **Settings → Advanced → Firmware** and press
   **Check for updates**.
2. If there's a new version, the app says what's new and shows an
   **Update** button. Press it.
3. Keep the page open and the SD card in. The panel goes dark for about a
   minute while the firmware installs, then the web app updates and the
   page reloads by itself.

The phone or computer you're using needs to be online: the app downloads
the update from this site before handing it to the player. If anything
goes wrong it tells you what, and nothing is left half-installed.

## With the installer

If the player won't start or you can't reach the web app, the
[installer](install.md) updates both parts over USB:

1. Open the installer in Chrome or Edge and plug the display's board into
   your computer by USB.
2. Click **Install** and pick the board's port.
3. **Leave Erase device unticked.** Ticking it wipes your WiFi details and
   settings.

The board restarts with the new version and rejoins your WiFi.

## From a file

You can also download the firmware update file (it ends in `.odfw`) from
the [installer page](install.md#firmware-update-file) and use **Update from
file…** next to Check for updates. This updates only the firmware; if the
file is older than what the player runs, the app asks first.

## Which version do I have?

**Settings → Advanced → Firmware** shows the firmware version, and
**Settings → Advanced → About** shows the web app's.
