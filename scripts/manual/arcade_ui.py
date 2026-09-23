#!/usr/bin/env python3
"""Exercise the arcade and its complete channel offline, under the production CSP.

Run with a Python environment that has Playwright installed:
    python scripts/manual/arcade_ui.py
All requests are intercepted. No note, quest, or network write reaches a responder.
Screenshots are saved in /tmp/emem-arcade-review.
"""
import base64
import hashlib
import json
import re
from pathlib import Path
from urllib.parse import urlsplit

from playwright.sync_api import sync_playwright, expect

ROOT = Path(__file__).resolve().parents[2]
OUT = Path("/tmp/emem-arcade-review")
ORIGIN = "http://localhost:8777"


def csp(html):
    def hashes(tag):
        blocks = re.findall(r"<" + tag + r"\b[^>]*>([\s\S]*?)</" + tag + ">", html)
        return " ".join("'sha256-" + base64.b64encode(
            hashlib.sha256(block.encode()).digest()).decode() + "'" for block in blocks)
    return ("default-src 'self'; script-src 'self' https://esm.sh https://cdn.redocly.com "
            + hashes("script") + "; style-src 'self' https://fonts.googleapis.com "
            + hashes("style") + "; style-src-attr 'unsafe-inline'; "
            "connect-src 'self' https://esm.sh https://server.arcgisonline.com; "
            "img-src 'self' data: blob: https:; font-src 'self' data: https://fonts.gstatic.com; "
            "worker-src 'self' blob:; frame-ancestors 'self'; base-uri 'self'; form-action 'self'")


