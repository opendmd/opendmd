# The SD card

Your GIFs live on the microSD card in the display. OpenDMD plays a card
made for DMDoS exactly as it is: no conversion, nothing to rename. The card
also keeps working with DMDoS.

OpenDMD doesn't come with any GIFs. A card you set up for DMDoS works as
it is, and the DMDoS GIF pack (**DMDos GUI v1 + GIFs**) is in the Software
section of [dmdos.net](https://dmdos.net/). To set up a card, format it
FAT32 and copy your DMDoS card, or the DMDoS GIF pack, onto it as it is.
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

- **`gifs/`** holds the animations, one folder per **category**. The
  Library shows the same folders. Make a new folder and you have a new
  category.
- **`filelist.txt`** is the list the panel plays: **All files** in the web
  app. It doesn't have to list every GIF on the card. When you add GIFs
  from the web app, it offers to add them to the playlist that's playing.
- **`filelist.hall`** and **`filelist.xmas`** are the Halloween and
  Christmas playlists, if your card has them.

Playlists you make in the web app are saved on the card too, so they move
with it.

## How names become details

The web app reads each GIF's platform, title and author from its file name:

| File name | Platform | Title | Author |
| --- | --- | --- | --- |
| `SNES_Super_Metroid_01.gif` | SNES | Super Metroid | |
| `MD_Streets_of_Rage_2_03_ross.gif` | MD | Streets of Rage 2 | ross |
| `ARCADE_GoldenAxe16.gif` | ARCADE | GoldenAxe | |
| `Sunset.gif` | none | Sunset | |

The pattern is `PLATFORM_Title_Words_01.gif`: the platform first, then the
title with underscores between words, then a number so several GIFs of the
same game can sit side by side. An author name, in lower case, can go at
the end.

The first word only counts as the platform if it's one the player knows:
the usual short names such as `SNES`, `NES`, `MD`, `N64`, `GBA`, `PS1`,
`ARCADE`, `PINBALL`, `AMIGA`, `C64`, `SPECTRUM` and `PICO-8`, and the rest
of the list in the Platform menu of **Add GIFs**. Any other first word
isn't a platform: the GIF has no platform, and its whole file name becomes
the title, so `THE_END.gif` is titled "THE END".

If a platform you need is missing, it can't be added from the web app yet:
the list is part of the player's software, and new platforms arrive with
updates.

You rarely need to type these yourself. **+ Add GIFs**, **Edit details…**
and Create Art ask for a title, platform, category and author and build the
name for you. If you copy GIFs onto the card from a computer, name them this
way and they'll sort into the right platform.

## Using a new card

The player reads cards formatted as **FAT32**. Most cards up to 32 GB come
that way. Bigger cards usually come formatted as exFAT, which the player
can't read: put one in and the web app's **Now** view says it needs FAT32
and offers **Format card…**, which sets it up for you. Formatting erases
the card, so do it before adding GIFs.

On a computer, a tool such as [Rufus](https://rufus.ie) (Windows) can
format a card as FAT32 too, as dmdos.net suggests for its pack.

To move a DMDoS collection to a new card, copy the `gifs` folder and the
`filelist` files from the old card to the new one with a computer.

You don't have to do anything after filling a card: it plays as it is,
and the player gets each GIF ready by itself so it starts a little faster.
It does that while the panel is dark at night or the player is paused,
and only a little while it plays. The whole DMDoS pack is ready after
about four hours of the panel dark or paused: turn on **Night mode**
(off, not dimmed) under **Schedule**, or pause the player for a while.
**Settings** shows how far it has got, and **Prepare remaining files**
there does the rest from your browser.

## If a card starts failing

Cards wear out. If the player keeps hitting read errors it slows the card
down to keep going, and **Settings** says "Card running at reduced speed
after errors", or shows a count of card errors. Either message means it's
time for a new card.
