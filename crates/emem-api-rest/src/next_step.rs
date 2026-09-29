//! Structured next steps: `{verb, noun, args?, code?}`.
//!
//! Advisory prose on this surface drifted from the code it described, while
//! the typed codes beside it did not. So an advisory that tells a caller what
//! to call next also carries that call as data: `verb` is an MCP tool name or
//! a routed REST route (`"GET /v1/cells/:cell64/info"`), `noun` names the
//! field or thing it acts on, and `args` is valid input for that verb. The
//! sentence stays for people to read; the step is what a test can hold to
//! the tool registry and the router.

use serde_json::{json, Map, Value as JsonValue};

/// One step. `args` is left out when there is nothing to pass.
pub(crate) fn step(verb: &str, noun: &str, args: JsonValue) -> JsonValue {
    let mut s = json!({ "verb": verb, "noun": noun });
    if args.as_object().is_some_and(|m| !m.is_empty()) {
        s["args"] = args;
    }
    s
}

/// The step a signed-write refusal hands back: resend to the same tool with
/// the attester block the refusal's `how_to_sign` describes.
pub(crate) fn attester_step(tool: &str, code: &str) -> JsonValue {
    let mut s = step(
        tool,
        "attester",
        json!({ "attester": {
            "pubkey_b32": "<52-char base32 ed25519 public key>",
            "sig_b32": "<base32 ed25519 signature over the raw bytes of how_to_sign.sign_this.digest_hex>",
        }}),
    );
    s["code"] = json!(code);
    s
}

/// A refusal's `details.next`: the attester step, when `tool` is one this
/// responder registers. A name that is not a tool gets no step rather than a
/// verb nothing answers to.
pub(crate) fn attester_next(tool: &str, code: &str) -> JsonValue {
    if emem_mcp::lookup(tool).is_none() {
        return json!([]);
    }
    json!([attester_step(tool, code)])
}

/// A step with the sentence it replaces kept beside it, for people.
fn said(mut s: JsonValue, text: &str) -> JsonValue {
    s["text"] = json!(text);
    s
}

/// `/v1/locate`'s `next`: what to call with the cell it resolved. `place` is
/// passed only when the place has an extent worth sampling.
pub(crate) fn locate_steps(
    cell: &str,
    neighborhood: &[String],
    extent_place: Option<&str>,
) -> JsonValue {
    let mut out = vec![
        said(
            step("emem_recall", "cell", json!({ "cell": cell })),
            "POST /v1/recall {\"cell\": \"<cell64>\", \"bands\": [...]}",
        ),
        said(
            step(
                "emem_recall_many",
                "neighborhood_cells",
                json!({ "cells": neighborhood }),
            ),
            "for a point feature, recall the ~9 cells around it and union the results",
        ),
    ];
    if let Some(p) = extent_place {
        out.push(said(
            step("emem_recall_polygon", "polygon_bbox", json!({ "place": p })),
            "for a wide feature, recall across its extent rather than one cell",
        ));
    }
    out.extend([
        said(
            step("emem_find_similar", "cell", json!({ "key": cell })),
            "POST /v1/find_similar",
        ),
        said(
            step("emem_compare", "cell", json!({ "a": cell })),
            "POST /v1/compare",
        ),
        said(
            step(
                "GET /v1/cells/:cell64/info",
                "cell",
                json!({ "cell64": cell }),
            ),
            "GET  /v1/cells/{cell64}/info",
        ),
        said(
            step("GET /v1/grid_info", "grid", json!({})),
            "GET  /v1/grid_info , actual vs spec-target resolution",
        ),
    ]);
    JsonValue::Array(out)
}

/// `/v1/locate`'s `next` when it was given nothing to resolve.
pub(crate) fn locate_needs_location_steps() -> JsonValue {
    json!([
        step(
            "emem_locate",
            "place",
            json!({ "place": "<place name from the user's turn>" })
        ),
        step("emem_locate", "lat", json!({ "lat": 35.36, "lng": 138.73 })),
    ])
}

/// `/v1/ask`'s `next` when it has no place it trusts: ask again with one
/// (the guessed `candidate` when there is one), or ground it first.
pub(crate) fn ask_needs_place_steps(q: &str, candidate: Option<&str>) -> JsonValue {
    let place = candidate.unwrap_or("<place name from the user's turn>");
    json!([
        step("emem_ask", "place", json!({ "q": q, "place": place })),
        step("emem_locate", "place", json!({ "place": place })),
    ])
}

