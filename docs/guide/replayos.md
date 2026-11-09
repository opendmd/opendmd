# Follow RePlayOS

If you play games on a [RePlayOS](https://replayos.com/) box, the
display can follow it. When you start a game the panel shows that game,
and while you browse the menus it plays a playlist of your choice. The
display asks the box what it's doing every couple of seconds over your
WiFi. Nothing needs installing on the box.

## Set it up

<!-- site:steps -->

1. **Turn on NET CONTROL.** On the RePlayOS box, open **System** and turn
   on **NET CONTROL**.
2. **Find the address and code.** Still on the box, open **System ›
   Information**. It shows the box's address, such as `192.168.1.20`, and
   its **NET CONTROL code**.
3. **Enter them.** In the web app, open **Settings › RePlayOS**. Type the
   address under **RePlayOS address** and the code under **NET CONTROL
   code**. A name works too, such as `replay.local` if your network has one
   for the box. Add `:port` only if you've moved RePlayOS off its usual
   port.
4. **Test.** Press **Test**. It says which RePlayOS version answered and
   what the box is doing, or why it couldn't reach it, such as a wrong
   code.
5. **Pick the attract playlist.** Choose the **Attract playlist**, which
   plays while the box is in its menus: any playlist, or **The current
   playlist**. Then turn on **Follow RePlayOS**.

The code is kept on the display and never shown again. If you change the
address, type the code again.

## What the panel does

- **When a game starts**, the panel plays a short power-on effect, then
  the console's logo (one from your card if the playlist has one, else a
  card with its name), then a title card with the game's name.
- **While you play**, it shows GIFs of that console, with GIFs of that
  game coming up far more often than the rest.
- **In the menus**, it plays the attract playlist. After a game it waits
  a little before going back to it, in case you start another.
- **When the box is off or can't be reached**, the panel plays as usual,
  and picks up again once the box answers.

Night mode still comes first: while it has the panel off, nothing starts,
and a game that's running shows once the panel comes back.

Now shows what the display is following under the title, such as
"Following RePlayOS · Mega Drive · Sonic the Hedgehog 3". Settings ›
RePlayOS shows the same.

## Tips

- For a clock in the menus, add the bundled **Big clock** Formula to a
  playlist and choose that one as the attract playlist. It follows the
  time zone in **Settings › Schedule**.
- The GIFs come from the playlist that's playing, so a game shows only if
  that playlist has GIFs of its console. Name them as described in
  [The SD card](card.md) and they're found by their platform and title.
