# Remote control

Remote control lets you use your display from a phone or computer that
isn't on your home WiFi: on mobile data, at work, anywhere. You can see
what's playing, skip, go back, pause, change the brightness, pick a
playlist or a Formula, and, on a board that supports it, send a GIF you
made in the [Studio](https://opendmd.dev/studio/) or
[Create Art](https://opendmd.dev/create/). (On the standard board, sending
GIFs is off for now: see [Send a GIF](#send-a-gif).) You don't need an
account.

Remote control is new, and it's in the beta versions only for now (see
[Beta updates](updating.md#beta-updates)). It is **off** until you switch it
on. While it's off, the display doesn't contact the Remote control relay at
all.

## How it works

When you switch Remote control on, the display connects out to
`connect.opendmd.dev`, a small relay run for OpenDMD, and keeps that one
connection open. Your phone connects to the same relay, which passes
messages between the two. Nothing on your home network is opened up, and
there's nothing to set up on your router.

The relay can't control your display by itself. Each phone or computer you
link gets its own key, kept in that browser, and the display only does what
a linked device asks.

## What you need

- The display on a beta version from **1.1.0-beta.3** on. To get it, switch
  on **Get beta updates** in **Settings › Device › Firmware**, then
  **Check for updates**.
- A recent browser on the phone or computer you'll use: Safari 17, Chrome
  137 or Firefox 129, or newer.
- To link a device, you need to be at home once, on the same WiFi as the
  display.

## Set an API key first

Anyone on your WiFi who opens the web app can link their own phone, and
keep control of the display after they leave. To stop that, set an API key
first, in **Settings › Device › Security** (**Require a key for the API**).
The Remote control card reminds you until you do. With a key set, only
someone who knows it can link a device.

## Switch it on

<!-- site:steps -->

1. At home, open the web app and go to **Settings › Device**.
2. Under **Remote control**, tick **Remote control**.
3. Wait a minute. The card says **On: ready for your devices** once the
   display is connected. Just after a restart, it waits a few minutes
   before it connects.

**Name your devices show** is the name your phone shows for this display.
Leave it empty to use the display's name.

## Link this browser

Use this to control the display from the browser you're in, for example
your phone's.

<!-- site:steps -->

1. In **Settings › Device › Remote control**, press **Control from this
   browser away from home**.
2. Press **Open Remote control**. The Remote control page opens on
   opendmd.dev.
3. Give this browser a name, such as "Sam's phone", and press **Link**.

The link works once, for 5 minutes. The display then shows **Remote control
added**, and the browser appears under **Linked devices**. Keep the Remote
control page as a bookmark, or add it to your home screen:
[opendmd.dev/connect](https://opendmd.dev/connect/).

## Link a phone with the QR code

Use this to link a phone that doesn't have the web app open.

<!-- site:steps -->

1. In **Settings › Device › Remote control**, press **Add a device**.
2. The display shows a QR code for 5 minutes. Point the phone's camera at
   it. If it doesn't scan, step back a little.
3. The Remote control page opens on the phone. Give the phone a name and
   press **Link**.

The card also shows the link itself, with **Copy link**, so you can send it
to the other device instead. Press **Cancel** to stop before it's used. At
night, when the display is dark, it shows no code: send the link.

## Add another device from one that's linked

When you're away from home, a device that's already linked can add another
one.

<!-- site:steps -->

1. On the linked device, open [opendmd.dev/connect](https://opendmd.dev/connect/)
   and press **Add another device**.
2. Open the link it shows on the new device, or scan its code with the new
   device's camera. The link works once, for 5 minutes.
3. Give the new device a name and press **Link**. Its screen shows a short
   code.
4. The first device asks whether to add the new one, and shows the code it
   expects. If it's the same as the new device's code, press **Yes, add
   it**. If not, press **No**.

A display can have up to 8 linked devices.

## Use it

Open [opendmd.dev/connect](https://opendmd.dev/connect/) on a linked device.
It shows each display it's linked to, with:

- **Now playing**, and what plays next;
- **Previous**, **Pause** and **Next**;
- **Brightness**;
- **Playlist**: any of your playlists, or **All GIFs**;
- **Show a Formula**: one of the built-in Formulas.

If the page says the display isn't connected, it may be off, or offline.
Remote control picks up again by itself when the display is back.

## Send a GIF

Only on a board that supports it: on the standard board, sending GIFs is
off for now (see below). On a board that supports it, in the [Studio](https://opendmd.dev/studio/) or
[Create Art](https://opendmd.dev/create/), on a linked device:

<!-- site:steps -->

1. Make your GIF, then press **Send to panel** in the Studio, or **Send to
   my panel** in Create Art.
2. Under **Send to**, pick your display.
3. Leave **Play it as soon as it arrives** ticked to see it straight away,
   and press **Send**.

The GIF goes into the **Sent** folder on the display's SD card. It isn't
added to a playlist, so add it to one from the web app at home if you want
to keep it in rotation. A GIF can be up to 512 KB. A display takes about
8 MB of sent GIFs a day, counting the prepared copy that goes with each one
(each send counts as at least 64 KB; the count starts again if the display
restarts), and no more than 60 sends a day. The display needs its SD card
in. If someone at
home is uploading to the display at that moment, the send waits for them.

**On the standard board, sending GIFs is off for now.** Every DMDoS
display has the original ESP32 board, and we're still checking that it has
the memory to save a sent GIF while it plays. Until a later version switches
it on, a display on that board doesn't appear under **Send to**, and
everything else in Remote control works. At home, send your GIF to the
display over your WiFi as before.

## Remove a device

To stop a phone or computer from controlling the display:

- **At home**: in **Settings › Device › Remote control**, find it under
  **Linked devices** and press **Remove**.
- **On the device itself**: on the Remote control page, press **Forget this
  panel in this browser**.

A lost phone can also be removed from another linked device: on its Remote
control page, under **Devices with Remote control**, press **Remove** beside
the lost one.

## Forget all devices

**Forget all devices**, at the bottom of the Remote control card, removes
every linked device at once and gives the display a new identity on
opendmd.dev. Use it if you've lost track of who has access, or if the card
says **opendmd.dev no longer knows this panel**. You then link your devices
again.

Switching Remote control off closes the connection, but keeps your linked
devices for when you switch it back on.

## Privacy

- **GIFs are relayed, never stored.** A GIF you send passes through the
  relay on its way and goes only to your display's SD card.
- **What the relay keeps**: your display's identity (a random code and a
  check of its password, never the password itself) and when it first
  registered; the public keys of the devices you linked (their names stay
  on the display, and the relay only passes them on); the name you gave the
  display; and, for today's limit, how much it received and a fingerprint
  of each GIF sent that day (replaced the next time you send on a later
  day).
- **What it sees on the way**, and doesn't keep: the addresses your display
  and your phone connect from (part of each address is used for a moment
  to limit how often anyone can connect), the display's firmware version,
  and the messages it passes, such as the title of what's playing, your
  playlists' names and your devices' names.
- **What it never gets**: your WiFi details, your API key, your RePlayOS
  code, your location, or any other file on your card.
- Nothing is logged and there are no analytics. The relay runs on
  Cloudflare.

## If something's wrong

- **The Remote control page says this browser no longer has its Remote
  control.** The browser cleared what opendmd.dev kept. Safari does this
  after 7 days without a visit, and so does clearing a site's data. Link
  the browser again, and remove the old entry under **Linked devices**.
- **The card says opendmd.dev isn't reachable right now.** The display
  keeps trying by itself, waiting longer between tries. Check that the
  display is online, for example that the web app's other pages work.
- **The card says Paused after two restarts in a row.** Untick **Remote
  control** and tick it again.
- **The card says the panel needs a firmware update.** Update it at home
  ([Updating](updating.md)).
