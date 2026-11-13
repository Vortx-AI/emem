//! Claim gating: finding a physical-world assertion that cites nothing.
//!
//! Every other rule in this crate denies on a check that FAILED. This one
//! denies on a check that was never offered, which is a much stronger thing to
//! say about someone's prompt, and the reason it ships off by default and
//! behind a measured false-positive rate rather than behind an opinion.
//!
//! # The definition, stated so it can be argued with
//!
//! A transcript is gateable when **both** hold:
//!
//! 1. it carries no emem citation at all, and
//! 2. at least one sentence in it is a *gateable claim*.
//!
//! A sentence is a gateable claim when **all four** hold:
//!
//! - **G1 measurable magnitude.** A number followed by a unit this node's own
//!   band registry reports. Not "a unit that looks physical": a unit some band
//!   actually produces, each entry in [`UNITS`] naming the band it came from.
//!   That is the whole discriminator, and it is self-limiting by construction:
//!   the gate only fires on claims this node could have verified had they been
//!   cited. `10 MB`, `800 ms` and `$4.2bn` are not claims about the world emem
//!   observes, and no band reports them, so they never fire.
//! - **G2 world anchor.** A place or a time, because an observation without
//!   one is not checkable even in principle. Four exact forms: an ISO date, a
//!   standalone year, a decimal coordinate pair in range, or a preposition
//!   followed by a capitalised name.
//! - **G3 assertive.** Not a question, not hedged, not conditional, not an
//!   instruction, and not already attributed to a source.
//! - **G4 prose.** Not inside a fenced code block and not a `key: value`
//!   configuration line.
//!
//! # Why a conjunction, and why in that order
//!
//! Each clause can only ever REMOVE firings. There is no clause here that
//! makes the gate fire more often, so the false-positive rate is bounded above
//! by the rate of G1 alone, and every additional clause pushes it down. That is
//! the structural argument.
//!
//! The empirical one is [`tests::the_measured_false_positive_rate_on_real_prose`],
//! which runs the detector over this repository's own documentation: a corpus
//! of exactly the technical English this gate will sit in front of, dense with
//! numbers, units and place names, and written years before the detector
//! existed. Measured on 2026-08-06: **3 firings in 8739 sentences across 8
//! files**, a rate of 0.034%. Two of the three are this crate's own positive
//! test fixtures, sentences written to be gateable claims, so they are the
//! rule working rather than false positives; the third is a changelog line
//! that genuinely asserts a rainfall rate on a date and cites nothing.
//!
//! Two earlier firings were real defects the corpus found, both now fixed and
//! both pinned by tests: a citation key (`Reiche-2018`) read as a year, and a
//! model name (`gpt-4o-2024`) read the same way.
//!
//! That number is one input, not a licence. Prose in this repository is not
//! the traffic of an organisation that turns this rule on, which is what
//! [`crate::policy::Mode::Shadow`] and `emem-guard --report` are for: measure
//! it where it will run, on the same code path, before it blocks anyone.
//!
//! # What this is not
//!
//! Not a fact checker. It says nothing about whether `918 m` is the right
//! number; it says the transcript asserted a measurable quantity about a
//! place and offered nothing anyone could check. The remedy is
//! [`Fix::CiteObservation`](crate::policy::Fix::CiteObservation): resolve it
//! through emem and cite the token. An agent that learns that from a denial
//! starts carrying citations, which is the entire point of the gate.

/// What a unit measures.
///
/// Coarser than a dimension algebra on purpose: the gate needs to say "this is
/// a quantity emem observes" and to name it in a report, not to do arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantity {
    Length,
    Area,
    Volume,
    Temperature,
    PrecipitationRate,
    MassConcentration,
    AreaDensity,
    MassFraction,
    Density,
    PowerRatio,
    Fraction,
    PopulationDensity,
    SpectralIndex,
}

impl Quantity {
    /// The wire form, used in the open verdict and the shadow report.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Length => "length",
            Self::Area => "area",
            Self::Volume => "volume",
            Self::Temperature => "temperature",
            Self::PrecipitationRate => "precipitation_rate",
            Self::MassConcentration => "mass_concentration",
            Self::AreaDensity => "area_density",
            Self::MassFraction => "mass_fraction",
            Self::Density => "density",
            Self::PowerRatio => "power_ratio",
            Self::Fraction => "fraction",
            Self::PopulationDensity => "population_density",
            Self::SpectralIndex => "spectral_index",
        }
    }
}

/// A unit some band in the registry actually reports.
pub struct MeasurableUnit {
    /// The symbol as written in prose. Matched case-insensitively for the
    /// alphabetic ones; the symbols carrying `°`, `%`, `²` or `³` are matched
    /// as written, since no case variation of them exists.
    pub symbol: &'static str,
    pub quantity: Quantity,
    /// A band key that can actually be recalled to answer a claim in this
    /// unit, or `None` when this responder observes no such quantity.
    ///
    /// Load-bearing, not decoration: it is the actionable half of
    /// [`Fix::CiteObservation`](crate::policy::Fix::CiteObservation). An agent
    /// told to cite an observation needs somewhere to go, so this must be a
    /// key `POST /v1/recall` accepts and a materializer can serve.
    ///
    /// # Why this is a leaf key and not a family
    ///
    /// `GET /v1/bands` publishes 43 FAMILY ROOTS (`dem`, `soilgrids`,
    /// `esa_cci_biomass`). Recall does not accept them: a family root returns
    /// `unknown_band`, or at best only pre-existing attestations. The
    /// materializable keys are the leaves under them
    /// (`copdem30m.elevation_mean`, `soilgrids.soc_0_30cm`), published by
    /// `GET /v1/materializers`. This table named families until 2026-08-08, so
    /// every pointer it emitted was uncitable, and a caller who trusted one
    /// concluded emem could not answer a question it answers fine. Naming a
    /// family here is the same defect wearing a new coat.
    ///
    /// # Why `None` is a real answer
    ///
    /// Nothing here reports hectares, cubic metres or tonnes per hectare. The
    /// gate still fires on those claims, because a measurable assertion about
    /// a place with no citation is exactly what it is for, but the verdict
    /// must not invent a band to send the agent after. `None` says the claim
    /// is ungrounded AND that this node cannot ground it, which is two useful
    /// facts rather than one confident wrong one.
    ///
    /// A claim names this band only when its sentence also names what the
    /// band measures ([`BAND_EVIDENCE`]). A unit alone is not evidence: `80 %
    /// cloud` is not tree cover, and `31 m` of canopy is not an elevation.
    ///
    /// Asserted against the live registry by
    /// [`tests::every_unit_names_a_recallable_band`].
    pub source_band: Option<&'static str>,
}

