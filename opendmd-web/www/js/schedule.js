// Time, schedule and night mode: the Settings cards, and the banner Now
// shows while the schedule has the panel. The device keeps the clock (NTP)
// and a timezone, and evaluates the schedule once a minute; rules pick a
// playlist and/or a brightness for a stretch of the week (and of the year:
// Halloween's and Christmas's playlists come on by themselves), night mode turns
// the panel off (or dims it) every night, between two times or from sunset
// to sunrise (the device works out the sun for the saved location). None of
// it changes the playlist or brightness saved in Playback: when a rule
// ends, those come back.

import { app, playlistLabel, setPath } from './state.js';
import { h, clear, on, toast, note, saver, debounce } from './util.js';

// A short list of common zones (POSIX TZ strings, what the device's clock
// library reads); anything else goes in "Custom".
export const ZONES = [
  ['Europe/Dublin', 'GMT0IST,M3.5.0/1,M10.5.0'],
  ['Europe/London', 'GMT0BST,M3.5.0/1,M10.5.0'],
  ['UTC', 'UTC0'],
  ['Europe/Paris, Berlin, Madrid', 'CET-1CEST,M3.5.0,M10.5.0/3'],
  ['Europe/Helsinki, Athens', 'EET-2EEST,M3.5.0/3,M10.5.0/4'],
  ['America/New_York', 'EST5EDT,M3.2.0,M11.1.0'],
  ['America/Chicago', 'CST6CDT,M3.2.0,M11.1.0'],
  ['America/Denver', 'MST7MDT,M3.2.0,M11.1.0'],
  ['America/Los_Angeles', 'PST8PDT,M3.2.0,M11.1.0'],
  ['Australia/Sydney', 'AEST-10AEDT,M10.1.0,M4.1.0/3'],
  ['Asia/Tokyo', 'JST-9'],
];

const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
// Rules' dates are "MM-DD", inclusive, the second earlier for one over New Year.
const mdText = (md) => `${+md.slice(3)} ${MONTHS[md.slice(0, 2) - 1]}`;
const realDay = (m, d) => d >= 1 && d <= [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1];
// Quick rules, offered when the card has the playlist: all day, every day, in season.
const SEASONS = [['filelist.hall', 'Halloween', ['10-01', '10-31']], ['filelist.xmas', 'Christmas', ['12-01', '12-26']]];

export function daysText(days) {
  const d = [...new Set(days?.length ? days : [1, 2, 3, 4, 5, 6, 7])].sort();
  const k = d.join('');
  if (k === '1234567') return 'Every day';
  if (k === '12345') return 'Weekdays';
  if (k === '67') return 'Weekends';
  return d.map((n) => DAYS[n - 1]).join(', ');
}

export function ruleText(r) {
  const what = [];
  if (r.set != null) what.push(playlistLabel(setPath(r.set)));
  if (r.brightness != null) what.push(`brightness ${r.brightness}`);
  const when = r.from === r.to ? ', all day' : ` ${r.from}–${r.to}`;
  return `${r.dates ? `${mdText(r.dates[0])}–${mdText(r.dates[1])}, ` : ''}${daysText(r.days)}${when} → ${what.join(', ') || 'no change'}`;
}

function ago(s) {
  if (s == null) return 'not synced yet';
  return s < 90 ? 'synced just now' : s < 5400 ? `synced ${Math.round(s / 60)} min ago` : `synced ${Math.round(s / 3600)} h ago`;
}

