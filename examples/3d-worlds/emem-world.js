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
      throw e;
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
  function buildCloud(scene) {
    const g = SM.buildGaussians(scene, CFG);
    const n = g.count;
    const ctx = { hMin: g.hMin, hMax: g.hMax, spacingM: g.spacingM, rows: g.rows };
    if (CFG.prepare) CFG.prepare(g.rows, ctx);

    const colors = new Float32Array(3 * n);
    const c = new THREE.Color();
    for (let i = 0; i < n; i++) {
      CFG.colorize(g.rows[i], c, ctx);
      colors[3 * i] = c.r; colors[3 * i + 1] = c.g; colors[3 * i + 2] = c.b;
    }

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
      uniforms: { uViewport: { value: new THREE.Vector2(1, 1) } },
      vertexShader: [
        "attribute vec3 iPos; attribute vec3 iColor;",
        "attribute vec3 iCovA; attribute vec3 iCovB;",
        "attribute float iOpacity;",
        "uniform vec2 uViewport;",
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
        "  vec2 offsetPx = position.x * v1 * sqrt(l1) * 3.0",
        "                + position.y * v2 * sqrt(l2) * 3.0;",
        "  vColor = iColor; vOpacity = iOpacity; vXY = position.xy * 3.0;",
        "  gl_Position = clip;",
        "  gl_Position.xy += offsetPx * (2.0 / uViewport) * clip.w;",
        "}",
      ].join("\n"),
      fragmentShader: [
        "varying vec3 vColor; varying float vOpacity; varying vec2 vXY;",
        "void main(){",
        "  float r2 = dot(vXY, vXY);",
        "  float alpha = vOpacity * exp(-0.5 * r2);",
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

    return { mesh: mesh, mat: mat, sort: sortSplats,
             span: g.spanX, n: n, spacingM: g.spacingM };
  }

  // ---- scene -----------------------------------------------------------
  async function main() {
    const scene3 = new THREE.Scene();
    scene3.background = new THREE.Color(CFG.background || 0x0b0a08);
    scene3.fog = null;   // sorted alpha compositing replaces the old additive haze

    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setPixelRatio(window.devicePixelRatio || 1);
    renderer.setSize(window.innerWidth, window.innerHeight);
    document.body.appendChild(renderer.domElement);

    const cam = new THREE.PerspectiveCamera(
      50, window.innerWidth / window.innerHeight, 0.1, 4000);

    let data = window.EMEM_DATA;
    if (!data) data = await fetchScene();
    const world = buildCloud(data);
    scene3.add(world.mesh);

    function setViewport() {
      world.mat.uniforms.uViewport.value.set(
        renderer.domElement.width, renderer.domElement.height);
    }
    setViewport();

    if (hud) {
      hud.querySelector("#hud-n").textContent =
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

    // deterministic frame stepping for headless capture (docs GIFs)
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

    let t0 = null;
    function frame(t) {
      if (t0 === null) t0 = t;
      const s = (t - t0) / 1000;
      const a = s * (CFG.orbitSpeed || 0.12);
      cam.position.set(Math.sin(a) * R, R * camH + Math.sin(s * 0.3) * R * 0.03,
                       Math.cos(a) * R);
      cam.lookAt(target);
      sortIfNeeded(false);
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
    window.__eememWorldReady = true;
  }

  main().catch((e) => { status("error: " + e.message); console.error(e); });
})();