/// The unit table, derived from `GET /v1/bands` on 2026-08-06 (43 bands).
///
/// Ordered longest symbol first, because matching is greedy: `m²` must be
/// tried before `m`, and `mm/day` before `mm`.
///
/// # What is deliberately absent
///
/// Bytes, seconds, hertz, currency and generic counts. No band reports them,
/// so a claim measured in them is not a claim this node could verify, and
/// firing on one would be the gate reaching outside its own competence. That
/// exclusion is what keeps the detector quiet in engineering prose, where
/// `800 ms` and `10 MB` are on nearly every page.
///
/// Kelvin is absent for a different reason: `K` is unresolvably ambiguous with
/// the thousands suffix in ordinary writing (`120K users`), and a temperature
/// band already contributes `°C`.
pub const UNITS: &[MeasurableUnit] = &[
    // Rates, before their bare-unit prefixes.
    MeasurableUnit {
        symbol: "mm/month",
        quantity: Quantity::PrecipitationRate,
        source_band: Some("weather.precipitation_mm"),
    },
    MeasurableUnit {
        symbol: "mm/year",
        quantity: Quantity::PrecipitationRate,
        source_band: Some("weather.precipitation_mm"),
    },
    MeasurableUnit {
        symbol: "mm/day",
        quantity: Quantity::PrecipitationRate,
        source_band: Some("weather.precipitation_mm"),
    },
    MeasurableUnit {
        symbol: "mm/yr",
        quantity: Quantity::PrecipitationRate,
        source_band: Some("weather.precipitation_mm"),
    },
    // Concentrations. The micro sign has two Unicode spellings in the wild
    // (U+00B5 MICRO SIGN and U+03BC GREEK SMALL LETTER MU) and text arriving
    // from a model uses both, so both are listed rather than normalised.
    MeasurableUnit {
        symbol: "µg/m³",
        quantity: Quantity::MassConcentration,
        source_band: Some("cams.pm25"),
    },
    MeasurableUnit {
        symbol: "μg/m³",
        quantity: Quantity::MassConcentration,
        source_band: Some("cams.pm25"),
    },
    MeasurableUnit {
        symbol: "µg/m3",
        quantity: Quantity::MassConcentration,
        source_band: Some("cams.pm25"),
    },
    MeasurableUnit {
        symbol: "μg/m3",
        quantity: Quantity::MassConcentration,
        source_band: Some("cams.pm25"),
    },
    MeasurableUnit {
        symbol: "ug/m3",
        quantity: Quantity::MassConcentration,
        source_band: Some("cams.pm25"),
    },
    MeasurableUnit {
        symbol: "mg/m³",
        quantity: Quantity::MassConcentration,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "mg/m3",
        quantity: Quantity::MassConcentration,
        source_band: None,
    },
    // Biomass and soil.
    MeasurableUnit {
        symbol: "mg/ha",
        quantity: Quantity::AreaDensity,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "t/ha",
        quantity: Quantity::AreaDensity,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "g/kg",
        quantity: Quantity::MassFraction,
        source_band: Some("soilgrids.soc_0_30cm"),
    },
    MeasurableUnit {
        symbol: "kg/dm³",
        quantity: Quantity::Density,
        source_band: Some("soilgrids.bdod_0_30cm"),
    },
    MeasurableUnit {
        symbol: "kg/dm3",
        quantity: Quantity::Density,
        source_band: Some("soilgrids.bdod_0_30cm"),
    },
    // Population.
    MeasurableUnit {
        symbol: "persons/km²",
        quantity: Quantity::PopulationDensity,
        source_band: Some("population"),
    },
    MeasurableUnit {
        symbol: "persons/km2",
        quantity: Quantity::PopulationDensity,
        source_band: Some("population"),
    },
    MeasurableUnit {
        symbol: "people/km²",
        quantity: Quantity::PopulationDensity,
        source_band: Some("population"),
    },
    MeasurableUnit {
        symbol: "people/km2",
        quantity: Quantity::PopulationDensity,
        source_band: Some("population"),
    },
    // Areas and volumes, before the lengths they are built from.
    MeasurableUnit {
        symbol: "km²",
        quantity: Quantity::Area,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "km2",
        quantity: Quantity::Area,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "m³",
        quantity: Quantity::Volume,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "m3",
        quantity: Quantity::Volume,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "m²",
        quantity: Quantity::Area,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "m2",
        quantity: Quantity::Area,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "hectares",
        quantity: Quantity::Area,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "hectare",
        quantity: Quantity::Area,
        source_band: None,
    },
    MeasurableUnit {
        symbol: "ha",
        quantity: Quantity::Area,
        source_band: None,
    },
    // Temperature.
    MeasurableUnit {
        symbol: "°c",
        quantity: Quantity::Temperature,
        source_band: Some("weather.temperature_2m"),
    },
    MeasurableUnit {
        symbol: "degrees celsius",
        quantity: Quantity::Temperature,
        source_band: Some("weather.temperature_2m"),
    },
    MeasurableUnit {
        symbol: "celsius",
        quantity: Quantity::Temperature,
        source_band: Some("weather.temperature_2m"),
    },
    // `degC` is the unit emem's own temperature facts carry, so an agent
    // quoting one writes "20.4 degC"; without these rows that sentence went
    // ungated. Bare "C" stays out: after a number it is as often a grade or
    // a label as a temperature.
    MeasurableUnit {
        symbol: "degrees c",
        quantity: Quantity::Temperature,
        source_band: Some("weather.temperature_2m"),
    },
    MeasurableUnit {
        symbol: "degc",
        quantity: Quantity::Temperature,
        source_band: Some("weather.temperature_2m"),
    },
    MeasurableUnit {
        symbol: "deg c",
        quantity: Quantity::Temperature,
        source_band: Some("weather.temperature_2m"),
    },
    // Lengths. `dem` reports metres relative to mean sea level; the SI
    // multiples of a metre are the same quantity by definition, not by guess.
    MeasurableUnit {
        symbol: "kilometres",
        quantity: Quantity::Length,
        source_band: Some("copdem30m.elevation_mean"),
    },
    MeasurableUnit {
        symbol: "kilometers",
        quantity: Quantity::Length,
        source_band: Some("copdem30m.elevation_mean"),
    },
    MeasurableUnit {
        symbol: "metres",
        quantity: Quantity::Length,
        source_band: Some("copdem30m.elevation_mean"),
    },
    MeasurableUnit {
        symbol: "meters",
        quantity: Quantity::Length,
        source_band: Some("copdem30m.elevation_mean"),
    },
    MeasurableUnit {
        symbol: "metre",
        quantity: Quantity::Length,
        source_band: Some("copdem30m.elevation_mean"),
    },
    MeasurableUnit {
        symbol: "meter",
        quantity: Quantity::Length,
        source_band: Some("copdem30m.elevation_mean"),
    },
    MeasurableUnit {
        symbol: "km",
        quantity: Quantity::Length,
        source_band: Some("copdem30m.elevation_mean"),
    },
    MeasurableUnit {
        symbol: "cm",
        quantity: Quantity::Length,
        source_band: Some("copdem30m.elevation_mean"),
    },
    MeasurableUnit {
        symbol: "mm",
        quantity: Quantity::Length,
        source_band: Some("weather.precipitation_mm"),
    },
    MeasurableUnit {
        symbol: "m",
        quantity: Quantity::Length,
        source_band: Some("copdem30m.elevation_mean"),
    },
    // Radar backscatter.
    MeasurableUnit {
        symbol: "db",
        quantity: Quantity::PowerRatio,
        source_band: Some("sentinel1_raw"),
    },
    // Cover fractions. `%` is the one symbol common enough in non-physical
    // prose to be a real source of noise, which is exactly what the anchor
    // and assertiveness clauses are for.
    MeasurableUnit {
        symbol: "%",
        quantity: Quantity::Fraction,
        source_band: Some("forest_change.treecover2000"),
    },
];

/// Words that show a sentence is about what a band measures.
///
/// A unit names a quantity, not a band. `%` is tree cover, cloud cover, a
/// slope or a growth rate; `m` is an elevation, a canopy height or a
/// distance. Until 2026-11-12 every `%` claim named
/// `forest_change.treecover2000`, so "80 % cloud" sent the agent to cite tree
/// cover. A verified tree-cover citation then turned the claim gate off for
/// the whole transcript. A claim now names a band only when the sentence also
/// names what that band measures. Matched at the start of a word.
const BAND_EVIDENCE: &[(&str, &[&str])] = &[
    (
        "forest_change.treecover2000",
        &["tree", "canopy", "forest", "woodland"],
    ),
    (
        "copdem30m.elevation_mean",
        &[
            "elevation",
            "altitude",
            "above sea level",
            "above mean sea level",
            "amsl",
            "asl",
        ],
    ),
    (
        "weather.precipitation_mm",
        &["rain", "precipitation", "drizzle", "shower", "downpour"],
    ),
    (
        "weather.temperature_2m",
        &[
            "temperature",
            "temps",
            "air",
            "hot",
            "warm",
            "cold",
            "cool",
            "heat",
            "freez",
            "frost",
        ],
    ),
    (
        "cams.pm25",
        &["pm2.5", "pm 2.5", "pm25", "particulate", "fine particle"],
    ),
    (
        "soilgrids.soc_0_30cm",
        &["organic carbon", "soc", "soil carbon"],
    ),
    ("soilgrids.bdod_0_30cm", &["bulk density"]),
    (
        "population",
        &["population", "people", "persons", "inhabitant", "resident"],
    ),
    (
        "sentinel1_raw",
        &[
            "backscatter",
            "radar",
            "sar",
            "sentinel-1",
            "sentinel 1",
            "sigma",
            "vv",
            "vh",
        ],
    ),
    ("indices.ndvi", &["ndvi"]),
];

