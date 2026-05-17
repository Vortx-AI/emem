// emem-home.js — inject a return-to-home affordance into mdbook's menu bar.
//
// mdbook renders an <h1 class="menu-title"> at the top of every page (the
// book title). On its own that title is plain text — readers landing on
// any /docs/* page have no one-click return to https://emem.dev/. We turn
// the title text into a hyperlink to "/" and prefix it with "←" so the
// affordance reads as "back to home" without changing the book chrome.
//
// Idempotent: re-running the script (e.g. mdbook's client-side nav) is a
// no-op once the title has been wrapped.
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
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', wrap);
  } else {
    wrap();
  }
})();
