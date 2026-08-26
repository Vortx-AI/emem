//! Who may write, and what they had to prove to get there.
//!
//! `NousResearch/hermes-agent#79583` was closed on 2026-08-25 with an objection
//! worth quoting exactly, because it is correct and it is not "add OAuth":
//!
//! > a no-auth shared memory that arbitrary agents read AND write is a textbook
//! > cross-agent prompt-injection and data-poisoning surface: anyone can plant
//! > content that other agents will recall as fact.
//! >
//! > Signed provenance mitigates attribution, not injected-instruction risk.
//!
//! The second sentence is the one to internalise. A signature says WHO wrote a
//! thing. It does not stop a reader obeying it, and it does not say the writer
//! was entitled to write there. Anyone can mint an ed25519 key, so "signed" is
//! a floor, not a bar.
//!
//! # What this module is, and is not
//!
//! It is NOT an auth wall. Reads stay anonymous and free at every tier, for
//! ever: gating reads would trade the one property that makes this substrate
//! worth using for a security benefit it does not produce, since a reader
//! cannot poison anything.
//!
//! It gates WRITES, and it gates them on BLAST RADIUS rather than on identity.
//! The question is never "how much do we trust this key" — it is "how far can
//! what this key writes reach, and what did it prove commensurate with that".
//! Writing prose into your own namespace reaches nobody who did not ask for
//! it, and stays free on first contact. Writing the SHARED entity address
//! space changes what every other agent resolves a name to, and is the one
//! genuine poisoning surface in the current design.
//!
//! # A tier is a record of what was checked, never a score
//!
//! `trust: caller_decides` is the best property on the roster and this must
//! not erode it. Every tier below names a CHECK THAT PASSED and what a peer
//! may conclude from it. None of them says a party is trustworthy. A caller
//! that collapses these into a boolean has thrown away the distinction on
//! purpose, and the roster says so.
//!
//! # Why DNS and `.well-known` rather than OAuth
//!
//! Browser OAuth 2.1 + Dynamic Client Registration authenticates a SESSION:
//! did a human, in a browser, just now authorise this client. An autonomous
//! agent has no human and no browser, so DCR degrades to a bearer token that
//! proves possession and says nothing about accountability. It is also
//! structurally uncompletable headless, which is why the catalog offers a
//! second door at all.
//!
//! An agent needs the PRINCIPAL authenticated: who is accountable for what
//! this key says. That is a name-control problem, and name control was solved
//! three times already — DKIM, ACME, Certificate Transparency.
//!
//! The decisive property is re-verification by a third party. A bearer token
//! proves nothing to a third agent. A DNS TXT record is checkable by anyone,
//! for ever, without trusting this responder — the same property that makes
//! our receipts worth having. It survives our compromise, because the evidence
//! lives off our server.

use serde::{Deserialize, Serialize};

/// How an organisation vouched for a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// `_emem-agent.<domain>  TXT  "v=emem1; k=<52-char key>; nick=<name>"`.
    ///
    /// Strongest: only the domain holder can publish it, revocation is
    /// deletion, and any third party can re-check it without us.
    Dns,
    /// `https://<domain>/.well-known/emem-agents.json`.
    ///
    /// Equal trust via TLS + origin control, and easier for a team without
    /// DNS access. Can carry roles and an expiry the TXT form cannot.
    WellKnown,
    /// An organisation's own emem key countersigns the agent's profile.
    ///
    /// Pure ledger, no external dependency — and its limit is stated rather
    /// than glossed: it proves one key vouched for another, NOT that the
    /// organisation exists. Trust in the org key still has to come from
    /// [`Method::Dns`] or [`Method::WellKnown`].
    CrossSig,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Dns => "dns",
            Method::WellKnown => "well_known",
            Method::CrossSig => "cross_sig",
        }
    }
}

