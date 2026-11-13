// The home page's scene, for one viewer: /?scene=NAME shows another of the
// scenes site.css holds ([data-scene=NAME]) and is remembered here; /?scene=synth
// goes back to everyone's. A name that isn't one changes nothing. A classic
// script in the head, so the name is on <html> before the first paint and only
// that scene's picture is fetched. No storage (private windows): the address
// still works, nothing is remembered.
(function () {
  var all = ["synth", "night", "day", "chunky", "shelf", "neon", "arcade"], key = "opendmd-scene", pick = null;
  try {
    pick = new URLSearchParams(location.search).get("scene");
    if (pick === null) pick = localStorage.getItem(key);
    else if (all.indexOf(pick) > 0) localStorage.setItem(key, pick);
    else if (pick === all[0]) localStorage.removeItem(key);
  } catch (e) {}
  if (all.indexOf(pick) > 0) document.documentElement.setAttribute("data-scene", pick);
})();
