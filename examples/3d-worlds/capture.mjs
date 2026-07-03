/* capture.mjs — deterministic headless capture for the 3D world templates.
 *
 * Drives headless Chromium over raw CDP (no dependencies; Node >= 22 for
 * the global WebSocket), loads a template with a pre-fetched scene as
 * window.EMEM_DATA plus window.EMEM_CAPTURE, steps the deterministic
 * window.__renderFrame(i, total) hook, screenshots each frame, and
 * assembles a GIF with ffmpeg's two-pass palette.
 *
 *   node capture.mjs --template carbon-world.html --scene out/carbon.scene.json \
 *        --frames 240 --size 1200x900 --gif ../../docs/media/world-rondonia.gif
 *
 * Options: --png out.png (write the first frame and stop), --fps 24,
 * --width-out 800 (GIF width), --keep-frames, --config '{...}' (JSON merged
 * over the template's window.EMEM_WORLD before load).
 * Env: CHROME and FFMPEG override the binary paths.
 */
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));

function arg(name, dflt) {
  const i = process.argv.indexOf("--" + name);
  return i > -1 ? process.argv[i + 1] : dflt;
}
function flag(name) { return process.argv.includes("--" + name); }

function findBin(env, candidates, what) {
  if (process.env[env] && existsSync(process.env[env])) return process.env[env];
  for (const c of candidates) if (existsSync(c)) return c;
  return what;   // hope it is on PATH
}

const CHROME = findBin("CHROME", [
  "/opt/pw-browsers/chromium-1194/chrome-linux/chrome",
  "/opt/pw-browsers/chromium",
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
  "/usr/bin/google-chrome",
], "chromium");
const FFMPEG = findBin("FFMPEG", [
  "/opt/pw-browsers/ffmpeg-1011/ffmpeg-linux",
  "/usr/bin/ffmpeg",
], "ffmpeg");

// ---- minimal CDP client over the global WebSocket ------------------------
class Cdp {
  constructor(ws) {
    this.ws = ws; this.id = 0; this.pending = new Map(); this.handlers = new Map();
    ws.addEventListener("message", (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id && this.pending.has(m.id)) {
        const { res, rej } = this.pending.get(m.id);
        this.pending.delete(m.id);
        m.error ? rej(new Error(m.error.message)) : res(m.result);
      } else if (m.method && this.handlers.has(m.method)) {
        this.handlers.get(m.method)(m.params, m.sessionId);
      }
    });
  }
  on(method, fn) { this.handlers.set(method, fn); }
  send(method, params = {}, sessionId) {
    const id = ++this.id;
    return new Promise((res, rej) => {
      this.pending.set(id, { res, rej });
      this.ws.send(JSON.stringify({ id, method, params, sessionId }));
    });
  }
}

function waitPort(child) {
  return new Promise((res, rej) => {
    let buf = "";
    child.stderr.on("data", (d) => {
      buf += d.toString();
      const m = buf.match(/DevTools listening on (ws:\/\/[^\s]+)/);
      if (m) res(m[1]);
    });
    child.on("exit", (code) => rej(new Error("chromium exited " + code + "\n" + buf)));
    setTimeout(() => rej(new Error("no DevTools endpoint after 30s\n" + buf)), 30000);
  });
}

