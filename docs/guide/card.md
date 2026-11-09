# The SD card

Your GIFs live on the microSD card in the display. A card set up for DMDoS
plays on OpenDMD without any change: no conversion, nothing to rename. It
also keeps working with DMDoS.

OpenDMD comes with no GIFs. The DMDoS GIF pack, DMDos GUI v1 + GIFs, is in
the Software section of [dmdos.net](https://dmdos.net/). To set up a card,
format it FAT32 and copy your DMDoS card, or the pack, onto it as it is.
You can also add your own GIFs from the [Library](web-app.md#library), or
make some with [Create Art](create-art.md).

## What's on a card

```
gifs/
  Consoles/
    SNES_Super_Metroid_01.gif
    MD_Streets_of_Rage_2_03.gif
  Arcade/
  Pinball_Short/
  ...
filelist.txt
filelist.hall
filelist.xmas
```

- `gifs/` holds the animations, one folder per category. The
  Library shows the same folders. Make a new folder and you have a new
  category.
- `filelist.txt` is the list the panel plays: **All files** in the web
  app. It doesn't have to list every GIF on the card. When you add GIFs
  from the web app, it offers to add them to the playlist that's playing.
- `filelist.hall` and `filelist.xmas` are the Halloween and
  Christmas playlists, if your card has them.

Playlists you make in the web app are saved on the card too, so they move
with it.

## The settings file

OpenDMD keeps the display's settings in `opendmd.json` at the top of the
card, so they move with it. You can edit it on a computer. Only the
settings you write in it change: a line you leave out leaves that setting
as the display has it. Keep the file valid JSON (mind the commas): a file
the display can't read is replaced with its own settings.

To play from the card only, with no WiFi, the file needs just this line:

```json
{"network": "off"}
```

The display then starts no WiFi and no web app; it just plays the card.
To turn WiFi back on, change `"off"` to `"on"`. Deleting the line doesn't
turn it back on. See [Turning WiFi back on](install.md#turning-wifi-back-on)
for the other way, over USB.

With WiFi off the display doesn't know the time, so the schedule and night
mode do nothing, and Formulas that show the time are skipped.

## Formulas on the card

Your Formulas are in `formulas/`, each as a `.psi` file. You can copy one
there from a computer and name it in a playlist (`/formulas/<name>.psi`).
The display prepares it for the panel shortly after it starts with the
card (it plays from then on), and again after an update that needs it. A Formula with a mistake in
it is skipped where it plays.

## How names become details

The web app reads each GIF's platform, title and author from its file name:

| File name | Platform | Title | Author |
| --- | --- | --- | --- |
| `SNES_Super_Metroid_01.gif` | SNES | Super Metroid | |
| `MD_Streets_of_Rage_2_03_alex.gif` | MD | Streets of Rage 2 | alex |
| `ARCADE_GoldenAxe16.gif` | ARCADE | GoldenAxe | |
| `Sunset.gif` | none | Sunset | |

The pattern is `PLATFORM_Title_Words_01.gif`: the platform first, then the
title with underscores between words, then a number so several GIFs of the
same game can sit side by side. An author name, in lower case, can go at
the end.

The first word only counts as the platform if it's one OpenDMD knows:
the usual short names such as `SNES`, `NES`, `MD`, `N64`, `GBA`, `PS1`,
`ARCADE`, `PINBALL`, `AMIGA`, `C64`, `SPECTRUM` and `PICO-8`, and the rest
of the list in the Platform menu of **Add GIFs**. Any other first word
isn't a platform, so the GIF has none and its whole file name becomes the
title: `THE_END.gif` is titled "THE END".

If a platform you need is missing, it can't be added from the web app yet:
the list is part of OpenDMD, and new platforms arrive with
updates.

You rarely need to type these yourself. **+ Add GIFs**, **Edit details…**
and Create Art ask for a title, platform, category and author and build the
name for you. If you copy GIFs onto the card from a computer, name them this
way and they'll sort into the right platform.

## Using a new card

The display reads cards formatted as FAT32. Most cards up to 32 GB come
that way. Bigger cards usually come formatted as exFAT, which the display
can't read: put one in and the web app's **Now** view says it needs FAT32
and offers **Format card…**, which sets it up for you. Formatting erases
the card, so do it before adding GIFs.

On a computer, a tool such as [Rufus](https://rufus.ie) (Windows) can
format a card as FAT32 too, as dmdos.net suggests for its pack.

To move a DMDoS collection to a new card, copy the `gifs` folder and the
`filelist` files from the old card to the new one with a computer. If
OpenDMD has used the old card, copy its `sets` and `formulas` folders too,
for your playlists and Formulas.

OpenDMD gets each GIF ready so it starts a little faster, while the panel
is dark at night or playback is paused. The whole DMDoS pack is ready
after about four hours of that. Turn on **Night mode** under Settings ›
Schedule, set to turn the panel off rather than dim it, and it happens
overnight; or pause playback for a while. **Ready to play** under Settings
› Device shows how far it has got, and **Prepare remaining files** does the
rest from your browser.

## If a card starts failing

See [Settings shows card errors](troubleshooting.md#settings-shows-card-errors).
