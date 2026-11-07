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
//! where `<dir>` holds `in/frame_001.tif` and the records a run wrote for it
//! in `out/`, i.e. a directory a run has already been pointed at. A record is
//! named `frame_001.tif.<node>.<time>.custody.json`, so one payload can have
//! several records (several nodes, or several runs); each one is checked.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .ok_or("usage: verify_a_record <dir with in/ and out/>")?;
    let payload = std::fs::read(format!("{dir}/in/frame_001.tif"))?;

    let mut records: Vec<std::path::PathBuf> = std::fs::read_dir(format!("{dir}/out"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("frame_001.tif.") && n.ends_with(".custody.json"))
        })
        .collect();
    records.sort();
    if records.is_empty() {
        return Err(format!(
            "no frame_001.tif.<node>.<time>.custody.json in {dir}/out; run the node on {dir}/in first"
        )
        .into());
    }

    for path in &records {
        println!("{}", path.display());
        let record: emem_airgap::Custody = serde_json::from_slice(&std::fs::read(path)?)?;

        // Two questions, deliberately separate. This one asks whether the
        // record is genuine: signed by the node it names, unedited since.
        record.verify()?;
        println!(
            "  verify()  the record is genuine, signed by {}",
            &record.node.node_key[..8]
        );

        // And this one asks whether the file in front of you is the one it is
        // about. A reader holding only the record can answer the first
        // question but not this one.
        assert!(
            record.covers(&payload),
            "record does not cover this payload"
        );
        println!(
            "  covers()  it is about this exact file, {} bytes",
            record.size_bytes
        );

        assert!(
            !record.covers(b"not the payload"),
            "a record must not cover bytes it never saw"
        );
        println!("  covers()  and correctly refuses different bytes");

        println!("\n  what it does NOT say:\n  {}\n", record.assurance);
    }
    Ok(())
}
