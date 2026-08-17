#!/usr/bin/env node
/* Does the browser verifier actually verify, and does it actually refuse?
 *
 * web/emem-verify-core.js is the code that tells a person whether a signed
 * fact is genuine, running in their own browser with no network. Two ways it
 * can fail, and the second is the dangerous one:
 *
 *   1. It accepts nothing. Loud, obvious, someone reports it.
 *   2. It accepts everything, or rejects sound receipts. A verifier that
 *      returns true regardless is indistinguishable from a working one until
 *      somebody forges a fact; a verifier that returns false on genuine
 *      receipts tells users their real data was tampered with.
 *
 * So a positive test alone proves nothing. Every case below that MUST be
 * rejected is a specific forgery: change the value's address, rewind the
 * clock, flip the inclusion root, or strip the proof entirely, which is the
 * attack `preimage_version 2` was introduced to make detectable.
 *
 *   node scripts/verify_core_test.cjs
 *   node scripts/verify_core_test.cjs --live   # also re-fetch from production
 */
const path = require("path");
const REPO = path.dirname(__dirname);
require(path.join(REPO, "web/emem-verify-core.js"));
const V = globalThis.ememVerify;

const enc = new TextEncoder();
const hex = (u8) => Array.from(u8).map((b) => b.toString(16).padStart(2, "0")).join("");
let failures = 0;
const check = (name, cond, detail) => {
  if (!cond) { failures++; console.log(`  FAIL ${name}${detail ? "  " + detail : ""}`); }
  else console.log(`  ok   ${name}`);
};

// 1. The primitives, against the BLAKE3 project's own vectors. If these move,
// nothing downstream means anything.
console.log("blake3, against the reference vectors:");
check('blake3("")', V.blake3Hex(enc.encode("")) ===
  "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262");
check('blake3("abc")', V.blake3Hex(enc.encode("abc")) ===
  "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85");

// 2. The emem encoders, against vectors the Rust signer emitted. This is what
// stands between us and a confident wrong answer.
console.log("\nencoders, against the Rust signer's vectors:");
check("self-test", V.selfTest() === true,
  "the JS encoders disagree with emem-attest; every digest here would be wrong");

// 3. A real receipt from production, pinned so CI does not depend on the
// network to know whether its own verifier works.
const fixture = require(path.join(REPO, "web/data/receipt-fixture.json"));
console.log("\na genuine signed receipt:");
const good = V.verifyReceipt(fixture.receipt);
check("accepts an untampered receipt", good.ok === true, good.why);
check("reports the preimage version it used", good.preimage_version >= 1);

// 4. Forgeries. Each must be refused.
console.log("\nforgeries, each of which must be refused:");
const clone = () => JSON.parse(JSON.stringify(fixture.receipt));
const tampered = {};
let r;
r = clone(); r.fact_cids[0] = "a" + r.fact_cids[0].slice(1);
tampered["the fact's content address is swapped"] = r;
r = clone(); r.request_id = "01TAMPERED000000000000000";
tampered["the request id is rewritten"] = r;
r = clone(); r.served_at = "2020-01-01T00:00:00Z";
tampered["the clock is rewound"] = r;
r = clone(); r.primitive = "emem.something_else";
tampered["the primitive is relabelled"] = r;
if (Array.isArray(fixture.receipt.merkle_proof && fixture.receipt.merkle_proof.root)) {
  r = clone(); r.merkle_proof.root[0] = (r.merkle_proof.root[0] + 1) % 256;
  tampered["the inclusion root is flipped"] = r;
  r = clone(); delete r.merkle_proof;
  tampered["the inclusion proof is stripped entirely"] = r;
}
for (const [name, rec] of Object.entries(tampered)) {
  const v = V.verifyReceipt(rec);
  check(name, v.ok === false, `accepted a forged receipt (state=${v.state})`);
}

// 5. A verifier must never claim to have checked something it did not.
console.log("\nrefusals name themselves:");
check("no receipt is not a pass", V.verifyReceipt(null).ok === false);
check("garbage is not a pass", V.verifyReceipt({ nonsense: true }).ok === false);
check("a refusal says which state it is in",
  typeof V.verifyReceipt(null).state === "string");

// 6. The card ships the verifier by splice, not by import. If the marker
// survives, or the spliced script does not evaluate, every card silently
// falls back to "the verifier did not load" and checks nothing. That failure
// is invisible from the Rust side, which only sees a longer string.
console.log("\nthe card's spliced verifier:");
const fs = require("fs");
const tpl = fs.readFileSync(path.join(REPO, "web/mcp-fact-card.html"), "utf8");
const core = fs.readFileSync(path.join(REPO, "web/emem-verify-core.js"), "utf8");
check("the template carries the splice marker", tpl.includes("/*EMEM_VERIFY_CORE*/"));
const spliced = tpl.replace("/*EMEM_VERIFY_CORE*/", core);
check("no marker survives the splice", !spliced.includes("/*EMEM_VERIFY_CORE*/"));
const m = spliced.match(/<script>([\s\S]*?)<\/script>/);
check("the spliced script block is extractable", !!m);
if (m) {
  const sandbox = {};
  try {
    new Function("globalThis", m[1]).call(sandbox, sandbox);
    const V2 = sandbox.ememVerify;
    check("the spliced verifier evaluates and exports", !!(V2 && V2.verifyReceipt));
    if (V2) {
      check("the spliced verifier passes its own self-test", V2.selfTest() === true);
      check("the spliced verifier accepts the genuine receipt",
        V2.verifyReceipt(fixture.receipt).ok === true);
      const forged = JSON.parse(JSON.stringify(fixture.receipt));
      forged.request_id = "01FORGED0000000000000000";
      check("the spliced verifier refuses a forgery",
        V2.verifyReceipt(forged).ok === false);
    }
  } catch (e) { check("the spliced script evaluates", false, String(e.message)); }
}

if (process.argv.includes("--live")) {
  console.log("\nlive: re-fetching a receipt from production");
  const https = require("https");
  const body = JSON.stringify({ cell: "defi.zb493.xuqA.zcb5f", bands: ["copdem30m.elevation_mean"] });
  const req = https.request("https://emem.dev/v1/recall",
    { method: "POST", headers: { "content-type": "application/json" } }, (res) => {
      let buf = "";
      res.on("data", (c) => (buf += c));
      res.on("end", () => {
        try {
          const v = V.verifyReceipt(JSON.parse(buf).receipt);
          check("a receipt minted just now verifies", v.ok === true, v.why);
        } catch (e) { check("live fetch parsed", false, String(e)); }
        done();
      });
    });
  req.on("error", (e) => { console.log("  (production unreachable: " + e.message + ")"); done(); });
  req.end(body);
} else { done(); }

function done() {
  console.log(failures === 0
    ? "\nverify-core: the browser verifier accepts what is genuine and refuses every forgery tried."
    : `\nverify-core: ${failures} failure(s). Do NOT ship a verifier in this state.`);
  // Both branches of the expression that used to sit here evaluated to 0, so
  // this suite could not fail the build no matter what it found. Left as a
  // plain comparison; a gate's own exit code is not the place to be clever.
  process.exit(failures === 0 ? 0 : 1);
}
