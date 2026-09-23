//! emem-demo-device: one public, honestly labelled device, so `emem:trace:`
//! and `emem:attestation:` tokens exist that anyone can resolve and verify.
//!
//! What it is: this host, enrolled under `generic.linux-host` (root of trust
//! `software_only`, no measured boot) and endorsed by the operator anchor
//! `operator.vortx.v1`. The roster will say `assurance: operator_endorsed` and
//! `endorsed_by: operator.vortx.v1`; nothing here claims a manufacturer or a
//! hardware root of trust, and the attestation's `hwmodel` says so in words.
//!
//! What it traces: a real ftrace window on this machine, one log per layer the
//! `host.counters.v1` profile requires (scheduler, memory, storage, network),
//! captured by `scripts/manual/capture_ftrace.sh` because tracefs needs root and
//! this program should not. Segment clocks and event counts are read out of
//! those logs; nothing is synthesized. The attestation carries no facts: the
//! profile registers no bands, and the point is the trace and the evidence, not
//! a reading.
//!
//! ```bash
//! sudo scripts/manual/capture_ftrace.sh /tmp/emem-demo-trace 2
//! cargo run -p emem-cli --bin emem-demo-device -- --trace-dir /tmp/emem-demo-trace --dry-run
//! cargo run -p emem-cli --bin emem-demo-device -- --trace-dir /tmp/emem-demo-trace
//! ```
//!
//! Keys: the device key persists at `~/.config/emem/demo_device.json` and is
//! never regenerated once present; the operator endorser seed is read from
//! `~/.config/emem/operator_endorser.json` and never printed.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};

use emem_core::key::{AttesterKey, KeyEpoch};
use emem_core::substrates::TraceLayerKind;
use emem_fact::{Attestation, RegistryCid, SchemaCid};
use emem_trace::{DeviceIdentity, OsTrace, PlatformAttestation, TraceSegment};

const PROFILE: &str = "host.counters.v1";
const PLATFORM: &str = "generic.linux-host";
const ANCHOR_ID: &str = "operator.vortx.v1";
const ENCODING: &str = "linux.ftrace.v1";
const HWMODEL: &str =
    "emem-demo-device: a generic Linux host, software_only, no hardware root of trust, no measured boot";
const OEMID: &str = "vortx.ai (operator-endorsed demo)";
const LAYERS: [(TraceLayerKind, &str); 4] = [
    (TraceLayerKind::Scheduler, "scheduler"),
    (TraceLayerKind::Memory, "memory"),
    (TraceLayerKind::Storage, "storage"),
    (TraceLayerKind::Network, "network"),
];

struct Args {
    base: String,
    trace_dir: PathBuf,
    dry_run: bool,
    re_enroll: bool,
}

fn parse_args() -> Result<Args> {
    let mut base = std::env::var("EMEM_BASE").unwrap_or_else(|_| "https://emem.dev".into());
    let mut trace_dir = None;
    let mut dry_run = false;
    let mut re_enroll = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dry-run" => dry_run = true,
            "--re-enroll" => re_enroll = true,
            "--base" => base = it.next().context("--base needs a url")?,
            "--trace-dir" => {
                trace_dir = Some(PathBuf::from(
                    it.next().context("--trace-dir needs a path")?,
                ))
            }
            other => bail!("unknown argument {other}; see the module docs for usage"),
        }
    }
    Ok(Args {
        base: base.trim_end_matches('/').to_string(),
        trace_dir: trace_dir.context("--trace-dir <dir from capture_ftrace.sh> is required")?,
        dry_run,
        re_enroll,
    })
}

fn b32(bytes: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(bytes).to_lowercase()
}

fn config_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(Path::new(&home).join(".config/emem"))
}