/// Whether `sentence` names what `band` measures. See [`BAND_EVIDENCE`].
fn band_named_in(sentence: &str, band: &str) -> bool {
    let Some((_, words)) = BAND_EVIDENCE.iter().find(|(b, _)| *b == band) else {
        return false;
    };
    let lower = sentence.to_lowercase();
    words.iter().any(|w| {
        lower.match_indices(w).any(|(i, _)| {
            !lower[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
        })
    })
}

/// Words that make a sentence hypothetical, interrogative or attributed.
///
/// Each entry is here because it changes what the sentence CLAIMS, not because
/// it looked cautious. A conditional asserts nothing; an attributed statement
/// has already named its source, so demanding a citation would be asking twice.
const NON_ASSERTIVE: &[&str] = &[
    // Conditional and hypothetical.
    " if ",
    "if ",
    " unless ",
    " would ",
    " could ",
    " might ",
    " may ",
    " suppose",
    "suppose ",
    " assume",
    "assume ",
    " imagine",
    "imagine ",
    "hypothetical",
    "for example",
    "e.g.",
    "i.e.",
    " say the ",
    " pretend",
    // Hedged.
    " probably ",
    " maybe ",
    " perhaps ",
    " i think ",
    " i believe ",
    " seems ",
    " appears to ",
    " guess",
    // Already attributed: the speaker named a source, so the claim is not
    // uncited, it is cited to something we cannot check. Denying it would be
    // the gate substituting its own corpus for the user's.
    "according to",
    " per the ",
    " you said",
    " they said",
    " reported by ",
    " cited in ",
    " source:",
    " reference:",
    // Instruction rather than assertion. An agent being ASKED for a number
    // has not claimed one.
    "calculate",
    "compute",
    "estimate the",
    "what is",
    "what was",
    "how many",
    "how much",
    "tell me",
    "show me",
    "give me",
    "find the",
];

/// A gateable claim, with everything a report or a verdict needs to explain it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// The sentence, trimmed. Bounded by [`SENTENCE_MAX`] so a pathological
    /// input cannot make a log entry unbounded.
    pub sentence: String,
    /// The magnitude as written, e.g. `918 m`.
    pub magnitude: String,
    /// What was measured.
    pub quantity: Quantity,
    /// A recallable band key that would answer this claim. `None` when this
    /// responder observes no band in this quantity, or when the sentence does
    /// not name what the band measures. See [`MeasurableUnit::source_band`].
    pub source_band: Option<&'static str>,
    /// What anchored it to a place or a time.
    pub anchor: Anchor,
}

/// What tied a claim to somewhere or somewhen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// `2026-08-05`.
    IsoDate,
    /// A standalone year in [1970, 2100].
    Year,
    /// A decimal `lat, lon` pair, both in range.
    Coordinates,
    /// A preposition followed by a capitalised name.
    PlaceName,
}

impl Anchor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IsoDate => "iso_date",
            Self::Year => "year",
            Self::Coordinates => "coordinates",
            Self::PlaceName => "place_name",
        }
    }
}

/// How much of a sentence is kept on a claim.
///
/// A log entry and a verdict both carry this, and a transcript can contain a
/// megabyte without a full stop. Truncation here costs context in a report and
/// never changes a decision.
pub const SENTENCE_MAX: usize = 240;

/// Find every gateable claim across a transcript.
///
/// The caller is responsible for the transcript-level condition: this returns
/// claims regardless of whether the transcript also carries citations, because
/// the shadow report wants the raw count either way. [`crate::policy::evaluate`]
/// applies the "and nothing was cited" half.
pub fn scan_claims<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<Claim> {
    let mut out = Vec::new();
    for text in texts {
        for sentence in sentences(text) {
            if let Some(c) = claim_in(sentence) {
                out.push(c);
            }
        }
    }
    out
}

/// Evaluate one sentence against G1 through G4.
///
/// Ordered cheapest-first: most sentences fail G1, and the clauses after it
/// never run on those.
fn claim_in(sentence: &str) -> Option<Claim> {
    let (magnitude, quantity, source_band) =
        measurable_magnitude(sentence).or_else(|| named_index_value(sentence))?; // G1
    let source_band = source_band.filter(|band| band_named_in(sentence, band));
    let anchor = world_anchor(sentence)?; // G2
    if !is_assertive(sentence) {
        return None; // G3
    }
    if looks_like_configuration(sentence) {
        return None; // G4
    }
    Some(Claim {
        sentence: truncate_chars(sentence.trim(), SENTENCE_MAX),
        magnitude,
        quantity,
        source_band,
        anchor,
    })
}

// ── G1: a number followed by a unit some band reports ────────────────────

/// The first `<number><unit>` in a sentence whose unit is in [`UNITS`].
fn measurable_magnitude(s: &str) -> Option<(String, Quantity, Option<&'static str>)> {
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // A digit that continues a word (`B12`, `s2`, `cell64`) is an
        // identifier, not a magnitude.
        if i > 0 && (b[i - 1].is_ascii_alphabetic() || b[i - 1] == b'_') {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            continue;
        }
        let num_start = if i > 0 && (b[i - 1] == b'-' || b[i - 1] == b'+') {
            i - 1
        } else {
            i
        };
        let mut j = i;
        while j < b.len() && (b[j].is_ascii_digit() || b[j] == b',' || b[j] == b'.') {
            // A separator only continues the number when a digit follows it,
            // so `918.` at the end of a sentence stops at the digit.
            if (b[j] == b',' || b[j] == b'.') && !(j + 1 < b.len() && b[j + 1].is_ascii_digit()) {
                break;
            }
            j += 1;
        }
        // Optional single space between the number and its unit. Two spaces
        // or a newline is a layout break, not a magnitude.
        let unit_start = if j < b.len() && b[j] == b' ' {
            j + 1
        } else {
            j
        };
        if let Some((unit, quantity, source_band)) = unit_at(s, unit_start) {
            let magnitude = format!("{} {}", &s[num_start..j], unit);
            return Some((magnitude, quantity, source_band));
        }
        i = j.max(i + 1);
    }
    None
}

/// A spectral index has no unit, so "NDVI in Bengaluru is 0.9" never met
/// G1 and went ungated. The index's own name stands in for the unit: the
/// name as a whole word, then within a few words a decimal inside the
/// index's range. Only NDVI, the index a recallable band reports by name.
fn named_index_value(s: &str) -> Option<(String, Quantity, Option<&'static str>)> {
    const WINDOW: usize = 40;
    let lower = s.to_lowercase();
    let at = lower.match_indices("ndvi").map(|(i, _)| i).find(|&i| {
        let before = lower[..i].chars().next_back();
        let after = lower[i + 4..].chars().next();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        !before.is_some_and(word) && !after.is_some_and(word)
    })?;
    let rest = &s[at + 4..];
    let tail: String = rest.chars().take(WINDOW).collect();
    numbers_in(&tail)
        .into_iter()
        .find(|(value, decimals)| *decimals > 0 && (-1.0..=1.0).contains(value))
        .map(|(value, _)| {
            (
                format!("NDVI {value}"),
                Quantity::SpectralIndex,
                Some("indices.ndvi"),
            )
        })
}

/// The longest unit symbol starting at `at`, if the match ends cleanly.
fn unit_at(s: &str, at: usize) -> Option<(&'static str, Quantity, Option<&'static str>)> {
    if at >= s.len() || !s.is_char_boundary(at) {
        return None;
    }
    let rest = &s[at..];
    let lower = rest.to_lowercase();
    for u in UNITS {
        // `symbol` is already lowercase for every alphabetic entry.
        if !lower.starts_with(u.symbol) {
            continue;
        }
        // The match must not be the head of a longer word. `918 mbps` is not
        // metres, `10 MB` is not metres, and `5 kmh` is not kilometres. This
        // is the check that keeps the bare `m` and `%` rows safe.
        let after = lower[u.symbol.len()..].chars().next();
        let clean = match after {
            None => true,
            Some(c) => !c.is_alphanumeric() && c != '/' && c != '²' && c != '³' && c != '°',
        };
        if clean {
            return Some((u.symbol, u.quantity, u.source_band));
        }
    }
    None
}

