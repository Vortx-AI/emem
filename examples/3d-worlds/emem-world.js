/* emem-world.js — render a live emem region as a 3-D gaussian splat world.
 *
 * Every splat is one cell of signed facts recalled from an emem responder,
 * and every gaussian parameter traces to a measurement: position from the
 * fact's derivation.args (lat/lng) and its height band, ground footprint
 * from the sampled grid pitch, thickness from the cell's own measured
 * sub-cell relief (elevation p90 - p10) or a std-error band, tilt from
 * measured slope and aspect, opacity from the fact's attested confidence.
 * The math lives in splat-math.js; the receipts arrive with the data and
 * any splat's fact_cid re-checks at /verify.
 *
 * Rendering is standard 3D Gaussian Splatting: each gaussian's covariance
 * Sigma = R S^2 R^T is projected to screen space with the EWA Jacobian
 * (Zwicker et al. 2001; Kerbl et al. 2023), Sigma' = J W Sigma W^T J^T,
 * drawn as an instanced quad over the 3-sigma ellipse, and composited
 * back-to-front with premultiplied alpha after a CPU depth sort.
 *
 * Usage: set window.EMEM_WORLD before loading this script (see the
 * template HTML files). If window.EMEM_DATA is set (a pre-fetched scene
 * object), no network calls are made — that path is used for headless
 * capture and for offline demos.
 *
 * Interaction: the world is orbit/zoom/pan draggable, auto-rotates while
 * idle, and exposes window.__ememWorld (also CFG.onReady(api)) so a host
 * page can pick a splat, recolour by a different signed band, rebuild with
 * new exaggeration/shape, or drape a georeferenced satellite basemap under
 * the signed geometry. The deterministic capture path (EMEM_CAPTURE) is
 * unchanged, so the README GIFs stay byte-for-byte reproducible.
 */
