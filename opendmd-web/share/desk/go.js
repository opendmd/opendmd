// The demo's old addresses: /demo/#now, #formulas, #import… are the app's (app.html), as they were
// before the desk scene took /demo/. A classic script in the head, so it runs before anything loads.
if (location.hash.length > 1) location.replace('app.html' + location.hash);