/// The outcome of one check, always carrying WHEN it was checked.
///
/// `checked_at` is not decoration. An attestation with no age is a claim about
/// the present made from the past, and a domain holder who deleted the record
/// this morning is still vouched for by a check from March. Every render of
/// this shows the age, and [`Evidence::is_fresh`] decides whether it may still
/// carry a tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub method: Method,
    pub domain: String,
    pub nick: Option<String>,
    /// Unix seconds.
    pub checked_at: u64,
    /// Which check passed, or why it did not. Never a bare boolean: a caller
    /// deciding for itself needs the reason, not our verdict.
    pub detail: String,
    pub ok: bool,
}

/// How long an org attestation stands before it must be re-checked.
///
/// Thirty days is the same order as a TLS certificate's practical rotation and
/// short enough that a deleted record stops vouching within a month. The
/// re-check is cheap; the failure mode of never re-checking is a permanent
/// claim about a name someone gave up.
pub const EVIDENCE_TTL_SECS: u64 = 30 * 24 * 3600;

impl Evidence {
    pub fn is_fresh(&self, now: u64) -> bool {
        self.ok && now.saturating_sub(self.checked_at) < EVIDENCE_TTL_SECS
    }
}

/// What was checked about a key. Ascending, and each is a check, not a score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// A signed note exists. A peer may conclude: this key wrote this.
    T0Anonymous,
    /// Full key resolvable and the namespace proven by signature. A peer may
    /// conclude: this key controls this namespace.
    T1Keyed,
    /// A signed `profile.md` with a unique nick. A peer may conclude: a stable
    /// identity, one party or a declared subkey.
    T2Named,
    /// A reachable endpoint with declared skills. A peer may conclude:
    /// callable and testable.
    T3Declared,
    /// An organisation vouched for the key by `dns` / `well_known` /
    /// `cross_sig`. A peer may conclude: someone accountable in the real
    /// world is named.
    T4Affiliated,
    /// Three or more distinct keys received this agent's token and confirmed
    /// it matched. A peer may conclude: other agents checked its work.
    ///
    /// The only tier that reflects work rather than paperwork, and the only
    /// one that cannot be bought.
    T5Corroborated,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::T0Anonymous => "T0_anonymous",
            Tier::T1Keyed => "T1_keyed",
            Tier::T2Named => "T2_named",
            Tier::T3Declared => "T3_declared",
            Tier::T4Affiliated => "T4_affiliated",
            Tier::T5Corroborated => "T5_corroborated",
        }
    }

    /// What a peer may conclude. Deliberately phrased as an observation and
    /// never as a recommendation.
    pub fn means(self) -> &'static str {
        match self {
            Tier::T0Anonymous => "this key signed this note",
            Tier::T1Keyed => "this key controls this namespace",
            Tier::T2Named => "a stable identity: one party, or a declared subkey",
            Tier::T3Declared => "callable and testable at a declared endpoint",
            Tier::T4Affiliated => "an organisation controlling a name vouches for this key",
            Tier::T5Corroborated => "other agents received its tokens and confirmed they matched",
        }
    }
}

/// The observable facts a tier is computed from. Every field is something the
/// store already holds or can check; none of it is an opinion.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub has_signed_note: bool,
    /// A caller signature proved control of the namespace.
    pub namespace_proven: bool,
    pub has_profile_with_nick: bool,
    pub declares_endpoint: bool,
    /// A FRESH org attestation. Staleness is resolved before this is set.
    pub org_verified: bool,
    /// Distinct peer keys that confirmed one of this agent's tokens matched.
    pub corroborating_peers: usize,
}

/// Peers required before an agent is corroborated.
///
/// Three, not two: two keys can be one operator, and the whole point of the
/// tier is that it cannot be self-issued. It is also the one tier already
/// paid for — arcade `ack`/`receive` acts carry `match` and `from_pk8` and are
/// signed, so this is an aggregation over evidence we hold, not new ceremony.
pub const CORROBORATING_PEERS: usize = 3;