fn seed_from_hex(hex: &str) -> Result<SigningKey> {
    let raw = data_encoding::HEXLOWER_PERMISSIVE
        .decode(hex.trim().as_bytes())
        .map_err(|e| anyhow!("seed_hex is not hex: {e}"))?;
    let seed: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("seed_hex must be 32 bytes"))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn write_private(path: &Path, v: &Value) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("json.tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .with_context(|| format!("open {}", tmp.display()))?;
    f.write_all(serde_json::to_string_pretty(v)?.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// The demo device's key and run state. A file that exists but does not parse
/// is an error, never a reason to mint a new key: a new key is a new device,
/// and the old one would sit enrolled on the roster with nobody holding it.
fn load_or_create_device(path: &Path) -> Result<(SigningKey, Value)> {
    if path.exists() {
        let v: Value = serde_json::from_slice(&std::fs::read(path)?).with_context(|| {
            format!(
                "{} exists but is not JSON; refusing to replace it",
                path.display()
            )
        })?;
        let hex = v["seed_hex"].as_str().with_context(|| {
            format!("{} has no seed_hex; refusing to replace it", path.display())
        })?;
        return Ok((seed_from_hex(hex)?, v));
    }
    use rand::RngCore;
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    let v = json!({
        "_note": "emem demo device key (generic.linux-host, software_only). Never regenerate: a new key is a new device.",
        "seed_hex": data_encoding::HEXLOWER.encode(&seed),
    });
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_private(path, &v)?;
    Ok((SigningKey::from_bytes(&seed), v))
}

fn load_endorser(dir: &Path) -> Result<SigningKey> {
    let path = dir.join("operator_endorser.json");
    let v: Value = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("read {}", path.display()))?,
    )?;
    seed_from_hex(
        v["seed_hex"]
            .as_str()
            .context("operator_endorser.json has no seed_hex")?,
    )
}

/// `secs.frac` as nanoseconds, from the string, so a float never rounds a
/// timestamp the log states exactly.
fn ts_ns(s: &str) -> Option<u64> {
    let (int, frac) = s.split_once('.')?;
    if frac.is_empty() || frac.len() > 9 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let int: u64 = int.parse().ok()?;
    let frac: u64 = format!("{frac:0<9}").parse().ok()?;
    int.checked_mul(1_000_000_000)?.checked_add(frac)
}

/// First and last event timestamp and the event count of one ftrace `trace`
/// dump. An event line is `<task>-<pid> [cpu] <flags> <secs.frac>: <event>: ...`.
fn read_ftrace(raw: &str) -> Option<(u64, u64, u64)> {
    let mut first = None;
    let mut last = 0u64;
    let mut n = 0u64;
    for line in raw.lines() {
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let Some(t) = line
            .split_whitespace()
            .find_map(|tok| tok.strip_suffix(':').and_then(ts_ns))
        else {
            continue;
        };
        first.get_or_insert(t);
        last = last.max(t);
        n += 1;
    }
    Some((first?, last, n))
}

fn segments(dir: &Path) -> Result<Vec<TraceSegment>> {
    let mut out = Vec::new();
    for (layer, name) in LAYERS {
        let path = dir.join(format!("{name}.log"));
        let raw = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let text = String::from_utf8_lossy(&raw);
        let (start, end, count) = read_ftrace(&text)
            .with_context(|| format!("{} holds no ftrace events; capture again", path.display()))?;
        out.push(TraceSegment {
            layer,
            seq: 0,
            clock_start_ns: start,
            clock_end_ns: end,
            event_count: count,
            log_digest: b32(blake3::hash(&raw).as_bytes()),
            prev_digest: None,
            encoding: ENCODING.to_string(),
        });
    }
    Ok(out)
}

fn read_trim(path: &str) -> Result<String> {
    Ok(std::fs::read_to_string(path)
        .with_context(|| format!("read {path}"))?
        .trim()
        .to_string())
}

fn os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("PRETTY_NAME=")
                    .map(|v| v.trim_matches('"').to_string())
            })
        })
        .unwrap_or_else(|| "linux".into())
}

