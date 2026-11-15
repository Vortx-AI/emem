//! `POST /ops/orphan-facts`: the operator's handle on the orphan pass in
//! `emem_storage::orphans` (find facts the transparency log never received,
//! back them up, remove them from `facts.redb`).
//!
//! It is maintenance, not protocol: outside `/v1`, absent from the API
//! description, and it answers exactly as an unrouted path does unless both
//! hold:
//!
//! - the connection comes from loopback. The container shares the host's
//!   network, so this admits processes on this host and nothing else;
//! - `Authorization: Bearer <token>` matches `$EMEM_DATA/maint/ops.token`,
//!   a file the operator creates with mode 0600. The secret is never in the
//!   repository, the unit file or the container's environment (both of
//!   which `docker inspect` and a world-readable unit would show). No file,
//!   no route; deleting the file turns it off without a restart.
//!
//! A pass runs for minutes to hours, so a request starts it on the blocking
//! pool and returns; `status` reports progress and the result. One pass at a
//! time, shared with `/ops/log-repair`. A restart ends a pass; the same
//! request resumes it from its saved state.
//!
//! The scan takes `accept_log_problems`, a list of `"<segment>:<offset>"`
//! positions where the operator knows the log is torn. A tear the list does
//! not name, or a position in it that is not a tear, fails the log check.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use axum::extract::{ConnectInfo, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use emem_storage::orphans::{self, DeleteOptions, JobControl, OrphanStore, ScanOptions};

use crate::AppState;

struct Job {
    action: String,
    run: String,
    started: std::time::Instant,
    ctl: Arc<JobControl>,
    result: Arc<Mutex<Option<Result<Value, String>>>>,
}

impl Job {
    fn running(&self) -> bool {
        self.result.lock().map(|r| r.is_none()).unwrap_or(false)
    }

    fn describe(&self) -> Value {
        let result = self.result.lock().ok().and_then(|r| r.clone());
        json!({
            "action": self.action,
            "run": self.run,
            "running": result.is_none(),
            "elapsed_s": self.started.elapsed().as_secs(),
            "progress": self.ctl.progress(),
            "result": match result {
                Some(Ok(v)) => v,
                Some(Err(e)) => json!({"error": e}),
                None => Value::Null,
            },
        })
    }
}

static JOB: LazyLock<Mutex<Option<Job>>> = LazyLock::new(|| Mutex::new(None));

pub(crate) fn maint_root() -> PathBuf {
    emem_core::data_dir::data_dir().join("maint")
}

/// Loopback, a token file that only its owner can read, and a bearer token
/// equal to it. The reason is for the log, never for the caller.
fn authorize(
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
    token_file: &Path,
) -> Result<(), &'static str> {
    use std::os::unix::fs::PermissionsExt;
    let peer = peer.ok_or("no peer address")?;
    if !peer.ip().to_canonical().is_loopback() {
        return Err("not loopback");
    }
    let meta = std::fs::metadata(token_file).map_err(|_| "no token file")?;
    if !meta.is_file() {
        return Err("the token path is not a file");
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err("the token file is readable by group or others");
    }
    let want = std::fs::read_to_string(token_file).map_err(|_| "the token file is unreadable")?;
    let want = want.trim();
    if want.len() < 32 {
        return Err("the token file holds fewer than 32 characters");
    }
    let got = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or("no bearer token")?;
    // blake3::Hash compares in constant time.
    if blake3::hash(got.trim().as_bytes()) != blake3::hash(want.as_bytes()) {
        return Err("wrong token");
    }
    Ok(())
}

pub(crate) fn reply(status: StatusCode, v: Value) -> Response {
    (status, Json(v)).into_response()
}

/// The request's JSON body when the gate admits it; otherwise the response
/// an unrouted path gives.
// The error is the response the handler returns as it is.
#[allow(clippy::result_large_err)]
pub(crate) async fn admit(
    route: &'static str,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Value, Response> {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    if let Err(why) = authorize(peer, req.headers(), &maint_root().join("ops.token")) {
        if peer.is_some_and(|p| p.ip().to_canonical().is_loopback()) {
            tracing::warn!(target: "emem::ops", route, why, "ops request refused");
        }
        return Err(crate::serve_404(req).await);
    }
    let body = axum::body::to_bytes(req.into_body(), 1 << 16)
        .await
        .map_err(|e| reply(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})))?;
    serde_json::from_slice(&body)
        .map_err(|e| reply(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})))
}