/// The highest tier whose check has passed.
///
/// Monotonic on purpose: a tier is the ceiling of what was proven, so a key
/// that is corroborated but never published a profile still reads T5. The
/// individual checks stay visible alongside it, because "which check passed"
/// is the answer a caller deciding for itself actually needs.
pub fn tier_for(f: &Facts) -> Tier {
    if f.corroborating_peers >= CORROBORATING_PEERS {
        return Tier::T5Corroborated;
    }
    if f.org_verified {
        return Tier::T4Affiliated;
    }
    if f.declares_endpoint {
        return Tier::T3Declared;
    }
    if f.has_profile_with_nick {
        return Tier::T2Named;
    }
    if f.namespace_proven {
        return Tier::T1Keyed;
    }
    let _ = f.has_signed_note;
    Tier::T0Anonymous
}

/// A write surface, ordered by how far its effect reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// Anything a caller reads. Never gated, at any tier, ever.
    Read,
    /// Prose under the caller's own `/memories/by_attester/<pk8>/`.
    /// Reaches nobody who did not ask for it.
    OwnNamespace,
    /// The SHARED entity address space: `entity`, `entity_link`. Changes what
    /// every other agent resolves a name to. This is the one genuine
    /// poisoning surface in the current design.
    SharedEntitySpace,
    /// Proposing a fact into the band-typed plane.
    FactPlane,
}

/// Minimum tier for a surface.
///
/// Two properties matter more than the numbers. **T1 is the floor and stays
/// free** — a stranger's agent still writes prose on first contact with
/// nothing but a signature, exactly as today, so this is not a regression
/// dressed as security. And **nothing below T4 reaches the fact plane**, which
/// is today's behaviour (no caller can write a fact at all) made explicit and
/// checkable rather than emergent from there being no route.
pub fn min_tier(surface: Surface) -> Tier {
    match surface {
        Surface::Read => Tier::T0Anonymous,
        Surface::OwnNamespace => Tier::T1Keyed,
        Surface::SharedEntitySpace => Tier::T3Declared,
        Surface::FactPlane => Tier::T4Affiliated,
    }
}

/// Whether `tier` may write `surface`, and if not, what would let it.
pub fn may_write(tier: Tier, surface: Surface) -> Result<(), String> {
    let need = min_tier(surface);
    if tier >= need {
        return Ok(());
    }
    Err(format!(
        "this surface needs {} ({}); this key is at {} ({}). This is not a \
         paywall and not an account: raise the tier by passing a check. See \
         GET /v1/enlist for the checks and what each one proves. Reads are \
         never gated at any tier.",
        need.as_str(),
        need.means(),
        tier.as_str(),
        tier.means(),
    ))
}

// ── The two external checks ─────────────────────────────────────────────

/// The DNS label an organisation publishes under.
pub const DNS_LABEL: &str = "_emem-agent";
/// The `.well-known` document an organisation may serve instead.
pub const WELL_KNOWN_PATH: &str = "/.well-known/emem-agents.json";

/// Parse one TXT record body, returning the nick if it vouches for `key`.
///
/// Format: `v=emem1; k=<52-char key>; nick=<name>`. Semicolon-separated,
/// whitespace-insensitive, order-insensitive. A record that names a DIFFERENT
/// key is not a failure of the domain — a domain may vouch for many agents —
/// so this returns None and the caller keeps looking.
pub fn parse_dns_txt(record: &str, key: &str) -> Option<String> {
    let mut version_ok = false;
    let mut matched = false;
    let mut nick = None;
    for part in record.split(';') {
        let part = part.trim().trim_matches('"').trim();
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        match k.trim() {
            "v" => version_ok = v.trim() == "emem1",
            "k" => matched = v.trim() == key,
            "nick" => nick = Some(v.trim().to_string()),
            _ => {}
        }
    }
    if version_ok && matched {
        Some(nick.unwrap_or_default())
    } else {
        None
    }
}

