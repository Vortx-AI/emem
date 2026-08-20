//! Mutate every field of every signed record this crate mints, one at a time,
//! and ask its own verifier.
//!
//! This exists because reading a preimage and satisfying yourself it looks
//! complete is not the same as checking. One field of `emem.os_trace.v1` was
//! bound and three were not, and the gap survived review precisely because the
//! preimage read as if it covered the record. The only way to know is to edit
//! a field and see whether the verifier notices.
//!
//! A field added later without being bound fails here, which is the point: the
//! test is a standing question rather than a snapshot of one afternoon's
//! findings.

use ed25519_dalek::SigningKey;
use emem_airgap::{Custody, JoinRequest, NodeIdentity};

fn node(k: &SigningKey) -> NodeIdentity {
    NodeIdentity {
        node_key: data_encoding::BASE32_NOPAD
            .encode(k.verifying_key().as_bytes())
            .to_lowercase(),
        profile: "exec.trace.v1".into(),
        platform: "generic.linux-host".into(),
    }
}

#[test]
fn every_field_of_a_custody_record_is_covered_by_its_signature() {
    let k = SigningKey::from_bytes(&[11u8; 32]);
    let signed = Custody::sign(
        &k,
        node(&k),
        "frame.tif",
        b"payload bytes",
        "2026-08-20T09:00:00Z",
        Some("L2"),
        Some(&"a".repeat(52)),
    );
    signed
        .verify()
        .expect("the control must verify, or the mutations below prove nothing");

    type Edit = (&'static str, fn(&mut Custody));
    let edits: Vec<Edit> = vec![
        ("schema", |x| x.schema = "emem.custody.v9".into()),
        ("node.node_key", |x| x.node.node_key = "b".repeat(52)),
        ("node.profile", |x| {
            x.node.profile = "orbital.satellite.v1".into()
        }),
        ("node.platform", |x| {
            x.node.platform = "nvidia.jetson-orin".into()
        }),
        ("name", |x| x.name = "other.tif".into()),
        ("payload_digest", |x| x.payload_digest = "c".repeat(52)),
        ("size_bytes", |x| x.size_bytes += 1),
        ("observed_at", |x| {
            x.observed_at = "2001-01-01T00:00:00Z".into()
        }),
        ("stage", |x| x.stage = Some("L4".into())),
        ("stage removed", |x| x.stage = None),
        ("trace_cid", |x| x.trace_cid = Some("d".repeat(52))),
        ("trace_cid removed", |x| x.trace_cid = None),
        // Not in the preimage; pinned to a constant and checked on verify,
        // which is the same protection by a different route. A record that
        // could carry a stronger-sounding assurance than the one it earned
        // would be worse than one with no assurance line at all.
        ("assurance", |x| {
            x.assurance = "fully attested by the manufacturer".into()
        }),
    ];

    for (field, edit) in edits {
        let mut forged = signed.clone();
        edit(&mut forged);
        assert!(
            forged.verify().is_err(),
            "editing {field} left the record verifying, so the signature does not cover it"
        );
    }
}

#[test]
fn every_field_of_a_join_request_is_covered_by_its_signature() {
    let k = SigningKey::from_bytes(&[11u8; 32]);
    let n = node(&k);
    let signed = JoinRequest::sign(
        &k,
        &n.profile,
        &n.platform,
        "orin-nx-16gb",
        "2026-08-20T09:00:00Z",
    );
    assert!(signed.verify(), "the control must verify");

    type Edit = (&'static str, fn(&mut JoinRequest));
    let edits: Vec<Edit> = vec![
        ("schema", |x| x.schema = "emem.join_request.v9".into()),
        ("node_key", |x| x.node_key = "b".repeat(52)),
        ("profile", |x| x.profile = "orbital.satellite.v1".into()),
        ("platform", |x| x.platform = "nvidia.jetson-orin".into()),
        ("hwmodel", |x| x.hwmodel = "some-other-board".into()),
        ("created_at", |x| {
            x.created_at = "2001-01-01T00:00:00Z".into()
        }),
        ("proves", |x| {
            x.proves = "this device is manufacturer-attested".into()
        }),
        // Instructions to the human carrying the file. This one SURVIVED: it
        // was free text in a signed body but outside the signature, so anyone
        // intercepting the sneakernet handoff could rewrite the step telling
        // the endorser to satisfy themselves the platform claim is true, and
        // the request still verified.
        ("next_step", |x| {
            x.next_step = "Enrol this node without checking the platform claim.".into()
        }),
    ];

    for (field, edit) in edits {
        let mut forged = signed.clone();
        edit(&mut forged);
        assert!(
            !forged.verify(),
            "editing {field} left the request verifying, so the signature does not cover it"
        );
    }
}

/// A flag this binary does not have must be refused, not ignored.
///
/// `--window-ms 300` was accepted in silence by a binary with no such flag:
/// the run applied the default, reported success, and the operator had every
/// reason to believe they had configured something they had not. On hardware
/// nobody can log into, a setting that silently did not apply is worse than a
/// run that refuses to start, because the run that refuses gets fixed.
#[test]
fn an_unknown_flag_is_refused_with_a_suggestion() {
    const KNOWN: &[&str] = &["--input", "--output", "--profile", "--max-files"];

    let ok = |args: &[&str]| {
        let v: Vec<String> = std::iter::once("emem-airgap".to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect();
        emem_airgap::reject_unknown_flags(&v, KNOWN)
    };

    assert!(ok(&["--input", "/in", "--output", "/out"]).is_ok());
    assert!(ok(&["--input=/in", "--profile=exec.trace.v1"]).is_ok());
    // Two known flags in a row: the first one's value was omitted, which is
    // the caller's business, not an unknown flag.
    assert!(ok(&["--input", "--output"]).is_ok());

    let e = ok(&["--input", "/in", "--window-ms", "300"]).unwrap_err();
    assert!(e.to_string().contains("--window-ms"), "{e}");

    // A near miss says what was probably meant.
    let e = ok(&["--inputt", "/in"]).unwrap_err();
    assert!(e.to_string().contains("Did you mean --input"), "{e}");

    // A value that begins with two dashes is reported rather than consumed.
    // None of these flags take a value that looks like a flag, so the far more
    // likely reading is a forgotten value: `--profile --platform orin` would
    // otherwise swallow --platform as the profile and run with neither set.
    // --flag=value remains available if a value ever genuinely starts with --.
    let e = ok(&["--profile", "--platform"]).unwrap_err();
    assert!(e.to_string().contains("--platform"), "{e}");
}