// ── G2: a place or a time ────────────────────────────────────────────────

/// What anchors this sentence to somewhere or somewhen, if anything does.
fn world_anchor(s: &str) -> Option<Anchor> {
    if has_iso_date(s) {
        return Some(Anchor::IsoDate);
    }
    if has_coordinates(s) {
        return Some(Anchor::Coordinates);
    }
    if has_year(s) {
        return Some(Anchor::Year);
    }
    if has_place_name(s) {
        return Some(Anchor::PlaceName);
    }
    None
}

/// `YYYY-MM-DD` with the month and day in range.
fn has_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    for i in 0..b.len().saturating_sub(9) {
        if i > 0 && (b[i - 1].is_ascii_digit() || b[i - 1].is_ascii_alphabetic()) {
            continue;
        }
        let w = &b[i..i + 10];
        let digits = |r: std::ops::Range<usize>| w[r].iter().all(u8::is_ascii_digit);
        if !(digits(0..4) && w[4] == b'-' && digits(5..7) && w[7] == b'-' && digits(8..10)) {
            continue;
        }
        let num = |r: std::ops::Range<usize>| std::str::from_utf8(&w[r]).unwrap().parse::<u32>();
        if let (Ok(y), Ok(mo), Ok(d)) = (num(0..4), num(5..7), num(8..10)) {
            if (1970..=2100).contains(&y) && (1..=12).contains(&mo) && (1..=31).contains(&d) {
                return true;
            }
        }
    }
    false
}

/// A bare four-digit year in [1970, 2100], not part of a longer number.
///
/// A full stop on either side disqualifies only when a digit sits beyond it,
/// which is the difference between the decimal in `1990.5` and the sentence
/// that simply ended after the year.
fn has_year(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i + 4 <= b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let decimal_before = i >= 2 && b[i - 1] == b'.' && b[i - 2].is_ascii_digit();
        // A hyphen after a letter makes an identifier, not a date. `Reiche-2018`
        // is a citation key and `gpt-4o-2024` is a model name; neither says
        // anything about when something was measured. Found by running the
        // detector over this repository's own changelog.
        let hyphenated_name = i >= 2 && b[i - 1] == b'-' && b[i - 2].is_ascii_alphabetic();
        let starts_clean =
            i == 0 || !(b[i - 1].is_ascii_alphanumeric() || decimal_before || hyphenated_name);
        let ends = i + 4;
        let decimal_after = ends + 1 < b.len() && b[ends] == b'.' && b[ends + 1].is_ascii_digit();
        let ends_clean = ends == b.len() || !(b[ends].is_ascii_alphanumeric() || decimal_after);
        if starts_clean && ends_clean && b[i..ends].iter().all(u8::is_ascii_digit) {
            if let Ok(y) = std::str::from_utf8(&b[i..ends]).unwrap().parse::<u32>() {
                if (1970..=2100).contains(&y) {
                    return true;
                }
            }
        }
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        i += 1;
    }
    false
}

/// A decimal `lat, lon` pair with both halves in range.
///
/// Both halves must carry a decimal point. An integer pair like `1, 2` is
/// ordinary writing far more often than it is a position, and treating it as
/// one would anchor half the prose in the world.
fn has_coordinates(s: &str) -> bool {
    let bytes = s.as_bytes();
    for (i, _) in s.match_indices(',') {
        let lat = decimal_ending_at(s, i);
        let mut j = i + 1;
        while j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        let lon = decimal_starting_at(s, j);
        if let (Some(a), Some(b)) = (lat, lon) {
            if (-90.0..=90.0).contains(&a) && (-180.0..=180.0).contains(&b) {
                return true;
            }
        }
    }
    false
}

/// The signed decimal immediately before byte `end`, if there is one.
fn decimal_ending_at(s: &str, end: usize) -> Option<f64> {
    let b = s.as_bytes();
    let mut start = end;
    let mut saw_dot = false;
    while start > 0 {
        let c = b[start - 1];
        if c.is_ascii_digit() {
            start -= 1;
        } else if c == b'.' && !saw_dot {
            saw_dot = true;
            start -= 1;
        } else if (c == b'-' || c == b'+') && start >= 1 {
            start -= 1;
            break;
        } else {
            break;
        }
    }
    if !saw_dot || start == end {
        return None;
    }
    s[start..end].parse::<f64>().ok()
}

/// The signed decimal starting at byte `at`, if there is one.
fn decimal_starting_at(s: &str, at: usize) -> Option<f64> {
    let b = s.as_bytes();
    if at >= b.len() {
        return None;
    }
    let mut end = at;
    if b[end] == b'-' || b[end] == b'+' {
        end += 1;
    }
    let mut saw_dot = false;
    while end < b.len() {
        let c = b[end];
        if c.is_ascii_digit() {
            end += 1;
        } else if c == b'.' && !saw_dot && end + 1 < b.len() && b[end + 1].is_ascii_digit() {
            saw_dot = true;
            end += 1;
        } else {
            break;
        }
    }
    if !saw_dot {
        return None;
    }
    s[at..end].parse::<f64>().ok()
}

/// A locative preposition followed by a capitalised word.
///
/// A syntactic pattern, not a gazetteer. `over Sumatra` anchors; `over 40`
/// does not; and a capitalised word at the start of a sentence is not reached,
/// because the preposition has to come first.
fn has_place_name(s: &str) -> bool {
    const LOCATIVE: &[&str] = &[
        " in ",
        " at ",
        " near ",
        " over ",
        " across ",
        " around ",
        " within ",
        " throughout ",
        " outside ",
        " north of ",
        " south of ",
        " east of ",
        " west of ",
        " upstream of ",
        " downstream of ",
    ];
    let lower = s.to_lowercase();
    for p in LOCATIVE {
        let mut from = 0usize;
        while let Some(rel) = lower[from..].find(p) {
            let after = from + rel + p.len();
            // Read the original casing at the same offset: `lower` and `s`
            // can differ in length under Unicode case folding, so anchor the
            // lookup on a byte the original agrees about.
            if after <= s.len() && s.is_char_boundary(after) {
                if let Some(c) = s[after..].chars().next() {
                    if c.is_uppercase() {
                        return true;
                    }
                }
            }
            from = after.max(from + 1);
            if from >= lower.len() {
                break;
            }
        }
    }
    false
}

// ── G3 and G4 ────────────────────────────────────────────────────────────

/// Whether the sentence asserts rather than asks, supposes or attributes.
fn is_assertive(s: &str) -> bool {
    if s.contains('?') {
        return false;
    }
    let lower = format!(" {} ", s.to_lowercase());
    !NON_ASSERTIVE.iter().any(|m| lower.contains(m))
}

/// Whether this is a config line or a code fragment rather than a statement.
///
/// A `key: value` line and an assignment both carry a magnitude and no verb,
/// and both are ubiquitous in the transcripts of an agent that reads files.
fn looks_like_configuration(s: &str) -> bool {
    let t = s.trim();
    if t.starts_with('#') || t.starts_with("//") || t.starts_with('|') {
        return true;
    }
    // `client_max_body_size 10m;` and `timeout = 30` and `depth: 4`.
    if t.ends_with(';') || t.ends_with(',') || t.ends_with('{') || t.ends_with('}') {
        return true;
    }
    // A colon or equals with no space-separated verb-like word after it, on a
    // line short enough to be a setting rather than a sentence.
    let assignment = t.contains(" = ") || t.contains('=');
    if assignment && t.split_whitespace().count() <= 6 {
        return true;
    }
    // `depth: 4` and `client_max_body_size: 10m`. A single unspaced word
    // before a colon is a key, not a subject.
    if let Some((head, _)) = t.split_once(':') {
        if !head.is_empty() && !head.contains(' ') {
            return true;
        }
    }
    false
}

// ── Sentence splitting ───────────────────────────────────────────────────