/// `/v1/ask`'s `next` for a definitional question: the registries that
/// define the terms, or ask again about a place.
pub(crate) fn ask_definitional_steps(q: &str) -> JsonValue {
    json!([
        step("GET /v1/bands", "bands", json!({})),
        step("GET /v1/algorithms", "algorithms", json!({})),
        step(
            "emem_ask",
            "place",
            json!({ "q": q, "place": "<the place the question is about>" })
        ),
    ])
}

/// `/v1/ask`'s `next_steps` entry on a cold cell: recall the bands the routed
/// algorithms needed. The REST fields predate the step and stay.
pub(crate) fn ask_missing_bands_step(cell: &str, bands: &[String], origin: &str) -> JsonValue {
    let mut s = step(
        "emem_recall",
        "bands",
        json!({ "cell": cell, "bands": bands }),
    );
    if let Some(m) = s.as_object_mut() {
        m.insert("action".into(), json!("recall"));
        m.insert("why".into(), json!("These bands are not yet materialized at this cell, so the routed algorithms could not evaluate. Recall them once to fetch and sign them, then re-issue the question."));
        m.insert("method".into(), json!("POST"));
        m.insert("path".into(), json!("/v1/recall"));
        m.insert("url".into(), json!(format!("{origin}/v1/recall")));
        m.insert("body".into(), json!({ "cell": cell, "bands": bands }));
    }
    s
}

/// The REST route whose OpenAPI `operationId` is `tool`, as `(METHOD, path)`.
/// A path with parameters is skipped: its arguments are not ours to fill.
fn route_of_tool(tool: &str) -> Option<(String, String)> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static BY_OP: OnceLock<HashMap<String, (String, String)>> = OnceLock::new();
    if tool.is_empty() {
        return None;
    }
    let by_op = BY_OP.get_or_init(|| {
        let spec = crate::openapi_spec();
        let mut m = HashMap::new();
        for (path, ops) in spec["paths"].as_object().into_iter().flatten() {
            if path.contains('{') || path.contains(':') {
                continue;
            }
            for (method, op) in ops.as_object().into_iter().flatten() {
                if let Some(id) = op["operationId"].as_str() {
                    m.entry(id.to_string())
                        .or_insert_with(|| (method.to_ascii_uppercase(), path.clone()));
                }
            }
        }
        m
    });
    by_op.get(tool).cloned()
}

