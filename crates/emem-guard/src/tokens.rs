//! Finding emem tokens in a transcript.
//!
//! This is the part that makes a grounding verdict possible at all: before
//! anything can be verified, the citations have to be located in prose that
//! was never structured for the purpose. The transcript is what the user
//! sees, so a token arrives inside a sentence, in a tool result, in a
//! markdown link, or wrapped in backticks.
//!
//! The scan is deliberately conservative in one direction and generous in
//! the other. A token this misses is a verification that does not happen,
//! which under an allow-by-default policy is a missed check rather than a
//! wrong answer. A token this HALLUCINATES is a verification against
//! something the user never cited, and that can only produce a wrong deny.
//! So the grammar is anchored and the character set is exact, and anything
//! ambiguous is skipped.

/// A citation found in the transcript, with where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundToken {
    /// The token exactly as it appeared, with surrounding punctuation
    /// stripped but the token itself unmodified.
    pub token: String,
    /// Which token family this is. Different families verify differently:
    /// a fact token resolves to signed bytes, an entity token only to a
    /// shared name.
    pub kind: TokenKind,
}

/// The token families this responder mints.
///
/// Kept as an enum rather than a string because the verification strength
/// genuinely differs between them, and a caller that treats them alike would
/// over-claim on entity tokens. The identity of an entity is hashed from an
/// anchor rather than from the whole record, so it converges two agents on
/// the same NAME, not on the same bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// `emem:fact:<cell64>:<fact_cid>` — resolves to byte-identical signed
    /// content. The strong one.
    Fact,
    /// `emem:entity:<entity_cid>` — a shared reference, not shared bytes.
    Entity,
    /// `emem:bundle:<bundle_cid>` — an ordered set of facts.
    Bundle,
    /// `emem:cell:<cell64>` — an address with no observation attached.
    Cell,
    /// `emem:raster:` / `emem:cube:` / `emem:rasterset:` — field tokens.
    Field,
}

impl TokenKind {
    fn from_prefix(p: &str) -> Option<Self> {
        Some(match p {
            "fact" => Self::Fact,
            "entity" => Self::Entity,
            "bundle" => Self::Bundle,
            "cell" => Self::Cell,
            "raster" | "cube" | "rasterset" => Self::Field,
            _ => return None,
        })
    }

    /// Whether resolving this token yields bytes a third party can check,
    /// as opposed to a name two agents agree on.
    ///
    /// The distinction matters for what a verdict may claim: a passing
    /// `emem:fact:` means the cited observation is exactly what was signed;
    /// a passing `emem:entity:` means only that both parties mean the same
    /// object.
    pub fn resolves_to_bytes(self) -> bool {
        matches!(self, Self::Fact | Self::Bundle | Self::Field)
    }
}

/// Characters that may appear inside a token body.
///
/// base32-nopad-lowercase (a-z, 2-7) plus the separators our token grammar
/// uses: `:` between segments, `.` inside a cell64, `,` and `@` in the
/// spoken-form fact token, and `-` and digits for tslot ranges and dates.
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b',' | b'@' | b'-' | b'_')
}

/// Trailing bytes that are punctuation around a citation rather than part of
/// it. A token at the end of a sentence picks up the full stop; one in a
/// markdown link picks up the bracket.
fn trim_trailing_punctuation(s: &str) -> &str {
    s.trim_end_matches(|c: char| {
        matches!(
            c,
            '.' | ',' | ';' | ':' | ')' | ']' | '}' | '>' | '"' | '\'' | '!' | '?'
        )
    })
}

/// Scan one string for emem tokens.
///
/// Finds every `emem:<kind>:...` occurrence, anywhere in the text, including
/// inside backticks, parentheses and markdown links. Order is preserved and
/// duplicates are kept: a caller that wants uniqueness can dedupe, but the
/// count of citations in a transcript is itself signal.
pub fn scan(text: &str) -> Vec<FoundToken> {
    const MARK: &str = "emem:";
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while let Some(rel) = text[i..].find(MARK) {
        let start = i + rel;
        // Must start at a boundary, so `notemem:fact:...` is not a token and
        // neither is the tail of a longer word.
        let boundary = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let mut end = start;
        while end < bytes.len() && is_token_byte(bytes[end]) {
            end += 1;
        }
        // `end` walks byte-wise over an ASCII-only predicate, so it always
        // lands on a char boundary.
        let raw = trim_trailing_punctuation(&text[start..end]);
        if boundary {
            if let Some(kind) = raw
                .strip_prefix(MARK)
                .and_then(|rest| rest.split(':').next())
                .and_then(TokenKind::from_prefix)
            {
                // A bare `emem:fact:` with no body is a mention of the
                // grammar, not a citation. Require something after the kind.
                let body = raw.splitn(3, ':').nth(2).unwrap_or("");
                if !body.is_empty() {
                    out.push(FoundToken {
                        token: raw.to_string(),
                        kind,
                    });
                }
            }
        }
        i = end.max(start + MARK.len());
    }
    out
}

