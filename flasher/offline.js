// The install page's "Play from the card only, no WiFi" box (docs/guide/install.md;
// flasher/index.html and the site's install page, which takes no inline script).
// Each box (`data-offline-for`: an <esp-web-install-button>'s id) points its button
// at the offline manifest (data-manifest-offline: the release's parts and an NVS
// image saying network = off) while it is ticked, and back at the normal one
// (data-manifest-online) when it is not. ESP Web Tools reads the button's
// `manifest` attribute when it is clicked, so the swap needs nothing else.
for (const box of document.querySelectorAll('input[data-offline-for]')) {
  const button = document.getElementById(box.dataset.offlineFor);
  const { manifestOnline: online, manifestOffline: offline } = button?.dataset ?? {};
  if (!online || !offline) {
    box.closest('.offline-box')?.remove();
    continue;
  }
  const apply = () => button.setAttribute('manifest', box.checked ? offline : online);
  box.addEventListener('change', apply);
  // A box the browser kept ticked across a reload.
  apply();
}
