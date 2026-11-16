// Settings › Device's Remote control card (a build with it: status
// `features` has "connect"; imported by settings.js, its own size line):
// the switch, how it is doing, the name the devices show, linking this
// browser (POST /api/connect/pair: a link to open) or another device (the
// same with qr=1: a code on the panel for 5 minutes, and the link to send),
// the linked devices with Remove, and Forget all. docs/api.md "Remote
// control"; the panel's side is firmware/src/connect/api.rs. Read again
// every 5 s while the tab is open (the app's scheduler, its own read).

import { app } from './state.js';
import { h, clear, saver, toast, confirmBox } from './util.js';

// The panel's state, in words (no internals).
function said(c) {
  if (!c.enabled) return 'Off';
  if (c.refused) return 'opendmd.dev no longer knows this panel: use Forget all, then link your devices again';
  if (c.too_old) return 'This panel needs a firmware update to use Remote control';
  const s = c.state || '';
  if (s == 'open') return 'On: ready for your devices';
  if (s == 'waiting after boot') return 'On: starts a few minutes after a restart';
  if (s == 'backoff' || s == 'waiting for network') return 'On: opendmd.dev isn’t reachable right now, trying again';
  if (s == 'held for an update') return 'Paused while the panel updates';
  if (s.startsWith('off after')) return 'Paused after two restarts in a row: switch it off and on again';
  // `waiting` is the panel's own plain words (shown, never parsed).
  if (s == 'waiting for memory') return 'On: waiting for free memory' + (c.waiting ? ` (${c.waiting})` : '');
  if (s == 'waiting for quiet') return 'On: waiting until uploads to the panel finish';
  if (s == 'looking up the server') return 'On: looking up opendmd.dev…';
  return 'On: connecting…';
}

