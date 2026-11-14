// The desk scene's words, shared by both scenes (desk/flat.js, desk/hd/scene.js) and the page
// (desk/desk.js), so the two cannot drift.

// The parts on the display's back: id, name, what it is, its section of the build guide.
export const PARTS = [
  ['panels', 'LED panel', 'one of two P4 64×32 panels: driver chips under a plastic frame', '4-two-p4-64x32-led-panels'],
  ['carrier', 'Carrier board', 'joins the DevKit, the card and the panels', '2-dmdos-board-v3'],
  ['leads', 'Power leads', '5 V from the carrier’s terminal to each panel', '5-a-usb-c-cable-and-power'],
  ['ribbon', 'Ribbon cable', 'carries the picture on to the second panel', '4-two-p4-64x32-led-panels'],
  ['devkit', 'ESP32 DevKit', 'runs OpenDMD, drives the panels, serves the web app', '1-esp32-devkit-v1-usb-c-or-micro-usb'],
  ['sd', 'microSD card', 'your GIFs and playlists', '3-a-microsd-card'],
  ['usb', 'USB-C lead', 'power, from any phone charger', '5-a-usb-c-cable-and-power'],
];
export const GUIDE = '../guide/hardware.html#';
// The part's label: its name, then what it is (or where the card went).
// (The card's is a button: it pulls the card, or puts it back.)
export const tipHtml = (p, cardOut) => `${p[1]}<small>${p[0] != 'sd' ? p[2] + ' · Build guide ›' : cardOut ? 'out of its socket · click to put it back' : p[2] + ' · click to pull it out'}</small>`;
export const partLabel = (p, cardOut) => p[0] != 'sd' ? `${p[1]}: ${p[2]}. In the build guide` : cardOut ? 'microSD card: out of its socket. Put it back' : 'microSD card: your GIFs and playlists. Pull it out';
// The status line under the scene (the page's polite live region).
export function says(W, back) {
  return !W ? 'The app is starting.'
    : !W.power ? 'Unplugged: the panel is dark and the app has lost it. Plug it in to watch it start.'
    : !W.up ? 'Starting up: the name, then the card. The app finds it again in a moment (a real display takes up to a minute).'
    : !W.card ? 'No card: the panel says so, and so does the app.'
    : back ? 'The back: the DevKit on its carrier board, the card, a ribbon between the panels, power leads. Point at a part, or Tab to one, for its name.'
    : 'Playing from its card. Skip, dim or change it from the app.';
}