// --- Settings → Time --------------------------------------------------------
export function timeCard() {
  const c = app.config;
  const zone = h('select', { id: 'tm-zone' },
    ZONES.map(([n, tz]) => h('option', { value: tz }, n)), h('option', { value: '' }, 'Custom…'));
  const known = ZONES.find(([, tz]) => tz === c.tz);
  zone.value = known ? c.tz : '';
  const custom = h('input', { type: 'text', id: 'tm-custom', placeholder: 'e.g. GMT0IST,M3.5.0/1,M10.5.0', value: known ? '' : c.tz || '' });
  const customRow = h('label.stack', { hidden: !!known }, 'Custom zone (POSIX TZ)', custom);
  const now = h('b', { id: 'tm-now' }, '…');
  const synced = h('span.muted', { id: 'tm-synced' });

  function render(s) {
    if (!s) return;
    now.textContent = s.time ? `${s.time} · ${s.tz}` : 'clock not set yet';
    synced.textContent = ' · ' + ago(s.time_synced_s);
  }

  const head = h('h3', 'Time'), run = saver(head);
  const save = () => run(async () => {
    const tz = zone.value || custom.value.trim();
    if (!tz || tz === app.config.tz) return false;
    const name = zone.value ? ZONES.find(([, t]) => t === tz)[0] : 'Custom';
    await app.src.putConfig({ tz, tz_name: name });
    await app.refreshConfig();
    render(await app.refreshStatus());
  });
  zone.onchange = () => { customRow.hidden = !!zone.value; save(); };
  custom.onchange = save;

  const el = h('div.grp', { id: 'tm' }, head,
    h('p', now, synced),
    h('div.form.f2', h('label.stack', 'Time zone', zone), customRow));
  return { el, render };
}

