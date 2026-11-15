//! `POST /ops/log-repair`: the operator's handle on `emem_storage::log_repair`
//! (find log entries that are on disk but are not leaves of the transparency
//! tree, and append them again at the head).
//!
//! The same gate as `/ops/orphan-facts` (`ops_orphans.rs`): loopback, and a
//! bearer token equal to `$EMEM_DATA/maint/ops.token`; anything else gets the
//! answer of an unrouted path. The two routes share one pass at a time,
//! because each reads the whole log.
//!
//! Actions, each but `status` and `stop` with `{"run": "<name>"}`. A run's
//! files are under `$EMEM_DATA/maint/log-repair/<run>/`.
//!
//! - `scan`: walk every segment, and list each tear, each leaf the tree took
//!   from a torn frame, and each entry that is on disk but is not a leaf.
//!   Reads only.
//! - `relog`: append each listed entry that is not yet a leaf, byte for
//!   byte, in batches; link its facts' proofs to it; write
//!   `relocations.json`. When every listed entry is a leaf, the responder
//!   signs and publishes the erratum (`log_erratum.rs`).
//! - `erratum`: publish the erratum of a completed relog. The same body keeps
//!   its first signature.
//! - `status`, `stop`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};

use emem_storage::log_repair::{self, RepairOptions, RepairStore};

use crate::ops_orphans::{admit, job, launch, maint_root, reply, stop};
use crate::AppState;

pub(crate) async fn post_log_repair(
    State(s): State<AppState>,
    req: axum::http::Request<axum::body::Body>,
) -> Response {
    let v = match admit("/ops/log-repair", req).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let action = v.get("action").and_then(Value::as_str).unwrap_or("status");
    let run = v.get("run").and_then(Value::as_str).map(str::to_string);
    match action {
        "status" => status(run.as_deref()),
        "stop" => stop(),
        "scan" | "relog" | "erratum" => start(&s, action, run, &v),
        other => reply(
            StatusCode::BAD_REQUEST,
            json!({"error": format!("unknown action {other:?}; use scan, relog, erratum, status or stop")}),
        ),
    }
}

fn status(run: Option<&str>) -> Response {
    let files = run
        .and_then(|r| log_repair::run_dir(&maint_root(), r).ok())
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
                "relog": read("relog.report.json"),
            })
        });
    reply(
        StatusCode::OK,
        json!({"job": job(), "run": files, "errata": crate::log_erratum::all()}),
    )
}

/// The erratum id of a run.
fn erratum_id(run: &str) -> String {
    format!("log-repair-{run}")
}

fn start(s: &AppState, action: &str, run: Option<String>, v: &Value) -> Response {
    let Some(run) = run else {
        return reply(
            StatusCode::BAD_REQUEST,
            json!({"error": "name the run: {\"run\": \"...\"}"}),
        );
    };
    let dir = match log_repair::run_dir(&maint_root(), &run) {
        Ok(d) => d,
        Err(e) => return reply(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})),
    };
    if s.storage.transparency_log().is_none() {
        return reply(
            StatusCode::CONFLICT,
            json!({"error": "this node has no transparency log"}),
        );
    }
    let opts: RepairOptions = match serde_json::from_value(v.clone()) {
        Ok(o) => o,
        Err(e) => return reply(StatusCode::BAD_REQUEST, json!({"error": e.to_string()})),
    };
    let rt = tokio::runtime::Handle::current();
    let s = s.clone();
    let action_owned = action.to_string();
    let id = erratum_id(&run);
    tracing::info!(target: "emem::ops", action, run = %run, dir = %dir.display(), "log repair requested");
    launch(action, &run, move |ctl| {
        let log = s
            .storage
            .transparency_log()
            .ok_or("this node has no transparency log")?;
        let redb = s.storage.redb();
        let store = RepairStore {
            log,
            redb: redb.as_deref(),
            sled: s.storage.hot_sled_db(),
        };
        match action_owned.as_str() {
            "scan" => log_repair::scan(&store, &dir, &opts, ctl).map_err(|e| e.to_string()),
            "relog" => {
                let out =
                    log_repair::relog(&store, &dir, &opts, ctl, &rt).map_err(|e| e.to_string())?;
                let erratum = if out["complete"] == true {
                    crate::log_erratum::publish(&s, &dir, &id)
                        .unwrap_or_else(|e| json!({"not_published": e}))
                } else {
                    Value::Null
                };
                Ok(json!({"relog": out, "erratum": erratum}))
            }
            _ => crate::log_erratum::publish(&s, &dir, &id).map(|e| json!({"erratum": e})),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::AUTHORIZATION;

    #[tokio::test]
    async fn a_refused_request_looks_like_an_unrouted_path() {
        use tower::ServiceExt;
        let app = axum::Router::new()
            .route("/ops/log-repair", axum::routing::post(post_log_repair))
            .with_state(crate::tests::test_app_state());
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/ops/log-repair")
            .header("accept", "application/json")
            .header(AUTHORIZATION, "Bearer a6f3kq2m7xw4bt9zr8cy5hn1ld0pve6s")
            .body(axum::body::Body::from(r#"{"action":"scan","run":"r1"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn run_names_cannot_leave_the_maintenance_directory() {
        let root = std::path::Path::new("/data/maint");
        assert_eq!(
            log_repair::run_dir(root, "2026-11-14a").unwrap(),
            root.join("log-repair").join("2026-11-14a")
        );
        for bad in ["", "../x", "a/b", "a b", &"x".repeat(65)] {
            assert!(log_repair::run_dir(root, bad).is_err(), "{bad:?}");
        }
        assert_eq!(erratum_id("r1"), "log-repair-r1");
    }
}