def main():
    OUT.mkdir(exist_ok=True)
    arcade = (ROOT / "web/arcade.html").read_text()
    channel = (ROOT / "web/channel.html").read_text()
    theme = (ROOT / "scripts/templates/arcade-channel.html").read_text().strip()
    assert theme in channel, "Generated channel lost its canonical arcade template"
    expected_notes = len(re.findall(r'<article class="msg"', channel))
    blocked = {"channel": False}
    writes = []
    checked_tokens = []
    errors = []
    csp_errors = []
    source_pages = {"/arcade": arcade, "/channel": channel}

    def route_request(route):
        req = route.request
        path = urlsplit(req.url).path
        if urlsplit(req.url).netloc != "localhost:8777":
            route.fulfill(status=200, body="", content_type="text/css")
            return
        if path in source_pages:
            if path == "/channel" and blocked["channel"]:
                route.fulfill(status=503, body="Channel temporarily unavailable")
                return
            html = source_pages[path]
            route.fulfill(body=html, content_type="text/html",
                          headers={"Content-Security-Policy": csp(html)})
            return
        asset = ROOT / "web" / path.lstrip("/")
        if asset.is_file() and asset.resolve().is_relative_to(ROOT / "web"):
            content_type = "text/css" if path.endswith(".css") else "application/octet-stream"
            route.fulfill(body=asset.read_bytes(), content_type=content_type)
            return
        data = {}
        status = 200
        if path == "/v1/channel/geo":
            data = {"messages": [], "count": 0}
        elif path == "/v1/agents":
            data = {"agents": [{"prefix": "k572x7go", "notes": 515,
                               "correspondence": 20, "last_seen": "2026-09-10T18:00:00Z"}]}
        elif path == "/v1/log/sth":
            data = {"tree_size": 100, "root_hash": "offline-test", "timestamp": 1}
        elif path == "/v1/memory/search":
            data = {"results": []}
        elif path.endswith("/sse"):
            route.fulfill(status=200, body=": offline test\n\n", content_type="text/event-stream")
            return
        elif path == "/v1/memory_token/resolve_many":
            tokens = req.post_data_json.get("tokens", [])
            assert len(tokens) <= 256
            checked_tokens.extend(tokens)
            data = {"items": [{"ok": False, "error": {"code": "cid_not_found"}} for _ in tokens]}
            if data["items"]:
                data["items"][-1] = None
        elif path.startswith("/v1/memory_bundle/"):
            status = 404
        elif path in ("/mcp", "/mcp/full"):
            args = req.post_data_json or {}
            if args.get("method") == "tools/list":
                data = {"jsonrpc": "2.0", "id": 1, "result": {"tools": []}}
            else:
                tool = args.get("params", {}).get("name", "")
                if tool == "memory_write":
                    writes.append(tool)
                data = {"jsonrpc": "2.0", "id": 1, "result": {
                    "content": [{"type": "text", "text": json.dumps(
                        {"content": "Offline test: exact note bytes.", "entries": []})}]}}
        elif req.method == "POST":
            writes.append(path)
            status = 503
        route.fulfill(status=status, body=json.dumps(data), content_type="application/json")

    with sync_playwright() as pw:
        browser = pw.chromium.launch(
            executable_path="/home/ubuntu/.cache/ms-playwright/chromium-1234/chrome-linux/chrome",
            headless=True, args=["--no-sandbox", "--enable-unsafe-swiftshader"])
        context = browser.new_context(viewport={"width": 1440, "height": 1000}, reduced_motion="reduce")
        context.route("**/*", route_request)
        context.add_init_script("""{
          const Native = EventSource;
          window.__streams = [];
          window.EventSource = class extends Native {
            constructor(...args) { super(...args); window.__streams.push(this); }
          };
        }""")
        page = context.new_page()
        page.on("pageerror", lambda e: errors.append(str(e)))
        # Playwright's page.evaluate("...") helper itself triggers Chromium's
        # unsafe-eval diagnostic under a strict CSP; page scripts remain
        # hash-authorized. Keep only CSP reports that are not that harness
        # diagnostic.
        page.on("console", lambda m: csp_errors.append(m.text)
                if ("violates the following Content Security Policy" in m.text
                    and "Evaluating a string as JavaScript" not in m.text) else None)
        page.goto(ORIGIN + "/arcade", wait_until="domcontentloaded")
        expect(page.locator('[data-view="world"]')).to_have_attribute("aria-pressed", "true")
        expect(page.locator("#ow-first")).not_to_be_visible()
        page.screenshot(path=str(OUT / "world-desktop.png"))
        page.locator("#arcade-help").click()
        expect(page.locator("#ow-first")).to_be_visible()
        page.keyboard.press("Escape")
        expect(page.locator("#ow-first")).not_to_be_visible()
        page.locator('[data-view="missions"]').click()
        expect(page.locator("#missions")).to_be_visible()
        page.screenshot(path=str(OUT / "missions-desktop.png"))
        page.locator('[data-view="channel"]').click()
        expect(page.locator("#channel-frame")).to_be_visible(timeout=30000)
        frame = page.frame_locator("#channel-frame")
        expect(frame.locator(".deck-nav")).to_be_visible()
        assert frame.locator("article.msg").count() == expected_notes
        expect(frame.locator(".sitebar")).not_to_be_visible()
        expect(frame.locator("#deck-conversation")).to_be_visible()
        assert frame.locator(".msg:visible").count() == 60
        page.screenshot(path=str(OUT / "channel-desktop.png"))
        frame.locator("#q").fill("world model game changers")
        expect(frame.locator('[id="4bkwuwq2gbplwlryhqlp7uca4u"]')).to_be_visible()
        frame.get_by_role("button", name="Reset filters").click()
        frame.locator("#q").fill("no-match-arcade-fixture-8675309")
        expect(frame.locator("#findn")).to_have_text("0 of " + str(expected_notes))
        assert frame.locator(".msg:visible").count() == 0
        frame.get_by_role("button", name="Reset filters").click()
        frame.get_by_role("button", name="Evidence", exact=True).click()
        expect(frame.locator("#desk")).to_be_visible()
        checked_tokens.clear()
        frame.locator("#recheck").click()
        expect(frame.locator("#recheck")).to_contain_text("checked in your browser", timeout=30000)
        expected_tokens = frame.locator('[data-token]').evaluate_all(
            "(nodes) => nodes.map(n => n.dataset.token).filter(t => /^emem:(fact|cube):/.test(t))")
        assert set(expected_tokens) <= set(checked_tokens)
        assert frame.locator('.tok[data-state="unchecked"]').count() > 0
        page.screenshot(path=str(OUT / "evidence-desktop.png"))
        frame.get_by_role("button", name="Agents", exact=True).click()
        expect(frame.locator(".substrate")).to_be_visible()
        frame.get_by_role("button", name="Field guide", exact=True).click()
        assert frame.locator(".drawer").count() >= 4
        frame.locator(".drawer").last.locator("summary").click()
        expect(frame.locator(".drawer").last.locator(".drawer-body")).to_be_visible()
        frame.get_by_role("button", name="Conversations", exact=True).click()
        frame.locator(".deck-speakers summary").click()
        frame.locator(".chip").first.click()
        expect(frame.locator(".chip").first).to_have_attribute("aria-pressed", "false")
        frame.get_by_role("button", name="Reset filters").click()
        expect(frame.locator(".chip").first).to_have_attribute("aria-pressed", "true")
        # A thread can include notes outside the initial sixty-message window.
        frame.locator('[id="4bkwuwq2gbplwlryhqlp7uca4u"] .thr').click()
        expect(frame.locator("body")).to_have_class(re.compile("threading"))
        assert frame.locator(".msg.inthread:visible").count() > 1
        frame.locator("#thrx").click()
        page.locator("#comms-focus").click()
        expect(page.locator("#stagewrap")).not_to_be_visible()
        page.locator("#comms-focus").click()
        expect(page.locator("#stagewrap")).to_be_visible()
        inner = page.frames[1]
        inner.evaluate("""() => {
          for (const stream of window.__streams) stream.dispatchEvent(new MessageEvent('message', {
            data: JSON.stringify({path:'/memories/by_attester/testagent/new-note.md',
              file_cid:'offline-live-note', signed_at:'2026-09-18T12:00:00Z'})
          }));
        }""")
        expect(frame.locator("#livefeed .live-msg")).to_be_visible()
        frame.get_by_role("button", name="Evidence", exact=True).click()
        expect(frame.locator("#livefeed")).not_to_be_visible()
        frame.get_by_role("button", name="Conversations", exact=True).click()
        for width, height in [(390, 844), (768, 1024), (320, 740)]:
            page.set_viewport_size({"width": width, "height": height})
            frame.locator("#q").fill("world model game changers")
            expect(frame.locator('[id="4bkwuwq2gbplwlryhqlp7uca4u"]')).to_be_visible()
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"), width
            inner = page.frames[1]
            assert inner.evaluate("document.documentElement.scrollWidth <= innerWidth"), width
            page.screenshot(path=str(OUT / ("channel-" + str(width) + ".png")))
            page.locator('[data-view="world"]').click()
            expect(page.locator("#owu-q")).to_be_visible()
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"), width
            page.screenshot(path=str(OUT / ("world-" + str(width) + ".png")))
            page.locator('[data-view="missions"]').click()
            expect(page.locator("#go")).to_be_visible()
            page.locator('[data-view="channel"]').click()
        page.goto(ORIGIN + "/arcade?view=channel#4bkwuwq2gbplwlryhqlp7uca4u",
                  wait_until="domcontentloaded")
        expect(page.locator("#channel-frame")).to_be_visible(timeout=30000)
        frame = page.frame_locator("#channel-frame")
        expect(frame.locator('[id="4bkwuwq2gbplwlryhqlp7uca4u"]')).to_be_visible()
        # Standalone channel retains its original theme and navigation.
        standalone = context.new_page()
        standalone.goto(ORIGIN + "/channel", wait_until="domcontentloaded")
        expect(standalone.locator("html")).not_to_have_class(re.compile("arcade-channel"))
        expect(standalone.locator(".sitebar")).to_be_visible()
        assert standalone.locator(".msg").count() == expected_notes
        standalone.close()
        # Accelerate only the channel timeout; the request still genuinely fails.
        blocked["channel"] = True
        page.evaluate("""() => {
          const original = window.setTimeout;
          window.__arcadeOriginalTimeout = original;
          window.setTimeout = (fn, ms, ...args) => original(fn, ms === 30000 ? 30 : ms, ...args);
        }""")
        page.locator("#comms-refresh").click()
        expect(page.locator("#comms-error")).to_be_visible(timeout=5000)
        blocked["channel"] = False
        page.evaluate("window.setTimeout = window.__arcadeOriginalTimeout")
        page.locator("#comms-retry").click()
        expect(page.locator("#channel-frame")).to_be_visible(timeout=30000)
        assert not writes, writes
        assert not errors, errors
        assert not csp_errors, csp_errors
        browser.close()
    print(f"PASS: {expected_notes} messages preserved; navigation, search, threads, all four panels,")
    print("      guide, standalone channel, deep links, errors, CSP, and 320/390/768/1440px layouts.")
    print(f"Screenshots: {OUT}")


if __name__ == "__main__":
    main()
