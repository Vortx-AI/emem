/**
 * Minimal end-to-end smoke example: locate → recall → cite a receipt.
 *
 *   npx tsx examples/recall.ts
 *
 * Or against a local responder:
 *
 *   EMEM_BASE_URL=http://localhost:5051 npx tsx examples/recall.ts
 */

import { Client } from "../src/index.js";

async function main(): Promise<void> {
  const em = new Client();

  const loc = (await em.locate({ place: "Mount Fuji" })) as { cell64?: string };
  const cell = loc.cell64;
  if (!cell) throw new Error(`Mount Fuji did not resolve to a cell64: ${JSON.stringify(loc)}`);
  console.log(`resolved Mount Fuji → cell64 = ${cell}`);

  const facts = (await em.recall({
    cell,
    bands: ["copdem30m.elevation_mean"],
  })) as {
    facts?: Array<{ band?: string; value?: number; cid?: string; tslot?: number }>;
    receipt?: { responder_pubkey?: string; fact_cids?: string[] };
  };

  for (const f of facts.facts ?? []) {
    console.log(
      `  ${f.band ?? "?"} = ${f.value ?? "?"} ` +
        `(cid=${(f.cid ?? "").slice(0, 12)}…, tslot=${f.tslot ?? "?"})`,
    );
  }

  const pk = facts.receipt?.responder_pubkey ?? "";
  const cids = facts.receipt?.fact_cids ?? [];
  console.log(`  receipt: signed by ${pk.slice(0, 16)}… over ${cids.length} fact CID(s)`);
}

main().catch((err) => {
  console.error(err);
  const proc = (globalThis as { process?: { exit: (code: number) => void } }).process;
  proc?.exit(1);
});
