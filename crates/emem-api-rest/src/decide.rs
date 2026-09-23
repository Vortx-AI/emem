//! `POST /v1/decide`: typed decisions from a small open-weights model.
//!
//! A "system one" call: the caller hands over a state and a few questions whose
//! possible answers are known in advance (pick one option, yes or no, a score on
//! a small scale), and gets back only values from that set, each with the
//! probability the model put on every option. No free text is generated, so an
//! answer cannot be malformed and cannot smuggle an instruction.
//!
//! The model is whatever this node's local OpenAI-compatible endpoint serves
//! (`EMEM_DECIDE_LLM_URL`, default the llama.cpp server on 127.0.0.1:5014).
//! Each option is shown as a letter and the answer is the model's distribution
//! over those letters on the first token. When the letters together carry less
//! than half of that token's probability, the model was not answering the
//! question asked, and the call abstains rather than returning the argmax.
//!
//! The probabilities are the model's, not calibrated: nothing here trains or
//! checks them against outcomes. They rank options; read them as confidence
//! only after measuring them on your own labelled cases. The receipt signs
//! which input, model and output went together, never that the decision is
//! right.

// Every Err here is a finished response handed straight back to the handler, once.
#![allow(clippy::result_large_err)]

use std::sync::OnceLock;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use ed25519_dalek::Signer;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use crate::range_hash::refuse;
use crate::AppState;

pub(crate) const DECIDE_DOMAIN: &str = "emem.decide.v1";

pub(crate) mod tag {
    pub const INPUT_BLAKE3: u8 = 1;
    pub const MODEL: u8 = 2;
    pub const OUTPUT_BLAKE3: u8 = 3;
    pub const DECIDED_AT: u8 = 4;
    pub const RESPONDER_PUBKEY: u8 = 5;
}

const MAX_QUESTIONS: usize = 8;
const MAX_OPTIONS: usize = 8;
const MAX_STATE_CHARS: usize = 8_000;
/// Below this share of first-token probability on the option letters, abstain.
const ABSTAIN_BELOW: f64 = 0.5;

#[derive(Debug, Deserialize)]
pub(crate) struct DecideReq {
    /// The situation, as text or any JSON value.
    state: JsonValue,
    questions: Vec<Question>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Question {
    #[serde(default)]
    id: Option<String>,
    /// `choice` (one of `options`), `bool` (yes or no), `score` (an integer in `scale`).
    kind: String,
    /// The question, in words.
    ask: String,
    #[serde(default)]
    options: Vec<String>,
    /// Inclusive `[min, max]` for `score`, at most eight steps.
    #[serde(default)]
    scale: Option<[i64; 2]>,
}

/// The options a question is answered from, as labels shown to the model.
pub(crate) fn options_for(q: &Question) -> Result<Vec<String>, String> {
    match q.kind.as_str() {
        "choice" => {
            if q.options.len() < 2 || q.options.len() > MAX_OPTIONS {
                return Err(format!("a choice needs 2 to {MAX_OPTIONS} options"));
            }
            Ok(q.options.clone())
        }
        "bool" => Ok(vec!["yes".into(), "no".into()]),
        "score" => {
            let [lo, hi] = q.scale.unwrap_or([1, 5]);
            if hi <= lo || hi - lo + 1 > MAX_OPTIONS as i64 {
                return Err(format!("a score scale needs 2 to {MAX_OPTIONS} steps"));
            }
            Ok((lo..=hi).map(|v| v.to_string()).collect())
        }
        k => Err(format!("kind {k:?} is not choice, bool or score")),
    }
}

/// The model's distribution over option letters, from the first token's top
/// log-probabilities: `(per-option probability renormalised over the letters,
/// the letters' share of the token's probability)`.
pub(crate) fn letter_distribution(top: &[(String, f64)], n: usize) -> (Vec<f64>, f64) {
    let mut p = vec![0.0; n];
    for (tok, logprob) in top {
        let t = tok
            .trim()
            .trim_end_matches([')', '.', ':'])
            .to_ascii_uppercase();
        if t.len() == 1 {
            let i = (t.as_bytes()[0] as i64) - ('A' as i64);
            if (0..n as i64).contains(&i) {
                p[i as usize] += logprob.exp();
            }
        }
    }
    let mass: f64 = p.iter().sum();
    if mass > 0.0 {
        for x in &mut p {
            *x /= mass;
        }
    }
    (p, mass.min(1.0))
}

fn llm_url() -> String {
    std::env::var("EMEM_DECIDE_LLM_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:5014/v1/chat/completions".into())
}

fn permits() -> &'static tokio::sync::Semaphore {
    static P: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    P.get_or_init(|| tokio::sync::Semaphore::new(2))
}

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default()
    })
}