(function () {
  "use strict";

  const CFG = window.EMEM_WORLD;
  const SM = window.EMEM_SPLAT_MATH;
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
  function allBands() {
    const s = CFG.shape || {};
    const extra = [].concat(s.reliefBands || [], s.aspectBands || [],
                            s.slopeBand ? [s.slopeBand] : [],
                            s.sigmaBand ? [s.sigmaBand] : []);
    const seen = {};
    return CFG.bands.concat(extra).filter((b) => (seen[b] ? false : (seen[b] = true)));
  }

  // a fully cold batch must materialize inside the gateway timeout; on a
  // timeout the batch splits in half and retries, down to 8 cells
  async function recallBatch(scene, cells, band, depth) {
    let d;
    try {
      d = await post("/v1/recall_many", { cells: cells, bands: [band] });
    } catch (e) {
      if (cells.length > 8) {
        const mid = cells.length >> 1;
        await recallBatch(scene, cells.slice(0, mid), band, (depth || 0) + 1);
        await recallBatch(scene, cells.slice(mid), band, (depth || 0) + 1);
        return;
      }
      if ((depth || 0) < 6) {                       // small cold batch: wait, retry
        await new Promise((r) => setTimeout(r, 3000));
        return recallBatch(scene, cells, band, (depth || 0) + 1);
      }
      // still failing after splits and retries: skip — those cells simply
      // have no fact and no splat this run; a warm reload fills them
      console.warn("emem-world: skipped", cells.length, "cells for", band, e);
      scene._skipped = (scene._skipped || 0) + cells.length;
      return;
    }
    for (const [cell, entry] of Object.entries(d.by_cell || {})) {
      for (const f of entry.facts || []) {
        if (f.value === null || f.value === undefined) continue;
        const rec = scene.cells[cell] || (scene.cells[cell] = {});
        const args = (f.derivation || {}).args || [];
        if (rec.lat === undefined && args.length > 1) {
          rec.lat = args[0]; rec.lng = args[1];
        }
        rec[f.band] = f.value;
        const cids = rec._cids || (rec._cids = {});
        if (f.fact_cid) cids[f.band] = f.fact_cid;
        if (typeof f.confidence === "number") {
          rec._conf = rec._conf === undefined
            ? f.confidence : Math.min(rec._conf, f.confidence);
        }
        scene.fact_count += 1;
      }
    }
  }

  async function fetchScene() {
    status("sampling " + CFG.maxCells + " cells over the bbox…");
    const qr = await post("/v1/query_region", {
      bbox: CFG.bbox, bands: [CFG.bands[0]], max_cells: CFG.maxCells,
    });
    const cells = qr.receipt.cells;
    const bands = allBands();
    const batch = CFG.batchSize || (bands.length > 2 ? 128 : 256);
    const scene = { cells: {}, fact_count: 0 };
    for (const band of bands) {
      for (let i = 0; i < cells.length; i += batch) {
        status(band + " — recalling cells " + i + "–" +
               Math.min(i + batch, cells.length) + " of " + cells.length +
               " (first pass materializes and signs)…");
        await recallBatch(scene, cells.slice(i, i + batch), band, 0);
      }
    }
    return scene;
  }

  // ---- splat cloud -------------------------------------------------------
  function buildCloud(scene, cfg) {
    const g = SM.buildGaussians(scene, cfg);
    const n = g.count;
    const ctx = { hMin: g.hMin, hMax: g.hMax, spacingM: g.spacingM, rows: g.rows };
    if (cfg.prepare) cfg.prepare(g.rows, ctx);

    const colors = new Float32Array(3 * n);
    const c = new THREE.Color();
    function paint(colorize) {
      for (let i = 0; i < n; i++) {
        colorize(g.rows[i], c, ctx);
        colors[3 * i] = c.r; colors[3 * i + 1] = c.g; colors[3 * i + 2] = c.b;
      }
    }
    paint(cfg.colorize);

    // one unit quad, instanced once per gaussian; position.xy is the corner
    const geo = new THREE.InstancedBufferGeometry();
    geo.setAttribute("position", new THREE.BufferAttribute(new Float32Array([
      -1, -1, 0,  1, -1, 0,  1, 1, 0,  -1, 1, 0]), 3));
    geo.setIndex([0, 1, 2, 0, 2, 3]);
    const iPos = new THREE.InstancedBufferAttribute(new Float32Array(3 * n), 3);
    const iColor = new THREE.InstancedBufferAttribute(new Float32Array(3 * n), 3);
    const iCovA = new THREE.InstancedBufferAttribute(new Float32Array(3 * n), 3);
    const iCovB = new THREE.InstancedBufferAttribute(new Float32Array(3 * n), 3);
    const iOpacity = new THREE.InstancedBufferAttribute(new Float32Array(n), 1);
    geo.setAttribute("iPos", iPos);
    geo.setAttribute("iColor", iColor);
    geo.setAttribute("iCovA", iCovA);
    geo.setAttribute("iCovB", iCovB);
    geo.setAttribute("iOpacity", iOpacity);
    geo.instanceCount = n;

    const mat = new THREE.ShaderMaterial({
      transparent: true,
      depthTest: false,
      depthWrite: false,
      blending: THREE.CustomBlending,
      blendSrc: THREE.OneFactor,
      blendDst: THREE.OneMinusSrcAlphaFactor,
      blendSrcAlpha: THREE.OneFactor,
      blendDstAlpha: THREE.OneMinusSrcAlphaFactor,
      uniforms: {
        uViewport: { value: new THREE.Vector2(1, 1) },
        uSplatScale: { value: 1.0 },
        uOpacityScale: { value: 1.0 },
      },
      vertexShader: [
        "attribute vec3 iPos; attribute vec3 iColor;",
        "attribute vec3 iCovA; attribute vec3 iCovB;",
        "attribute float iOpacity;",
        "uniform vec2 uViewport; uniform float uSplatScale;",
        "varying vec3 vColor; varying float vOpacity; varying vec2 vXY;",
        "void main(){",
        "  vec4 cam = modelViewMatrix * vec4(iPos, 1.0);",
        "  vec4 clip = projectionMatrix * cam;",
        "  if (cam.z > -0.1) { gl_Position = vec4(0.0, 0.0, 2.0, 1.0); return; }",
        // world covariance, symmetric, from its upper triangle
        "  mat3 Vrk = mat3(iCovA.x, iCovA.y, iCovA.z,",
        "                  iCovA.y, iCovB.x, iCovB.y,",
        "                  iCovA.z, iCovB.y, iCovB.z);",
        // camera rotation W and EWA Jacobian J at the gaussian centre
        "  mat3 W = mat3(modelViewMatrix);",
        "  mat3 Wt = mat3(W[0][0], W[1][0], W[2][0],",
        "                 W[0][1], W[1][1], W[2][1],",
        "                 W[0][2], W[1][2], W[2][2]);",
        "  float focal = 0.5 * uViewport.y * projectionMatrix[1][1];",
        "  float invZ = 1.0 / cam.z;",
        "  mat3 J = mat3(focal * invZ, 0.0, 0.0,",
        "                0.0, focal * invZ, 0.0,",
        "                -focal * cam.x * invZ * invZ, -focal * cam.y * invZ * invZ, 0.0);",
        "  mat3 Jt = mat3(J[0][0], J[1][0], J[2][0],",
        "                 J[0][1], J[1][1], J[2][1],",
        "                 J[0][2], J[1][2], J[2][2]);",
        "  mat3 cov2m = J * (W * Vrk * Wt) * Jt;",
        // screen-space 2x2 with the 0.3 px^2 low-pass of Kerbl et al.
        "  float a = cov2m[0][0] + 0.3;",
        "  float b = 0.5 * (cov2m[0][1] + cov2m[1][0]);",
        "  float d = cov2m[1][1] + 0.3;",
        "  float mid = 0.5 * (a + d);",
        "  float disc = sqrt(max(0.0, mid * mid - (a * d - b * b)));",
        "  float l1 = mid + disc;",
        "  float l2 = max(mid - disc, 0.05);",
        "  vec2 v1 = (abs(b) > 1e-6) ? normalize(vec2(b, l1 - a))",
        "                            : ((a >= d) ? vec2(1.0, 0.0) : vec2(0.0, 1.0));",
        "  vec2 v2 = vec2(-v1.y, v1.x);",
        "  vec2 offsetPx = (position.x * v1 * sqrt(l1) * 3.0",
        "                +  position.y * v2 * sqrt(l2) * 3.0) * uSplatScale;",
        "  vColor = iColor; vOpacity = iOpacity; vXY = position.xy * 3.0;",
        "  gl_Position = clip;",
        "  gl_Position.xy += offsetPx * (2.0 / uViewport) * clip.w;",
        "}",
      ].join("\n"),
      fragmentShader: [
        "varying vec3 vColor; varying float vOpacity; varying vec2 vXY;",
        "uniform float uOpacityScale;",
        "void main(){",
        "  float r2 = dot(vXY, vXY);",
        "  float alpha = vOpacity * uOpacityScale * exp(-0.5 * r2);",
        "  if (alpha < 0.0039) discard;",
        "  gl_FragColor = vec4(vColor * alpha, alpha);",   // premultiplied
        "}",
      ].join("\n"),
    });

    const mesh = new THREE.Mesh(geo, mat);
    mesh.frustumCulled = false;

    // back-to-front CPU sort; instanced buffers are rewritten in depth order
    const order = new Array(n);
    for (let i = 0; i < n; i++) order[i] = i;
    function sortSplats(camPos, fwd) {
      const key = new Float32Array(n);
      for (let i = 0; i < n; i++) {
        key[i] = fwd.x * (g.positions[3 * i] - camPos.x)
               + fwd.y * (g.positions[3 * i + 1] - camPos.y)
               + fwd.z * (g.positions[3 * i + 2] - camPos.z);
      }
      order.sort((p, q) => key[q] - key[p]);   // farthest first
      for (let k = 0; k < n; k++) {
        const i = order[k];
        iPos.array[3 * k] = g.positions[3 * i];
        iPos.array[3 * k + 1] = g.positions[3 * i + 1];
        iPos.array[3 * k + 2] = g.positions[3 * i + 2];
        iColor.array[3 * k] = colors[3 * i];
        iColor.array[3 * k + 1] = colors[3 * i + 1];
        iColor.array[3 * k + 2] = colors[3 * i + 2];
        iCovA.array[3 * k] = g.cov6[6 * i];
        iCovA.array[3 * k + 1] = g.cov6[6 * i + 1];
        iCovA.array[3 * k + 2] = g.cov6[6 * i + 2];
        iCovB.array[3 * k] = g.cov6[6 * i + 3];
        iCovB.array[3 * k + 1] = g.cov6[6 * i + 4];
        iCovB.array[3 * k + 2] = g.cov6[6 * i + 5];
        iOpacity.array[k] = g.opacities[i];
      }
      iPos.needsUpdate = true; iColor.needsUpdate = true;
      iCovA.needsUpdate = true; iCovB.needsUpdate = true;
      iOpacity.needsUpdate = true;
    }

    return {
      mesh: mesh, mat: mat, sort: sortSplats, g: g, ctx: ctx, colors: colors,
      recolor: paint, span: g.spanX, n: n, spacingM: g.spacingM,
    };
  }

  // ---- georeferenced satellite basemap ---------------------------------
  // The signed geometry rises above a plane textured with real satellite
  // imagery for the same bounding box. The imagery is reference only, from
  // an external provider, and is never confused with the signed facts.
  function basemapExtent(g) {
    let latMin = Infinity, latMax = -Infinity, lngMin = Infinity, lngMax = -Infinity;
    for (const r of g.rows) {
      if (r.lat < latMin) latMin = r.lat; if (r.lat > latMax) latMax = r.lat;
      if (r.lng < lngMin) lngMin = r.lng; if (r.lng > lngMax) lngMax = r.lng;
    }
    const U = g.unitsPerMeter, MLAT = SM.M_PER_DEG_LAT;
    const mLng = MLAT * Math.cos(g.lat0 * Math.PI / 180);
    // half a cell of padding so the plane reaches the outer splat centres
    const pad = (g.spacingM * 0.5) / MLAT;
    const padLng = (g.spacingM * 0.5) / mLng;
    const x0 = (lngMin - padLng - g.lng0) * mLng * U;
    const x1 = (lngMax + padLng - g.lng0) * mLng * U;
    const z0 = -(latMax + pad - g.lat0) * MLAT * U;
    const z1 = -(latMin - pad - g.lat0) * MLAT * U;
    return {
      bbox: [lngMin - padLng, latMin - pad, lngMax + padLng, latMax + pad],
      cx: (x0 + x1) / 2, cz: (z0 + z1) / 2,
      w: Math.abs(x1 - x0), h: Math.abs(z1 - z0),
    };
  }

  // ---- interaction: orbit / zoom / pan, auto-rotate when idle -----------
  function makeControls(dom, cam, state) {
    let dragging = false, mode = 0, px = 0, py = 0;
    function onDown(e) {
      dragging = true; mode = (e.button === 2 || e.shiftKey) ? 1 : 0;
      px = e.clientX; py = e.clientY; state.idleAt = Infinity;
      dom.setPointerCapture && dom.setPointerCapture(e.pointerId);
    }
    function onMove(e) {
      if (!dragging) return;
      const dx = e.clientX - px, dy = e.clientY - py; px = e.clientX; py = e.clientY;
      if (mode === 0) {
        state.az -= dx * 0.005;
        state.pol = Math.max(0.04, Math.min(Math.PI - 0.04, state.pol - dy * 0.005));
      } else {
        const s = state.r * 0.0015;
        const right = new THREE.Vector3().setFromMatrixColumn(cam.matrix, 0);
        const up = new THREE.Vector3().setFromMatrixColumn(cam.matrix, 1);
        state.target.addScaledVector(right, -dx * s).addScaledVector(up, dy * s);
      }
    }
    function onUp(e) {
      dragging = false; state.idleAt = state.now + 2500;
      dom.releasePointerCapture && e.pointerId != null && dom.releasePointerCapture(e.pointerId);
    }
    function onWheel(e) {
      e.preventDefault();
      state.r = Math.max(state.rMin, Math.min(state.rMax, state.r * Math.exp(e.deltaY * 0.0012)));
      state.idleAt = state.now + 2500;
    }
    dom.addEventListener("pointerdown", onDown);
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    dom.addEventListener("wheel", onWheel, { passive: false });
    dom.addEventListener("contextmenu", (e) => e.preventDefault());
  }

  // ---- scene -----------------------------------------------------------
  async function main() {
    const scene3 = new THREE.Scene();
    scene3.background = new THREE.Color(CFG.background || 0x0b0a08);
    scene3.fog = null;   // sorted alpha compositing replaces the old additive haze

    const renderer = new THREE.WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
    renderer.setPixelRatio(window.devicePixelRatio || 1);
    renderer.setSize(window.innerWidth, window.innerHeight);
    document.body.appendChild(renderer.domElement);

    const cam = new THREE.PerspectiveCamera(
      50, window.innerWidth / window.innerHeight, 0.1, 4000);

    let data = window.EMEM_DATA;
    if (!data) data = await fetchScene();
    let curCfg = Object.assign({}, CFG);
    let world = buildCloud(data, curCfg);
    scene3.add(world.mesh);

    function setViewport() {
      world.mat.uniforms.uViewport.value.set(
        renderer.domElement.width, renderer.domElement.height);
    }
    setViewport();

    if (hud) {
      const hn = hud.querySelector("#hud-n");
      if (hn) hn.textContent =
        world.n.toLocaleString() + " cells · " +
        (data.fact_count || 0).toLocaleString() + " signed facts · " +
        CFG.bands.join(" · ");
      status(CFG.subtitle || "every splat re-checks at " + CFG.responder + "/verify");
    }

    const R = Math.max(world.span, 2) * (CFG.cameraDistance || 0.62);
    const camH = CFG.cameraHeight !== undefined ? CFG.cameraHeight : 0.42;
    const lastFwd = new THREE.Vector3(0, 0, 0);
    const fwd = new THREE.Vector3();
    const target = new THREE.Vector3(0, R * 0.14, 0);
    function sortIfNeeded(force) {
      fwd.copy(target).sub(cam.position).normalize();
      if (force || lastFwd.dot(fwd) < 0.9995) {
        world.sort(cam.position, fwd);
        lastFwd.copy(fwd);
      }
    }

    // deterministic frame stepping for headless capture (docs GIFs).
    // Unchanged from the original fixed-orbit engine so the README GIFs
    // stay byte-for-byte reproducible.
    if (window.EMEM_CAPTURE) {
      window.__renderFrame = function (i, total) {
        const a = (2 * Math.PI * i) / total;
        cam.position.set(Math.sin(a) * R,
                         R * camH + Math.sin((i / total) * 2 * Math.PI) * R * 0.03,
                         Math.cos(a) * R);
        cam.lookAt(target);
        sortIfNeeded(true);          // sort every frame: deterministic output
        renderer.render(scene3, cam);
      };
      window.__renderFrame(0, 1);
      window.__eememWorldReady = true;
      return;
    }

    // ---- interactive orbit ------------------------------------------------
    const st = {
      az: 0, pol: Math.acos(Math.max(-1, Math.min(1, camH))), r: R,
      rMin: R * 0.12, rMax: R * 4, target: target.clone(),
      now: 0, idleAt: 2500, autoSpeed: CFG.orbitSpeed || 0.12,
    };
    makeControls(renderer.domElement, cam, st);

    function applyCam() {
      const sp = Math.sin(st.pol), cp = Math.cos(st.pol);
      cam.position.set(
        st.target.x + st.r * sp * Math.sin(st.az),
        st.target.y + st.r * cp,
        st.target.z + st.r * sp * Math.cos(st.az));
      cam.lookAt(st.target);
    }

    let t0 = null;
    function frame(t) {
      if (t0 === null) t0 = t;
      st.now = t;
      if (t > st.idleAt) st.az += st.autoSpeed * 0.016;   // idle auto-rotate
      applyCam();
      fwd.copy(st.target).sub(cam.position).normalize();
      if (lastFwd.dot(fwd) < 0.9995) { world.sort(cam.position, fwd); lastFwd.copy(fwd); }
      renderer.render(scene3, cam);
      window.__frameCount = (window.__frameCount || 0) + 1;
      requestAnimationFrame(frame);
    }
    requestAnimationFrame(frame);

    window.addEventListener("resize", () => {
      cam.aspect = window.innerWidth / window.innerHeight;
      cam.updateProjectionMatrix();
      renderer.setSize(window.innerWidth, window.innerHeight);
      setViewport();
    });

    // ---- host API --------------------------------------------------------
    // Pick the front-most splat whose projected centre is within `tolPx` of
    // the click. n is <= ~1024 so an O(n) projection per click is trivial.
    const proj = new THREE.Vector3();
    function pick(clientX, clientY, tolPx) {
      const rect = renderer.domElement.getBoundingClientRect();
      const mx = clientX - rect.left, my = clientY - rect.top;
      const tol = tolPx || 16;
      let best = -1, bestDepth = Infinity;
      for (let i = 0; i < world.n; i++) {
        proj.set(world.g.positions[3 * i], world.g.positions[3 * i + 1],
                 world.g.positions[3 * i + 2]).project(cam);
        if (proj.z < -1 || proj.z > 1) continue;
        const sx = (proj.x * 0.5 + 0.5) * rect.width;
        const sy = (-proj.y * 0.5 + 0.5) * rect.height;
        const dpx = Math.hypot(sx - mx, sy - my);
        if (dpx < tol && proj.z < bestDepth) { best = i; bestDepth = proj.z; }
      }
      if (best < 0) return null;
      const rect2 = renderer.domElement.getBoundingClientRect();
      proj.set(world.g.positions[3 * best], world.g.positions[3 * best + 1],
               world.g.positions[3 * best + 2]).project(cam);
      return {
        index: best, cellId: world.g.cellIds[best], row: world.g.rows[best],
        screenX: (proj.x * 0.5 + 0.5) * rect2.width + rect2.left,
        screenY: (-proj.y * 0.5 + 0.5) * rect2.height + rect2.top,
      };
    }

    let basemapMesh = null;
    function setBasemap(url, opts) {
      opts = opts || {};
      if (basemapMesh) { scene3.remove(basemapMesh); basemapMesh.geometry.dispose();
        if (basemapMesh.material.map) basemapMesh.material.map.dispose();
        basemapMesh.material.dispose(); basemapMesh = null; }
      if (!url) return Promise.resolve(false);
      const ext = basemapExtent(world.g);
      return new Promise((resolve) => {
        new THREE.TextureLoader().setCrossOrigin("anonymous").load(url, (tex) => {
          if (tex.colorSpace !== undefined) tex.colorSpace = THREE.SRGBColorSpace;
          const geo = new THREE.PlaneGeometry(ext.w, ext.h);
          const mat = new THREE.MeshBasicMaterial({
            map: tex, transparent: true,
            opacity: opts.opacity !== undefined ? opts.opacity : 0.9,
            depthWrite: false, side: THREE.DoubleSide,
          });
          basemapMesh = new THREE.Mesh(geo, mat);
          basemapMesh.rotation.x = -Math.PI / 2;      // lie in the xz ground plane
          basemapMesh.position.set(ext.cx, opts.y !== undefined ? opts.y : -0.02, ext.cz);
          basemapMesh.renderOrder = -1;               // paint under the splats
          scene3.add(basemapMesh);
          resolve(true);
        }, undefined, () => resolve(false));
      });
    }

    const api = {
      THREE: THREE, scene: scene3, camera: cam, renderer: renderer,
      cfg: CFG, ctx: world.ctx, g: world.g,
      count: world.n, factCount: data.fact_count || 0,
      pick: pick,
      recolor: function (fn) {
        curCfg.colorize = fn || curCfg.colorize;
        world.recolor(curCfg.colorize); lastFwd.set(0, 0, 0);   // force resort next frame
      },
      // rebuild the gaussians with a patched cfg (e.g. a new
      // heightExaggeration or shape). Re-runs the exact same signed-fact
      // math; only the requested parameters change.
      rebuild: function (patch) {
        Object.assign(curCfg, patch || {});
        scene3.remove(world.mesh);
        world.mesh.geometry.dispose(); world.mat.dispose();
        world = buildCloud(data, curCfg);
        scene3.add(world.mesh);
        setViewport(); lastFwd.set(0, 0, 0);
        api.g = world.g; api.ctx = world.ctx; api.count = world.n;
      },
      setSplatScale: function (s) { world.mat.uniforms.uSplatScale.value = s; },
      setOpacityScale: function (o) { world.mat.uniforms.uOpacityScale.value = o; },
      setBasemap: setBasemap, basemapExtent: function () { return basemapExtent(world.g); },
      resetView: function () { st.az = 0; st.pol = Math.acos(Math.max(-1, Math.min(1, camH)));
        st.r = R; st.target.copy(target); st.idleAt = st.now + 2500; },
      focus: function (i) {
        st.target.set(world.g.positions[3 * i], world.g.positions[3 * i + 1],
                      world.g.positions[3 * i + 2]);
        st.r = Math.max(st.rMin, R * 0.35); st.idleAt = st.now + 6000;
      },
      spin: function (on) { st.idleAt = on ? 0 : Infinity; },
    };
    window.__ememWorld = api;
    window.__eememWorldReady = true;
    if (typeof CFG.onReady === "function") CFG.onReady(api);
  }

  main().catch((e) => { status("error: " + e.message); console.error(e); });
})();