async function main() {
  const template = arg("template");
  if (!template) throw new Error("--template <file.html> is required");
  const sceneFile = arg("scene");
  const frames = parseInt(arg("frames", "240"), 10);
  const [W, H] = arg("size", "1200x900").split("x").map(Number);
  const gifOut = arg("gif");
  const pngOut = arg("png");
  const fps = parseInt(arg("fps", "24"), 10);
  const widthOut = parseInt(arg("width-out", "800"), 10);
  const cfgOverride = arg("config");

  const work = mkdtempSync(join(tmpdir(), "emem-capture-"));
  const profile = join(work, "profile");

  let inject = "window.EMEM_CAPTURE = {};\n";
  if (sceneFile) {
    inject += "window.EMEM_DATA = " + readFileSync(sceneFile, "utf8") + ";\n";
  }
  if (cfgOverride) {
    // merge the moment the template assigns window.EMEM_WORLD, before the
    // engine reads it (a timer would race the engine's first microtask)
    inject += "window.__EMEM_CFG_OVERRIDE = " + cfgOverride + ";\n" +
      "(function(){var v;Object.defineProperty(window,'EMEM_WORLD',{configurable:true," +
      "get:function(){return v;}," +
      "set:function(x){v=Object.assign(x, window.__EMEM_CFG_OVERRIDE);}});})();\n";
  }

  // environments that route egress through a proxy set HTTPS_PROXY, but
  // chromium only honours an explicit flag (its CA must already be in the
  // NSS store, which managed sandboxes set up)
  const proxy = process.env.HTTPS_PROXY || process.env.https_proxy;
  const child = spawn(CHROME, [
    "--headless=new", "--no-sandbox", "--disable-dev-shm-usage",
    ...(proxy ? ["--proxy-server=" + proxy] : []),
    "--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader",
    "--hide-scrollbars", "--force-device-scale-factor=1",
    "--window-size=" + W + "," + H,
    "--user-data-dir=" + profile,
    "--remote-debugging-port=0",
    "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });

  try {
    const wsUrl = await waitPort(child);
    const ws = new WebSocket(wsUrl);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
    const cdp = new Cdp(ws);

    const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
    const s = (m, p) => cdp.send(m, p, sessionId);

    await s("Page.enable");
    await s("Runtime.enable");
    await s("Emulation.setDeviceMetricsOverride",
            { width: W, height: H, deviceScaleFactor: 1, mobile: false });
    await s("Page.addScriptToEvaluateOnNewDocument", { source: inject });

    // relay the page's http(s) requests through node's fetch: sandboxes and
    // egress proxies that admit node/curl often reset chromium's own TLS,
    // and a live (no --scene) capture still has to reach the responder
    await s("Fetch.enable",
            { patterns: [{ urlPattern: "http://*" }, { urlPattern: "https://*" }] });
    cdp.on("Fetch.requestPaused", async (p) => {
      try {
        if (p.request.method === "OPTIONS") {      // answer CORS preflights here
          await cdp.send("Fetch.fulfillRequest", {
            requestId: p.requestId, responseCode: 204,
            responseHeaders: [
              { name: "access-control-allow-origin", value: "*" },
              { name: "access-control-allow-methods", value: "GET, POST, OPTIONS" },
              { name: "access-control-allow-headers", value: "content-type" },
            ],
          }, sessionId);
          return;
        }
        const ct = Object.entries(p.request.headers || {})
          .find(([k]) => k.toLowerCase() === "content-type");
        const resp = await fetch(p.request.url, {
          method: p.request.method,
          headers: ct ? { "content-type": ct[1] } : undefined,
          body: p.request.postData,
        });
        const body = Buffer.from(await resp.arrayBuffer());
        // the body arrives decoded, and a second allow-origin would be a
        // CORS "multiple values" error, so those headers are rebuilt
        const drop = new Set(["content-encoding", "content-length",
                              "transfer-encoding", "access-control-allow-origin"]);
        const headers = [...resp.headers.entries()]
          .filter(([k]) => !drop.has(k.toLowerCase()))
          .map(([name, value]) => ({ name, value }));
        headers.push({ name: "access-control-allow-origin", value: "*" });
        await cdp.send("Fetch.fulfillRequest", {
          requestId: p.requestId, responseCode: resp.status,
          responseHeaders: headers, body: body.toString("base64"),
        }, sessionId);
      } catch (e) {
        if (process.env.EMEM_CAPTURE_DEBUG) {
          console.error("relay fail:", p.request.url.slice(0, 80), e.message);
        }
        cdp.send("Fetch.failRequest",
                 { requestId: p.requestId, errorReason: "ConnectionFailed" },
                 sessionId).catch(() => {});
      }
    });

    const url = "file://" + resolve(HERE, template);
    await s("Page.navigate", { url });

    // the engine sets __eememWorldReady after the scene is built
    const t0 = Date.now();
    for (;;) {
      const r = await s("Runtime.evaluate",
                        { expression: "window.__eememWorldReady === true" });
      if (r.result.value === true) break;
      const err = await s("Runtime.evaluate", {
        expression: "(document.getElementById('hud-sub')||{}).textContent || ''" });
      if (/^error:/.test(err.result.value || "")) throw new Error(err.result.value);
      if (Date.now() - t0 > 600000) throw new Error("scene not ready after 600s");
      await new Promise((r2) => setTimeout(r2, 200));
    }

    if (pngOut) {
      const fi = parseInt(arg("frame", "0"), 10);
      const tot = parseInt(arg("total", String(frames)), 10);
      await s("Runtime.evaluate",
              { expression: "window.__renderFrame(" + fi + "," + tot + ")" });
      const shot = await s("Page.captureScreenshot", { format: "png" });
      writeFileSync(pngOut, Buffer.from(shot.data, "base64"));
    } else {
      for (let i = 0; i < frames; i++) {
        await s("Runtime.evaluate",
                { expression: "window.__renderFrame(" + i + "," + frames + ")" });
        const shot = await s("Page.captureScreenshot", { format: "png" });
        writeFileSync(join(work, "f_" + String(i).padStart(4, "0") + ".png"),
                      Buffer.from(shot.data, "base64"));
        if (i % 24 === 0) process.stderr.write("frame " + i + "/" + frames + "\n");
      }
    }

    if (gifOut) {
      const palette = join(work, "palette.png");
      const vf = "fps=" + fps + ",scale=" + widthOut + ":-1:flags=lanczos";
      execFileSync(FFMPEG, ["-y", "-framerate", String(fps),
        "-i", join(work, "f_%04d.png"),
        "-vf", vf + ",palettegen=stat_mode=diff", palette], { stdio: "inherit" });
      execFileSync(FFMPEG, ["-y", "-framerate", String(fps),
        "-i", join(work, "f_%04d.png"), "-i", palette,
        "-lavfi", vf + "[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=4",
        "-loop", "0", gifOut], { stdio: "inherit" });
      process.stderr.write("wrote " + gifOut + "\n");
    }
  } finally {
    child.kill("SIGKILL");
    if (!flag("keep-frames")) rmSync(work, { recursive: true, force: true });
    else process.stderr.write("frames kept in " + work + "\n");
  }
}

main().catch((e) => { console.error(e.message || e); process.exit(1); });
