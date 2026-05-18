// emem-home.js — inject a return-to-home affordance into mdbook's menu bar
// + replace mdbook's default favicons with the canonical Vortx mark so the
// browser tab matches the rest of emem.dev.
//
// mdbook renders an <h1 class="menu-title"> at the top of every page (the
// book title). On its own that title is plain text — readers landing on
// any /docs/* page have no one-click return to https://emem.dev/. We turn
// the title text into a hyperlink to "/" and prefix it with "←" so the
// affordance reads as "back to home" without changing the book chrome.
//
// Idempotent: re-running the script (e.g. mdbook's client-side nav) is a
// no-op once the title has been wrapped / favicons swapped.
(function () {
  function wrap() {
    var title = document.querySelector('h1.menu-title');
    if (!title) return;
    if (title.querySelector('a[data-emem-home]')) return; // already wired
    var label = title.textContent.trim();
    title.textContent = '';
    var a = document.createElement('a');
    a.href = '/';
    a.title = 'Return to emem.dev';
    a.setAttribute('data-emem-home', '');
    a.style.color = 'inherit';
    a.style.textDecoration = 'none';
    a.textContent = '← ' + label;
    title.appendChild(a);
  }
  // Strip the hashed mdbook favicons and install the Vortx mark so the
  // browser tab on /docs/* matches the rest of emem.dev. mdbook bakes its
  // favicons in as `favicon-<hash>.svg` / `favicon-<hash>.png` with a
  // cache-busting fingerprint — we don't want to ship a second copy of
  // the asset on every page so the swap happens client-side at first
  // paint. Idempotent: existing Vortx links short-circuit the install.
  function favicons() {
    if (document.querySelector('link[rel~="icon"][data-emem-vortx]')) return;
    document
      .querySelectorAll('link[rel="icon"], link[rel="shortcut icon"], link[rel="apple-touch-icon"]')
      .forEach(function (l) { l.parentNode.removeChild(l); });
    var head = document.head;
    var icon = document.createElement('link');
    icon.rel = 'icon';
    icon.type = 'image/gif';
    icon.href = 'https://vortx.ai/assets/vortx-logo-36.gif';
    icon.setAttribute('data-emem-vortx', '');
    head.appendChild(icon);
    var touch = document.createElement('link');
    touch.rel = 'apple-touch-icon';
    touch.href = 'https://vortx.ai/assets/vortx-logo-200.gif';
    touch.setAttribute('data-emem-vortx', '');
    head.appendChild(touch);
  }
  function init() { wrap(); favicons(); }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
