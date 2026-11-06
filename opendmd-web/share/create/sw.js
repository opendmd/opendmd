// Create Art's service worker (/create/ only): the Android share target and
// nothing else. A GIF, picture or video shared to "Create Art" arrives as
// a POST to /create/share (manifest.webmanifest `share_target`); the file
// is put aside in the browser for the page to pick up (?shared=1), a link
// becomes ?url=. Every other request is left to the browser as if there
// were no worker: nothing of the site or the web app is ever stored or
// answered here (opendmd-notes docs/notes/web-app.md "Module addresses").
const SHARE = new URL('share', self.registration.scope).pathname;
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));
self.addEventListener('fetch', (e) => {
  if (e.request.method === 'POST' && new URL(e.request.url).pathname === SHARE) e.respondWith(shared(e.request));
});

async function shared(req) {
  const to = (q) => Response.redirect(new URL(q, self.registration.scope).href, 303);
  let f;
  try { f = await req.formData(); } catch { return to('./'); }
  const file = f.getAll('media').find((x) => typeof x === 'object' && x.size);
  if (file) {
    const box = await caches.open('create-shared');
    await box.put('shared', new Response(file, { headers: { 'Content-Type': file.type || 'application/octet-stream' } }));
    return to('?shared=1');
  }
  const link = /https?:\/\/\S+/.exec(['url', 'text', 'title'].map((k) => f.get(k) || '').join(' '));
  return to(link ? '?url=' + encodeURIComponent(link[0]) : './');
}