/// The pass running or last run, if any.
pub(crate) fn job() -> Option<Value> {
    JOB.lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(Job::describe)
}

/// Ask the running pass to stop at its next batch boundary.
pub(crate) fn stop() -> Response {
    let g = JOB.lock().unwrap_or_else(|e| e.into_inner());
    match g.as_ref().filter(|j| j.running()) {
        Some(j) => {
            j.ctl.cancel();
            reply(StatusCode::OK, json!({"stopping": j.describe()}))
        }
        None => reply(StatusCode::OK, json!({"stopping": null})),
    }
}

/// Start `work` on the blocking pool as the one maintenance pass, unless a
/// pass is running.
pub(crate) fn launch(
    action: &str,
    run: &str,
    work: impl FnOnce(&JobControl) -> Result<Value, String> + Send + 'static,
) -> Response {
    let mut g = JOB.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(j) = g.as_ref().filter(|j| j.running()) {
        return reply(
            StatusCode::CONFLICT,
            json!({"error": "a pass is running; stop it or wait", "job": j.describe()}),
        );
    }
    let ctl = Arc::new(JobControl::new());
    let result: Arc<Mutex<Option<Result<Value, String>>>> = Arc::new(Mutex::new(None));
    let job = Job {
        action: action.to_string(),
        run: run.to_string(),
        started: std::time::Instant::now(),
        ctl: ctl.clone(),
        result: result.clone(),
    };
    let (action_owned, run_owned) = (action.to_string(), run.to_string());
    tracing::info!(target: "emem::ops", action, run, "maintenance pass started");
    tokio::task::spawn_blocking(move || {
        // A panic must end the job, or `status` reports it running forever
        // and no other pass can start until a restart.
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&ctl)))
            .unwrap_or_else(|_| Err("the pass panicked; see the server log".to_string()));
        match &out {
            Ok(_) => {
                tracing::info!(target: "emem::ops", action = %action_owned, run = %run_owned, "maintenance pass finished")
            }
            Err(e) => {
                tracing::warn!(target: "emem::ops", action = %action_owned, run = %run_owned, error = %e, "maintenance pass ended")
            }
        }
        if let Ok(mut r) = result.lock() {
            *r = Some(out);
        }
    });
    let started = job.describe();
    *g = Some(job);
    reply(StatusCode::ACCEPTED, json!({"started": started}))
}

pub(crate) async fn post_orphan_facts(
    State(s): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
) -> Response {
    let v = match admit("/ops/orphan-facts", req).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let action = v.get("action").and_then(Value::as_str).unwrap_or("status");
    let run = v.get("run").and_then(Value::as_str).map(str::to_string);
    match action {
        "status" => status(run.as_deref()),
        "stop" => stop(),
        "scan" | "delete" | "verify" => start(&s, action, run, &v),
        other => reply(
            StatusCode::BAD_REQUEST,
            json!({"error": format!("unknown action {other:?}; use scan, delete, verify, status or stop")}),
        ),
    }
}

fn status(run: Option<&str>) -> Response {
    let files = run
        .and_then(|r| orphans::run_dir(&maint_root(), r).ok())
        .map(|dir| {
            let read = |name: &str| {
                std::fs::read(dir.join(name))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .unwrap_or(Value::Null)
            };
            json!({
                "dir": dir.display().to_string(),
                "report": read("report.json"),
                "delete": read("delete.report.json"),
                "verify": read("verify.report.json"),
            })
        });
    reply(StatusCode::OK, json!({"job": job(), "run": files}))
}