/// One entry in a `.well-known/emem-agents.json` document.
#[derive(Debug, Clone, Deserialize)]
pub struct WellKnownAgent {
    pub key: String,
    #[serde(default)]
    pub nick: Option<String>,
    /// Unix seconds. An entry past its expiry does not vouch.
    #[serde(default)]
    pub expires: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WellKnownDoc {
    #[serde(default)]
    pub agents: Vec<WellKnownAgent>,
}

/// Whether a `.well-known` document vouches for `key` at time `now`.
pub fn well_known_vouches(doc: &WellKnownDoc, key: &str, now: u64) -> Option<String> {
    doc.agents
        .iter()
        .find(|a| a.key == key && a.expires.is_none_or(|e| e > now))
        .map(|a| a.nick.clone().unwrap_or_default())
}

/// Refuse a host that would turn this verifier into a probe of our own network.
///
/// Fetching a caller-supplied URL from inside the responder is server-side
/// request forgery unless the destination is bounded. A verifier that can be
/// pointed at `169.254.169.254` or `127.0.0.1:5051` reads cloud metadata and
/// our own admin surfaces on behalf of a stranger, and it does it with our
/// source address.
///
/// This checks the NAME. It cannot close the gap between this check and the
/// connection — a name that resolves publicly now can resolve privately a
/// moment later (DNS rebinding), and closing that needs resolution pinned to
/// the socket, which reqwest does not expose. So this is a bound, not a proof,
/// and it is written down as one rather than described as SSRF-safe.
pub fn host_is_publicly_routable(host: &str) -> Result<(), String> {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if h.is_empty() {
        return Err("empty host".into());
    }
    if h == "localhost"
        || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || h.ends_with(".home.arpa")
    {
        return Err(format!("{h} is a local name"));
    }
    // An IP literal is never a domain an organisation controls a NAME for, and
    // the whole mechanism is about name control, so refuse all of them rather
    // than only the private ones. That also closes the literal half of SSRF
    // without enumerating every reserved range.
    if h.parse::<std::net::IpAddr>().is_ok() {
        return Err(format!(
            "{h} is an IP literal; org verification is about controlling a NAME"
        ));
    }
    if !h.contains('.') {
        return Err(format!("{h} is not a fully qualified domain"));
    }
    if h.contains('/') || h.contains(':') || h.contains('@') {
        return Err(format!("{h} is not a bare hostname"));
    }
    Ok(())
}

/// Whether a resolved address may be connected to.
///
/// The second half of the bound above, applied after resolution: a public NAME
/// that resolves into private space is the interesting attack, not a private
/// literal that any check catches.
pub fn addr_is_publicly_routable(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                // CGNAT 100.64.0.0/10, which is_private() does not cover.
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1])))
        }
        std::net::IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                // Unique-local fc00::/7 and link-local fe80::/10.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80)
        }
    }
}

// ── Performing the checks ───────────────────────────────────────────────

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn fail(method: Method, domain: &str, detail: impl Into<String>) -> Evidence {
    Evidence {
        method,
        domain: domain.to_string(),
        nick: None,
        checked_at: now_secs(),
        detail: detail.into(),
        ok: false,
    }
}

/// Resolve `host` and refuse if ANY address is not publicly routable.
///
/// Any, not all: a name with one public and one loopback address is a
/// rebinding attack with the work already done. The gap between this check and
/// the connection reqwest makes is real and unclosed — see
/// [`host_is_publicly_routable`].
async fn refuse_private_targets(host: &str) -> Result<(), String> {
    host_is_publicly_routable(host)?;
    let addrs = tokio::net::lookup_host((host.to_string(), 443u16))
        .await
        .map_err(|e| format!("{host} did not resolve: {e}"))?;
    let mut any = false;
    for a in addrs {
        any = true;
        if !addr_is_publicly_routable(a.ip()) {
            return Err(format!(
                "{host} resolves to {}, which is not publicly routable; refusing to \
                 fetch on a caller's behalf",
                a.ip()
            ));
        }
    }
    if !any {
        return Err(format!("{host} resolved to no addresses"));
    }
    Ok(())
}