/// The model the endpoint reports, so a receipt names what decided.
async fn model_id(url: &str) -> String {
    static M: OnceLock<String> = OnceLock::new();
    if let Some(m) = M.get() {
        return m.clone();
    }
    let models = url.replace("/chat/completions", "/models");
    let id = match client().get(&models).send().await {
        Ok(r) => r
            .json::<JsonValue>()
            .await
            .ok()
            .and_then(|j| j["data"][0]["id"].as_str().map(str::to_string)),
        Err(_) => None,
    };
    match id {
        Some(id) => {
            let name = id.rsplit('/').next().unwrap_or(&id).to_string();
            let _ = M.set(name.clone());
            name
        }
        None => "unknown".into(),
    }
}

async fn ask_model(state: &str, q: &Question, opts: &[String]) -> Result<(Vec<f64>, f64), String> {
    let letters: Vec<char> = (0..opts.len()).map(|i| (b'A' + i as u8) as char).collect();
    let listed: String = letters
        .iter()
        .zip(opts)
        .map(|(l, o)| format!("{l}) {o}\n"))
        .collect();
    let user = format!(
        "State:\n{state}\n\nQuestion: {}\nOptions:\n{listed}\nReply with the option letter only.",
        q.ask
    );
    let body = json!({
        "messages": [
            {"role": "system", "content": "You classify. Reply with exactly one option letter."},
            {"role": "user", "content": user},
        ],
        "max_tokens": 1,
        "temperature": 0,
        "logprobs": true,
        "top_logprobs": 20,
        // Ignored by models without a thinking mode; stops one that has one
        // from spending its single token on "<think>".
        "chat_template_kwargs": {"enable_thinking": false},
    });
    let r: JsonValue = client()
        .post(llm_url())
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("the local model did not answer: {e}"))?
        .json()
        .await
        .map_err(|e| format!("the local model's answer did not parse: {e}"))?;
    let top: Vec<(String, f64)> = r["choices"][0]["logprobs"]["content"][0]["top_logprobs"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| Some((x["token"].as_str()?.to_string(), x["logprob"].as_f64()?)))
                .collect()
        })
        .unwrap_or_default();
    if top.is_empty() {
        return Err("the local model returned no token probabilities".into());
    }
    Ok(letter_distribution(&top, opts.len()))
}

