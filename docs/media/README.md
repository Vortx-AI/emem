# emem in pictures

Each picture on this page answers one question about emem. The values come from live calls to emem.dev on
7 November 2026, or from the code. Read the pictures in order. They follow one fact from a place name to a check
by a second agent, then show notes, the guard and `/v1/ask`. Each section links to the page that explains it.

The rules for these pictures are in [visual-language.md](visual-language.md). The scripts and the saved
responses are in [stills/src/](stills/src/) and [films/src/](films/src/).

## What is emem, in one live fact?

A place name becomes a cell. emem signs a fact at that cell. Another agent gets a token and checks it offline.

<picture>
<source media="(max-width: 700px)" srcset="stills/one-token-hero-phone.webp">
<img src="stills/one-token-hero.webp" alt="Live reads of emem.dev on 7 November 2026, in three steps. 1: the name Mount Fuji resolves to the cell defi.zb592.nUkO.zEzE, ringed on a Sentinel-2 image of the summit crater from 12 December 2025. 2: emem signs one fact there: copdem30m.elevation_mean 3,539.04 m, read from a Copernicus GLO-30 DEM tile, fact id qejpz7cg...bplq, signed 28 September 2026. 3: another agent gets the 83-character emem:fact token. Resolve gives the same fact id and value. The receipt signature is valid offline. Log entry 2,463,431 of 3,248,898 proves with 22 hashes. One changed character of the fact id fails. Contains modified Copernicus Sentinel data 2025.">
</picture>

More: [intro.md](../intro.md).

## Do two names for one place give the same cell?

Only when they resolve to the same point. A different point can give a different cell and a different value.

<img src="stills/one-place-many-names.webp" alt="Five phrasings of Mount Fuji sent to locate on 7 November 2026 gave four cells. "Mount Fuji" and "mount fuji" gave defi.zb592.nUkO.zEzE, elevation 3539.04 m. "Fujisan" gave the Wikidata point 390 m away, 3766.73 m. The coordinates 35.3606, 138.7274 gave the next cell, 10 m from that point, 3768.41 m. "Fuji-san, Japan" gave a shrine 13.3 km away, 871.37 m, with disambiguation_required true. A map shows the points. The emem:fact: token for the first cell resolves to the same signed bytes for any agent. All four receipts check valid offline." width="540">

More: [start-here.md](../start-here.md).

## What does one recall return at one cell?

It returns the signed facts at the cell, from several sources, with one receipt over all the fact ids.

<img src="stills/recall-one-cell.webp" alt="Still from live emem.dev calls on 7 November 2026. A 10 km Sentinel-2 crop of Lake Geneva from 12 October 2026, with a ring on cell defi.zb610.tAhU.hejI at 46.4500 N, 6.5500 E. One recall returned 8 facts from 6 sources, oldest first: climate class Cfb (30 Oct 2018), elevation 368 m (30 Apr 2021), water occurrence 98 percent (1 Jan 2022), land cover class 80, permanent water bodies (28 Oct 2022), NDVI 0.055 with pixel class Water, so not a vegetation reading, and NDWI 0.226 (15 Oct 2026), precipitation 0 mm and air temperature 9.9 degrees C (7 Nov 2026, 09:00 UTC). Green dots mark the 3 facts that are recomputable from the source: elevation, NDVI and NDWI. One receipt covers the 8 fact ids; signature valid, merkle proof valid." width="540">

More: [agents.md](../agents.md).

## If one bit of a fact changes, does its id change?

Yes. The id is the BLAKE3 hash of the canonical CBOR bytes of the fact.

[![Film poster. A grid of 743 squares, one for each byte of the Mount Fuji elevation fact. The band name, the cell64 and the 4 value bytes are in colour, and one value byte is red. A card shows the value bytes 45 5d 30 ad change to 45 5d 30 ac, and 3539.0422 m change to 3539.0420 m.](films/emem-one-byte-poster.webp)](films/emem-one-byte.mp4)

Silent film, 33 seconds. Select the picture to play the MP4, or get the [WebM](films/emem-one-byte.webm).
More: [whitepaper.md](../whitepaper.md), section 5.

## What bytes does a receipt signature cover?

A fixed header and the receipt fields, in a set order. A change of one character makes the check fail.

<img src="stills/receipt-preimage.webp" alt="Still from live emem.dev reads on 7 November 2026 at 15:44 UTC. One recall of elevation at Mount Fuji, 3539.04 m, with its receipt preimage rebuilt offline. The stream is 328 bytes. It opens with emem.preimage.v1, a zero byte, u32 7 and the word receipt. Then seven segments follow, each a tag, a u32 length and bytes: 0x01 request_id 01M6VAY5SE7K7ARPYCPG02760R, 31 bytes. 0x02 served_at 2026-11-07T15:44:50Z, 25 bytes. 0x06 manifest_hex, the BLAKE3 of four manifest ids, 69 bytes. 0x07 primitive emem.recall, 16 bytes. 0x08 cells, a list of one cell, defi.zb592.nUkO.zEzE, 29 bytes. 0x09 fact_cids, a list of one fact, qejpz7cg and so on, 61 bytes. 0x0b merkle_hex, the BLAKE3 of the inclusion proof, 69 bytes. The segments 0x03 scope, 0x04 as_of, 0x05 edges and 0x0a field are not in this read, so they are not written. The BLAKE3 digest of the stream is 0ea5c613 and so on, the same digest that /v1/verify_receipt reports. The Ed25519 signature checks as valid offline with the key 777er3yi from /.well-known/emem.json. When served_at changes by one character, from 15:44:50Z to 15:44:51Z, the digest becomes 5cf49f72 and so on, and the same signature fails." width="540">

