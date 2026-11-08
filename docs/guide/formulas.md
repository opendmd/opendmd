# Formulas

![Formulas: the bundled ones, and one playing in its details](../images/formulas.png)

A Formula is a few lines of maths that draw on the panel, every pixel at
once. Change a number and the picture changes as you type. Formulas are
written in **Psi** (Ψ), OpenDMD's small language for the panel's pixels.
You don't need to know it to start. Open one, drag a number and watch.

Open **Formulas** (Ψ) in the web app's menu. Formulas work in the
[online demo](https://opendmd.dev/demo/) too, with no display at all.

To write your own, start with [Make a Formula](psi-1.md). [How Psi
thinks](psi.md) explains the ideas, and [The editor](editor.md) has every
button and key.

## The Formulas view

Formulas opens on the **Bundled** ones, the ten that come with OpenDMD,
and the Formulas you've saved. Pick one to see it playing, with its
sliders if it has any. The bundled ten are these.

- **Big clock**, the real time in big rainbow digits, with a bar along
  the bottom that fills once a minute.
- Four screensavers, **Matrix**, **Starfield**, **Endless road** and
  **Time gate**. Endless road is a synthwave drive through a day and a
  night, and Time gate a swirling portal.
- Five looks, which draw over whatever GIF plays: **Scanlines**, **Old
  TV**, **Vignette**, **Old photo** and **Glitter**. They are marked
  *look*. See [Over a GIF](#over-a-gif).

**More Formulas** opens the online [formula gallery](#the-formula-gallery).
It has every Formula OpenDMD has made, among them the demo-scene classics
such as Tunnel, Rotozoom and Plasma, and the ones people have shared.
Press **Open on my panel** on one to open it in your web app's editor (see
[Open on my panel](#open-on-my-panel)).

From a Formula's details:

- **Remix** (a bundled one) or **Edit** (one of yours) opens it in the
  editor.
- **Save as GIF…** makes a GIF of it in [Create Art](create-art.md).
- **Settings…** opens it in the editor with its settings: its name, its
  controls and whether it can be the panel's look.
- **Delete…** takes away one of yours.
- **+ New formula** starts a fresh one.

If you leave the editor without saving, the Formulas view offers
**Continue editing** at the top, and the web app asks before anything
replaces it.

## On the panel

**Show on panel** plays a Formula on your display until the playlist moves
on, for one turn like any GIF. To watch one for longer, set **Keep on the
panel** beside it, or in Settings › Playback, to **1 minute**, **5
minutes** or **Until I change it**. Now then says it is being kept, and
**Back to the playlist** ends it. So do Skip, Previous, another Formula
or another playlist.

**Add to playlist…** puts a Formula in one of your playlists. There the
display draws it itself. It follows the clock, loops for as long as it
plays, and takes one line of the playlist instead of the space of a GIF.
The playlist shows it by its name, marked *Formula*, and so does Now while
it plays.

A bundled Formula goes into a playlist by its name. One of yours is saved
first, and the playlist plays it with the sliders as they were when you
added it. Save a change and the playlist plays the change.

To have Tunnel or another Formula on the Follow RePlayOS menus, make a
playlist of it and choose that as the attract playlist.

**Your formulas** are kept on the display when it has a card, so every
browser that opens the web app sees them. Formulas you had saved in this
browser before go onto the display the first time you open Formulas
there. In the online demo, or on a display without a card, they are kept
in the browser. The demo's Now preview skips Formulas in a playlist,
because only a display draws them there.

## Over a GIF

A Formula can draw over a GIF instead of replacing it, with scanlines, a
colour grade or a clock in the corner. The bundled looks are made this
way, and the gallery has more. [How Psi thinks](psi.md#over-a-gif) shows
how to write one.

In the editor and in a Formula's details, such a Formula is drawn over a
sample picture. **Pick a GIF…** draws it over one of the GIFs in your
playlist, or a GIF file from your computer, moving as the GIF does.
**Save as GIF…** then saves the Formula over that GIF. Played on its own,
it has black under it.

On your display:

- **Show over the panel's GIF** draws it over whatever the panel is
  playing, until the playlist moves on. Changes you make follow it there.
  Press it again to take it off.
- **Set as the panel's look** keeps it over every GIF the panel plays,
  under the colour and the phosphor glow. Settings › Look shows which
  Formula it is under **Formula over every GIF**, changes it, or turns it
  off with **None**.

Any Formula can be the look once its settings say so. In the editor,
**⚙ Settings** (or **Settings…** in a Formula's details) has **Can be the
panel's look**. Turned on, the look has a **Blend** and an **Amount**.

- The blend says how the Formula goes onto the GIF. **Over** covers it,
  **Screen** lightens it, so the GIF's bright parts stay and the Formula
  shows in its dark ones. **Add** lightens more, **Multiply** darkens and
  tints, and **Lighten** and **Darken** keep the brighter or the darker of
  the two.
- The amount is how much of the Formula shows, from 0 to 100 %.

A Formula that draws over a GIF starts at Over, 100 %. One that covers
the whole panel, like Plasma, starts at Screen, 40 %, or Over, 30 % when
that is lighter for the panel. The preview shows the choice as you move
it. A Formula with layers goes Over only, because its layers keep their
own blends.

Settings › Look lists your Formulas and the bundled ones that can be a
look, and **Change…** opens the chosen one's settings. Over at 100 % of a
Formula that covers the whole panel would hide the GIF, and the sheet
says so. A Formula that would take more than about 12 ms a frame over
every GIF is listed as too heavy. The sheet also says how fast the look
moves, about 50 times a second, or 25 for a heavy one that takes more than
8 ms a frame. The GIFs play at their own speed either way.

The display keeps the whole Formula, so editing or deleting it in the web
app does not change the look. When the display is short of memory, such as
during several uploads at once, it shows the GIFs without the look for a
while.

## Open on my panel

**Open on my panel** is on every tile in the [formula
gallery](#the-formula-gallery) and on every Formula's page there. Press it
and the Formula is saved on your display, under **Your formulas**, and
opens in your web app's editor. From there you can change it, show it on
the panel, add it to a playlist or use it as the look.

A bundled Formula is on the display already, so nothing new is saved. The
same Formula opened twice is saved once. One that doesn't work isn't
saved, and the web app says why and keeps it in the editor.

The first time, the site asks for your display's address. It suggests
`opendmd.local`, and the name or IP address is also in the web app under
Settings › Device. After that one press is enough. Your phone or computer
must be on the same network as the display. Your browser may ask you to
allow pop-ups for opendmd.dev the first time.

## Sharing

To share a Formula, submit it to the [gallery](#the-formula-gallery).
**Submit to the gallery…** is in the editor, in your web app, in the demo
and on every Formula's page on this site. A link never carries a
Formula's text, so no one can put code in front of you by sending you a
link.

## The formula gallery

The [formula gallery](https://opendmd.dev/formulas/) on this site has
every Formula OpenDMD has made, and the ones people have shared. The ones
marked *on the panel* are the bundled ten. Each tile shows the Formula's
name, its line about it, its controls and whether it can be a look.

**Open on my panel** opens it in your web app's editor. **Open** opens it
in the editor on this site, live, with its sliders, its errors and the
cheat sheet. Your changes there are kept in this browser. Come back to
the Formula and **Continue where you left off** puts them back, and
**Earlier versions…** has it as it came and as it went to your panel.

To add yours, press **Submit to the gallery…** in the editor. Give it a
name and a line about it, which become its `name` and `about` lines. Add
your name or GitHub handle and a licence, either CC0 for anyone to use or
CC BY 4.0 with credit, then press **Open on GitHub**. GitHub opens with the
file ready and makes the pull request for you. You need a free GitHub
account.

A check says if anything needs fixing, a maintainer looks at it, and once
it is merged it appears in the gallery within a few minutes. The gallery
takes a Formula that the editor estimates at 10 ms a frame or less, that
is at most 4,500 bytes once compiled for the panel, in a file of up to 36
KB. Comments cost nothing. The details are in the gallery's
[README](https://github.com/opendmd/opendmd.github.io/tree/main/gallery).

## Saving as a GIF

**Save as GIF…** draws one loop of the Formula and opens it in [Create
Art](create-art.md), titled with its name. It saves at 50 frames a second,
or 25 for a loop longer than about 5 seconds or a Formula with `fps 25`,
and at most 240 frames. GIFs can't play 100 frames a second, so `fps 100`
saves at 50.

In Create Art you can add text, trim it or change its speed, then save it
to your card. It plays on the panel like any other GIF and goes in
playlists. A Formula with a long loop, or one that follows the clock,
keeps only those few seconds, as they look when you save. One that goes
on without repeating keeps its first 10 seconds.
