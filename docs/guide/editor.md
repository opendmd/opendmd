# The editor

You write and change a Formula in the editor. It is in your web app
under **Formulas**, in the [demo](https://opendmd.dev/demo/), and on
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
or **heavy**: light is an estimate of 5 ms a frame or less, medium 8 ms
or less. On a computer, hover over that line to see the estimate in
milliseconds a frame. The preview draws its frames at about the moments the panel would,
as far as your screen's own refresh allows.

The rating is for the way the panel usually draws the Formula. A heavy
Formula still plays. The panel starts it at a slower rate, such as every
4th refresh, and draws it more often once it can. The line under the
preview says which rate it starts at.

The panel can also draw a Formula a slower way, without its faster path,
and it refuses a Formula that would take more than about 40 ms a frame
that way, or 12 ms for one that draws over a GIF. (A `smooth` one past
32 ms is drawn every 2nd refresh instead, and the line under the preview
says so.) A refused Formula is rated **too heavy**, and a line under the
preview says so, even when it would usually be light. [What makes a
Formula heavy](psi.md#what-makes-a-formula-heavy) says how to make one
lighter.

Under the preview are the controls for time.

- **❚❚** holds the picture still, and **▶** plays on.
- **‹** and **›** step a frame back or on. With the preview selected, the
  `,` and `.` keys do the same.
- The slider drags through the loop, or through the first 10 seconds of a
  Formula that goes on without repeating. `t` is shown beside it.

If you edit while the preview is paused, it draws the new text at the
same moment of the loop.

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

- Drag a number left or right to try other values. On a computer,
  drag it in the text, or put the cursor on it and press Alt+↑ or Alt+↓.
  Hold Shift for steps ten times bigger. On a phone, tap a number and drag
  the bar that appears under the code, or use its **−** and **+**. Ctrl+Z
  (⌘Z on a Mac) undoes a whole drag at once.
- Each colour in the text, such as `#f80`, has a bar
  under it in that colour. Click or tap in one and a swatch appears at the
  end of its line. The swatch opens your browser's colour picker, and the
  colour you pick takes the old one's place. Ctrl+Z (⌘Z) puts the old
  colour back. With the keyboard, put the
  cursor in the colour and press Esc, then Tab.
- To paint a sprite, put the cursor in its `sprite` block and press **Edit
  the picture** at its first line. The picture opens as a grid. Pick a
  character from the key, where the checked one is see-through, then click
  or drag to paint. **+ Column**, **− Column**, **+ Row** and **− Row**
  change its size at the right and the bottom, and **+ Colour** adds a
  colour to the key. Each stroke goes into the text, and **Undo** takes it
  back.
- To make a sprite from a picture, press **Picture…** under the code,
  or drop a picture on the code, or paste one into it. PNG, JPEG, GIF,
  WebP, BMP and SVG work, up to 8 MB and 4,096 pixels a side. The picture
  stays in your browser: it is never uploaded. Drag on it to crop it (on
  a phone, with one finger), or move the crop with the arrow keys and
  change its size with Alt and the arrows. Pick a size up to 64 × 32,
  **As it is** keeps a small picture's own pixels, and **Fit** says
  whether it is cropped to fill, fitted inside or stretched. **Smooth**
  averages the pixels and **Sharp** keeps hard edges. The **Colours**
  buttons choose 2, 4 or 16 colours and say what each costs: a sprite
  takes some of the 1,024 numbers a Formula's tables and sprites share,
  and more colours take more. **Palette** can hold the colours to the
  panel's own or to one of the Formula's palettes; the sprite keeps the
  colours themselves, so changing the palette later won't change it.
  **Background** makes a background see-through, or turns the picture
  into one colour. The line by the buttons says what the Formula would
  use with it, or what stops it. **Add the sprite** puts it in after the
  Formula's first lines. With **Draw it in the middle** ticked, it also
  puts in a `draw { }` line that shows the sprite in the middle. Ctrl+Z (⌘Z) takes it out again. With the
  cursor in a sprite, or from **Replace from a picture…** in its picture
  editor, the picture replaces that sprite and its key keeps the
  characters it had for colours that stay. Use pictures you made or may
  share.
- To edit a palette, put the cursor in its `palette` line and press
  **Edit the palette**. A strip shows its colours as the panel reads them, from
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
- **Cheat sheet** lists the language's names, with a line about each.
  Tap a name to put it in.
- A line such as `use Plasma as p` puts the Formula called Plasma in as a
  function, `p`, once the cursor leaves the line. It works for your
  Formulas and the bundled ones that have a single colour line and no
  layers, drawing, tables, sprites or palettes. See [How a Formula is
  run](psi.md#functions).

## Settings and controls

**⚙ Settings** opens the Formula's name, description, controls and look
in one sheet. Each change there is a line at the top of the text.

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
- **Save as GIF…** makes a GIF of it to add to your card ([Saving as a GIF](formulas.md#saving-as-a-gif)).
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
