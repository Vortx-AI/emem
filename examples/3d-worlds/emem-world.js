/* emem-world.js — render a live emem region as a 3-D gaussian splat world.
 *
 * Every splat is one signed fact recalled from an emem responder: its
 * position comes from the fact's cell (lat/lng in derivation.args) and its
 * elevation band, its colour from whatever bands the template maps. The
 * receipts arrive with the data; any splat's fact_cid re-checks at /verify.
 *
 * Usage: set window.EMEM_WORLD before loading this script (see the two
 * template HTML files). If window.EMEM_DATA is set (a pre-fetched scene
 * object), no network calls are made — that path is used for headless
 * capture and for offline demos.
 */
(function () {
  "use strict";

  const CFG = window.EMEM_WORLD;
  const hud = document.getElementById("hud");
  const sub = document.getElementById("hud-sub");

  function status(msg) { if (sub) sub.textContent = msg; }

  async function post(path, body) {
    const r = await fetch(CFG.responder + path, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!r.ok) throw new Error(path + " -> " + r.status);
    return r.json();
  }

  // ---- data ------------------------------------------------------------
  async function fetchScene() {
    status("sampling " + CFG.maxCells + " cells over the bbox…");
    const qr = await post("/v1/query_region", {
      bbox: CFG.bbox, bands: [CFG.bands[0]], max_cells: CFG.maxCells,
    });
    const cells = qr.receipt.cells;
    const scene = { cells: {}, fact_count: 0 };
    for (const band of CFG.bands) {
      for (let i = 0; i < cells.length; i += 256) {
        status(band + " — recalling cells " + i + "–" +
               Math.min(i + 256, cells.length) + " of " + cells.length +
               " (first pass materializes and signs)…");
        const d = await post("/v1/recall_many", {
          cells: cells.slice(i, i + 256), bands: [band],
        });
        for (const [cell, entry] of Object.entries(d.by_cell || {})) {
          for (const f of entry.facts || []) {
            if (f.value === null || f.value === undefined) continue;
            const rec = scene.cells[cell] || (scene.cells[cell] = {});
            const args = (f.derivation || {}).args || [];
            if (rec.lat === undefined && args.length > 1) {
              rec.lat = args[0]; rec.lng = args[1];
            }
            rec[f.band] = f.value;
            scene.fact_count += 1;
          }
        }
      }
    }
    return scene;
  }

  // ---- geometry --------------------------------------------------------
  function buildCloud(scene) {
    const rows = Object.values(scene.cells).filter(
      (r) => r.lat !== undefined && r[CFG.heightBand] !== undefined);
    if (!rows.length) throw new Error("no facts with positions");

    const lat0 = rows.reduce((s, r) => s + r.lat, 0) / rows.length;
    const lng0 = rows.reduce((s, r) => s + r.lng, 0) / rows.length;
    const mLat = 111320, mLng = 111320 * Math.cos(lat0 * Math.PI / 180);
    const hs = rows.map((r) => r[CFG.heightBand]);
    const hMin = Math.min(...hs), hMax = Math.max(...hs);
    const S = 1 / 120;                       // 1 world unit = 120 m
    const zx = CFG.heightExaggeration || 1.6;

    // one signed fact renders as a vertical gaussian column: a bright cap
    // splat at the fact's height plus fading splats down to the valley
    // floor, so relief reads as volume. No fact, no column.
    const K = CFG.columnSplats || 7;
    const N = rows.length * K;
    const pos = new Float32Array(N * 3);
    const col = new Float32Array(N * 3);
    const aAmp = new Float32Array(N);
    const aSize = new Float32Array(N);
    const c = new THREE.Color();
    rows.forEach((r, i) => {
      const x = (r.lng - lng0) * mLng * S;
      const y = ((r[CFG.heightBand] - hMin) * zx) * S;
      const z = -(r.lat - lat0) * mLat * S;
      CFG.colorize(r, c, { hMin, hMax });
      for (let j = 0; j < K; j++) {
        const k = i * K + j;
        const f = j / K;                       // 0 = cap, towards 1 = base
        pos[3 * k] = x;
        pos[3 * k + 1] = y * (1 - f);
        pos[3 * k + 2] = z;
        const dim = j === 0 ? 1.0 : 0.28 * (1 - f);
        col[3 * k] = c.r * dim; col[3 * k + 1] = c.g * dim; col[3 * k + 2] = c.b * dim;
        aAmp[k] = j === 0 ? 1.0 : 0.5;
        aSize[k] = j === 0 ? 1.0 : 0.8;
      }
    });

    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
    geo.setAttribute("color", new THREE.BufferAttribute(col, 3));
    geo.setAttribute("aAmp", new THREE.BufferAttribute(aAmp, 1));
    geo.setAttribute("aSize", new THREE.BufferAttribute(aSize, 1));

    // gaussian sprite: alpha falls off as exp(-r^2 / (2 sigma^2))
    const mat = new THREE.ShaderMaterial({
      transparent: true,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
      uniforms: { uSize: { value: CFG.splatSize || 60 } },
      vertexShader:
        "attribute vec3 color; attribute float aAmp; attribute float aSize;\n" +
        "varying vec3 vC; varying float vA;\n" +
        "uniform float uSize;\n" +
        "void main(){ vC = color; vA = aAmp;\n" +
        "  vec4 mv = modelViewMatrix * vec4(position, 1.0);\n" +
        "  gl_PointSize = uSize * aSize * (60.0 / -mv.z);\n" +
        "  gl_Position = projectionMatrix * mv; }",
      fragmentShader:
        "varying vec3 vC; varying float vA;\n" +
        "void main(){ vec2 d = gl_PointCoord - vec2(0.5);\n" +
        "  float r2 = dot(d, d);\n" +
        "  float a = exp(-r2 / 0.05) * vA;\n" +
        "  if (a < 0.02) discard;\n" +
        "  gl_FragColor = vec4(vC, a); }",
    });
    const points = new THREE.Points(geo, mat);
    const spanX = (Math.max(...rows.map((r) => r.lng)) - Math.min(...rows.map((r) => r.lng))) * mLng * S;
    return { points, span: spanX, hMin, hMax, n: rows.length };
  }

  // ---- scene -----------------------------------------------------------
  async function main() {
    const scene3 = new THREE.Scene();
    scene3.background = new THREE.Color(0x0b0a08);
    scene3.fog = new THREE.FogExp2(0x0b0a08, 0.004);

    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setPixelRatio(window.devicePixelRatio || 1);
    renderer.setSize(window.innerWidth, window.innerHeight);
    document.body.appendChild(renderer.domElement);

    const cam = new THREE.PerspectiveCamera(
      50, window.innerWidth / window.innerHeight, 0.1, 4000);

    let data = window.EMEM_DATA;
    if (!data) data = await fetchScene();
    const { points, span, n } = buildCloud(data);
    scene3.add(points);

    if (hud) {
      hud.querySelector("#hud-n").textContent =
        n.toLocaleString() + " signed facts · " + CFG.bands.join(" · ");
      status(CFG.subtitle || "every splat re-checks at " + CFG.responder + "/verify");
    }

    const R = span * (CFG.cameraDistance || 0.62);

    // deterministic frame stepping for headless capture (docs GIFs)
    if (window.EMEM_CAPTURE) {
      window.__renderFrame = function (i, total) {
        const a = (2 * Math.PI * i) / total;
        cam.position.set(Math.sin(a) * R,
                         R * 0.42 + Math.sin((i / total) * 2 * Math.PI) * R * 0.03,
                         Math.cos(a) * R);
        cam.lookAt(0, R * 0.14, 0);
        renderer.render(scene3, cam);
      };
      window.__renderFrame(0, 1);
      window.__eememWorldReady = true;
      return;
    }

    let t0 = null;
    function frame(t) {
      if (t0 === null) t0 = t;
      const s = (t - t0) / 1000;
      const a = s * (CFG.orbitSpeed || 0.12);
      cam.position.set(Math.sin(a) * R, R * 0.42 + Math.sin(s * 0.3) * R * 0.03,
                       Math.cos(a) * R);
      cam.lookAt(0, R * 0.14, 0);
      renderer.render(scene3, cam);
      window.__frameCount = (window.__frameCount || 0) + 1;
      requestAnimationFrame(frame);
    }
    requestAnimationFrame(frame);

    window.addEventListener("resize", () => {
      cam.aspect = window.innerWidth / window.innerHeight;
      cam.updateProjectionMatrix();
      renderer.setSize(window.innerWidth, window.innerHeight);
    });
    window.__eememWorldReady = true;
  }

  main().catch((e) => { status("error: " + e.message); console.error(e); });
})();