/// Scan an entire transcript, in order.
pub fn scan_all<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<FoundToken> {
    texts.into_iter().flat_map(scan).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        scan(s).into_iter().map(|t| t.token).collect()
    }

    /// A citation arrives inside prose, not on a line of its own.
    #[test]
    fn tokens_are_found_in_the_shapes_a_transcript_actually_carries() {
        let cell = "defi.zb493.xuqA.zcb5f";
        let cid = "yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala";
        let t = format!("emem:fact:{cell}:{cid}");

        // Bare, end of sentence, backticked, parenthesised, markdown link.
        assert_eq!(toks(&t), vec![t.clone()]);
        assert_eq!(
            toks(&format!("The elevation is 918 m ({t}).")),
            vec![t.clone()]
        );
        assert_eq!(toks(&format!("cite `{t}` in your answer")), vec![t.clone()]);
        assert_eq!(toks(&format!("see [the fact]({t}) above")), vec![t.clone()]);
        assert_eq!(toks(&format!("{t}, which verifies")), vec![t.clone()]);

        // Several in one turn, order preserved.
        let two = format!("{t} and emem:entity:abc123");
        assert_eq!(scan(&two).len(), 2);
        assert_eq!(scan(&two)[0].kind, TokenKind::Fact);
        assert_eq!(scan(&two)[1].kind, TokenKind::Entity);
    }

    /// The direction that matters: a hallucinated token would be verified
    /// against something nobody cited, and can only produce a wrong deny.
    #[test]
    fn near_misses_are_not_treated_as_citations() {
        // Not at a word boundary.
        assert!(toks("notemem:fact:x:y").is_empty());
        // A kind we do not mint.
        assert!(toks("emem:banana:abc").is_empty());
        // The grammar being discussed rather than used.
        assert!(toks("the emem:fact: prefix").is_empty());
        assert!(toks("emem:").is_empty());
        // A different product's URI scheme.
        assert!(toks("mem:fact:a:b").is_empty());
    }

    /// Every family is recognised, and the ones that resolve to bytes are
    /// distinguished from the one that only shares a name.
    #[test]
    fn each_family_is_classified_by_what_it_can_prove() {
        for (s, kind, bytes) in [
            ("emem:fact:cell:cid", TokenKind::Fact, true),
            ("emem:bundle:bcid", TokenKind::Bundle, true),
            ("emem:raster:a:b:c:d", TokenKind::Field, true),
            ("emem:cube:a:b:c:d", TokenKind::Field, true),
            ("emem:entity:ecid", TokenKind::Entity, false),
            ("emem:cell:defi.zb493.xuqA.zcb5f", TokenKind::Cell, false),
        ] {
            let f = scan(s);
            assert_eq!(f.len(), 1, "{s} should be one token");
            assert_eq!(f[0].kind, kind, "{s}");
            assert_eq!(f[0].kind.resolves_to_bytes(), bytes, "{s}");
        }
    }

    /// The spoken form carries commas and an @, which the character set must
    /// admit without swallowing the sentence around it.
    #[test]
    fn the_spoken_fact_form_survives_punctuation() {
        let t = "emem:fact:12.97,77.59@2026-08-05@indices.ndvi:abc123";
        assert_eq!(
            toks(&format!("as of today, {t}. Next sentence.")),
            vec![t.to_string()]
        );
    }

    /// Multi-byte text must not panic the byte-wise walk.
    #[test]
    fn unicode_around_a_token_is_handled() {
        let t = "emem:fact:a:b";
        assert_eq!(toks(&format!("héllo — {t} — wörld")), vec![t.to_string()]);
        assert!(
            toks("emem:fact:日本語").is_empty() || true,
            "must not panic"
        );
    }

    /// A tool result is where a grounded agent's citations actually live.
    #[test]
    fn scanning_a_whole_transcript_preserves_order() {
        let got = scan_all(vec![
            "first emem:fact:a:b",
            "no tokens here",
            "then emem:entity:c",
        ]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].token, "emem:fact:a:b");
        assert_eq!(got[1].token, "emem:entity:c");
    }
}
