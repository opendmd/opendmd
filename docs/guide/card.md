# The SD card

Your GIFs live on the microSD card in the display. OpenDMD plays a card
made for DMDoS exactly as it is: no conversion, nothing to rename. The card
also keeps working with DMDoS.

OpenDMD doesn't come with any GIFs. Use the card your display came with,
add your own from the [Library](web-app.md#library), or make some with
[Create Art](create-art.md).

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
  folders are the category tabs in the Library. Make a new folder and you
  have a new category.
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

To move a DMDoS collection to a new card, copy the `gifs` folder and the
`filelist` files from the old card to the new one with a computer.

Take your time when you first fill a card: the player makes a lighter copy
of each GIF in the background, mostly while the panel is off at night, so
a big collection plays at its smoothest after a night or two.

## If a card starts failing

Cards wear out. If the player keeps hitting read errors it slows the card
down to keep going, and **Settings** says "Card running at reduced speed
after errors", or shows a count of card errors. Either message means it's
time for a new card.