pub(crate) async fn post_decide(
    axum::extract::State(s): axum::extract::State<AppState>,
    req: axum::http::Request<axum::body::Body>,
) -> Response {
    let ip = crate::client_ip(&req).unwrap_or_else(|| "unknown".to_string());
    let Ok(raw) = axum::body::to_bytes(req.into_body(), 32 * 1024).await else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_argument",
            "body over 32 KiB".into(),
        );
    };
    let r: DecideReq = match serde_json::from_slice(&raw) {
        Ok(r) => r,
        Err(e) => {
            return refuse(
                StatusCode::BAD_REQUEST,
                "invalid_argument",
                format!("expected {{state, questions:[{{kind, ask, options?, scale?}}]}}: {e}"),
            )
        }
    };
    if r.questions.is_empty() || r.questions.len() > MAX_QUESTIONS {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_argument",
            format!("1 to {MAX_QUESTIONS} questions"),
        );
    }
    let state = match &r.state {
        JsonValue::String(t) => t.clone(),
        v => v.to_string(),
    };
    if state.chars().count() > MAX_STATE_CHARS {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_argument",
            format!("state over {MAX_STATE_CHARS} characters"),
        );
    }
    let mut prepared = Vec::with_capacity(r.questions.len());
    for (i, q) in r.questions.iter().enumerate() {
        match options_for(q) {
            Ok(o) => prepared.push((i, q, o)),
            Err(e) => {
                return refuse(
                    StatusCode::BAD_REQUEST,
                    "invalid_argument",
                    format!("question {i}: {e}"),
                )
            }
        }
    }
    let quota = std::env::var("EMEM_DECIDE_DAILY_QUOTA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    if !crate::check_daily_quota(&ip, "decide", quota) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "86400")],
            Json(json!({"code": "decide_quota", "error": format!("decide daily quota hit ({quota}/day per IP)")})),
        )
            .into_response();
    }
    let Ok(_permit) = permits().acquire().await else {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "no decision slot".into(),
        );
    };
    let started = std::time::Instant::now();
    let model = model_id(&llm_url()).await;
    let mut answers = Vec::with_capacity(prepared.len());
    for (i, q, opts) in prepared {
        let (dist, mass) = match ask_model(&state, q, &opts).await {
            Ok(v) => v,
            Err(e) => return refuse(StatusCode::SERVICE_UNAVAILABLE, "model_unavailable", e),
        };
        let (best, p_best) =
            dist.iter()
                .enumerate()
                .fold((0, 0.0), |a, (j, p)| if *p > a.1 { (j, *p) } else { a });
        let abstained = mass < ABSTAIN_BELOW;
        let value = if abstained {
            JsonValue::Null
        } else {
            match q.kind.as_str() {
                "bool" => json!(best == 0),
                "score" => json!(opts[best].parse::<i64>().unwrap_or_default()),
                _ => json!(opts[best]),
            }
        };
        let probabilities: serde_json::Map<String, JsonValue> = opts
            .iter()
            .zip(&dist)
            .map(|(o, p)| (o.clone(), json!((p * 1000.0).round() / 1000.0)))
            .collect();
        let mut a = json!({
            "id": q.id.clone().unwrap_or_else(|| i.to_string()),
            "kind": q.kind,
            "value": value,
            "probabilities": probabilities,
            "confidence": if abstained { 0.0 } else { (p_best * 1000.0).round() / 1000.0 },
            "valid_mass": (mass * 1000.0).round() / 1000.0,
            "abstained": abstained,
        });
        if q.kind == "bool" {
            a["p_true"] = json!((dist[0] * 1000.0).round() / 1000.0);
        }
        answers.push(a);
    }
    let answers = JsonValue::Array(answers);
    let input_b3 = *blake3::hash(&raw).as_bytes();
    let output_b3 = *blake3::hash(answers.to_string().as_bytes()).as_bytes();
    let decided_at = crate::chrono_iso8601_utc();
    let pk = s.identity.pubkey.0;
    let mut pb = emem_attest::PreimageV1::new(DECIDE_DOMAIN);
    pb.seg(tag::INPUT_BLAKE3, &input_b3);
    pb.seg(tag::MODEL, model.as_bytes());
    pb.seg(tag::OUTPUT_BLAKE3, &output_b3);
    pb.seg(tag::DECIDED_AT, decided_at.as_bytes());
    pb.seg(tag::RESPONDER_PUBKEY, &pk);
    let sig = s.identity.signing.sign(&pb.finalize()).to_bytes();
    let b32 = |b: &[u8]| data_encoding::BASE32_NOPAD.encode(b).to_ascii_lowercase();
    Json(json!({
        "answers": answers,
        "model": model,
        "latency_ms": started.elapsed().as_millis() as u64,
        "provenance_class": "model_output",
        "calibration": "uncalibrated: the model's own probabilities over the listed options; measure them on labelled cases before reading them as a hit rate",
        "abstain_rule": format!("value is null when the option letters carry under {ABSTAIN_BELOW} of the first token's probability"),
        "decided_at": decided_at,
        "receipt": {
            "domain": DECIDE_DOMAIN,
            "preimage": "PreimageV1(\"emem.decide.v1\"){1:blake3(request body), 2:model, 3:blake3(answers json), 4:decided_at, 5:responder_pubkey}",
            "input_blake3_b32": b32(&input_b3),
            "output_blake3_b32": b32(&output_b3),
            "responder_pubkey_b32": b32(&pk),
            "signature_b32": b32(&sig),
            "note": "model_output: this signs which input, model and answers went together, not that the answers are right",
        },
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(kind: &str, options: &[&str], scale: Option<[i64; 2]>) -> Question {
        Question {
            id: None,
            kind: kind.into(),
            ask: "?".into(),
            options: options.iter().map(|s| s.to_string()).collect(),
            scale,
        }
    }

    #[test]
    fn questions_have_a_closed_answer_set() {
        assert_eq!(
            options_for(&q("bool", &[], None)).unwrap(),
            vec!["yes", "no"]
        );
        assert_eq!(
            options_for(&q("score", &[], Some([0, 3]))).unwrap(),
            vec!["0", "1", "2", "3"]
        );
        assert_eq!(
            options_for(&q("choice", &["a", "b"], None)).unwrap(),
            vec!["a", "b"]
        );
        assert!(options_for(&q("choice", &["only"], None)).is_err());
        assert!(options_for(&q("score", &[], Some([1, 20]))).is_err());
        assert!(options_for(&q("freeform", &[], None)).is_err());
    }

    #[test]
    fn the_distribution_is_over_letters_and_says_how_much_they_carry() {
        let top = vec![
            (" A".to_string(), (0.6f64).ln()),
            ("B)".to_string(), (0.2f64).ln()),
            ("The".to_string(), (0.15f64).ln()),
            ("Z".to_string(), (0.05f64).ln()),
        ];
        let (p, mass) = letter_distribution(&top, 3);
        assert!((mass - 0.8).abs() < 1e-9, "{mass}");
        assert!((p[0] - 0.75).abs() < 1e-9 && (p[1] - 0.25).abs() < 1e-9 && p[2] == 0.0);
        // A token outside the option set is never counted, even a letter.
        let (_, m) = letter_distribution(&[("D".into(), 0.0)], 3);
        assert_eq!(m, 0.0);
    }
}
