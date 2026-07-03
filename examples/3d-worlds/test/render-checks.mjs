/* render-checks.mjs — pixel-level correctness checks for the splat renderer.
 *
 *   node test/render-checks.mjs          (run from examples/3d-worlds/)
 *
 * Check 1, gaussian profile: a single isotropic gaussian is rendered and the
 * image intensity around its peak must follow I(r) = I0 exp(-r^2 / 2 s^2):
 * the log-intensity vs r^2 fit must be linear (R^2 > 0.98) and the fitted
 * sigma must agree between the horizontal and vertical axes within 5 %
 * (circularity — an isotropic covariance must project to a circle head-on).
 *
 * Check 2, compositing order: two nearly concentric gaussians (near red,
 * far blue) are rendered from both sides of the orbit. Back-to-front
 * premultiplied compositing means the near splat dominates the peak pixel:
 * red wins from the first side, blue after a half orbit. Additive blending
 * (the old renderer) or an unsorted draw fails this.
 */
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { inflateSync } from "node:zlib";

const HERE = dirname(fileURLToPath(import.meta.url));
const work = mkdtempSync(join(tmpdir(), "emem-render-checks-"));
let failures = 0;

function check(ok, what) {
  console.log((ok ? "ok   " : "FAIL ") + what);
  if (!ok) failures++;
}

// ---- minimal PNG decode (truecolor 8-bit, filters 0-4) --------------------
function decodePng(buf) {
  let pos = 8, w = 0, h = 0, bpp = 0, idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos), type = buf.toString("ascii", pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === "IHDR") {
      w = data.readUInt32BE(0); h = data.readUInt32BE(4);
      const depth = data[8], color = data[9];
      if (depth !== 8 || (color !== 6 && color !== 2)) throw new Error("unsupported PNG " + depth + "/" + color);
      bpp = color === 6 ? 4 : 3;
    } else if (type === "IDAT") idat.push(data);
    else if (type === "IEND") break;
    pos += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = w * bpp, out = Buffer.alloc(h * stride);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const prev = y > 0 ? out.subarray((y - 1) * stride, y * stride) : null;
    const cur = out.subarray(y * stride, (y + 1) * stride);
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? cur[x - bpp] : 0;
      const b = prev ? prev[x] : 0;
      const c = prev && x >= bpp ? prev[x - bpp] : 0;
      let v = line[x];
      if (f === 1) v += a;
      else if (f === 2) v += b;
      else if (f === 3) v += (a + b) >> 1;
      else if (f === 4) {
        const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      cur[x] = v & 0xff;
    }
  }
  return { w, h, bpp, px: out,
           at(x, y) { const o = (y * w + x) * bpp; return [out[o], out[o + 1], out[o + 2]]; } };
}

function capture(scene, png, frame, total) {
  writeFileSync(join(work, "scene.json"), JSON.stringify(scene));
  execFileSync(process.execPath, [
    join(HERE, "..", "capture.mjs"),
    "--template", "test/render-test.html",
    "--scene", join(work, "scene.json"),
    "--png", png, "--frame", String(frame), "--total", String(total),
    "--size", "900x900",
  ], { stdio: ["ignore", "ignore", "inherit"] });
  return decodePng(readFileSync(png));
}

function peak(img) {
  let best = -1, bx = 0, by = 0;
  for (let y = 2; y < img.h - 2; y += 1) {
    for (let x = 2; x < img.w - 2; x += 1) {
      const [r, g, b] = img.at(x, y);
      if (r + g + b > best) { best = r + g + b; bx = x; by = y; }
    }
  }
  return { x: bx, y: by, v: best };
}