fn start(s: &AppState, action: &str, run: Option<String>, v: &Value) -> Response {
    let Some(run) = run else {
        return reply(
            StatusCode::BAD_REQUEST,
            json!({"error": "name the run: {\"run\": \"...\"}"}),
        );
    };
    let dir = match orphans::run_dir(&maint_root(), &run) {
        Ok(d) => d,
        Err(e) => return reply(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})),
    };
    let (Some(redb), Some(log_root)) = (
        s.storage.redb(),
        s.storage.transparency_log().map(|l| l.root.clone()),
    ) else {
        return reply(
            StatusCode::CONFLICT,
            json!({"error": "this node has no redb fact store or no log"}),
        );
    };
    let sled = s.storage.hot_sled_db().cloned();
    let scan_opts: ScanOptions = match serde_json::from_value(v.clone()) {
        Ok(o) => o,
        Err(e) => return reply(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})),
    };
    let delete_opts: DeleteOptions = match serde_json::from_value(v.clone()) {
        Ok(o) => o,
        Err(e) => return reply(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})),
    };
    let duty = v.get("duty").and_then(Value::as_f64).unwrap_or(0.25);
    let action_owned = action.to_string();
    tracing::info!(target: "emem::ops", action, run = %run, dir = %dir.display(), "orphan pass requested");
    launch(action, &run, move |ctl| {
        let store = OrphanStore {
            redb: &redb,
            sled: sled.as_ref(),
            log_root: &log_root,
        };
        match action_owned.as_str() {
            "scan" => orphans::scan(&store, &dir, &scan_opts, ctl),
            "delete" => orphans::delete(&store, &dir, &delete_opts, ctl),
            _ => orphans::verify(&store, &dir, duty, ctl),
        }
        .map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn token_dir(mode: u32, token: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("ops.token");
        std::fs::write(&p, format!("{token}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        (d, p)
    }

    fn bearer(t: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {t}")).unwrap(),
        );
        h
    }

    const TOKEN: &str = "a6f3kq2m7xw4bt9zr8cy5hn1ld0pve6s";

    #[test]
    fn only_loopback_with_the_right_token_from_a_private_file_is_let_in() {
        let (_d, file) = token_dir(0o600, TOKEN);
        let local: SocketAddr = "127.0.0.1:40000".parse().unwrap();
        let mapped: SocketAddr = "[::ffff:127.0.0.1]:40000".parse().unwrap();
        let remote: SocketAddr = "203.0.113.9:40000".parse().unwrap();
        assert_eq!(authorize(Some(local), &bearer(TOKEN), &file), Ok(()));
        assert_eq!(authorize(Some(mapped), &bearer(TOKEN), &file), Ok(()));
        assert_eq!(
            authorize(Some(remote), &bearer(TOKEN), &file),
            Err("not loopback")
        );
        assert_eq!(
            authorize(None, &bearer(TOKEN), &file),
            Err("no peer address")
        );
        assert_eq!(
            authorize(
                Some(local),
                &bearer("wrong-but-long-enough-0123456789"),
                &file
            ),
            Err("wrong token")
        );
        assert_eq!(
            authorize(Some(local), &HeaderMap::new(), &file),
            Err("no bearer token")
        );
    }

    #[test]
    fn the_route_is_off_without_a_private_token_file() {
        let local: SocketAddr = "127.0.0.1:40000".parse().unwrap();
        let (d, _) = token_dir(0o600, TOKEN);
        assert_eq!(
            authorize(Some(local), &bearer(TOKEN), &d.path().join("absent")),
            Err("no token file")
        );
        let (_d2, shared) = token_dir(0o640, TOKEN);
        assert_eq!(
            authorize(Some(local), &bearer(TOKEN), &shared),
            Err("the token file is readable by group or others")
        );
        let (_d3, short) = token_dir(0o600, "short");
        assert_eq!(
            authorize(Some(local), &bearer("short"), &short),
            Err("the token file holds fewer than 32 characters")
        );
    }

    /// The request body carries `accept_log_problems` to the scan.
    #[test]
    fn the_scan_reads_accepted_log_problems_from_the_request() {
        let v = json!({"action": "scan", "run": "r1",
                       "accept_log_problems": ["235:183293", "1633:509270"]});
        let o: ScanOptions = serde_json::from_value(v).unwrap();
        assert_eq!(o.accept_log_problems, vec!["235:183293", "1633:509270"]);
        let none: ScanOptions = serde_json::from_value(json!({"action": "scan"})).unwrap();
        assert!(none.accept_log_problems.is_empty());
    }

    #[tokio::test]
    async fn a_refused_request_looks_like_an_unrouted_path() {
        use tower::ServiceExt;
        let app = axum::Router::new()
            .route("/ops/orphan-facts", axum::routing::post(post_orphan_facts))
            .with_state(crate::tests::test_app_state());
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/ops/orphan-facts")
            .header("accept", "application/json")
            .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(axum::body::Body::from(r#"{"action":"status"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