async fn call(
    cli: &reqwest::Client,
    base: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{base}{path}");
    let req = match body {
        Some(b) => cli.post(&url).json(b),
        None => cli.get(&url),
    };
    let resp = req
        .send()
        .await
        .with_context(|| format!("{method} {url}"))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        bail!("{method} {path} answered {status}: {v}");
    }
    Ok(v)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = parse_args()?;
    let dir = config_dir()?;
    let state_path = dir.join("demo_device.json");
    let (device, mut state) = load_or_create_device(&state_path)?;
    let endorser = load_endorser(&dir)?;
    let device_key = AttesterKey(device.verifying_key().to_bytes());
    let device_b32 = b32(&device_key.0);
    let endorser_fp = b32(blake3::hash(endorser.verifying_key().as_bytes()).as_bytes());
    println!("device key      {device_b32}");
    println!("endorser anchor {ANCHOR_ID} fingerprint {endorser_fp}");

    // Evidence: the operator's word, and it says so. No measurements are
    // claimed because this host has no measured boot to report.
    let mut nonce = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce);
    let evidence = PlatformAttestation::build_and_sign_v0(
        PLATFORM,
        device_key,
        HWMODEL,
        OEMID,
        &b32(&nonce),
        Vec::new(),
        &endorser,
    );
    let ev_ok = ed25519_dalek::VerifyingKey::from_bytes(&evidence.endorser_key.0)
        .map(|vk| {
            vk.verify_strict(
                &evidence.preimage(),
                &ed25519_dalek::Signature::from_bytes(&evidence.endorser_sig.0),
            )
            .is_ok()
        })
        .unwrap_or(false);
    if !ev_ok {
        bail!("the platform attestation does not verify under its own endorser key");
    }

    let boot_id = read_trim("/proc/sys/kernel/random/boot_id")?;
    let identity = DeviceIdentity {
        device_key,
        key_epoch: KeyEpoch(0),
        substrate_profile: PROFILE.to_string(),
        platform: PLATFORM.to_string(),
        os: os_name(),
        kernel: read_trim("/proc/sys/kernel/osrelease")?,
        boot_id: boot_id.clone(),
    };
    let segs = segments(&args.trace_dir)?;
    let window_start = segs.iter().map(|s| s.clock_start_ns).min().unwrap_or(0);
    let window_end = segs.iter().map(|s| s.clock_end_ns).max().unwrap_or(0);
    if window_start >= window_end {
        bail!("the captured window is empty; capture a longer one");
    }
    // A device's traces chain within a boot; the gate refuses a window that
    // does not name this boot's previous trace.
    let prev = (state["last_trace"]["boot_id"].as_str() == Some(boot_id.as_str()))
        .then(|| {
            state["last_trace"]["trace_cid"]
                .as_str()
                .map(str::to_string)
        })
        .flatten();
    let trace = OsTrace::build_and_sign_chained_v1(
        identity,
        window_start,
        window_end,
        segs,
        Vec::new(),
        prev.clone(),
        &device,
    )
    .map_err(|e| anyhow!("build trace: {e:?}"))?;
    let trace_cid = trace.trace_cid().map_err(|e| anyhow!("trace cid: {e:?}"))?;

    let profile = emem_core::substrates::DEFAULT
        .lookup(PROFILE)
        .context("profile host.counters.v1 is not in this build's substrates manifest")?;
    let report = emem_trace::verify_os_trace(&trace, profile, None);
    println!("local verify    {:?} {:?}", report.verdict, report.reasons);
    if report.verdict != emem_trace::Verdict::Admit {
        bail!("the trace does not verify locally; not sending it");
    }
    println!(
        "trace           {} segments, window {} ns, chained to {:?}",
        trace.segments.len(),
        window_end - window_start,
        prev
    );
    println!("attestation     {}", evidence.token().unwrap_or_default());
    println!("trace (to be)   {}", emem_trace::trace_token(&trace_cid));

    if args.dry_run {
        println!("dry run: nothing sent");
        return Ok(());
    }

    let cli = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .user_agent("emem-demo-device/1")
        .build()?;
    let base = args.base.as_str();

    // Only send evidence the responder's own anchor will recognise.
    let platforms = call(&cli, base, "GET", "/v1/device_platforms", None).await?;
    let anchor_fp = platforms["registry"]["platforms"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["id"] == PLATFORM))
        .and_then(|p| p["trust_anchors"].as_array())
        .and_then(|a| {
            a.iter()
                .find(|t| t["id"] == ANCHOR_ID && t["provisional"] == false)
        })
        .and_then(|t| t["fingerprint"].as_str())
        .map(str::to_string);
    if anchor_fp.as_deref() != Some(endorser_fp.as_str()) {
        bail!("{base} does not list {ANCHOR_ID} with this endorser's fingerprint (it lists {anchor_fp:?})");
    }
    let manifests = call(&cli, base, "GET", "/v1/manifests", None).await?;
    let registry_cid = manifests["registry_cid"]
        .as_str()
        .context("no registry_cid")?
        .to_string();
    let schema_cid = manifests["schema_cid"]
        .as_str()
        .context("no schema_cid")?
        .to_string();

    // Enrolling again resets the listing decision, so it happens once.
    if state["enrolled"] != true || args.re_enroll {
        let r = call(
            &cli,
            base,
            "POST",
            "/v1/enroll_attested",
            Some(&json!({
                "device_key": device_b32,
                "profile": PROFILE,
                "platform": PLATFORM,
                "attestation": serde_json::to_value(&evidence)?,
            })),
        )
        .await?;
        println!("enroll          {r}");
        if r["enrolled"] != true {
            bail!("enrolment refused");
        }
        state["enrolled"] = json!(true);
        state["attestation_token"] = json!(evidence.token());
        write_private(&state_path, &state)?;
    }

    let att = Attestation::build_and_sign_v1(
        Vec::new(),
        Vec::new(),
        RegistryCid::new(registry_cid),
        SchemaCid::new(schema_cid),
        &device,
        KeyEpoch(0),
        emem_storage::server::iso8601_now(),
        None,
    )
    .map_err(|e| anyhow!("build attestation: {e:?}"))?;
    let r = call(
        &cli,
        base,
        "POST",
        "/v1/attest_traced",
        Some(&json!({
            "attestation": serde_json::to_value(&att)?,
            "trace": serde_json::to_value(&trace)?,
        })),
    )
    .await?;
    println!("attest_traced   {r}");
    let trace_token = r["trace_token"]
        .as_str()
        .context("no trace_token: the trace was not admitted")?
        .to_string();
    state["last_trace"] = json!({ "boot_id": boot_id, "trace_cid": trace_cid });
    write_private(&state_path, &state)?;

    let decided_at = emem_storage::server::iso8601_now();
    let pre = emem_attest::publish_decision_preimage_v1(
        "emem.publish_decision.v1",
        &device_b32,
        true,
        &decided_at,
    );
    let r = call(
        &cli,
        base,
        "POST",
        "/v1/device_publish",
        Some(&json!({
            "device_key": device_b32,
            "publish": true,
            "decided_at": decided_at,
            "signature": b32(&device.sign(&pre).to_bytes()),
        })),
    )
    .await?;
    println!("publish         {r}");

    let attestation_token = state["attestation_token"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_default();
    for token in [trace_token.as_str(), attestation_token.as_str()] {
        let r = call(
            &cli,
            base,
            "POST",
            "/v1/trace_resolve",
            Some(&json!({ "token": token })),
        )
        .await?;
        println!("resolve {token}\n  kind={} ", r["kind"]);
        if let Some(t) = r.get("trace").filter(|t| !t.is_null()) {
            let v = call(
                &cli,
                base,
                "POST",
                "/v1/trace_verify",
                Some(&json!({ "profile": PROFILE, "trace": t })),
            )
            .await?;
            println!("  verify: {}", v.get("verdict").unwrap_or(&v));
        }
    }
    let roster = call(&cli, base, "GET", "/v1/devices", None).await?;
    println!("roster count    {}", roster["count"]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ftrace_timestamps_are_read_exactly() {
        let raw = "# tracer: nop\n#\n          <idle>-0       [000] d..2. 1234.000001: sched_switch: prev_comm=swapper\n     kworker/0:1-12  [001] ..... 1234.500000: sched_switch: x\n";
        assert_eq!(
            read_ftrace(raw),
            Some((1_234_000_001_000, 1_234_500_000_000, 2))
        );
        assert_eq!(read_ftrace("# only a header\n"), None);
        assert_eq!(ts_ns("1.5"), Some(1_500_000_000));
        assert_eq!(ts_ns("1.x"), None);
    }
}