// least-squares fit of ln(I) = c - r^2 / (2 s^2) along one axis through the peak
function fitSigma(img, px, py, dx, dy, bg) {
  const xs = [], ys = [];
  for (let r = 0; r < 400; r++) {
    const x = px + dx * r, y = py + dy * r;
    if (x < 0 || y < 0 || x >= img.w || y >= img.h) break;
    const [R, G, B] = img.at(x, y);
    const I = (R + G + B) / 3 - bg;
    if (I < 12) break;                 // stay well above the background
    xs.push(r * r); ys.push(Math.log(I));
  }
  const n = xs.length;
  const mx = xs.reduce((a, b) => a + b, 0) / n, my = ys.reduce((a, b) => a + b, 0) / n;
  let sxy = 0, sxx = 0, syy = 0;
  for (let i = 0; i < n; i++) {
    sxy += (xs[i] - mx) * (ys[i] - my); sxx += (xs[i] - mx) ** 2; syy += (ys[i] - my) ** 2;
  }
  const slope = sxy / sxx;                       // = -1 / (2 sigma^2)
  const r2 = (sxy * sxy) / (sxx * syy);
  return { sigma: Math.sqrt(-1 / (2 * slope)), r2, n };
}

// ---- check 1: radial profile ----------------------------------------------
{
  // se = 60 m: with the 120 m default spacing of a single cell,
  // sigma_ground = 60 m, so this is a perfect sphere
  const scene = { fact_count: 1, cells: { c0: { lat: 0, lng: 0, h: 0, tag: 0, se: 60 } } };
  const img = capture(scene, join(work, "radial.png"), 0, 1);
  const p = peak(img);
  const bg = 10;                                  // #0b0a08 ~ (11,10,8)
  const right = fitSigma(img, p.x, p.y, 1, 0, bg);
  const left = fitSigma(img, p.x, p.y, -1, 0, bg);
  const down = fitSigma(img, p.x, p.y, 0, 1, bg);
  const up = fitSigma(img, p.x, p.y, 0, -1, bg);
  const sH = (right.sigma + left.sigma) / 2, sV = (down.sigma + up.sigma) / 2;
  for (const [name, f] of [["right", right], ["left", left], ["down", down], ["up", up]]) {
    check(f.n > 20 && f.r2 > 0.98,
          "profile " + name + " is gaussian in r^2 (R^2=" + f.r2.toFixed(4) +
          ", " + f.n + " samples, sigma=" + f.sigma.toFixed(1) + "px)");
  }
  check(Math.abs(sH / sV - 1) < 0.05,
        "isotropic splat projects to a circle (sigmaH/sigmaV=" + (sH / sV).toFixed(3) + ")");
}

// ---- check 2: compositing order --------------------------------------------
{
  // spacing 111 m -> sigma_ground 55.66 m; se matches it (spheres)
  const scene = { fact_count: 2, cells: {
    near: { lat: -0.0005, lng: 0, h: 0, tag: 1, se: 55.66 },   // red, closer at frame 0
    far:  { lat:  0.0005, lng: 0, h: 0, tag: 2, se: 55.66 },   // blue
  } };
  // the splats overlap rather than coincide on screen, so the near one wins
  // the peak by ~2.6x, not by the full (1-alpha) attenuation; additive or
  // unsorted rendering gives a symmetric ~1x, which 2x cleanly rejects
  const a = capture(scene, join(work, "sort0.png"), 0, 2);
  const pa = peak(a); const ca = a.at(pa.x, pa.y);
  check(ca[0] > 2 * ca[2],
        "front-facing side: near red splat wins the peak pixel (r=" + ca[0] + " b=" + ca[2] + ")");
  const b = capture(scene, join(work, "sort1.png"), 1, 2);
  const pb = peak(b); const cb = b.at(pb.x, pb.y);
  check(cb[2] > 2 * cb[0],
        "after half an orbit: blue splat is now in front (r=" + cb[0] + " b=" + cb[2] + ")");
}

rmSync(work, { recursive: true, force: true });
if (failures) { console.error(failures + " check(s) failed"); process.exit(1); }
console.log("all render checks passed");
