# Updating

OpenDMD has two parts that get updated: the **firmware**, which runs the
panel, and the **web app** you use at `opendmd.local`. There are two ways to
update. Your settings, WiFi details, playlists and GIFs stay as they are
either way.

## With the installer (firmware and web app)

This updates both parts together, so it's the one to use when in doubt.

1. Open the [installer](install.md) in Chrome or Edge and plug the
   display's board into your computer by USB.
2. Click **Install** and pick the board's port.
3. **Leave Erase device unticked.** Ticking it wipes your WiFi details and
   settings.

The board restarts with the new version and rejoins your WiFi.

## Over WiFi (firmware only)

You can update the firmware without unplugging anything:

1. Download the latest firmware update file (it ends in `.odfw`) from the
   [installer page](install.md#firmware-update-file).
2. In the web app, open **Settings → Advanced → Firmware** and choose
   **Update from file…**, then pick the file you downloaded.
3. Keep the SD card in: the update is stored on it while it installs. The
   panel goes dark for about a minute and then comes back on the new
   version.

If the file is older than what the player already runs, the app asks
before installing it.

This updates only the firmware. To update the web app as well, use the
installer.

## Which version do I have?

**Settings → Advanced → Firmware** shows the firmware version, and
**Settings → Advanced → About** shows the web app's.