// --- Settings → Schedule ----------------------------------------------------
export function scheduleCard() {
  let sched = { rules: [], night: null };
  let editing = -1; // rule index being edited; rules.length = a new one
  const list = h('div.list', { id: 'sc-rules' });
  const editor = h('div.sub', { id: 'sc-edit', hidden: true });
  const nightOn = h('input', { type: 'checkbox', id: 'sc-night-on' });
  const nFrom = h('input', { type: 'time', id: 'sc-night-from', value: '23:30' });
  const nTo = h('input', { type: 'time', id: 'sc-night-to', value: '07:00' });
  const nWhen = h('select', { id: 'sc-night-when', 'aria-label': 'When' }, h('option', { value: 'clock' }, 'at set times'), h('option', { value: 'sun' }, 'from sunset to sunrise'));
  const nOffset = h('select', { id: 'sc-night-offset', 'aria-label': 'Start and end' }, [[0, 'exactly'], [30, '30 min shorter'], [60, '1 hour shorter'], [-30, '30 min longer'], [-60, '1 hour longer']].map(([v, l]) => h('option', { value: v }, l)));
  const clockPart = h('span.row', h('span', 'from'), nFrom, h('span', 'to'), nTo), sunPart = h('span.row', { hidden: true }, nOffset);
  const sunNote = h('p.cap', { id: 'sc-night-sun-note', hidden: true });
  const nMode = h('select', { id: 'sc-night-mode' }, h('option', { value: 'off' }, 'Panel off'), h('option', { value: 'dim' }, 'Dim'));
  const nLevel = h('input', { type: 'range', min: 1, max: 255, id: 'sc-night-level', value: 8 });
  const nLevelOut = h('output.mono', '8');
  const levelRow = h('label', 'Level', nLevel, nLevelOut);
  nLevel.oninput = () => { nLevelOut.textContent = nLevel.value; };

  const head = h('h3', 'Schedule'), nightH = h('h3', 'Night mode');
  const run = saver(head);
  async function put(next) {
    await app.src.putSchedule(next);
    sched = next;
    renderRules();
    offerSeasons();
    app.refreshStatus().catch(() => {});
  }

  function renderRules() {
    clear(list, sched.rules.length ? sched.rules.map((r, i) => h('div', { 'data-rule': i },
      h('span.grow', ruleText(r)),
      h('button.sm', { onclick: () => edit(i) }, 'Edit'),
      h('button.sm', { onclick: () => run(() => put({ ...sched, rules: sched.rules.filter((_, k) => k !== i) })) }, 'Delete')))
      : h('div.muted', 'No rules: the playlist and brightness in Playback apply all the time'));
  }
  // The night fields from the device (on load only: they save as they change).
  function renderNight() {
    const n = sched.night;
    nightOn.checked = !!n;
    if (n) {
      if (n.from) { nFrom.value = n.from; nTo.value = n.to; }
      nWhen.value = n.sun ? 'sun' : 'clock'; nOffset.value = n.offset ?? 0;
      nMode.value = n.mode; nLevel.value = n.brightness ?? 8; nLevelOut.textContent = nLevel.value;
    }
    levelRow.hidden = nMode.value !== 'dim';
    sunFields();
  }
  // Sunset to sunrise needs the location (Location, above: the panel works
  // out the sun there); a player that keeps none doesn't offer it.
  function sunFields() {
    const c = app.config || {}, sun = nWhen.value === 'sun', place = c.lat != null;
    nWhen.options[1].disabled = !place && !sun;
    nWhen.hidden = !('lat' in c) && !sun;
    clockPart.hidden = sun;
    sunPart.hidden = !sun;
    sunNote.hidden = (place || !('lat' in c)) && !sun;
    sunNote.textContent = !place ? (sun ? 'Night mode is off until the panel has a location again: set it under Location.' : 'For sunset to sunrise, set the panel’s location under Location.')
      : 'Shorter starts after sunset and ends before sunrise; longer starts before sunset and ends after sunrise.';
  }
  on('config', sunFields);

  async function edit(i) {
    editing = i;
    const r = sched.rules[i] || { days: [1, 2, 3, 4, 5], from: '18:00', to: '23:00' };
    const on = new Set(r.days?.length ? r.days : [1, 2, 3, 4, 5, 6, 7]);
    const days = h('div.chips', { id: 'sc-days' }, DAYS.map((d, k) => h('button.chip' + (on.has(k + 1) ? '.on' : ''), {
      'data-day': k + 1, onclick: (e) => e.currentTarget.classList.toggle('on'),
    }, d)));
    const from = h('input', { type: 'time', id: 'sc-from', value: r.from });
    const to = h('input', { type: 'time', id: 'sc-to', value: r.to });
    const set = h('select', { id: 'sc-set' }, h('option', { value: '\u0000' }, 'Keep the chosen playlist'));
    for (const p of await app.playlists().catch(() => [])) set.append(h('option', { value: p.value }, p.label));
    set.value = r.set ?? '\u0000';
    const bOn = h('input', { type: 'checkbox', id: 'sc-bright-on', checked: r.brightness != null });
    const bright = h('input', { type: 'range', min: 1, max: 255, id: 'sc-bright', value: r.brightness ?? 96 });
    const bOut = h('output.mono', bright.value);
    bright.oninput = () => { bOut.textContent = bright.value; bOn.checked = true; };
    // Dates: a day and a month each end; none is every day of the year.
    const dOn = h('input', { type: 'checkbox', id: 'sc-dates-on', checked: !!r.dates });
    const md = (id, v) => {
      const d = h('input', { type: 'number', id: id + '-d', min: 1, max: 31, value: +v.slice(3), style: 'width:64px', 'aria-label': 'day', oninput: () => { dOn.checked = true; } });
      const m = h('select', { id: id + '-m', 'aria-label': 'month', onchange: () => { dOn.checked = true; } }, MONTHS.map((n, k) => h('option', { value: k + 1 }, n)));
      m.value = +v.slice(0, 2);
      return { d, m, get: () => realDay(+m.value, +d.value) && `${String(m.value).padStart(2, '0')}-${String(d.value).padStart(2, '0')}` };
    };
    const dFrom = md('sc-date-from', r.dates?.[0] || '10-01'), dTo = md('sc-date-to', r.dates?.[1] || '10-31');
    const save = () => run(async () => {
      const picked = [...days.querySelectorAll('.chip.on')].map((c) => +c.dataset.day);
      if (!picked.length) throw new Error('pick at least one day');
      if (!from.value || !to.value) throw new Error('enter both times');
      const rule = { days: picked, from: from.value, to: to.value };
      if (dOn.checked) {
        rule.dates = [dFrom.get(), dTo.get()];
        if (!rule.dates.every(Boolean)) throw new Error('pick days that are in those months');
      }
      if (set.value !== '\u0000') rule.set = set.value;
      if (bOn.checked) rule.brightness = +bright.value;
      const rules = [...sched.rules];
      rules.splice(editing, editing < rules.length ? 1 : 0, rule);
      await put({ ...sched, rules });
      editor.hidden = true;
    });
    clear(editor, h('h3', { style: 'margin-top:0' }, editing < sched.rules.length ? 'Edit rule' : 'New rule'),
      days,
      h('div.form', { style: 'margin-top:12px' },
        h('label.stack', 'From', from), h('label.stack', 'To', to),
        h('label.stack', 'Play', set),
        h('label.stack', 'Brightness', h('div.row', bOn, bright, bOut)),
        h('div.stack', 'Dates', h('div.row', h('label', dOn, 'Only from'), dFrom.d, dFrom.m, h('span', 'to'), dTo.d, dTo.m))),
      h('div.row', { style: 'margin-top:12px' },
        h('button', { id: 'sc-rule-save', onclick: save }, 'Save rule'),
        h('button', { onclick: () => { editor.hidden = true; } }, 'Cancel')));
    editor.hidden = false;
  }

  // (The times stay with a night by the sun, for going back to them.)
  const saveNight = () => run(async () => {
    const sun = nWhen.value === 'sun';
    const night = nightOn.checked ? { from: nFrom.value, to: nTo.value, ...(sun && { sun, offset: +nOffset.value }), mode: nMode.value, brightness: +nLevel.value } : null;
    if (night && !sun && (!night.from || !night.to)) throw new Error('enter both times');
    await put({ ...sched, night });
  });
  nightOn.onchange = nFrom.onchange = nTo.onchange = nOffset.onchange = saveNight;
  nWhen.onchange = () => { sunFields(); saveNight(); };
  nMode.onchange = () => { levelRow.hidden = nMode.value !== 'dim'; saveNight(); };
  nLevel.addEventListener('input', debounce(saveNight, 400));

  async function load() {
    try { sched = await app.src.schedule(); sched.rules ||= []; renderRules(); renderNight(); offerSeasons(); } catch (e) { toast('schedule: ' + e.message, 'bad'); }
  }

  // A season's rule, first (the first that matches wins): its playlist all
  // day, every day, from its first date to its last.
  const seasons = h('span.row', { id: 'sc-seasons' });
  async function offerSeasons() {
    const have = new Set((await app.playlists().catch(() => [])).map((p) => p.value));
    clear(seasons, SEASONS.filter(([set]) => have.has(set) && !sched.rules.some((r) => r.set === set && r.dates)).map(([set, name, dates]) =>
      h('button', { 'data-season': set, onclick: () => run(() => put({ ...sched, rules: [{ days: [], from: '00:00', to: '00:00', set, dates }, ...sched.rules] })) },
        `Add ${name} (${mdText(dates[0])}–${mdText(dates[1])})`)));
  }

  const el = h('div.grp', { id: 'sc' }, head,
    note('The first rule that matches the day and time wins', 'A rule’s playlist and brightness last while it runs; afterwards the ones in Playback come back. A rule with dates plays only on those days of the year.'),
    list,
    h('div.row', { style: 'margin-top:12px' }, h('button', { id: 'sc-add', onclick: () => edit(sched.rules.length) }, 'Add rule'), seasons),
    editor,
    nightH,
    h('div.row', h('label', nightOn, 'Every night'), nWhen, clockPart, sunPart, nMode, levelRow), sunNote);
  load();
  return { el, load, night: () => sched.night };
}

// --- Now: what the schedule is doing ---------------------------------------
// Returns the banner's contents for a status, or null when there is nothing
// to say.
export function bannerFor(s, after) {
  const wake = async (minutes) => {
    try { await app.src.wake(minutes); after?.(); } catch (e) { toast(e.message, 'bad'); }
  };
  if (s.wake_until) {
    return [`Schedule paused until ${s.wake_until} · `, h('button.sm', { id: 'now-resume', onclick: () => wake(0) }, 'Resume schedule')];
  }
  const rule = s.schedule?.rule != null
    ? `Schedule: ${s.schedule.set != null ? playlistLabel(setPath(s.schedule.set)) : 'as chosen'}${s.schedule.until ? ` until ${s.schedule.until}` : ''}` : '';
  if (s.mode === 'off' || s.mode === 'dim') {
    return [`${s.mode === 'off' ? 'Night mode' : 'Dimmed for the night'}${s.mode_until ? ` until ${s.mode_until}` : ''} · `,
      h('button.sm', { id: 'now-wake', onclick: () => wake(60) }, 'Wake for an hour'), rule && h('span.muted', ' · ' + rule)];
  }
  return rule ? [rule] : null;
}