/// Check `_emem-agent.<domain>` TXT for a record vouching for `key`.
///
/// Over DNS-over-HTTPS rather than a resolver library, for two reasons: it
/// adds no dependency to a crate that already has an HTTP client, and the
/// answer is one a third party can reproduce with `dig` and compare. The
/// resolver is a convenience, never an authority — the record it returns is
/// the evidence, and anyone can fetch it themselves.
pub async fn check_dns(domain: &str, key: &str) -> Evidence {
    if let Err(e) = host_is_publicly_routable(domain) {
        return fail(Method::Dns, domain, e);
    }
    let name = format!("{DNS_LABEL}.{domain}");
    let url = format!("https://cloudflare-dns.com/dns-query?name={name}&type=TXT");
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => return fail(Method::Dns, domain, format!("client: {e}")),
    };
    let resp = client
        .get(&url)
        .header("accept", "application/dns-json")
        .send()
        .await;
    let body: serde_json::Value = match resp {
        Ok(r) => match r.json().await {
            Ok(v) => v,
            Err(e) => return fail(Method::Dns, domain, format!("resolver body: {e}")),
        },
        Err(e) => return fail(Method::Dns, domain, format!("resolver: {e}")),
    };
    let answers = body
        .get("Answer")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    if answers.is_empty() {
        return fail(
            Method::Dns,
            domain,
            format!("no TXT at {name}. Publish: \"v=emem1; k={key}; nick=<name>\""),
        );
    }
    for a in &answers {
        let Some(data) = a.get("data").and_then(|d| d.as_str()) else {
            continue;
        };
        if let Some(nick) = parse_dns_txt(data, key) {
            return Evidence {
                method: Method::Dns,
                domain: domain.to_string(),
                nick: Some(nick),
                checked_at: now_secs(),
                detail: format!("TXT at {name} names this key"),
                ok: true,
            };
        }
    }
    fail(
        Method::Dns,
        domain,
        format!(
            "{} TXT record(s) at {name}, none naming this key. A domain may vouch \
             for many agents; add one for this one.",
            answers.len()
        ),
    )
}

/// Check `https://<domain>/.well-known/emem-agents.json` for `key`.
pub async fn check_well_known(domain: &str, key: &str) -> Evidence {
    if let Err(e) = refuse_private_targets(domain).await {
        return fail(Method::WellKnown, domain, e);
    }
    let url = format!("https://{domain}{WELL_KNOWN_PATH}");
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        // A redirect is a second destination the guard above never saw, which
        // is the cheapest way around it.
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => return fail(Method::WellKnown, domain, format!("client: {e}")),
    };
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return fail(Method::WellKnown, domain, format!("fetch {url}: {e}")),
    };
    if !resp.status().is_success() {
        return fail(
            Method::WellKnown,
            domain,
            format!(
                "{url} answered {} (redirects are not followed)",
                resp.status()
            ),
        );
    }
    // Bounded read: a verifier that will stream an unbounded body on a
    // stranger's say-so is a memory exhaustion tool.
    let text = match resp.text().await {
        Ok(t) if t.len() <= 256 * 1024 => t,
        Ok(t) => {
            return fail(
                Method::WellKnown,
                domain,
                format!("{url} returned {} bytes; cap is 256 KiB", t.len()),
            )
        }
        Err(e) => return fail(Method::WellKnown, domain, format!("body: {e}")),
    };
    let doc: WellKnownDoc = match serde_json::from_str(&text) {
        Ok(d) => d,
        Err(e) => {
            return fail(
                Method::WellKnown,
                domain,
                format!("{url} is not the expected JSON: {e}"),
            )
        }
    };
    match well_known_vouches(&doc, key, now_secs()) {
        Some(nick) => Evidence {
            method: Method::WellKnown,
            domain: domain.to_string(),
            nick: Some(nick),
            checked_at: now_secs(),
            detail: format!("{url} lists this key"),
            ok: true,
        },
        None => fail(
            Method::WellKnown,
            domain,
            format!(
                "{url} has {} agent entr(ies), none naming this key unexpired",
                doc.agents.len()
            ),
        ),
    }
}

// ── The served surface ──────────────────────────────────────────────────

