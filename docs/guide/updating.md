# Updating

OpenDMD has two parts that get updated: the **firmware**, which runs the
panel, and the **web app** you use at `opendmd.local`. Your settings, WiFi
details, playlists and GIFs stay as they are whichever way you update.

## Over WiFi (the easy way)

1. In the web app, open **Settings → Device → Firmware** and press
   **Check for updates**.
2. If there's a new version, the app says what's new and shows an
   **Update** button. Press it.
3. Keep the page open and the SD card in. The panel goes dark for about a
   minute while the firmware installs, then the web app updates and the
   page reloads by itself.

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

## Once before 1.0: an update over USB

One update before OpenDMD 1.0 changes how the board's memory is laid out,
to give the firmware more room. That one can't go over WiFi: **Check for
updates** says so and points to the installer, and a firmware file for the
new layout is refused before anything is written. Use the
[installer](install.md) for it, as below: **Update OpenDMD** (or **Install**
with **Erase device** unticked) keeps your WiFi details and settings.
After that, updates go over WiFi again.

## With the installer

If the player won't start or you can't reach the web app, the
[installer](install.md) updates both parts over USB:

1. Open the installer in Chrome, Edge, Opera or Firefox and plug the
   display's board into your computer by USB.
2. Click **Install** and pick the board's port. The installer shows the
   version the board runs.
3. Click **Update OpenDMD**. (A board on an older version that the
   installer doesn't recognise gets **Install** and the erase question
   instead: **leave Erase device unticked.** Ticking it wipes your WiFi
   details and settings.)

The board restarts with the new version and rejoins your WiFi.

## From a file

You can also download the firmware update file (it ends in `.odfw`) from
the [installer page](install.md#firmware-update-file) and use **Update from
file…** next to Check for updates. This updates only the firmware; if the file is older than what the
player runs, the app asks first.

## Which version do I have?

**Settings → Device → Firmware** shows the firmware version and the
board it is for, and
**Settings → Device → About** shows the web app's.
