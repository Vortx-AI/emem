//! Check a custody record the way somebody on the ground would.
//!
//! The point of the exercise is what is NOT needed: no network, no server, no
//! account, no state from the node that wrote it. A record and the payload it
//! describes are enough, which is what makes an air-gapped node's output worth
//! anything after it comes down.
//!
//! ```bash
//! cargo run -p emem-airgap --example verify_a_record -- <dir>
//! ```
//!
//! where `<dir>` holds `in/frame_001.tif` and
//! `out/frame_001.tif.custody.json`, i.e. a directory a run has already been
//! pointed at.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .ok_or("usage: verify_a_record <dir with in/ and out/>")?;
    let record_bytes = std::fs::read(format!("{dir}/out/frame_001.tif.custody.json"))?;
    let payload = std::fs::read(format!("{dir}/in/frame_001.tif"))?;

    let record: emem_airgap::Custody = serde_json::from_slice(&record_bytes)?;

    // Two questions, deliberately separate. This one asks whether the record
    // is genuine: signed by the node it names, unedited since.
    record.verify()?;
    println!(
        "verify()  the record is genuine, signed by {}",
        &record.node.node_key[..8]
    );

    // And this one asks whether the file in front of you is the one it is
    // about. A reader holding only the record can answer the first question
    // but not this one.
    assert!(
        record.covers(&payload),
        "record does not cover this payload"
    );
    println!(
        "covers()  it is about this exact file, {} bytes",
        record.size_bytes
    );

    assert!(
        !record.covers(b"not the payload"),
        "a record must not cover bytes it never saw"
    );
    println!("covers()  and correctly refuses different bytes");

    println!("\nwhat it does NOT say:\n  {}", record.assurance);
    Ok(())
}