/// The ladder, as a machine-readable document.
///
/// Served so a curator, a peer, or an agent deciding whether to climb can read
/// the rules without reading us. Every row states the CHECK and what a peer
/// may conclude, and the capability table is the honest part: it shows that
/// reads sit at T0 and stay there.
pub fn ladder_doc() -> serde_json::Value {
    use serde_json::json;
    let tier = |t: Tier, req: String| json!({"tier": t.as_str(), "requirement": req, "a_peer_may_conclude": t.means()});
    json!({
        "schema": "emem.enlistment.v1",
        "principle": "Tier on what a write can REACH, never on who is asking. \
                      A tier records which check passed; it is not a score and \
                      this responder never asserts that a verified party is a \
                      trustworthy one. trust stays caller_decides.",
        "reads": "Never gated, at any tier, on any surface. A reader cannot \
                  poison anything, and gating reads would trade the property \
                  that makes this substrate worth using for no security gain.",
        "not_an_auth_wall": "There is no account, no bearer token that grants \
                             anything, and no payment anywhere in this ladder. \
                             Climbing is passing a check that a third party can \
                             re-run without us.",
        "tiers": [
            tier(Tier::T0Anonymous, "a signed note".into()),
            tier(Tier::T1Keyed, "full key resolvable; namespace proven by a caller signature".into()),
            tier(Tier::T2Named, "a signed profile.md carrying a unique nick".into()),
            tier(Tier::T3Declared, "a reachable endpoint with declared skills".into()),
            tier(Tier::T4Affiliated, "an organisation vouches for the key by dns, well_known or cross_sig".into()),
            tier(Tier::T5Corroborated, format!("{CORROBORATING_PEERS} distinct peer keys confirmed one of its tokens matched")),
        ],
        // Which rungs this responder actually COMPUTES, said plainly. A ladder
        // with an unreachable top is an ornament, and a reader has no way to
        // tell an unreached tier from an uncomputed one unless we say.
        "computed_here": {
            "T0_anonymous": true,
            "T1_keyed": true,
            "T2_named": "a signed profile.md in the namespace; the unique-nick half is not yet enforced",
            "T3_declared": true,
            "T4_affiliated": "on demand via POST /v1/enlist; evidence expires and is re-checked",
            "T5_corroborated": false
        },
        "t5_is_not_computed_yet": "It needs distinct peer keys confirming a token \
                                   MATCHED -- evidence of work rather than paperwork, and \
                                   the only rung that cannot be self-issued. The signed \
                                   ack/receive acts carrying it exist and are not yet \
                                   aggregated. Reporting a tier we do not compute would \
                                   be worse than the gap, so it reads as unreached for \
                                   everyone until that ships.",
        "write_surfaces": [
            {"surface": "read_anything", "min_tier": Tier::T0Anonymous.as_str(),
             "note": "never gated"},
            {"surface": "own_namespace_prose", "min_tier": Tier::T1Keyed.as_str(),
             "note": "the floor, and free: a stranger's agent writes on first contact with nothing but a signature"},
            {"surface": "shared_entity_address_space", "min_tier": Tier::T3Declared.as_str(),
             "note": "entity + entity_link change what every other agent resolves a name to. This is the one genuine poisoning surface in the current design."},
            {"surface": "fact_plane", "min_tier": Tier::T4Affiliated.as_str(),
             "note": "no caller can write a fact today by any route; this states the rule rather than relying on the absence of a door"}
        ],
        "org_verification": {
            "why_not_oauth": "Browser OAuth 2.1 + Dynamic Client Registration \
                              authenticates a SESSION — did a human, in a browser, \
                              just now authorise this client. An autonomous agent \
                              has neither, so DCR degrades to a bearer token proving \
                              possession and nothing about accountability. It is also \
                              structurally uncompletable headless.",
            "what_we_ask_instead": "Who is accountable for what this key says. That \
                                    is name control, solved three times already by \
                                    DKIM, ACME and Certificate Transparency.",
            "decisive_property": "A third party can re-verify without this responder, \
                                  for ever. A bearer token proves nothing to a third \
                                  agent; a DNS record proves the same thing to everyone \
                                  and survives our compromise, because the evidence is \
                                  not on our server.",
            "methods": [
                {"method": "dns", "rank": 1,
                 "record": format!("{DNS_LABEL}.<domain>  TXT  \"v=emem1; k=<52-char key>; nick=<name>\""),
                 "strength": "only the domain holder can publish it; revocation is deletion"},
                {"method": "well_known", "rank": 2,
                 "url": format!("https://<domain>{WELL_KNOWN_PATH}"),
                 "body": {"agents": [{"key": "<52-char key>", "nick": "<name>", "expires": "<unix seconds, optional>"}]},
                 "strength": "equal trust via TLS and origin control; easier without DNS access; carries expiry"},
                {"method": "cross_sig", "rank": 3,
                 "strength": "an org's emem key countersigns the agent profile",
                 "stated_limit": "proves one key vouched for another, NOT that the organisation exists. Trust in the org key still comes from dns or well_known."}
            ],
            "evidence_ttl_secs": EVIDENCE_TTL_SECS,
            "staleness": "Every attestation carries checked_at and is rendered with its \
                          age. An attestation past its TTL stops conferring a tier, \
                          because a check from last month is a claim about the present \
                          made from the past.",
            "ssrf": "A verification target must be a public name. IP literals, local \
                     names, and any name resolving into private, loopback, link-local \
                     or CGNAT space are refused, and redirects are not followed. The \
                     residual gap between resolving a name and connecting to it is not \
                     closed and is documented rather than described as safe."
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_txt_record_vouches_only_for_the_key_it_names() {
        let k = "alfrqiw7o7qvksz27g6azrpe2txmtgc5cvflojh42slgr7s4727a";
        assert_eq!(
            parse_dns_txt(&format!("v=emem1; k={k}; nick=cosmos-eye"), k),
            Some("cosmos-eye".to_string())
        );
        // Order and whitespace do not matter; quoting from a resolver does not.
        assert_eq!(
            parse_dns_txt(&format!("\"nick=x ;  k={k}   ; v=emem1\""), k),
            Some("x".to_string())
        );
        // A record for ANOTHER key is not a failure of the domain, it is
        // simply not about us: a domain may vouch for many agents.
        assert_eq!(
            parse_dns_txt(&format!("v=emem1; k={k}; nick=x"), "other"),
            None
        );
        // Wrong version is refused rather than guessed at.
        assert_eq!(parse_dns_txt(&format!("v=emem2; k={k}"), k), None);
        // Prose that happens to contain the key does not vouch for it.
        assert_eq!(parse_dns_txt(&format!("we use {k} internally"), k), None);
    }

    #[test]
    fn a_well_known_entry_stops_vouching_when_it_expires() {
        let doc: WellKnownDoc = serde_json::from_str(
            r#"{"agents":[{"key":"aaa","nick":"one","expires":100},
                          {"key":"bbb","nick":"two"}]}"#,
        )
        .unwrap();
        assert_eq!(well_known_vouches(&doc, "aaa", 50), Some("one".into()));
        assert_eq!(well_known_vouches(&doc, "aaa", 150), None, "expired");
        // No expiry means it stands until the document changes.
        assert_eq!(
            well_known_vouches(&doc, "bbb", 1_000_000),
            Some("two".into())
        );
        assert_eq!(well_known_vouches(&doc, "ccc", 50), None);
    }

    /// Reads are never gated. This is the property the whole design is built
    /// to preserve, so it is asserted rather than assumed.
    #[test]
    fn every_tier_may_read() {
        for t in [
            Tier::T0Anonymous,
            Tier::T1Keyed,
            Tier::T2Named,
            Tier::T3Declared,
            Tier::T4Affiliated,
            Tier::T5Corroborated,
        ] {
            assert!(
                may_write(t, Surface::Read).is_ok(),
                "{} was gated for reads",
                t.as_str()
            );
        }
    }

    /// The floor stays free: a stranger with nothing but a signature still
    /// writes prose in its own namespace on first contact.
    #[test]
    fn a_signature_alone_still_writes_its_own_namespace() {
        let keyed = tier_for(&Facts {
            has_signed_note: true,
            namespace_proven: true,
            ..Default::default()
        });
        assert_eq!(keyed, Tier::T1Keyed);
        assert!(may_write(keyed, Surface::OwnNamespace).is_ok());
        // ...and cannot reach the shared address space with only that.
        let refused = may_write(keyed, Surface::SharedEntitySpace).unwrap_err();
        assert!(refused.contains("T3_declared"), "{refused}");
        assert!(refused.contains("Reads are never gated"), "{refused}");
    }

    /// The control on the gate: it must actually refuse something. A ladder
    /// where every rung permits every surface is an ornament.
    #[test]
    fn the_gate_refuses_the_surface_it_exists_to_refuse() {
        let anon = Tier::T0Anonymous;
        assert!(may_write(anon, Surface::OwnNamespace).is_err());
        assert!(may_write(anon, Surface::SharedEntitySpace).is_err());
        assert!(may_write(anon, Surface::FactPlane).is_err());
        // And permits what it should, or it is refusing everything, which is
        // the same defect wearing the opposite sign.
        assert!(may_write(Tier::T5Corroborated, Surface::FactPlane).is_ok());
        assert!(may_write(Tier::T4Affiliated, Surface::FactPlane).is_ok());
        assert!(may_write(Tier::T3Declared, Surface::FactPlane).is_err());
    }

    #[test]
    fn corroboration_needs_three_distinct_peers_and_cannot_be_self_issued() {
        let two = tier_for(&Facts {
            corroborating_peers: 2,
            namespace_proven: true,
            ..Default::default()
        });
        assert_eq!(two, Tier::T1Keyed, "two peers can be one operator");
        let three = tier_for(&Facts {
            corroborating_peers: 3,
            ..Default::default()
        });
        assert_eq!(three, Tier::T5Corroborated);
    }

    /// Stale evidence is a false claim about the present. A domain holder who
    /// deleted the record last month must stop vouching.
    #[test]
    fn evidence_expires() {
        let e = Evidence {
            method: Method::Dns,
            domain: "vortx.ai".into(),
            nick: Some("n".into()),
            checked_at: 1_000_000,
            detail: "TXT matched".into(),
            ok: true,
        };
        assert!(e.is_fresh(1_000_000 + EVIDENCE_TTL_SECS - 1));
        assert!(!e.is_fresh(1_000_000 + EVIDENCE_TTL_SECS));
        // A failed check never vouches, however recent.
        let bad = Evidence {
            ok: false,
            ..e.clone()
        };
        assert!(!bad.is_fresh(1_000_001));
    }

    #[test]
    fn the_verifier_refuses_to_probe_our_own_network() {
        for bad in [
            "localhost",
            "foo.local",
            "svc.internal",
            "127.0.0.1",
            "169.254.169.254",
            "10.0.0.5",
            "::1",
            "notadomain",
            "host:5051",
            "user@host.com",
        ] {
            assert!(
                host_is_publicly_routable(bad).is_err(),
                "{bad} was accepted as a verification target"
            );
        }
        // The control: a real domain must pass, or the guard is refusing
        // everything and the mechanism never runs.
        for good in ["vortx.ai", "emem.dev", "geo.qa", "sub.example.co.uk"] {
            assert!(
                host_is_publicly_routable(good).is_ok(),
                "{good} was refused"
            );
        }
    }

    #[test]
    fn a_public_name_resolving_into_private_space_is_refused() {
        use std::net::IpAddr;
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(
                !addr_is_publicly_routable(bad.parse::<IpAddr>().unwrap()),
                "{bad} was treated as publicly routable"
            );
        }
        for good in ["1.1.1.1", "8.8.8.8", "2606:4700::1111"] {
            assert!(
                addr_is_publicly_routable(good.parse::<IpAddr>().unwrap()),
                "{good}"
            );
        }
    }
}