/// Split text into sentences, skipping fenced code blocks entirely.
///
/// Deliberately simple: a full stop, question mark, exclamation mark or
/// newline ends a sentence, except a full stop between two digits, which is a
/// decimal point. Over-splitting costs a claim its anchor and so can only make
/// the gate quieter, which is the safe direction.
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut in_fence = false;
    while i < b.len() {
        // A fence toggles at a line-initial ``` and everything between is
        // code, where a magnitude is a literal rather than a claim.
        if b[i] == b'`' && i + 2 < b.len() && &b[i..i + 3] == b"```" {
            if !in_fence {
                push_sentence(&mut out, text, start, i);
            }
            in_fence = !in_fence;
            i += 3;
            start = i;
            continue;
        }
        if in_fence {
            i += 1;
            continue;
        }
        let ends = match b[i] {
            b'\n' | b'!' | b'?' => true,
            b'.' => {
                let prev_digit = i > 0 && b[i - 1].is_ascii_digit();
                let next_digit = i + 1 < b.len() && b[i + 1].is_ascii_digit();
                !(prev_digit && next_digit)
            }
            _ => false,
        };
        if ends {
            // Keep the terminator: `?` is what disqualifies a question, and
            // dropping it here would make every question assertive.
            push_sentence(&mut out, text, start, (i + 1).min(b.len()));
            start = i + 1;
        }
        i += 1;
    }
    push_sentence(&mut out, text, start, b.len());
    out
}

fn push_sentence<'a>(out: &mut Vec<&'a str>, text: &'a str, from: usize, to: usize) {
    if from >= to || !text.is_char_boundary(from) || !text.is_char_boundary(to) {
        return;
    }
    let s = text[from..to].trim();
    if !s.is_empty() {
        out.push(s);
    }
}

/// Truncate on a character boundary.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// A sentence that cites exactly one observation and also states a number.
///
/// The pair a value-agreement rule needs: prose asserting a figure, and the
/// single citation offered as evidence for it. Restricted to ONE token on
/// purpose. With two, attributing which number answers which citation is the
/// same guess the claim gate refuses to make, and a wrong deny there is worse
/// than a miss.
#[derive(Debug, Clone, PartialEq)]
pub struct CitedNumber {
    /// The sentence, trimmed and bounded by [`SENTENCE_MAX`].
    pub sentence: String,
    /// The one citation it offers.
    pub token: String,
    /// Every figure stated in the prose, with its precision and its unit.
    pub numbers: Vec<Figure>,
}

/// One figure a sentence states.
///
/// The unit is what binds a figure to a citation. "About 2 m here and about
/// 5 m at the edge" beside a fact that reads 2.13 m states two lengths, and
/// both are about what the fact measured. A rule that let the sentence pass
/// because ONE of them agreed checked half of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Figure {
    /// The number, sign included.
    pub value: f64,
    /// The decimal places it was written to. The precision matters: `889.6`
    /// is a correct report of `889.6439208984375`, and comparing them at full
    /// width would call ordinary rounding a lie.
    pub decimals: usize,
    /// The unit written after it, folded so spellings of one unit compare
    /// equal: `metres` and `m` are both `m`. `None` for a bare number.
    pub unit: Option<&'static str>,
    /// It says where, not what was measured: half of a coordinate pair, a
    /// tolerance (`within 0.05`), or a length that reads as a distance or an
    /// offset (`500 m south`, `5 m above the river`). Never bound to a
    /// citation.
    pub positional: bool,
}

/// One sentence's ingredients: its text, the citations inside it, and the
/// figures it states.
type SentenceParts<'a> = (&'a str, Vec<String>, Vec<Figure>);

/// Find sentences that state a number and cite exactly one observation.
///
/// Numbers inside the token are never counted. A cell64 is full of digits, and
/// reading them as claims would make every citation contradict itself.
pub fn scan_cited_numbers<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<CitedNumber> {
    let mut out = Vec::new();
    for text in texts {
        // Sentence splitting must not cut inside a citation. A cell64 is
        // dotted, so `defi.zb4e4.zcced.fUrI` reads as four sentence endings
        // and the token arrives at the rule in pieces. Mask every token to a
        // filler of the SAME BYTE LENGTH first, split that, then read the
        // original text back by offset: the mask changes which bytes are
        // punctuation without moving any of them.
        let all = crate::tokens::scan(text);
        let mut masked = text.to_string();
        for t in &all {
            let filler = "x".repeat(t.token.len());
            masked = masked.replacen(&t.token, &filler, 1);
        }
        // Pass one: what each sentence contains.
        let mut parts: Vec<SentenceParts> = Vec::new();
        for msent in sentences(&masked) {
            let off = msent.as_ptr() as usize - masked.as_ptr() as usize;
            let Some(sentence) = text.get(off..off + msent.len()) else {
                continue;
            };
            let found = crate::tokens::scan(sentence);
            // Only tokens that resolve to bytes: an entity names an object and
            // a cell names an address, so neither carries a reading a sentence
            // could misreport.
            let citing: Vec<String> = found
                .iter()
                .filter(|t| t.kind.resolves_to_bytes())
                .map(|t| t.token.clone())
                .collect();
            // Blank every token out before reading numbers, so the digits in
            // an address are never mistaken for an assertion.
            let mut stripped = sentence.to_string();
            for t in &found {
                stripped = stripped.replace(&t.token, " ");
            }
            parts.push((sentence, citing, figures_in(&stripped)));
        }

        // Pass two: pair a citation with the figure it supports.
        //
        // Usually they share a sentence. But the ordinary way to cite is to
        // finish the sentence and put the reference after it, "sits at 5000 m
        // elevation. [emem:fact:...]", which splits the number away from the
        // token. So a fragment carrying ONLY a citation adopts the figures of
        // the sentence immediately before it, and only when that sentence
        // cited nothing itself. Anything less tidy is left alone: a wrong deny
        // here is worse than a miss.
        for i in 0..parts.len() {
            let (sentence, citing, own_numbers) = &parts[i];
            if citing.len() != 1 {
                continue;
            }
            // Candidates are the figures in this fragment AND, when the
            // fragment before it cited nothing of its own, that one's figures
            // too. Both, never either/or.
            //
            // Either/or made LINE WRAPPING SEMANTIC. "NDBI is -0.037 / at 500 m
            // south [token]" denied because 500 sat on the token's line and the
            // rule never looked back, while the identical paragraph unwrapped
            // allowed. A desk measured the same prose both ways and got two
            // verdicts, which is not a judgement anyone can act on. Widening
            // the candidate set can only make the rule quieter, and a wrong
            // deny costs more here than a miss.
            let mut numbers = own_numbers.clone();
            if i > 0 && parts[i - 1].1.is_empty() {
                numbers.extend(parts[i - 1].2.iter().copied());
            }
            if numbers.is_empty() {
                continue;
            }
            let sentence = if own_numbers.is_empty() && i > 0 {
                parts[i - 1].0
            } else {
                *sentence
            };
            out.push(CitedNumber {
                sentence: truncate_chars(sentence.trim(), SENTENCE_MAX),
                token: citing[0].clone(),
                numbers,
            });
        }
    }
    out
}

/// Every decimal number in a string, with the precision it was written to.
fn numbers_in(s: &str) -> Vec<(f64, usize)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // Walk back over a sign and forward over the whole literal, so
        // thousands separators and a decimal tail stay part of one number.
        let start = i;
        let mut j = i;
        let mut decimals = 0usize;
        let mut seen_dot = false;
        while j < b.len() {
            if b[j].is_ascii_digit() {
                if seen_dot {
                    decimals += 1;
                }
                j += 1;
            } else if b[j] == b',' && j + 1 < b.len() && b[j + 1].is_ascii_digit() && !seen_dot {
                j += 1;
            } else if b[j] == b'.' && !seen_dot && j + 1 < b.len() && b[j + 1].is_ascii_digit() {
                seen_dot = true;
                j += 1;
            } else {
                break;
            }
        }
        let neg = start > 0 && b[start - 1] == b'-';
        let lit: String = s[start..j].chars().filter(|c| *c != ',').collect();
        if let Ok(v) = lit.parse::<f64>() {
            out.push((if neg { -v } else { v }, decimals));
        }
        i = j.max(start + 1);
    }
    out
}

/// After a length, a word that makes it a distance or an offset from
/// somewhere, not a reading of the place. `500 m south`, `5 m above the
/// river`, `2 km from the coast`.
const OFFSET_AFTER: &[&str] = &[
    "from",
    "away",
    "above",
    "below",
    "beyond",
    "upstream",
    "downstream",
    "inland",
    "offshore",
    "off",
    "of",
    "apart",
    "radius",
    "buffer",
    "wide",
    "long",
    "higher",
    "lower",
    "taller",
    "shorter",
    "deeper",
    "shallower",
    "more",
    "less",
    "further",
    "farther",
    "closer",
    "nearer",
];

