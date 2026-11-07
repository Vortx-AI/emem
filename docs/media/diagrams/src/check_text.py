"""Report SVG text that leaves the card or overlaps other text, and the smallest font size.

Usage: python check_text.py docs/media/diagrams/*.svg   (needs Playwright with Chromium)
A clean file prints only its name and the "min font" line.
"""
import sys, pathlib
from playwright.sync_api import sync_playwright

JS = """
() => {
  const svg = document.querySelector('svg');
  const vb = svg.viewBox.baseVal;
  const ts = [...svg.querySelectorAll('text')].map(t => {
    const b = t.getBBox();
    return {s: t.textContent.slice(0, 40), x: b.x, y: b.y, r: b.x + b.width, b: b.y + b.height,
            fs: parseFloat(getComputedStyle(t).fontSize)};
  });
  const rot = [...svg.querySelectorAll('text')].map(t => t.hasAttribute('transform'));
  const out = [];
  let minfs = 99;
  ts.forEach((a, i) => {
    minfs = Math.min(minfs, a.fs);
    if (!rot[i] && (a.r > vb.width - 26 || a.x < 20 || a.b > vb.height - 6)) out.push('EDGE ' + a.s + ' r=' + a.r.toFixed(0) + ' b=' + a.b.toFixed(0));
    ts.forEach((c, j) => {
      if (j <= i || rot[i] || rot[j]) return;
      const ox = Math.min(a.r, c.r) - Math.max(a.x, c.x);
      const oy = Math.min(a.b, c.b) - Math.max(a.y, c.y);
      if (ox > 1 && oy > 1) out.push('OVERLAP ' + a.s + ' | ' + c.s);
    });
  });
  out.push('min font ' + minfs + ' -> ' + (minfs * 340 / vb.width).toFixed(1) + ' px at 340');
  return out;
}
"""
with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page()
    for s in sys.argv[1:]:
        pg.set_content(pathlib.Path(s).read_text())
        print(s)
        for line in pg.evaluate(JS):
            print('  ', line)
    b.close()
