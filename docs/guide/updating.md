# Updating

OpenDMD has two parts that get updated: the **firmware**, which runs the
panel, and the **web app** you use at `opendmd.local`. Your settings, WiFi
details, playlists and GIFs stay as they are whichever way you update.

## Over WiFi (the easy way)

1. In the web app, open **Settings → Device → Firmware** and press
   **Check for updates**.
2. If there's a new version, the app says what's new and shows an
   **Update** button. Press it.
3. Keep the page open and the SD card in. While the update comes in the
   panel says **Updating** and the version, with **Don't unplug**
   underneath; then **Installing**, which stays on the panel, a dot
   stepping across under it, for about a minute while the firmware
   installs (it goes dark for a moment each time the board restarts).
   When the player starts again the panel
   says **Updated to** the new version for a few seconds before your GIFs
   carry on, then the web app updates and the page reloads by itself. If
   the update doesn't go through, the message goes and your GIFs carry on
   as before.

The phone or computer you're using needs to be online: the app downloads
the update from this site before handing it to the player. If anything
goes wrong it tells you what, and nothing is left half-installed.

## Beta updates

Before each release there are **betas**: early versions of it, so you can
try what's new sooner. They may have problems. To get them, switch on
**Get beta updates** under **Settings → Device → Firmware**; **Check for
updates** then offers the newest beta as well as the releases. The switch
is kept on the player, so every phone or computer you use sees the same
choice.

You can switch it off at any time. That doesn't take the player back to
an older version: it keeps the beta it has, and gets the next regular
release once it is newer.

The [installer](install.md) offers the latest release, and the latest
beta too when there is one newer than it.

## Coming from a beta before 1.0.0-beta.16

Betas before 1.0.0-beta.16 used an older layout of the board's memory. A
player on one of them can't take 1.0 over WiFi: **Check for updates** says
so and points to the installer, and a firmware file for the new layout is
refused before anything is written. Use the [installer](install.md) once
(**Update OpenDMD** keeps your WiFi and settings); after that updates go
over WiFi again.

## With the installer

If the player won't start or you can't reach the web app, the
[installer](install.md) updates both parts over USB:

1. Open the installer in Chrome, Edge, Opera or Firefox and plug the
   display's board into your computer by USB.
2. Click **Install** and pick the board's port. The installer shows the
   version the board runs.
3. Click **Update OpenDMD**. **Erase device** stays unticked for an
   update: ticking it wipes your WiFi details and settings.

The panel stays dark while the installer writes. The board restarts with
the new version, says **Updated to** it on the panel, and rejoins your
WiFi.

## From a file

You can also download the firmware update file (it ends in `.odfw`) from
the [installer page](install.md#firmware-update-file) and use **Update from
file…** next to Check for updates. This updates only the firmware; if the file is older than what the
player runs, the app asks first.

## Which version do I have?

**Settings → Device → Firmware** shows the firmware version and the
board it is for, and
**Settings → Device → About** shows the web app's.