// `box`: the card's group (`#rc`, a child of the tab).
export function remoteCard(box) {
  const head = h('h3', 'Remote control'), run = saver(head);
  const sw = h('input', { type: 'checkbox', id: 'rc-on' });
  const state = h('p.muted', { id: 'rc-state' });
  const name = h('input', { type: 'text', id: 'rc-name', maxLength: 32, autocomplete: 'off' });
  const nudge = h('p.cap.bad', { id: 'rc-nudge', role: 'alert', hidden: true }, 'No API key: anyone on your WiFi can link a device that controls this panel. Set one in Security, below, first.');
  const say = h('p', { id: 'rc-pair-say' });
  const link = h('a', { id: 'rc-link', target: '_blank', rel: 'noopener noreferrer' }, 'Open Remote control');
  const url = h('input', { type: 'text', id: 'rc-url', readOnly: true, style: 'width:100%' });
  const left = h('span.muted', { id: 'rc-left' });
  const pair = h('div.banner', { id: 'rc-pair', hidden: true, style: 'margin:12px 0 0' }, say, h('div.row', link, h('button.sm', { id: 'rc-copy', onclick: copy }, 'Copy link'),
    h('button.sm', { id: 'rc-cancel', onclick: cancel }, 'Cancel'), left), url);
  const list = h('ul', { id: 'rc-list', style: 'padding-left:18px' });
  const none = h('p.muted', { id: 'rc-none' }, 'No devices linked yet');
  const forget = h('button', { id: 'rc-forget', onclick: forgetAll }, 'Forget all devices');
  const links = h('div', { id: 'rc-links' }, nudge, h('div.row', { style: 'margin-top:12px' },
    h('button', { id: 'rc-this', onclick: () => open(false) }, 'Control from this browser away from home'),
    h('button', { id: 'rc-add', onclick: () => open(true) }, 'Add a device')), pair);
  box.append(head, h('p.cap', 'Control this panel from a phone away from home, through opendmd.dev. Off, nothing leaves your home network.'),
    h('label', sw, 'Remote control'), state, h('label.stack', { style: 'margin-top:12px' }, 'Name your devices show', name), links,
    h('h4', 'Linked devices'), list, none, h('div.row', forget));

  // `seq`: reads started before the latest POST are stale (their
  // `pairing: null` predates the window); `count`: devices last seen.
  let until = 0, timer = 0, seq = 0, count = null;
  const api = (m, u, b) => app.src.json(m, '/api/connect' + u, b);
  function tickLeft() {
    const s = Math.round((until - Date.now()) / 1000);
    if (s <= 0) return endPair();
    left.textContent = `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')} left`;
  }
  function endPair() { clearInterval(timer); timer = 0; pair.hidden = true; url.value = ''; link.removeAttribute('href'); }

  function show(c) {
    sw.checked = c.enabled;
    state.textContent = said(c);
    if (document.activeElement !== name) name.value = c.name || '';
    name.placeholder = app.config?.hostname || '';
    links.hidden = !c.enabled;
    nudge.hidden = app.config?.api_key_set !== false;
    if (!c.pairing && timer) endPair();
    const cs = c.controllers || [];
    if (count !== null && cs.length > count) toast('A device was linked', 'ok');
    count = cs.length;
    none.hidden = cs.length > 0;
    forget.hidden = !cs.length;
    clear(list, cs.map((d) => h('li', { 'data-k': d.k }, h('b', d.label),
      h('span.muted', [d.added ? ` · linked ${new Date(d.added * 1000).toLocaleDateString()}` : '', d.added_by ? ' · added from another device' : ''].join('')), ' ',
      h('button.sm', { onclick: () => remove(d) }, 'Remove'))));
  }
  const read = () => { const n = seq; return api('GET', '').then((c) => { if (n === seq) show(c); }); };

  sw.onchange = () => run(async () => show(await api('PUT', '', { enabled: sw.checked })));
  name.onchange = () => run(async () => show(await api('PUT', '', { name: name.value.trim() })));

  async function open(qr) {
    try {
      seq++;
      const r = await api('POST', '/pair' + (qr ? '?qr=1' : ''));
      seq++;
      link.href = url.value = r.link;
      link.textContent = qr ? 'Open the link' : 'Open Remote control';
      const wait = r.online ? '' : 'The panel isn’t connected to opendmd.dev yet: the link works once it is. ';
      say.textContent = wait + (qr && r.qr ? (r.online ? 'A code is on the panel for 5 minutes: scan it with the other device’s camera (step back a little if it doesn’t scan), or send that device this link.'
        : 'A code is on the panel for 5 minutes, for once it is connected; or send the other device this link.')
        : qr && r.dark ? 'The panel is dark for the night, so it shows no code: send the other device this link. It works once, for 5 minutes.'
          : 'Open this link in this browser to finish. It works once, for 5 minutes.');
      until = Date.now() + r.expires_s * 1000;
      pair.hidden = false;
      clearInterval(timer);
      timer = setInterval(tickLeft, 1000);
      tickLeft();
    } catch (e) {
      seq++;
      toast(e.status == 409 ? (sw.checked ? 'The panel can’t link a device right now: ' + e.message : 'Switch Remote control on first') : 'The panel couldn’t start linking: ' + e.message, 'bad');
    }
  }
  function copy() {
    if (navigator.clipboard) navigator.clipboard.writeText(url.value).then(() => toast('Link copied'), () => {});
    else { url.select(); document.execCommand('copy'); toast('Link copied'); }
  }
  function cancel() { endPair(); api('DELETE', '/pair').then(show, () => {}); }
  async function remove(d) {
    if (!confirmBox(`Remove “${d.label}”? It can no longer control this panel.`)) return;
    run(async () => show(await api('DELETE', '/controllers/' + d.k)));
  }
  async function forgetAll() {
    if (!confirmBox('Forget every linked device? Each one has to be linked again.')) return;
    run(async () => { endPair(); show(await api('POST', '/forget')); });
  }

  read().catch(() => { state.textContent = 'The panel didn’t answer'; });
  // On while the tab is open (settings.js): read every 5 s.
  return (live) => live ? app.watch('remote', 5000, read) : app.unwatch('remote');
}