/// Property names of a tool's input schema, or None for an unknown tool.
fn tool_properties(tool: &str) -> Option<JsonValue> {
    let d = emem_mcp::lookup(tool)?;
    let schema: JsonValue = serde_json::from_str(d.input_schema).ok()?;
    Some(
        schema
            .get("properties")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
}

/// Bytes to reserve for the steps [`truncation_steps`] will add, sized from
/// the fields they copy out of the result.
pub(crate) fn truncation_steps_reserve(map: &Map<String, JsonValue>) -> usize {
    const COPIED: &[&str] = &[
        "cell",
        "cell64",
        "place",
        "question",
        "bundle_token",
        "path",
        "place_resolved",
    ];
    let copied: usize = COPIED
        .iter()
        .filter_map(|k| map.get(*k))
        .map(|v| match v {
            // Only `cell64` is copied out of `place_resolved`.
            JsonValue::Object(_) => 32,
            other => serde_json::to_string(other).map(|s| s.len()).unwrap_or(0),
        })
        .sum();
    256 + copied
}

/// The calls that return what a slimmed result omitted.
///
/// `fetch` is the REST request the slimmer already built (`method`, `path`,
/// `body`). An array cut to a prefix gets a paging step only when `tool`
/// itself takes `offset`: the stub's `_next_offset` is true of every cut
/// array, and naming it as an argument for a tool that has none sends the
/// caller to a call that ignores it.
pub(crate) fn truncation_steps(
    map: &Map<String, JsonValue>,
    dropped: &[JsonValue],
    fetch: Option<&JsonValue>,
    tool: &str,
) -> Vec<JsonValue> {
    let mut out = Vec::new();
    if let Some(t) = map.get("bundle_token").and_then(|v| v.as_str()) {
        out.push(step(
            "GET /v1/memory_bundle/:token",
            "omitted_fields",
            json!({ "token": t }),
        ));
    } else if let Some(f) = fetch {
        let method = f.get("method").and_then(|v| v.as_str()).unwrap_or("POST");
        if let Some(path) = f.get("path").and_then(|v| v.as_str()) {
            let args = f.get("body").cloned().unwrap_or_else(|| json!({}));
            out.push(step(&format!("{method} {path}"), "omitted_fields", args));
        }
    }
    // A result with no `schema` gives the slimmer nothing to rebuild a call
    // from, but the tool's own REST route is still known, by its OpenAPI
    // operationId: the same arguments sent there come back uncapped.
    if out.is_empty() {
        if let Some((method, path)) = route_of_tool(tool) {
            out.push(step(
                &format!("{method} {path}"),
                "omitted_fields",
                json!({}),
            ));
        }
    }
    let Some(props) = tool_properties(tool) else {
        return out;
    };
    if props.get("offset").is_none() {
        return out;
    }
    // (field in the result, argument it goes back in as)
    const ECHOED: &[(&str, &str)] = &[("path", "path"), ("cell", "cell"), ("filter_kind", "kind")];
    for d in dropped {
        let Some(next) = d["stub"].get("_next_offset").and_then(|v| v.as_u64()) else {
            continue;
        };
        let mut args = Map::new();
        for (from, to) in ECHOED {
            if props.get(*to).is_none() {
                continue;
            }
            if let Some(v) = map.get(*from).filter(|v| v.is_string()) {
                args.insert((*to).into(), v.clone());
            }
        }
        args.insert("offset".into(), json!(next));
        let noun = d["field"].as_str().unwrap_or("omitted_fields");
        out.push(step(tool, noun, JsonValue::Object(args)));
    }
    out
}

/// Routes the router serves, as `(method, pattern)`, read from the router's
/// own source so the gate cannot agree with a list kept beside it.
#[cfg(test)]
pub(crate) fn routed() -> Vec<(String, String)> {
    let src = include_str!("lib.rs");
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find(".route(") {
        rest = &rest[i + ".route(".len()..];
        let body = rest.trim_start();
        let Some(body) = body.strip_prefix('"') else {
            continue;
        };
        let Some(end) = body.find('"') else {
            break;
        };
        let path = &body[..end];
        // The handler expression runs to the next `.route(` at the latest.
        let tail = &body[end..];
        let span = &tail[..tail.find(".route(").unwrap_or(tail.len()).min(400)];
        for m in ["get", "post", "put", "delete", "patch"] {
            let call = format!("{m}(");
            let hit = span.match_indices(&call).any(|(j, _)| {
                j == 0
                    || (!span.as_bytes()[j - 1].is_ascii_alphanumeric()
                        && span.as_bytes()[j - 1] != b'_')
            });
            if hit {
                out.push((m.to_ascii_uppercase(), path.to_string()));
            }
        }
    }
    out
}

/// Every object carrying both `verb` and `noun`, anywhere in `v`.
#[cfg(test)]
pub(crate) fn collect_steps(v: &JsonValue, out: &mut Vec<JsonValue>) {
    match v {
        JsonValue::Object(m) => {
            if m.get("verb").is_some_and(|x| x.is_string()) && m.contains_key("noun") {
                out.push(v.clone());
            }
            for x in m.values() {
                collect_steps(x, out);
            }
        }
        JsonValue::Array(a) => {
            for x in a {
                collect_steps(x, out);
            }
        }
        _ => {}
    }
}

/// Keys of `args` a schema `properties` object does not declare, walking
/// into nested objects that declare their own.
#[cfg(test)]
fn undeclared(args: &JsonValue, props: &JsonValue, at: &str, bad: &mut Vec<String>) {
    let Some(a) = args.as_object() else {
        return;
    };
    for (k, v) in a {
        match props.get(k) {
            None => bad.push(format!("{at}{k}")),
            Some(p) => {
                if let Some(inner) = p.get("properties") {
                    undeclared(v, inner, &format!("{at}{k}."), bad);
                }
            }
        }
    }
}

/// Is `step` a call something here answers, with arguments it accepts?
///
/// A tool verb must be registered and every `args` key a property of its
/// input schema. A route verb must be served by the router with that method,
/// and every `args` key is a path parameter of the route or a property its
/// OpenAPI operation declares.
#[cfg(test)]
pub(crate) fn check_step(
    step: &JsonValue,
    routes: &[(String, String)],
    openapi: &JsonValue,
) -> Result<(), String> {
    let verb = step["verb"].as_str().ok_or("verb is not a string")?;
    if !step["noun"].as_str().is_some_and(|n| !n.is_empty()) {
        return Err(format!("{verb}: noun is missing or empty"));
    }
    let args = step.get("args").cloned().unwrap_or_else(|| json!({}));
    if !args.is_object() {
        return Err(format!("{verb}: args is not an object"));
    }
    let mut bad = Vec::new();
    if let Some((method, path)) = verb.split_once(' ') {
        if !routes.iter().any(|(m, p)| m == method && p == path) {
            return Err(format!("{verb}: no such route in the router"));
        }
        let params: Vec<&str> = path
            .split('/')
            .filter_map(|seg| seg.strip_prefix(':'))
            .collect();
        let op = &openapi["paths"][path][method.to_ascii_lowercase()];
        let mut declared = Map::new();
        let body = &op["requestBody"]["content"]["application/json"]["schema"];
        let body = match body.get("$ref").and_then(|r| r.as_str()) {
            Some(r) => openapi
                .pointer(r.trim_start_matches('#'))
                .cloned()
                .unwrap_or(JsonValue::Null),
            None => body.clone(),
        };
        if let Some(p) = body.get("properties").and_then(|p| p.as_object()) {
            declared.extend(p.clone());
        }
        for prm in op["parameters"].as_array().into_iter().flatten() {
            if let Some(n) = prm["name"].as_str() {
                declared.insert(n.into(), json!({}));
            }
        }
        for p in &params {
            declared.insert((*p).into(), json!({}));
        }
        undeclared(&args, &JsonValue::Object(declared), "", &mut bad);
    } else {
        let props =
            tool_properties(verb).ok_or_else(|| format!("{verb}: not a registered MCP tool"))?;
        undeclared(&args, &props, "", &mut bad);
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("{verb}: args not in its input schema: {bad:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_route_reader_sees_the_router() {
        let r = routed();
        assert!(r.len() > 300, "read {} routes", r.len());
        for want in [
            ("POST", "/v1/recall"),
            ("GET", "/v1/locate"),
            ("GET", "/v1/cells/:cell64/info"),
        ] {
            assert!(
                r.iter().any(|(m, p)| m == want.0 && p == want.1),
                "missing {want:?}"
            );
        }
    }

    /// Hold every step a response names to the registry and the router.
    fn gate(surface: &str, v: &JsonValue, min: usize) -> usize {
        let routes = routed();
        let api = crate::openapi_spec();
        let mut steps = Vec::new();
        collect_steps(v, &mut steps);
        assert!(
            steps.len() >= min,
            "{surface}: expected at least {min} steps, found {}: {v}",
            steps.len()
        );
        for s in &steps {
            if let Err(e) = check_step(s, &routes, &api) {
                panic!("{surface}: {e}\nstep: {s}");
            }
        }
        steps.len()
    }

    /// `details` out of an MCP refusal, which carries them in its message.
    fn mcp_details(msg: &str) -> JsonValue {
        let (_, d) = msg
            .split_once("details:\n")
            .expect("the refusal carries details");
        serde_json::from_str(d).expect("details are JSON")
    }

    /// Every structured step the four pilot surfaces emit names a real verb
    /// with arguments that verb accepts. Built from the handlers where they
    /// run offline, and from the builders they call where they do not.
    #[tokio::test]
    async fn every_emitted_step_names_a_real_call() {
        let s = crate::tests::test_app_state();

        // 1. Truncation, on real tool output slimmed below its size.
        let bands = crate::mcp_tool_call("emem_bands", json!({}), &s)
            .await
            .unwrap();
        let (_, note) = crate::mcp_slim_inner_to_budget_for(bands, 6_000, "emem_bands");
        gate("truncation: emem_bands", &note, 1);

        for i in 0..60 {
            crate::mcp_tool_call(
                "emem_memory_create",
                json!({"path": format!("/memories/gate/note-{i:03}-with-a-title-long-enough-to-matter.md"), "file_text": "x"}),
                &s,
            )
            .await
            .unwrap();
        }
        let listing =
            crate::mcp_tool_call("emem_memory_view", json!({"path": "/memories/gate/"}), &s)
                .await
                .unwrap();
        let (_, note) = crate::mcp_slim_inner_to_budget_for(listing, 3_000, "emem_memory_view");
        gate("truncation: emem_memory_view", &note, 1);
        let paging = note["next"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["verb"] == "emem_memory_view"))
            .unwrap_or_else(|| panic!("a cut listing names its own paging call: {note}"));
        assert_eq!(paging["args"]["path"], "/memories/gate/");
        assert!(paging["args"]["offset"].as_u64().is_some_and(|n| n > 0));

        let ask = crate::conceptual_question_response("what is ndvi", &[]);
        let (_, note) = crate::mcp_slim_inner_to_budget_for(ask, 1_500, "emem_ask");
        gate("truncation: emem_ask", &note, 1);

        let mut bundle = Map::new();
        bundle.insert("bundle_token".into(), json!("emem:bundle:abcdefghijklmnop"));
        gate(
            "truncation: bundle",
            &JsonValue::Array(truncation_steps(&bundle, &[], None, "emem_memory_bundle")),
            1,
        );

        // 2. Signed-write refusals.
        for (tool, args) in [
            (
                "emem_entity",
                json!({"label": "Lake Test", "lat": 1.0, "lng": 2.0}),
            ),
            (
                "emem_entity_link",
                json!({"entity_cid": "abcdefghijklmnopqrstuvwxyz", "alias": "Test Lake"}),
            ),
            (
                "emem_memory_create",
                json!({"path": "/memories/by_attester/abcd1234/n.md", "file_text": "x"}),
            ),
        ] {
            let (_, msg) = crate::mcp_tool_call(tool, args, &s)
                .await
                .expect_err("unsigned write is refused");
            let d = mcp_details(&msg);
            gate(&format!("refusal: {tool}"), &d["next"], 1);
            assert_eq!(
                d["next"][0]["verb"], tool,
                "{tool}: resend to the same tool"
            );
            assert!(d["next"][0]["code"].is_string());
        }
        let bh = [7u8; 32];
        let bad = crate::MemoryAttester {
            pubkey_b32: "not-a-key".into(),
            sig_b32: "x".into(),
        };
        for verb in [
            "create",
            "str_replace",
            "insert",
            "supersede",
            "delete",
            "rename",
        ] {
            for att in [None, Some(&bad)] {
                let e = crate::validate_attester_binding(
                    verb,
                    "/memories/by_attester/abcd1234/n.md",
                    &bh,
                    "absent",
                    att,
                )
                .expect_err("refused");
                let d = e.1.details.unwrap_or_default();
                gate(&format!("refusal: memory {verb}"), &d["next"], 1);
                assert_eq!(d["next"][0]["verb"], format!("emem_memory_{verb}"));
            }
        }
        for att in [None, Some(&bad)] {
            let e = crate::validate_derive_attester(&bh, att).expect_err("refused");
            gate(
                "refusal: derive",
                &e.1.details.unwrap_or_default()["next"],
                1,
            );
        }

        // 3. Locate, resolved and unresolved.
        let located = crate::locate_inner(crate::LocateReq {
            lat: Some(35.36),
            lng: Some(138.73),
            place: None,
        })
        .await
        .unwrap()
        .0;
        gate("locate", &located["next"], 6);
        let cell = located["cell64"].as_str().unwrap();
        assert_eq!(located["next"][0]["args"]["cell"], cell);
        let empty = crate::locate_inner(crate::LocateReq {
            lat: None,
            lng: None,
            place: None,
        })
        .await
        .unwrap()
        .0;
        gate("locate: needs_location", &empty["next"], 2);
        gate(
            "locate: extent",
            &locate_steps(cell, &[cell.to_string()], Some("Lake Test")),
            7,
        );

        // 4. Ask.
        gate(
            "ask: definitional",
            &crate::conceptual_question_response("what is ndvi", &[]),
            3,
        );
        gate(
            "ask: needs place",
            &ask_needs_place_steps("ndvi here?", Some("Pune")),
            2,
        );
        gate(
            "ask: missing bands",
            &ask_missing_bands_step(cell, &["indices.ndvi".to_string()], "https://emem.dev"),
            1,
        );
    }

    /// The gate has to be able to say no, for each way a step can be wrong.
    #[test]
    fn the_checker_rejects_bad_steps() {
        let routes = routed();
        let api = crate::openapi_spec();
        let good = step(
            "emem_recall",
            "cell",
            json!({"cell": "defi.zb572.xoso.zb1ec"}),
        );
        assert_eq!(check_step(&good, &routes, &api), Ok(()));
        for bad in [
            step("emem_recal", "cell", json!({"cell": "x"})),
            step("emem_recall", "cell", json!({"celll": "x"})),
            step("emem_memory_view", "entries", json!({"cursor": 3})),
            step("POST /v1/recal", "cell", json!({})),
            step("DELETE /v1/recall", "cell", json!({})),
            step("GET /v1/cells/:cell64/info", "cell", json!({"cell": "x"})),
            step(
                "emem_entity",
                "attester",
                json!({"attester": {"pubkey": "x"}}),
            ),
            json!({"verb": "emem_recall", "noun": ""}),
        ] {
            assert!(check_step(&bad, &routes, &api).is_err(), "accepted {bad}");
        }
    }
}