/// Before any figure, a word that makes it a position or a tolerance.
const POSITION_BEFORE: &[&str] = &[
    "within",
    "radius",
    "buffer",
    "lat",
    "lon",
    "lng",
    "latitude",
    "longitude",
];

/// Every figure in a string, with its precision, its unit, and whether it is
/// positional.
///
/// A digit joined to a letter (`B12`, `PM2.5`, `s2`) is part of a name and is
/// skipped, as in [`measurable_magnitude`].
fn figures_in(s: &str) -> Vec<Figure> {
    let b = s.as_bytes();
    let mut out: Vec<Figure> = Vec::new();
    // Byte span of each figure, sign included, to find coordinate pairs.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        if i > 0 && (b[i - 1].is_ascii_alphabetic() || b[i - 1] == b'_') {
            while i < b.len()
                && (b[i].is_ascii_alphanumeric()
                    || b[i] == b'_'
                    || (b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit()))
            {
                i += 1;
            }
            continue;
        }
        let start = i;
        let mut j = i;
        let mut decimals = 0usize;
        let mut seen_dot = false;
        while j < b.len() {
            if b[j].is_ascii_digit() {
                if seen_dot {
                    decimals += 1;
                }
                j += 1;
            } else if b[j] == b',' && j + 1 < b.len() && b[j + 1].is_ascii_digit() && !seen_dot {
                j += 1;
            } else if b[j] == b'.' && !seen_dot && j + 1 < b.len() && b[j + 1].is_ascii_digit() {
                seen_dot = true;
                j += 1;
            } else {
                break;
            }
        }
        i = j.max(start + 1);
        let neg = start > 0 && b[start - 1] == b'-';
        let lit: String = s[start..j].chars().filter(|c| *c != ',').collect();
        let Ok(v) = lit.parse::<f64>() else {
            continue;
        };
        let signed_start = if neg { start - 1 } else { start };

        let unit_start = if j < b.len() && b[j] == b' ' {
            j + 1
        } else {
            j
        };
        let found = unit_at(s, unit_start);
        let before = last_word(&s[..signed_start]);
        let mut positional = POSITION_BEFORE.contains(&before.as_str());
        match found {
            Some((symbol, quantity, _)) => {
                if quantity == Quantity::Length {
                    let after = s.get(unit_start + symbol.len()..).unwrap_or("");
                    let next = first_word(after);
                    let direction = ["north", "south", "east", "west"]
                        .iter()
                        .any(|d| next.starts_with(d));
                    positional |= direction || OFFSET_AFTER.contains(&next.as_str());
                }
            }
            // A degree sign on a bare number is an angle or a coordinate:
            // `12.97°N`. A temperature matched `°c` above.
            None => positional |= s[j..].starts_with('°'),
        }
        out.push(Figure {
            value: if neg { -v } else { v },
            decimals,
            unit: found.map(|(symbol, _, _)| unit_key(symbol)),
            positional,
        });
        spans.push((signed_start, j));
    }

    // `12.97, 77.59` is a position. Both halves are decimals in range, with
    // only a comma between them.
    for k in 1..out.len() {
        let (a, c) = (out[k - 1], out[k]);
        let between = &s[spans[k - 1].1..spans[k].0];
        if a.unit.is_none()
            && c.unit.is_none()
            && a.decimals > 0
            && c.decimals > 0
            && between.trim() == ","
            && (-90.0..=90.0).contains(&a.value)
            && (-180.0..=180.0).contains(&c.value)
        {
            out[k - 1].positional = true;
            out[k].positional = true;
        }
    }
    out
}

/// The first word of `s`, lowercased: letters only.
fn first_word(s: &str) -> String {
    s.trim_start()
        .chars()
        .take_while(|c| c.is_alphabetic())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The last word of `s`, lowercased: letters only.
fn last_word(s: &str) -> String {
    let rev: Vec<char> = s
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_alphabetic())
        .collect();
    rev.into_iter().rev().flat_map(char::to_lowercase).collect()
}