More: [protocol.md](../protocol.md), section 7.2.

## Can a second agent check a fact from a token?

Yes. It resolves the token and checks the receipt offline. It does not have to trust the first agent.

[![Film poster. Agent B resolves the emem:fact token for Mount Fuji: the same fact_cid and 3539.04 m. verify_receipt returns signature_valid true, merkle_proof_valid true and valid. A signature shows who signed. It does not show that the value is true.](films/emem-fact-to-check-poster.webp)](films/emem-fact-to-check.mp4)

Silent film, 33 seconds. Select the picture to play the MP4, or get the [WebM](films/emem-fact-to-check.webm).
More: [start-here.md](../start-here.md).

## Is this fact in the log, and does the log only grow?

An inclusion proof shows the entry under a signed tree head. A consistency proof shows that a later head
extends an earlier one.

![The Mount Fuji elevation fact, 3539.04 m, was signed in a batch of one fact. That batch is entry 2,463,431 of 3,248,898 in the log. A ladder of 22 bars shows its audit path: each bar is one hash, covering from 1 entry at the bottom to 2,097,152 entries at the top. The root computed from them matches the signed tree head of size 3,248,898, and the Ed25519 signature is valid. One character changed in hash 10 gives a different root, and inclusion is invalid. Below, a 14-hash consistency proof links the head of size 3,248,892 (09:28:13 UTC) to the head of size 3,248,898 (09:28:53 UTC): valid.](stills/log-proof.webp)

More: [protocol.md](../protocol.md). The film [emem-log-climb.mp4](films/emem-log-climb.mp4) climbs the same
audit path one hash at a time.

## Can a second agent check a bundle of facts?

Yes. One bundle token names many facts. The second agent resolves each one and checks each receipt.

<img src="stills/bundle-handoff.webp" alt="Live reads of emem.dev on 7 November 2026. 1: agent A hands over one 38-character line, emem:bundle:y2k6h2laeo4libfsiebucdw4mu, minted on 23 September 2026: four members, four resolved, one cell in Bengaluru at 12.972 N, 77.594 E. 2: agent B resolves the bundle and each fact token: elevation 915.07 m (observed 30 April 2021), NDVI 0.33 (12 May 2026), PM2.5 27.8 micrograms per cubic metre (28 May 2026) and air temperature 20.1 degrees C (21 July 2026). Each token resolves to the fact id that A cited, and each fact receipt is valid. 3: the bundle receipt rebuilds to digest 3d72e6e5 and its signature is valid; with one character of the NDVI fact id changed, the digest is ad3c1dbc and the check fails." width="540">

More: [emem-in-depth.md](../emem-in-depth.md).

## How does a refused note write become an accepted one?

The refusal gives the digest to sign. The agent builds the same digest, signs it with its own key and sends
the write again.

[![Film poster. The agent builds the digest itself: BLAKE3 over "emem.memory_write.v2|create|", the path, the BLAKE3 of file_text and "absent" gives 2ced569b3e75145a6d1ee66eed2c2a6d8ddeaf512b84e29157107eb3a9e7aada, the same digest as the refusal.](films/emem-note-write-poster.webp)](films/emem-note-write.mp4)

Silent film, 35 seconds. Select the picture to play the MP4, or get the [WebM](films/emem-note-write.webm).
The film wrote nothing to emem.dev. More: [memory.md](../memory.md).

## When does the guard deny a draft?

It denies a draft that states a value different from the fact it cites. By default it allows a draft with no
citation. With `claim_gating` true, it denies that draft.

<img src="stills/guard-check.webp" alt="Three drafts checked by live calls to POST /v1/guard/verdict on 7 November 2026. The cited fact is the Table Mountain elevation, 1069.01 m. Draft 1 states 1069.01 m and cites the fact: citation verified, allow. Draft 2 cites the same fact but states 1250 m: deny, PROV_VALUE, fix correct_value. Draft 3 states 1250 m with no citation: allow by default, deny CLAIM_UNGROUNDED with claim_gating=true. Each receipt verifies." width="540">

More: [agents.md](../agents.md).

## Which stages are behind one `/v1/ask` answer?

Four stages: located, routed, recalled and scored. Each stage has an address that commits to the stage before it.

<img src="stills/ask-trail.webp" alt="Still from one live call to POST /v1/ask on 7 November 2026: what is the NDVI in Central Park, New York? The answer is NDVI 0.79. Four reasoning stages follow, each with its own address that commits to the stage before it: located (cell defi.zb5d0.bAhE.zd9c1), routed (topic vegetation_condition), recalled (3 signed NDVI facts) and scored (5 computed values). The three Sentinel-2 NDVI facts are 0.60 on 4 May 2026, 0.88 on 21 October 2026 and 0.79 on 4 November 2026; the answer quotes the newest. The receipt over the 3 fact ids is valid offline and at /v1/verify_receipt. All 4 stage addresses recompute as BLAKE3 over the stored bytes. One changed byte in the recalled stage moves its address." width="540">

More: [agents.md](../agents.md).

## What these pictures do not prove

A valid signature shows which key signed the bytes. It does not show that the value is true. Values from
weather or new scenes can change after 7 November 2026. To check the pictures against emem.dev again, run
`python3 docs/media/stills/src/recheck.py` from the repository root.
