/* emem.dev homepage v3 (2026-11-20). Four small behaviours, no dependencies; the page reads fully without them.
   1. copy buttons  2. the hero's date toggle (same field, two dates)  3. code tabs  4. the live ask box, which posts
   the question to /v1/ask and shows the signed answer with a token any reader can check at /verify.
   The 174 KB v2 script served the old homepage (tokeniser, carousel, live counters); those moved off the homepage. */
(function () {
  "use strict";
  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };

  $$("[data-copy]").forEach(function (b) {
    b.addEventListener("click", function () {
      var t = b.getAttribute("data-copy");
      (navigator.clipboard ? navigator.clipboard.writeText(t) : Promise.reject()).then(function () {
        b.textContent = "Copied"; b.classList.add("done");
        setTimeout(function () { b.textContent = "Copy"; b.classList.remove("done"); }, 1600);
      }, function () { b.textContent = "Select and copy"; });
    });
  });

  var scene = $("[data-scene]");
  if (scene) {
    $$("[data-show]", scene).forEach(function (btn) {
      btn.addEventListener("click", function () {
        var d = btn.getAttribute("data-show");
        $$("img[data-date]", scene).forEach(function (im) { im.classList.toggle("on", im.getAttribute("data-date") === d); });
        $$("[data-show]", scene).forEach(function (o) { var on = o === btn; o.classList.toggle("on", on); o.setAttribute("aria-pressed", on ? "true" : "false"); });
      });
    });
  }

  $$("[role=tablist]").forEach(function (list) {
    var root = list.parentNode;
    $$("[data-tab]", list).forEach(function (tab) {
      tab.addEventListener("click", function () {
        var k = tab.getAttribute("data-tab");
        $$("[data-tab]", list).forEach(function (t) { t.setAttribute("aria-selected", t === tab ? "true" : "false"); });
        $$("[data-pane]", root).forEach(function (p) { p.hidden = p.getAttribute("data-pane") !== k; });
      });
    });
  });

  var form = $("[data-ask]"), out = $("[data-ask-out]");
  if (!form || !out) return;
  var input = $("input", form);
  function esc(s) { return String(s).replace(/[&<>"]/g, function (c) { return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]; }); }
  function show(html) { out.hidden = false; out.innerHTML = html; }
  function ask(q) {
    q = (q || "").trim();
    if (!q) { input.focus(); return; }
    show('<p class="a-l">Reading the place and signing the answer&hellip;</p>');
    fetch("/v1/ask", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ q: q }) })
      .then(function (r) { return r.json(); })
      .then(function (d) {
        var a = d.direct_answer || {};
        var sentence = a.sentence || d.answer_md || "emem could not answer that one.";
        var cell = d.place_resolved && d.place_resolved.cell64;
        var tok = cell && a.fact_cid ? "emem:fact:" + cell + ":" + a.fact_cid : "";
        var lim = (a.limits || [])[0] || "";
        var h = '<p class="a-s">' + esc(String(sentence).split("\n")[0]) + "</p>";
        if (lim) h += '<p class="a-l">Limits: ' + esc(lim) + "</p>";
        if (tok) h += '<a class="tok" href="/verify?q=' + encodeURIComponent(tok) + '"><i aria-hidden="true">&#10003;</i><span class="t">' +
          esc(tok) + '</span><span>check it</span></a>';
        show(h);
      })
      .catch(function () { show('<p class="a-s">The answer did not arrive. Try again in a moment.</p>'); });
  }
  form.addEventListener("submit", function (e) { e.preventDefault(); ask(input.value || input.placeholder); });
  $$(".chips [data-q]").forEach(function (c) {
    c.addEventListener("click", function () { input.value = c.getAttribute("data-q"); ask(input.value); });
  });
})();
