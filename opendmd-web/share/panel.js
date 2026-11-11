// The site's way to a panel, shared by a Formula's page (/f/) and Create
// Art's (/create/): its address, asked once and remembered (localStorage
// `odmd.display`), and the hand-off to its web app.
//
// An https page can open or navigate to the panel's http page, never fetch
// from it (mixed content; the firmware takes no cross-origin writes). So the
// hand-off is a window: `http://<addr>/#<what>`, whose app says it is ready
// (postMessage to its opener) and gets the thing back the same way, to
// that window and its origin alone: nothing in the link, nothing kept on
// the site. Works in every browser.

const store = {
  get(k) { try { return localStorage.getItem('odmd.' + k); } catch { return null; } },
  set(k, v) { try { localStorage.setItem('odmd.' + k, v); } catch { /* private mode */ } },
};

// The panel's address, if given before.
export const addr = () => store.get('display');

// The page's address form, by its ids' prefix `p`: `<p>-ask` (the form),
// `<p>-addr`, `<p>-addr-err`, `<p>-where` with `<p>-where-addr`, and
// `<p>-change`. `go(addr)` once one is given (in the submit: a window may
// open). Returns {ask}.
export function addressForm(p, go, what) {
  const $ = (id) => document.getElementById(`${p}-${id}`);
  const where = () => { const a = addr(); $('where').hidden = !a; $('where-addr').textContent = a || ''; };
  where();
  function ask() {
    $('ask').hidden = false;
    $('addr').value = addr() || 'opendmd.local';
    $('addr').focus();
    $('addr').select();
  }
  $('change').onclick = ask;
  $('ask').onsubmit = (e) => {
    e.preventDefault();
    // A host name or IPv4 address (an IPv6 one in brackets), maybe a port;
    // `http://` and a trailing slash typed with it are fine.
    const a = $('addr').value.trim().replace(/^https?:\/\//i, '').replace(/\/+$/, '');
    // (And one a URL can be made of: a port past 65535 is not, and would fail only at the hand-off.)
    const okAddr = /^(\[[0-9a-f:.]+\]|[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*)(:\d{1,5})?$/i.test(a) && url(a);
    const err = $('addr-err');
    err.hidden = okAddr;
    err.textContent = okAddr ? '' : 'That doesn’t look like an address: a name like opendmd.local, or numbers like 192.168.1.50.';
    if (!okAddr) return;
    // On this network: a .local name, a name with no dots, a private or
    // link-local address. Anything else could be anyone's: ask once.
    const host = a.replace(/:\d+$/, '').toLowerCase();
    const near = /\.local$/.test(host) || /^[^[.]+$/.test(host) || /^(10\.|192\.168\.|169\.254\.|127\.|172\.(1[6-9]|2\d|3[01])\.)/.test(host)
      || /^\[(fe80|fc|fd|::1\])/.test(host);
    if (!near && $('ask').dataset.sure !== a) {
      $('ask').dataset.sure = a;
      err.hidden = false;
      err.textContent = `${a} doesn’t look like a panel on your own network (like opendmd.local or 192.168.1.50): the ${what} would go to that site. Press ${$('go').textContent} again if you’re sure.`;
      return;
    }
    store.set('display', a);
    where();
    go(a);
  };
  return { ask };
}

const url = (a) => { try { return new URL(`http://${a}/`); } catch { return null; } };

// Open the panel's app at `#<hash>` in a new window (call it in a click: a
// pop-up), and when that window says `ready`, post it `msg` (a promise of
// {data, transfer}) to its origin alone. False if no window opened.
export function handOff(a, hash, ready, msg) {
  const u = url(a);
  if (!u) return false;
  const panel = u.origin, w = window.open(`${u.origin}/#${hash}`, '_blank');
  if (!w) return false;
  const on = (e) => {
    if (e.source !== w || e.origin !== panel || e.data?.type !== ready) return;
    removeEventListener('message', on);
    // (`msg` may be a function of what the app said and its window: null sends nothing.)
    Promise.resolve(typeof msg == 'function' ? msg(e.data, w) : msg).then((m) => m && w.postMessage(m.data, panel, m.transfer || []));
  };
  addEventListener('message', on);
  setTimeout(() => removeEventListener('message', on), 120000);
  return true;
}