/// One name for each unit, so the spellings of one unit compare equal.
fn unit_key(symbol: &'static str) -> &'static str {
    match symbol {
        "metres" | "meters" | "metre" | "meter" | "m" => "m",
        "kilometres" | "kilometers" | "km" => "km",
        "hectares" | "hectare" | "ha" => "ha",
        "°c" | "degrees celsius" | "celsius" | "degrees c" | "degc" | "deg c" => "degc",
        "µg/m³" | "μg/m³" | "µg/m3" | "μg/m3" | "ug/m3" => "ug/m3",
        "mg/m³" | "mg/m3" => "mg/m3",
        "kg/dm³" | "kg/dm3" => "kg/dm3",
        "persons/km²" | "persons/km2" | "people/km²" | "people/km2" => "persons/km2",
        "km²" | "km2" => "km2",
        "m³" | "m3" => "m3",
        "m²" | "m2" => "m2",
        "mm/year" | "mm/yr" => "mm/yr",
        other => other,
    }
}

/// Unit strings that facts carry for a quantity with no unit.
const UNITLESS: &[&str] = &[
    "index",
    "ratio",
    "reflectance",
    "fraction",
    "probability",
    "dimensionless",
    "unitless",
    "none",
    "1",
];

/// What a figure must be written in to report a fact with this unit.
enum FactUnit {
    /// A unit the [`UNITS`] table knows, folded by [`unit_key`].
    Written(&'static str),
    /// No unit: an index, a ratio, a reflectance.
    Unitless,
    /// A unit this crate cannot match in prose.
    Unknown,
}

fn fact_unit(unit: Option<&str>) -> FactUnit {
    let Some(u) = unit
        .map(|u| u.trim().to_lowercase())
        .filter(|u| !u.is_empty())
    else {
        return FactUnit::Unitless;
    };
    if UNITLESS.contains(&u.as_str()) || u.starts_with("unitless") {
        return FactUnit::Unitless;
    }
    if matches!(u.as_str(), "percent" | "pct" | "percentage") {
        return FactUnit::Written("%");
    }
    let u = u
        .replace(" per ", "/")
        .replace("_per_", "/")
        .replace('_', "/");
    match unit_at(&u, 0) {
        Some((symbol, _, _)) if symbol.len() == u.len() => FactUnit::Written(unit_key(symbol)),
        _ => FactUnit::Unknown,
    }
}

/// Whether the figures a sentence states agree with the one fact it cites.
///
/// A figure is BOUND to the citation when it reports what the fact measured:
///
/// - for a fact with a unit, a figure written in that unit;
/// - for a fact with no unit whose value is in [-1, 1] (an index, a ratio, a
///   reflectance), a bare decimal in [-1, 1];
/// - never a positional figure (see [`Figure::positional`]).
///
/// Every bound figure must agree. Until 2026-11-12 the rule passed when ANY
/// figure agreed, so "about 2 m here and about 5 m at the edge" beside a fact
/// of 2.13 m was allowed on the strength of the 2.
///
/// With no bound figure, the old test stands: at least one figure must agree.
/// A fact with no unit and a value outside [-1, 1] has nothing to bind by,
/// so a second figure beside it is still not checked.
pub(crate) fn figures_agree(figures: &[Figure], value: f64, unit: Option<&str>) -> bool {
    let bound: Vec<&Figure> = match fact_unit(unit) {
        FactUnit::Written(u) => figures
            .iter()
            .filter(|f| f.unit == Some(u) && !f.positional)
            .collect(),
        FactUnit::Unitless if value.abs() <= 1.0 => figures
            .iter()
            .filter(|f| f.unit.is_none() && f.decimals > 0 && f.value.abs() <= 1.0 && !f.positional)
            .collect(),
        _ => Vec::new(),
    };
    if bound.is_empty() {
        return figures.iter().any(|f| agrees(f.value, f.decimals, value));
    }
    bound.iter().all(|f| agrees(f.value, f.decimals, value))
}

/// Whether prose reporting `stated` to `decimals` places agrees with `actual`.
///
/// Agreement is judged at the precision the writer chose. Rounding `actual` to
/// the same width is what makes `889.6` a correct report of
/// `889.6439208984375` while `5000` is not, and it is the only comparison that
/// does not punish a human for writing a readable number.
pub fn agrees(stated: f64, decimals: usize, actual: f64) -> bool {
    let f = 10f64.powi(decimals.min(12) as i32);
    if (actual * f).round() / f == stated {
        return true;
    }
    // A relative fallback for very large or very small magnitudes, where a
    // half-ulp at the printed width is not the interesting question.
    let scale = actual.abs().max(stated.abs());
    scale > 0.0 && (actual - stated).abs() / scale <= 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(s: &str) -> Vec<Claim> {
        scan_claims([s])
    }

    /// Band keys `POST /v1/recall` accepts and a materializer can serve.
    ///
    /// Verified against `GET /v1/materializers` (127 keys) on 2026-08-08.
    /// This is an allow-list on purpose: adding an entry here is the step
    /// where someone confirms the key is a materializable LEAF and not a
    /// family root from `GET /v1/bands`. Family roots are the exact defect
    /// this test exists to prevent, because recall answers them with
    /// `unknown_band` and the agent we told to cite an observation has
    /// nowhere to go.
    const RECALLABLE_BANDS: &[&str] = &[
        "cams.pm25",
        "copdem30m.elevation_mean",
        "forest_change.treecover2000",
        "population",
        "sentinel1_raw",
        "soilgrids.bdod_0_30cm",
        "soilgrids.soc_0_30cm",
        "weather.precipitation_mm",
        "weather.temperature_2m",
    ];

    /// The table is only defensible if every row can name the band that
    /// produces it. A unit with no source is an opinion about what looks
    /// physical, which is the thing this design refuses to be.
    #[test]
    fn every_unit_names_a_recallable_band() {
        for u in UNITS {
            if let Some(b) = u.source_band {
                assert!(
                    RECALLABLE_BANDS.contains(&b),
                    "{} points at {b}, which POST /v1/recall does not accept. \
                     source_band must be a materializable leaf key from \
                     GET /v1/materializers, never a family root from GET /v1/bands.",
                    u.symbol
                );
            }
            assert!(
                u.symbol == u.symbol.to_lowercase()
                    || u.symbol.contains('²')
                    || u.symbol.contains('³'),
                "{} must be stored lowercase for case-insensitive matching",
                u.symbol
            );
        }
    }

    /// Greedy matching: a longer symbol must be tried before a prefix of it,
    /// or `m²` silently becomes `m` and an area becomes a length.
    #[test]
    fn the_table_is_ordered_so_the_longest_symbol_wins() {
        for (i, a) in UNITS.iter().enumerate() {
            for b in &UNITS[i + 1..] {
                assert!(
                    !b.symbol.starts_with(a.symbol) || a.symbol == b.symbol,
                    "{} is a prefix of {} but is listed before it, so {} can never match",
                    a.symbol,
                    b.symbol,
                    b.symbol
                );
            }
        }
    }

    /// The four clauses, each shown to be load-bearing by removing exactly
    /// one thing from a sentence that otherwise fires.
    #[test]
    fn each_clause_is_load_bearing() {
        let full = "The canopy height in Sumatra reached 31 m on 2026-08-05.";
        assert_eq!(claims(full).len(), 1, "the complete claim fires");

        // G1 gone: no measurable unit.
        assert!(claims("The canopy height in Sumatra reached 31 units on 2026-08-05.").is_empty());
        // G2 gone: no place and no time.
        assert!(claims("The canopy height reached 31 m.").is_empty());
        // G3 gone: it is a question.
        assert!(claims("Did the canopy height in Sumatra reach 31 m on 2026-08-05?").is_empty());
        // G3 gone: it is conditional.
        assert!(
            claims("If the canopy height in Sumatra reached 31 m on 2026-08-05, replant.")
                .is_empty()
        );
        // G4 gone: it is a config line.
        assert!(claims("sumatra_height_m = 31").is_empty());
    }

    /// An index has no unit; its name stands in for one.
    #[test]
    fn an_ndvi_value_is_a_claim_without_a_unit() {
        for s in [
            "NDVI in Bengaluru is 0.9 today.",
            "The NDVI at Cubbon Park reached 0.62 on 2026-09-01.",
        ] {
            assert_eq!(claims(s).len(), 1, "must fire: {s}");
        }
        for s in [
            "NDVI in Bengaluru rose by 12 percent points in 2026.",
            "We computed NDVI for Bengaluru in 2026.",
            "The ndvi_mean column in Bengaluru is 0.9 in the 2026 config.",
            "NDVI in Bengaluru is 1.8 today.",
        ] {
            assert!(claims(s).is_empty(), "must not fire: {s}");
        }
    }

    /// emem's own unit spelling is a temperature claim, like the symbol.
    #[test]
    fn the_units_emem_facts_carry_are_claims() {
        for s in [
            "The air temperature in Bengaluru is 40 degC today.",
            "The air temperature in Bengaluru was 40 deg C on 2026-09-28.",
            "The air temperature in Bengaluru is 40 °C today.",
        ] {
            assert_eq!(claims(s).len(), 1, "must fire: {s}");
        }
        assert!(claims("Bengaluru got a 40 C grade in 2026.").is_empty());
    }

    /// The exclusion that keeps the gate quiet in engineering prose. None of
    /// these units is reported by any band, so none of them is a claim about
    /// the world this node observes.
    #[test]
    fn units_no_band_reports_never_fire() {
        for s in [
            "The verdict path in Frankfurt took 800 ms on 2026-08-05.",
            "The transcript limit in Frankfurt is 10 MB as of 2026.",
            "We shipped 4.2 GB of tiles across Europe in 2026.",
            "Latency in Mumbai was 1200 microseconds on 2026-08-05.",
            "Revenue in Bengaluru was 4.2 million in 2026.",
            "The queue in Tokyo held 918 items on 2026-08-05.",
        ] {
            assert!(claims(s).is_empty(), "must not fire: {s}");
        }
    }

    /// A unit symbol that is the head of a longer word is not that unit.
    #[test]
    fn a_unit_must_not_match_the_start_of_another_word() {
        for s in [
            "Throughput near Chennai hit 918 mbps on 2026-08-05.",
            "The archive in Delhi is 12 mb as of 2026.",
            "The road near Pune runs 5 kmh faster in 2026.",
            "The sensor at Leh reported 40 dbm on 2026-08-05.",
        ] {
            assert!(claims(s).is_empty(), "must not fire: {s}");
        }
    }

    /// Each anchor form, alone, is enough.
    #[test]
    fn every_anchor_form_is_recognised() {
        let cases = [
            (
                "Elevation there is 918 m as of 2026-08-05.",
                Anchor::IsoDate,
            ),
            ("Elevation at 12.97, 77.59 is 918 m.", Anchor::Coordinates),
            ("Elevation reached 918 m in 2026.", Anchor::Year),
            ("Elevation reached 918 m during 1994.", Anchor::Year),
            ("Elevation in Bengaluru is 918 m.", Anchor::PlaceName),
        ];
        for (s, want) in cases {
            let c = claims(s);
            assert_eq!(c.len(), 1, "{s}");
            assert_eq!(c[0].anchor, want, "{s}");
        }
        // And a bare magnitude with no anchor is not checkable even in
        // principle, so it never fires.
        assert!(claims("Elevation is 918 m.").is_empty());

        // A four-digit number hanging off a name is an identifier, not a
        // date. Both of these were found by running the detector over this
        // repository's own prose.
        assert!(claims("The Reiche-2018 threshold is 3 db.").is_empty());
        assert!(claims("The gpt-4o-2024 run measured 918 m.").is_empty());
    }

    /// A coordinate pair must actually be a position. Ordinary comma-separated
    /// numbers are far more common than coordinates in prose.
    #[test]
    fn only_a_real_coordinate_pair_anchors() {
        assert!(claims("Elevation at 12.97, 77.59 is 918 m.").len() == 1);
        // Out of range for a latitude.
        assert!(claims("Elevation at 912.97, 77.59 is 918 m.").is_empty());
        // Integers are a list, not a position.
        assert!(claims("Elevation at 12, 77 is 918 m.").is_empty());
    }

    /// The magnitude and the band travel with the claim, because a denial has
    /// to tell the agent what to go and cite.
    #[test]
    fn a_claim_names_the_quantity_and_where_it_would_come_from() {
        let c = &claims("Canopy cover in Rondonia fell to 42 % last season.")[0];
        assert_eq!(c.magnitude, "42 %");
        assert_eq!(c.quantity, Quantity::Fraction);
        assert_eq!(c.source_band, Some("forest_change.treecover2000"));
        assert_eq!(c.anchor, Anchor::PlaceName);

        // A sentence with more than one anchor reports the first form found,
        // in the order `world_anchor` checks them. Which one is reported does
        // not change the decision; it only tells a report why the sentence was
        // considered checkable at all.
        let both = &claims("Canopy cover in Rondonia fell to 42 % during 2026.")[0];
        assert_eq!(both.anchor, Anchor::Year);
    }

    /// A unit names a quantity, not a band. The band hint needs the sentence
    /// to name what the band measures, or it is null.
    #[test]
    fn a_band_is_named_only_when_the_sentence_names_what_it_measures() {
        let band = |s: &str| {
            let c = claims(s);
            assert_eq!(c.len(), 1, "must still be a claim: {s}");
            c[0].source_band
        };
        // From a partner's friction log: cloud cover is not tree cover.
        assert_eq!(
            band("Cloud cover over Rondonia was 80 % on 2026-11-01."),
            None
        );
        assert_eq!(
            band("Tree cover in Rondonia was 80 % in 2000."),
            Some("forest_change.treecover2000")
        );
        // A canopy height is not an elevation, and no band here reports it.
        assert_eq!(
            band("The canopy in Sumatra reached 31 m on 2026-08-05."),
            None
        );
        assert_eq!(
            band("Elevation in Leh is 3500 m."),
            Some("copdem30m.elevation_mean")
        );
        assert_eq!(
            band("Rainfall in Kerala hit 12.4 mm/day on 2026-08-05."),
            Some("weather.precipitation_mm")
        );
        // A word inside another word is not evidence: "street" holds "tree".
        assert_eq!(band("The street in Pune is 40 % shade in 2026."), None);
    }

    /// Every band the table names has words that can show it, or the hint
    /// for that unit could never be given.
    #[test]
    fn every_named_band_has_evidence_words() {
        for u in UNITS {
            if let Some(b) = u.source_band {
                assert!(
                    BAND_EVIDENCE.iter().any(|(e, w)| *e == b && !w.is_empty()),
                    "{} names {b}, which has no evidence words",
                    u.symbol
                );
            }
        }
    }

    /// Code is where magnitudes live without asserting anything.
    #[test]
    fn fenced_code_is_not_scanned() {
        let text =
            "Config below.\n```\nclient_max_body_size 10m;\nelevation_m = 918\n```\nThat is all.";
        assert!(claims(text).is_empty());
    }

    /// An attributed statement has already named its source. Demanding a
    /// citation on top would be the gate insisting its own corpus is the only
    /// admissible one.
    #[test]
    fn an_already_attributed_statement_is_not_ungrounded() {
        for s in [
            "According to the survey, elevation in Leh is 3500 m.",
            "You said the canopy in Sumatra was 31 m in 2026.",
        ] {
            assert!(claims(s).is_empty(), "{s}");
        }
    }

    /// Several claims in one transcript are all reported, because the shadow
    /// report counts them and the count is what an org decides on.
    #[test]
    fn every_claim_in_a_transcript_is_found() {
        let found = scan_claims([
            "Elevation in Leh is 3500 m.",
            "no numbers here",
            "Rainfall in Kerala hit 12.4 mm/day on 2026-08-05.",
        ]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].quantity, Quantity::PrecipitationRate);
    }

    /// Multi-byte text must not panic any of the byte-wise walks, and the
    /// micro sign has two Unicode spellings that both arrive from models.
    #[test]
    fn unicode_is_handled_in_both_directions() {
        assert_eq!(
            claims("PM2.5 in Delhi reached 180 µg/m³ on 2026-08-05.").len(),
            1
        );
        assert_eq!(
            claims("PM2.5 in Delhi reached 180 \u{3bc}g/m³ on 2026-08-05.").len(),
            1
        );
        // Emoji, CJK and combining marks around a magnitude.
        let _ = claims("高度 in Kyōto は 918 m です 🏔 2026-08-05.");
        let _ = claims("\u{0301}\u{0301}\u{0301} 918 m in Zürich 2026");
    }

    /// A pathological sentence cannot make a log entry unbounded.
    #[test]
    fn a_claim_sentence_is_bounded() {
        let long = format!("Elevation in Leh is 918 m {}", "x".repeat(10_000));
        let c = &claims(&long)[0];
        assert_eq!(c.sentence.chars().count(), SENTENCE_MAX);
    }

    /// The measurement the ship note said had to exist before this rule could.
    ///
    /// The corpus is this repository's own prose: the README, the changelog,
    /// the self-host skill and the guard crate's doc comments. That is exactly
    /// the register the gate will sit in front of, dense with numbers, units
    /// and place names, and written without any awareness of this detector.
    ///
    /// The assertion is on the RATE, and every firing is printed so a reviewer
    /// can read it rather than trust the number. A firing here is not
    /// automatically a false positive: a sentence in the README that asserts a
    /// measured quantity about a place with no citation is precisely what the
    /// gate is for. It is printed so that judgement stays with a person.
    #[test]
    fn the_measured_false_positive_rate_on_real_prose() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf();
        let mut corpus: Vec<(String, String)> = Vec::new();
        for rel in [
            "README.md",
            "CHANGELOG.md",
            "AGENTS.md",
            "crates/emem-guard/SKILL.md",
            "crates/emem-guard/src/policy.rs",
            "crates/emem-guard/src/server.rs",
            "crates/emem-guard/src/store.rs",
            "crates/emem-guard/src/webhook.rs",
        ] {
            if let Ok(t) = std::fs::read_to_string(root.join(rel)) {
                corpus.push((rel.to_string(), t));
            }
        }
        assert!(
            corpus.len() >= 4,
            "the corpus must actually be readable, found {}",
            corpus.len()
        );

        let mut total_sentences = 0usize;
        let mut fired: Vec<(String, Claim)> = Vec::new();
        for (name, text) in &corpus {
            let ss = sentences(text);
            total_sentences += ss.len();
            for s in ss {
                if let Some(c) = claim_in(s) {
                    fired.push((name.clone(), c));
                }
            }
        }

        // Some of the corpus is source, and this crate's own source contains
        // sentences written to BE gateable claims, because they are the
        // positive fixtures for this detector. A firing on one of those is
        // the rule working, not a false positive, so they are counted apart
        // rather than quietly excluded: hiding them would flatter the number
        // and hiding the number is how it drifts.
        let self_fixture = |name: &str| name.starts_with("crates/emem-guard/");
        let (own, external): (Vec<_>, Vec<_>) = fired.iter().partition(|(n, _)| self_fixture(n));
        println!(
            "claim gate corpus: {} sentences across {} files, {} fired \
             ({} in this crate's own test fixtures, {} in prose)",
            total_sentences,
            corpus.len(),
            fired.len(),
            own.len(),
            external.len(),
        );
        for (name, c) in &fired {
            let tag = if self_fixture(name) {
                "fixture"
            } else {
                "prose  "
            };
            println!("  {tag} {name}: [{}] {}", c.magnitude, c.sentence);
        }
        println!(
            "  NOTE: these figures are quoted in README.md, web/llms.txt, \
             web/guard.html and this module's docs. Update them together."
        );

        assert!(
            total_sentences > 1000,
            "the corpus is too small to mean anything: {total_sentences}"
        );
        let rate = fired.len() as f64 / total_sentences as f64;
        // One in two hundred is the ceiling this rule ships under. It is a
        // budget, not a measurement: the measurement is printed above, and if
        // this assertion ever fails the right response is to read the firings
        // and decide whether the detector or the ceiling is wrong.
        assert!(
            rate < 0.005,
            "{} of {} sentences fired ({:.3}%), over the 0.5% ceiling",
            fired.len(),
            total_sentences,
            rate * 100.0
        );
    }
}
