# The editor

The editor is where a Formula is written and changed. It is in your web
app under **Formulas**, in the [demo](https://opendmd.dev/demo/), and on
this site behind **Open** in the [formula gallery](https://opendmd.dev/formulas/)
and **Open in editor** under an example. The preview plays as you type.

To get there in your web app, press **+ New formula**, or pick a Formula
and press **Remix** for a bundled one or **Edit** for one of yours.

## The preview

The preview is at the top, and on a phone it stays in view while you
scroll. The line under it says how long the loop is, or that the Formula
follows the clock, how often the panel draws it (every refresh, every 2nd
or every 4th, with about how many a second: your panel's own figure when
one is connected), and how hard it is for the panel: **light**, **medium**
or **heavy**. Hover over that line to see the estimate in milliseconds a
frame. The preview draws its frames at about the moments the panel would,
as far as your screen's own refresh allows.

The rating counts how fast the panel usually draws the Formula. The panel
also needs to be able to draw it the slow way, when it can't use its
faster path, and refuses a Formula that would take more than about 40 ms a
frame that way (32 ms for one drawn every refresh). Such a Formula is rated
**too heavy**, and a line under the preview says so, even when it would
usually be light.

The row under the preview holds the time.

- **❚❚** holds the picture still, and **▶** plays on.
- **‹** and **›** step a frame back or on. With the preview selected, the
  `,` and `.` keys do the same.
- The slider drags through the loop, or through the first 10 seconds of a
  Formula that goes on without repeating. `t` is shown beside it.

Edit while the picture is held and the new text is drawn at the same
moment.

A Formula that draws over a GIF is shown over a sample picture. **Pick a
GIF…** draws it over one of the GIFs in your playlist or a GIF file from
your computer, moving as the GIF does.

## When something is wrong

If the text has a mistake, the editor says what is wrong just above the
code, marks the place and keeps playing the last version that worked,
dimmed.

- **Line N** at the start of the message takes you to the line.
- **Fix it** appears when the message suggests a fix, such as "did you
  mean `t`", and writes it in.
- The code at the end, such as **E201**, opens the language reference's
  entry for that error.

## Changing numbers and colours

- **Drag a number** left or right to try other values. On a computer,
  drag it in the text, or put the cursor on it and press Alt+↑ or Alt+↓.
  Hold Shift for steps ten times bigger. On a phone, tap a number and drag
  the bar that appears under the code, or use its **−** and **+**. Ctrl+Z
  (⌘Z on a Mac) undoes a whole drag at once.
- **Pick a colour.** Each colour in the text, such as `#f80`, has a bar
  under it in that colour. Click or tap in one and a swatch appears at the
  end of its line. The swatch opens your browser's colour picker, and the
  colour you pick takes the old one's place. Ctrl+Z (⌘Z) puts the old
  colour back. With the keyboard, put the
  cursor in the colour and press Esc, then Tab.
- **Paint a sprite.** With the cursor in a `sprite` block, press **Edit
  the picture** at its first line. The picture opens as a grid. Pick a
  character from the key, where the checked one is see-through, then click
  or drag to paint. **+ Column**, **− Column**, **+ Row** and **− Row**
  change its size at the right and the bottom, and **+ Colour** adds a
  colour to the key. Each stroke goes into the text, and **Undo** takes it
  back.
- **Edit a palette.** With the cursor in a `palette` line, press **Edit
  the palette**. A strip shows its colours as the panel reads them, from
  left to right. Tap a colour to change it, drag it to move it, give it a
  name, take it away with **×**, or add one with **+ Colour**.

## Typing

- After two letters, a list shows the names that fit. ↑ and ↓ choose,
  Tab or Enter puts one in, and Esc closes the
  list. On a phone the names are buttons under the code.
- Inside a function's brackets, a line says what it takes, with the value
  you are typing in bold.
- Enter keeps the line's indent. After `{` it starts an indented line and
  puts the `}` under it.
- Tab and Shift+Tab indent and unindent. To leave the code with the
  keyboard, press Esc, then Tab.
- **Cheat sheet** lists every name in the language with a line about
  each. Tap a name to put it in.
- A line such as `use Plasma as p` puts the Formula called Plasma in as a
  function, `p`, once the cursor leaves the line. It works for your
  Formulas and the bundled ones that have a single colour line and no
  layers, drawing, tables, sprites or palettes. See [How Psi
  thinks](psi.md#functions).

## Settings and controls

**⚙ Settings** opens one sheet for what the Formula says about itself.
Each change there is a line at the top of the text.

- **Name** and **About** are its `name` and `about` lines.
- **Controls** are its sliders and buttons, three at most. **+ Add a
  control** adds one, and each can be a **Slider** with a range, a default
  and a label, or a **Choice** of two to six words.
- **Can be the panel's look** lets it be drawn over every GIF, with a
  **Blend** and an **Amount**.

A control line looks like `control speed 0..4 = 1 "Speed"`. `speed` is
the value the Formula reads, from 0 to 4, starting at 1, labelled "Speed".
A choice looks like `control shape = cube, ring`, and `shape` is 0 for
the first word, 1 for the next and so on. A Formula that reads `a`, `b` or
`c` without a control line gets a plain slider from 0 to 1 for each.

## Try it out

A Formula that reads the world around the panel, such as `daylight`,
`night` or `replay.game`, gets **Try it out** under the preview. It
changes only the preview.

- **Time travel** moves the sun, the moon and the clock to another
  moment, with a time of day and a date. **Play** runs a whole day in about
  30 seconds. It uses your panel's location from Settings › Schedule, and
  London when none is set. **Use this browser's location** is the other
  choice.
- **Start a game**, **Back to the menu** and **Night on** make the changes
  the panel would see, and there is a slider for each value it reads.
- **Back to the defaults** undoes it all.

## Saving and showing

In your web app, the buttons under the code are these.

- **Save** (Ctrl+S) keeps it under its name, on your panel when it has a
  card and in this browser otherwise.
- **Show on panel** (Ctrl+Enter) plays it on the panel until the playlist
  moves on. It is there when the web app is your display's own.
- **Add to playlist…** puts it in one of your playlists.
- **Set as the panel's look** keeps it over every GIF.
- **Save as GIF…** makes a GIF of it in [Create Art](create-art.md).
- **Copy code** puts the Formula's text on the clipboard.
- **Submit to the gallery…** shares it. See [Formulas](formulas.md#sharing).

On this site the editor has **Open on my panel** instead of Save, and
**Copy code** and **Submit to the gallery…** as well. It saves
the Formula on your panel under **Your formulas** and opens it in your web
app. Your changes on the site are kept in this browser, and **Continue
where you left off** puts them back when you come back to the Formula. In
the demo and on this site, **What costs most** marks the lines that take
the panel the most work, and **Tidy up** lays the code out the usual way.

**Earlier versions…** lists the last ten versions of this Formula kept in
this browser, from each save and from when it arrived. Tap one to see what
going back would change, then **Go back to this**. Ctrl+Z takes it away
again.

**‹ Formulas** goes back to your Formulas. If you leave without saving,
the Formulas view offers **Continue editing** at the top.
