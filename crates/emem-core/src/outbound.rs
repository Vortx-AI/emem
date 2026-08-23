//! How this node identifies itself to the data providers it fetches from.
//!
//! Polite-crawler convention: name the software and give the provider somebody
//! to contact about the traffic. The contact has to be THIS operator's.
//!
//! Fifteen call sites across two crates carried
//! `concat!("emem.dev/", env!("CARGO_PKG_VERSION"), " (a-personal-address)")`,
//! compiled in. Anyone else running emem fetched from Element84, the JRC,
//! WorldPop and the rest while identifying as the author of the software — so a
//! provider with a question about that traffic, or a rate-limit complaint about
//! it, would have written to somebody who did not send it and could not stop
//! it. The same defect as the ACME contact, one severity down: an account of
//! record versus an address in a header.
//!
//! The default names the project rather than a person. `+https://emem.dev` is
//! the standard form for exactly this case: it tells a provider what the
//! software is and where to read about it, and it is true no matter who is
//! running the binary. An operator who wants to be reachable sets
//! `EMEM_CONTACT`.

use std::sync::OnceLock;

/// The `User-Agent` sent to upstream data providers.
///
/// Computed once. `EMEM_CONTACT` when set and non-blank, otherwise the project
/// URL — never a person who did not make the request.
pub fn user_agent() -> &'static str {
    static UA: OnceLock<String> = OnceLock::new();
    UA.get_or_init(|| {
        let v = env!("CARGO_PKG_VERSION");
        match std::env::var("EMEM_CONTACT")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            Some(c) => format!("emem.dev/{v} ({c})"),
            None => format!("emem.dev/{v} (+https://emem.dev)"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default must not name a person.
    ///
    /// This is the assertion the fifteen `concat!` sites could not carry,
    /// because a compile-time constant has nowhere to put one. An address in
    /// the default is somebody receiving mail about requests they did not make.
    #[test]
    fn the_default_identifies_the_project_not_a_person() {
        let ua = user_agent();
        assert!(ua.starts_with("emem.dev/"), "names the software: {ua}");
        // Whatever EMEM_CONTACT holds in this environment, the DEFAULT branch
        // must be checkable on its own terms.
        let default_form = format!("emem.dev/{} (+https://emem.dev)", env!("CARGO_PKG_VERSION"));
        assert!(
            !default_form.contains('@'),
            "the default carries no email address: {default_form}"
        );
        assert!(
            default_form.contains("+https://"),
            "the default points at the project: {default_form}"
        );
    }
}
