// build.rs — emit build-time provenance into the compiled binary.
//
// Two env vars are surfaced to `option_env!`-aware Rust code at compile
// time so the running emem-server can publish a verifiable claim about
// which source tree produced it:
//
//   EMEM_GIT_COMMIT       — full SHA-1 of the HEAD commit, or "unknown"
//                            when the build tree is not a git checkout
//                            (release-tarball builds, vendored deps, etc.)
//   EMEM_BUILD_TIMESTAMP  — RFC 3339 UTC instant the build ran
//
// Both feed into the operator_attestation field of /.well-known/emem.json
// per whitepaper §5.7 / §20 (Open questions / operator attestation).
//
// The build script is intentionally minimal: zero new dependencies, one
// `git` shell-out, one timestamp from std. If `git` is missing or the
// tree is not a checkout, EMEM_GIT_COMMIT falls through to "unknown"
// rather than failing the build; the attestation publishes the absence
// honestly.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Re-run on commit / branch / index change so the embedded SHA stays fresh.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");

    let commit = Command::new("git")
        .args(["-C", "../..", "rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                String::from_utf8(out.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=EMEM_GIT_COMMIT={}", commit);

    // RFC 3339 UTC timestamp without pulling chrono. Hinnant's
    // civil-from-days, identical to the formatter inside lib.rs but
    // duplicated here so the build script stays dependency-free.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stamp = iso8601_utc(secs);
    println!("cargo:rustc-env=EMEM_BUILD_TIMESTAMP={}", stamp);
}

fn iso8601_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let sod = (secs % 86_400) as u32;
    let (y, m, d) = civil_from_days(days);
    let hh = sod / 3600;
    let mm = (sod / 60) % 60;
    let ss = sod % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if m <= 2 { y + 1 } else { y } as i32;
    (y, m, d)
}
