//! The question half of `/v1/ask`: what a question asks, read before any band
//! is fetched, and the direct answer composed once the readings are back.
//!
//! The topic router reads a question for its subject. Three other things in
//! the question decide whether the answer is an answer:
//!
//! * the TIME it is about. "Was there snow on the ground on 14 December 2022?"
//!   was answered with a car count from a camera clip seven minutes old,
//!   because nothing read the date.
//! * the FORM the answer takes. A yes-or-no question came back as six raw
//!   reflectances and no yes or no.
//! * the QUANTITY that answers it. "Is the Thames at high or low water?" is a
//!   water question, and a water index answers it, not the bands that happened
//!   to be recalled beside one.
//!
//! Everything here is pure. The clock is passed in and nothing is fetched, so
//! the classifier and the composer are checked offline against a fixed
//! question set (`testdata/ask_cases.json`).

use serde_json::{json, Value as JsonValue};

const DAY: i64 = 86_400;

// ── Time ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimePrecision {
    Day,
    Month,
    Year,
}

impl TimePrecision {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TimePrecision::Day => "day",
            TimePrecision::Month => "month",
            TimePrecision::Year => "year",
        }
    }
}

/// The time a question is about, when it names one. `end_unix` is exclusive.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AskedTime {
    pub(crate) phrase: String,
    pub(crate) start_unix: i64,
    pub(crate) end_unix: i64,
    pub(crate) precision: TimePrecision,
    /// "since 2020", "between 2019 and 2021": a question about change over a
    /// span, which a single dated reading does not answer.
    pub(crate) change_window: bool,
}

impl AskedTime {
    /// The window has closed, so "the newest reading" cannot stand in for it.
    pub(crate) fn is_past(&self, now_unix: i64) -> bool {
        self.end_unix <= now_unix
    }

    pub(crate) fn is_future(&self, now_unix: i64) -> bool {
        self.start_unix > now_unix
    }

    pub(crate) fn to_json(&self) -> JsonValue {
        json!({
            "phrase": self.phrase,
            "from": crate::iso8601_utc(self.start_unix.max(0) as u64),
            "until": crate::iso8601_utc(self.end_unix.max(0) as u64),
            "precision": self.precision.as_str(),
            "change_window": self.change_window,
        })
    }

    /// Where an observation at `t` sits against this window. `slot_s` is the
    /// cadence of the band that read it: a reading one revisit away is the
    /// nearest pass, not a different date.
    pub(crate) fn timing(&self, t: i64, slot_s: i64) -> Timing {
        if t >= self.start_unix && t < self.end_unix {
            return Timing::Inside;
        }
        let gap = if t < self.start_unix {
            self.start_unix - t
        } else {
            t - self.end_unix + 1
        };
        let near = match self.precision {
            // One Sentinel-2 revisit either side of a named day.
            TimePrecision::Day => slot_s.max(5 * DAY),
            TimePrecision::Month | TimePrecision::Year => slot_s,
        };
        let gap_days = (gap + DAY - 1) / DAY;
        if gap <= near {
            Timing::Near(gap_days)
        } else {
            Timing::Outside(gap_days)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timing {
    Inside,
    Near(i64),
    Outside(i64),
}

const MONTHS: &[(&str, u32)] = &[
    ("january", 1),
    ("february", 2),
    ("march", 3),
    ("april", 4),
    ("may", 5),
    ("june", 6),
    ("july", 7),
    ("august", 8),
    ("september", 9),
    ("october", 10),
    ("november", 11),
    ("december", 12),
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("sept", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
];

fn month_of(t: &str) -> Option<u32> {
    MONTHS.iter().find(|(n, _)| *n == t).map(|(_, m)| *m)
}

fn day_of(t: &str) -> Option<u32> {
    let digits = t
        .strip_suffix("st")
        .or_else(|| t.strip_suffix("nd"))
        .or_else(|| t.strip_suffix("rd"))
        .or_else(|| t.strip_suffix("th"))
        .unwrap_or(t);
    if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|d| (1..=31).contains(d))
}

fn year_of(t: &str) -> Option<i32> {
    if t.len() != 4 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok().filter(|y| (1900..=2100).contains(y))
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

fn day_window(y: i32, m: u32, d: u32) -> Option<(i64, i64)> {
    if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    let s = crate::days_from_civil(y, m, d) * DAY;
    Some((s, s + DAY))
}

fn month_window(y: i32, m: u32) -> Option<(i64, i64)> {
    if !(1..=12).contains(&m) {
        return None;
    }
    let s = crate::days_from_civil(y, m, 1) * DAY;
    Some((s, s + i64::from(days_in_month(y, m)) * DAY))
}

fn year_window(y: i32) -> (i64, i64) {
    (
        crate::days_from_civil(y, 1, 1) * DAY,
        crate::days_from_civil(y + 1, 1, 1) * DAY,
    )
}

/// Words that put a bare year in a sentence about time ("in 2021", "during
/// 2019"), as opposed to a count that happens to have four digits.
const YEAR_LEADS: &[&str] = &[
    "in", "during", "of", "for", "year", "since", "from", "between", "and", "to", "until",
    "before", "after", "by", "through", "early", "late", "mid", "summer", "winter", "spring",
    "autumn", "fall",
];

/// Words that turn a date into one end of a span.
const SPAN_LEADS: &[&str] = &[
    "since", "between", "from", "after", "before", "until", "through",
];

fn tokens(q: &str) -> Vec<(String, String)> {
    q.split_whitespace()
        .map(|raw| {
            let clean = raw.trim_matches(|c: char| {
                matches!(
                    c,
                    ',' | '.' | ';' | ':' | '!' | '?' | '(' | ')' | '"' | '\'' | '[' | ']'
                )
            });
            (clean.to_string(), clean.to_lowercase())
        })
        .filter(|(c, _)| !c.is_empty())
        .collect()
}

/// One date phrase starting at token `i`: (window, precision, tokens used).
fn date_at(t: &[(String, String)], i: usize) -> Option<((i64, i64), TimePrecision, usize)> {
    let w = |k: usize| t.get(k).map(|(_, l)| l.as_str());
    let cur = w(i)?;
    // 2022-12-14, 2022/12/14
    if cur.len() == 10 {
        let parts: Vec<&str> = cur.split(['-', '/']).collect();
        if parts.len() == 3 {
            if let (Some(y), Ok(m), Ok(d)) = (year_of(parts[0]), parts[1].parse(), parts[2].parse())
            {
                if let Some(win) = day_window(y, m, d) {
                    return Some((win, TimePrecision::Day, 1));
                }
            }
        }
    }
    // 14 December 2022, 14th of December 2022
    if let Some(d) = day_of(cur) {
        let mut j = i + 1;
        if w(j) == Some("of") {
            j += 1;
        }
        if let (Some(m), Some(y)) = (w(j).and_then(month_of), w(j + 1).and_then(year_of)) {
            if let Some(win) = day_window(y, m, d) {
                return Some((win, TimePrecision::Day, j + 2 - i));
            }
        }
    }
    if let Some(m) = month_of(cur) {
        // December 14, 2022
        if let (Some(d), Some(y)) = (w(i + 1).and_then(day_of), w(i + 2).and_then(year_of)) {
            if let Some(win) = day_window(y, m, d) {
                return Some((win, TimePrecision::Day, 3));
            }
        }
        // December 2022
        if let Some(y) = w(i + 1).and_then(year_of) {
            if let Some(win) = month_window(y, m) {
                return Some((win, TimePrecision::Month, 2));
            }
        }
    }
    if let Some(y) = year_of(cur) {
        let lead_ok = i == 0 || w(i - 1).is_some_and(|p| YEAR_LEADS.contains(&p));
        if lead_ok {
            return Some((year_window(y), TimePrecision::Year, 1));
        }
    }
    None
}

/// The time a question names, if it names one. "yesterday" is the only
/// relative form read; "last year" and "recently" are left to the caller,
/// because their window is a judgement this function should not make.
pub(crate) fn asked_time(q: &str, now_unix: i64) -> Option<AskedTime> {
    let t = tokens(q);
    let mut found: Option<(usize, (i64, i64), TimePrecision, usize)> = None;
    let mut second_date = false;
    let mut i = 0;
    while i < t.len() {
        if t[i].1 == "yesterday" && found.is_none() {
            let s = (now_unix.div_euclid(DAY) - 1) * DAY;
            found = Some((i, (s, s + DAY), TimePrecision::Day, 1));
            i += 1;
            continue;
        }
        if let Some((win, prec, used)) = date_at(&t, i) {
            if found.is_none() {
                found = Some((i, win, prec, used));
            } else {
                second_date = true;
            }
            i += used;
            continue;
        }
        i += 1;
    }
    let (at, (start, end), precision, used) = found?;
    let lead = (1..=2)
        .filter_map(|back| at.checked_sub(back))
        .map(|k| t[k].1.as_str())
        .find(|w| *w != "the" && *w != "of" && *w != "on" && *w != "in");
    let change_window = second_date || lead.is_some_and(|w| SPAN_LEADS.contains(&w));
    let phrase = t[at..at + used]
        .iter()
        .map(|(c, _)| c.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Some(AskedTime {
        phrase,
        start_unix: start,
        end_unix: end,
        precision,
        change_window,
    })
}

/// An explicit `date` field: `YYYY-MM-DD`, `YYYY-MM` or `YYYY`.
pub(crate) fn asked_time_field(s: &str) -> Option<AskedTime> {
    let s = s.trim();
    let parts: Vec<&str> = s.split('-').collect();
    let y = year_of(parts.first()?)?;
    let (win, precision) = match parts.len() {
        1 => (year_window(y), TimePrecision::Year),
        2 => (
            month_window(y, parts[1].parse().ok()?)?,
            TimePrecision::Month,
        ),
        3 => (
            day_window(y, parts[1].parse().ok()?, parts[2].parse().ok()?)?,
            TimePrecision::Day,
        ),
        _ => return None,
    };
    Some(AskedTime {
        phrase: s.to_string(),
        start_unix: win.0,
        end_unix: win.1,
        precision,
        change_window: false,
    })
}

/// Whether the question is about the present, by its own words.
pub(crate) fn says_now(q: &str) -> bool {
    let l = q.to_lowercase();
    [
        "now",
        "today",
        "currently",
        "current",
        "latest",
        "at the moment",
        "this week",
        "recent",
        "recently",
        "live",
        "tonight",
    ]
    .iter()
    .any(|w| has_term(&l, w))
}

/// `term` in `hay` with a word boundary on both sides. "snow" does not contain
/// "now" by this rule, and that is the case it exists for.
pub(crate) fn has_term(hay_lower: &str, term: &str) -> bool {
    if term.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(i) = hay_lower[from..].find(term) {
        let s = from + i;
        let e = s + term.len();
        let before = hay_lower[..s].chars().next_back();
        let after = hay_lower[e..].chars().next();
        if !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric) {
            return true;
        }
        from = s + hay_lower[s..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// The question with its date phrase removed, so the place extractor does not
/// geocode "December" or "2022".
pub(crate) fn without_time_phrase(q: &str, t: &AskedTime) -> String {
    if t.phrase.is_empty() {
        return q.to_string();
    }
    if let Some(i) = q.find(&t.phrase) {
        return format!("{}{}", &q[..i], &q[i + t.phrase.len()..]);
    }
    // The phrase was rebuilt from cleaned words ("December 14 2022" for
    // "December 14, 2022"), so take the words out one at a time.
    q.split_whitespace()
        .filter(|w| {
            let clean = w.trim_matches(|c: char| !c.is_alphanumeric());
            !t.phrase.split(' ').any(|p| p.eq_ignore_ascii_case(clean))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Form ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnswerForm {
    YesNo,
    /// "high or low", "wet or dry": one of two named options.
    Choice(String, String),
    Value,
}

impl AnswerForm {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            AnswerForm::YesNo => "yes_no",
            AnswerForm::Choice(..) => "choice",
            AnswerForm::Value => "value",
        }
    }
}

const FILLERS: &[&str] = &[
    "so", "and", "but", "ok", "okay", "please", "hey", "then", "also", "now", "tell", "me",
];
const YES_NO_LEADS: &[&str] = &[
    "is", "was", "are", "were", "does", "did", "do", "has", "have", "had", "can", "could", "will",
    "would", "should", "isn't", "wasn't", "aren't", "weren't", "doesn't", "didn't", "hasn't",
    "haven't",
];

pub(crate) fn answer_form(q: &str) -> AnswerForm {
    let words: Vec<String> = tokens(q).into_iter().map(|(_, l)| l).collect();
    let first = words
        .iter()
        .find(|w| !FILLERS.contains(&w.as_str()))
        .map(String::as_str)
        .unwrap_or("");
    let yes_no = YES_NO_LEADS.contains(&first) || words.iter().any(|w| w == "whether");
    if !yes_no {
        return AnswerForm::Value;
    }
    if let Some(k) = words.iter().position(|w| w == "or") {
        if k > 0 && k + 1 < words.len() {
            let (a, b) = (&words[k - 1], &words[k + 1]);
            if a != b && !["not", "no", "nothing"].contains(&b.as_str()) {
                return AnswerForm::Choice(a.clone(), b.clone());
            }
        }
    }
    AnswerForm::YesNo
}

// ── Quantity ────────────────────────────────────────────────────────────

/// A threshold that turns one reading into yes, no, or "cannot tell".
#[derive(Debug)]
pub(crate) struct Rule {
    pub(crate) band: &'static str,
    yes_from: f64,
    no_from: f64,
    higher_is_yes: bool,
    /// Distance from the line inside which a reading is weak evidence.
    margin: f64,
    pub(crate) yes: &'static str,
    pub(crate) no: &'static str,
    pub(crate) between: &'static str,
    pub(crate) basis: &'static str,
    /// At or above this value the reading does not measure the quantity at
    /// all, and the text says what it is instead: a glacier pack in a snow
    /// model is not snow that fell or lay on the date asked.
    void_from: Option<(f64, &'static str)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Judgement {
    Yes,
    No,
    Between,
}

impl Rule {
    pub(crate) fn judge(&self, v: f64) -> Judgement {
        if self.void_at(v).is_some() {
            return Judgement::Between;
        }
        let (yes, no) = if self.higher_is_yes {
            (v >= self.yes_from, v < self.no_from)
        } else {
            (v <= self.yes_from, v > self.no_from)
        };
        if yes {
            Judgement::Yes
        } else if no {
            Judgement::No
        } else {
            Judgement::Between
        }
    }

    fn void_at(&self, v: f64) -> Option<&'static str> {
        self.void_from.filter(|(x, _)| v >= *x).map(|(_, t)| t)
    }

    fn near_line(&self, v: f64) -> bool {
        (v - self.yes_from).abs() < self.margin || (v - self.no_from).abs() < self.margin
    }
}

/// What a question asks to be measured, and which reading measures it.
#[derive(Debug)]
pub(crate) struct Quantity {
    pub(crate) key: &'static str,
    pub(crate) noun: &'static str,
    terms: &'static [&'static str],
    /// A question about risk, likelihood or suitability is not a question
    /// about the reading: "is this flood-prone" is not "is it flooded".
    excludes: &'static [&'static str],
    /// In order of preference: the first one present answers.
    pub(crate) lead_bands: &'static [&'static str],
    /// Read beside the lead band because the answer depends on them.
    pub(crate) context_bands: &'static [&'static str],
    rules: &'static [Rule],
    uncertainty: &'static str,
}

const RISK_WORDS: &[&str] = &[
    "risk",
    "prone",
    "hazard",
    "vulnerable",
    "vulnerability",
    "likely",
    "chance",
    "probability",
    "insurance",
    "suitable",
    "safe",
];

/// What a lead band that is not an observation says about itself, in place of
/// the quantity's caution, which is written for the observation.
fn lead_caution(band: &str) -> Option<&'static str> {
    match band {
        "era5.snow_depth" => Some(
            "A model value, not an observation: ERA5-Land reanalysis at about 9 km, one grid \
             cell for the whole area, so a valley, a roof or a cleared square can differ. \
             Over glaciers the model holds a deep fixed pack (about 33 m) that says nothing \
             about fresh snow.",
        ),
        _ => None,
    }
}

pub(crate) static QUANTITIES: &[Quantity] = &[
    Quantity {
        key: "snow_cover",
        noun: "snow on the ground",
        terms: &["snow", "snowy", "snow-covered", "snowpack", "snow cover", "ndsi"],
        excludes: RISK_WORDS,
        // The observation first. The reanalysis answers only when no optical
        // pass is near the date asked, such as in the polar night.
        lead_bands: &["indices.ndsi", "era5.snow_depth"],
        context_bands: &[],
        rules: &[
            Rule {
                band: "indices.ndsi",
                yes_from: 0.4,
                no_from: 0.0,
                higher_is_yes: true,
                margin: 0.05,
                yes: "snow on the ground",
                no: "no snow on the ground",
                between: "snow or cloud, which this index cannot tell apart",
                basis: "snow_likelihood_from_ndsi@1: NDSI at or above 0.4 reads as snow, below 0 as no snow, and between them as snow or cloud (Salomonson and Appel 2004)",
                void_from: None,
            },
            Rule {
                band: "era5.snow_depth",
                yes_from: 0.05,
                no_from: 0.01,
                higher_is_yes: true,
                margin: 0.005,
                yes: "snow on the ground in the ERA5-Land reanalysis",
                no: "no snow on the ground in the ERA5-Land reanalysis",
                between: "a layer under 5 cm in the ERA5-Land reanalysis, too thin to call",
                basis: "ERA5-Land snow depth (Munoz-Sabater et al. 2021): 5 cm or more reads as snow on the ground, under 1 cm as none",
                void_from: Some((
                    crate::persist::GLACIER_PACK_M,
                    "a pack of 10 m or more, which ERA5-Land holds on a glacier all year, so it says nothing about snow on the date asked",
                )),
            },
        ],
        uncertainty: "One pixel on one scene. Cloud raises NDSI as snow does, so a reading between 0 and 0.4 is not decisive.",
    },
    Quantity {
        key: "surface_water",
        noun: "open water",
        terms: &[
            "high water",
            "low water",
            "high tide",
            "low tide",
            "tide",
            "tidal",
            "flooded",
            "flooding",
            "under water",
            "underwater",
            "inundated",
            "inundation",
            "water level",
            "open water",
            "surface water",
            "waterlogged",
            "water extent",
            "mndwi",
            "ndwi",
            "is there water",
        ],
        excludes: RISK_WORDS,
        lead_bands: &["indices.mndwi", "indices.ndwi", "sentinel1_raw"],
        context_bands: &["surface_water.occurrence"],
        rules: &[
            Rule {
                band: "indices.mndwi",
                yes_from: 0.0,
                no_from: 0.0,
                higher_is_yes: true,
                margin: 0.05,
                yes: "open water",
                no: "no open water",
                between: "",
                basis: "MNDWI above 0 reads as open water (Xu 2006)",
                void_from: None,
            },
            Rule {
                band: "indices.ndwi",
                yes_from: 0.0,
                no_from: 0.0,
                higher_is_yes: true,
                margin: 0.05,
                yes: "open water",
                no: "no open water",
                between: "",
                basis: "NDWI above 0 reads as open water (McFeeters 1996)",
                void_from: None,
            },
            Rule {
                band: "sentinel1_raw",
                yes_from: -16.0,
                no_from: -16.0,
                higher_is_yes: false,
                margin: 1.0,
                yes: "open water",
                no: "no open water",
                between: "",
                basis: "flood_extent_sar_threshold@1: Sentinel-1 VV backscatter below -16 dB reads as open water (Twele et al. 2016)",
                void_from: None,
            },
        ],
        uncertainty: "One pixel. A pixel that mixes water and land reads near the line, so a value close to it is weak evidence either way.",
    },
    Quantity {
        key: "ground_elevation",
        noun: "ground elevation",
        terms: &[
            "elevation",
            "altitude",
            "above sea level",
            "below sea level",
            "how high",
            "height above",
            "ground height",
            "terrain height",
            "dem",
        ],
        excludes: &["building", "tower", "tree", "canopy", "tide", "water level"],
        lead_bands: &["copdem30m.elevation_mean", "dem.elevation_mean", "gmrt.topobathy_mean"],
        context_bands: &[],
        rules: &[],
        uncertainty: "Copernicus DEM GLO-30 is a 30 m surface model, so buildings and trees raise it; its specification gives an absolute vertical accuracy better than 4 m for 90% of points.",
    },
    Quantity {
        key: "vegetation",
        noun: "vegetation",
        terms: &[
            "ndvi",
            "vegetation",
            "vegetated",
            "greenness",
            "how green",
            "green cover",
            "plant cover",
        ],
        excludes: RISK_WORDS,
        lead_bands: &["indices.ndvi", "modis.ndvi_mean"],
        context_bands: &[],
        rules: &[Rule {
            band: "indices.ndvi",
            yes_from: 0.2,
            no_from: 0.1,
            higher_is_yes: true,
            margin: 0.03,
            yes: "live vegetation",
            no: "little or no live vegetation",
            between: "sparse cover that this index does not separate from bare ground",
            basis: "NDVI below about 0.1 is bare ground, built surface, water or snow, and above about 0.2 live vegetation",
            void_from: None,
        }],
        uncertainty: "One pixel on one scene. Thin cloud and haze lower NDVI.",
    },
    Quantity {
        key: "air_temperature",
        noun: "air temperature",
        terms: &["temperature", "how hot", "how cold", "how warm"],
        excludes: &["surface temperature", "land surface", "lst", "risk"],
        lead_bands: &["weather.temperature_2m"],
        context_bands: &[],
        rules: &[],
        uncertainty: "",
    },
    Quantity {
        key: "precipitation",
        noun: "precipitation",
        terms: &["rain", "raining", "rainfall", "precipitation"],
        excludes: RISK_WORDS,
        lead_bands: &["weather.precipitation_mm", "chirps.precip_daily_mm"],
        context_bands: &[],
        rules: &[],
        uncertainty: "",
    },
    Quantity {
        key: "fine_particulates",
        noun: "PM2.5",
        terms: &["air quality", "pm2.5", "pm25", "particulate", "particulates", "smog"],
        excludes: &[],
        lead_bands: &["cams.pm25"],
        context_bands: &[],
        rules: &[],
        uncertainty: "CAMS is a model analysis on a coarse grid, not a street-level monitor.",
    },
];

/// The quantity a question asks about, if one of the known ones.
pub(crate) fn asked_quantity(q: &str) -> Option<&'static Quantity> {
    let l = q.to_lowercase();
    QUANTITIES.iter().find(|qt| {
        qt.terms.iter().any(|t| has_term(&l, t)) && !qt.excludes.iter().any(|x| has_term(&l, x))
    })
}

// ── Readings ────────────────────────────────────────────────────────────

/// What the product behind a static layer is, so an answer can name it in
/// place of an age it does not have.
pub(crate) fn static_layer_source(band: &str) -> Option<&'static str> {
    let b = band;
    Some(if b.starts_with("copdem30m.") {
        "Copernicus DEM GLO-30, from TanDEM-X radar acquired 2011 to 2015"
    } else if b.starts_with("gmrt.") {
        "GMRT global topography and bathymetry synthesis"
    } else if b.starts_with("dem.") {
        "a static elevation model"
    } else if b.starts_with("surface_water.") {
        "JRC Global Surface Water, from Landsat 1984 to 2021"
    } else if b.starts_with("esa_worldcover.") {
        "ESA WorldCover 2021"
    } else if b.starts_with("soilgrids.") {
        "ISRIC SoilGrids 2.0, modelled soil properties"
    } else if b.starts_with("jrc_gfc2020.") {
        "JRC Global Forest Cover 2020"
    } else if b.starts_with("hansen.") || b.starts_with("forest_change.") {
        "Hansen Global Forest Change"
    } else if b.starts_with("koppen") {
        "Beck et al. 2018 Koppen-Geiger map"
    } else {
        return None;
    })
}

/// One signed reading, as the direct answer needs it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reading {
    pub(crate) band: String,
    pub(crate) value: Option<f64>,
    pub(crate) unit: Option<String>,
    pub(crate) fact_cid: Option<String>,
    /// When the observation was made. None for a static layer.
    pub(crate) observed_unix: Option<i64>,
    pub(crate) observed_basis: String,
    /// Release date of a static layer.
    pub(crate) static_release: Option<String>,
    /// Cadence of the band, in seconds; 0 for a static layer.
    pub(crate) slot_s: i64,
}

impl Reading {
    /// From one `band_observations[]` entry. Absences and entries with no
    /// value carry `value: None` so "looked and found nothing" stays visible.
    pub(crate) fn from_observation(o: &JsonValue) -> Option<Reading> {
        let band = o
            .get("band_key")
            .or_else(|| o.get("band"))
            .and_then(|b| b.as_str())?
            .to_string();
        Some(Reading {
            band,
            value: o.get("value").and_then(|v| v.as_f64()),
            unit: o.get("unit").and_then(|v| v.as_str()).map(String::from),
            fact_cid: o.get("fact_cid").and_then(|v| v.as_str()).map(String::from),
            observed_unix: o.get("observed_unix").and_then(|v| v.as_i64()),
            observed_basis: o
                .get("observed_basis")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            static_release: o
                .get("static_release")
                .and_then(|v| v.as_str())
                .map(String::from),
            slot_s: o.get("slot_s").and_then(|v| v.as_i64()).unwrap_or(0),
        })
    }

    pub(crate) fn is_static(&self) -> bool {
        self.observed_basis == "static_layer"
    }
}

fn day_of_unix(t: i64) -> String {
    crate::iso8601_utc(t.max(0) as u64)
        .get(..10)
        .unwrap_or("")
        .to_string()
}

fn fmt_num(v: f64) -> String {
    if v.fract().abs() < 1e-9 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

fn reading_phrase(r: &Reading) -> String {
    let label = crate::band_display_label(&r.band);
    let v = r.value.map(fmt_num).unwrap_or_else(|| "no value".into());
    match r.unit.as_deref() {
        Some(u) if !u.is_empty() && u != "1" => format!("{label} {v} {u}"),
        _ => format!("{label} {v}"),
    }
}

/// "on the scene of 2026-10-30" / "in the static layer ...".
fn when_phrase(r: &Reading, now_unix: i64) -> String {
    if r.is_static() {
        let src = static_layer_source(&r.band).unwrap_or("a static layer");
        return match r.static_release.as_deref().and_then(|s| s.get(..10)) {
            Some(rel) => {
                format!("from {src} (released {rel}), a static layer with no observation date")
            }
            None => format!("from {src}, a static layer with no observation date"),
        };
    }
    match r.observed_unix {
        Some(t) => {
            let days = (now_unix - t).max(0) / DAY;
            let ago = match days {
                0 => "today".to_string(),
                1 => "1 day ago".to_string(),
                n => format!("{n} days ago"),
            };
            let what = if r.band.starts_with("s2.") || r.band.starts_with("indices.") {
                "the Sentinel-2 scene of"
            } else if r.band.starts_with("sentinel1") {
                "the Sentinel-1 pass of"
            } else {
                "a reading dated"
            };
            format!("on {what} {} ({ago})", day_of_unix(t))
        }
        None => "at an unrecorded observation time".to_string(),
    }
}

// ── The direct answer ───────────────────────────────────────────────────

pub(crate) struct AnswerInputs<'a> {
    pub(crate) form: &'a AnswerForm,
    pub(crate) quantity: Option<&'static Quantity>,
    pub(crate) time: Option<&'a AskedTime>,
    pub(crate) readings: &'a [Reading],
    pub(crate) place: &'a str,
    /// Notes about how the place was resolved (a point of interest stood in
    /// for an area name, one cell of a larger place).
    pub(crate) place_notes: &'a [String],
    pub(crate) now_unix: i64,
}

fn verdict_word(j: Judgement) -> &'static str {
    match j {
        Judgement::Yes => "yes",
        Judgement::No => "no",
        Judgement::Between => "cannot_tell",
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The answer to the question asked, in the form it was asked, or a plain
/// statement of why there is none. `None` only when the question names no
/// known quantity AND asks for a value, where the readings summary is already
/// the answer.
pub(crate) fn direct_answer(inp: &AnswerInputs) -> Option<JsonValue> {
    let now = inp.now_unix;
    let mut limits: Vec<String> = inp.place_notes.to_vec();
    let time_json = inp.time.map(AskedTime::to_json);
    let base = |verdict: Option<&str>, sentence: String, limits: Vec<String>| {
        json!({
            "schema": "emem.ask_direct.v1",
            "form": inp.form.as_str(),
            "quantity": inp.quantity.map(|q| q.key),
            "asked_time": time_json,
            "verdict": verdict,
            "sentence": sentence,
            "limits": limits,
        })
    };

    let Some(qty) = inp.quantity else {
        if *inp.form == AnswerForm::Value {
            return None;
        }
        let s = "This question asks for a yes or no, and no rule here turns these readings into \
                 one. The signed readings below are what was measured at the place; the \
                 judgement is left to you."
            .to_string();
        return Some(base(None, s, limits));
    };

    if let Some(t) = inp.time.filter(|t| t.is_future(now)) {
        let s = format!(
            "Cannot tell: {} is in the future, and emem holds observations, not forecasts.",
            t.phrase
        );
        return Some(base(Some("cannot_tell"), s, limits));
    }

    // The lead reading: the first lead band with a value, preferring one that
    // was observed inside the asked window.
    let dated = inp.time.filter(|t| !t.change_window && t.is_past(now));
    let candidates: Vec<&Reading> = qty
        .lead_bands
        .iter()
        .filter_map(|b| {
            inp.readings
                .iter()
                .find(|r| r.band == *b && r.value.is_some())
        })
        .collect();
    let timing_of = |r: &Reading| -> Option<Timing> {
        let t = dated?;
        if r.is_static() {
            return Some(Timing::Inside);
        }
        r.observed_unix.map(|o| t.timing(o, r.slot_s))
    };
    let lead: Option<&Reading> = candidates.iter().copied().find(|r| {
        matches!(
            timing_of(r),
            None | Some(Timing::Inside) | Some(Timing::Near(_))
        )
    });

    let place = inp.place;
    let Some(lead) = lead else {
        let bands = qty
            .lead_bands
            .iter()
            .map(|b| crate::band_display_label(b))
            .collect::<Vec<_>>()
            .join(", ");
        let s = match (dated, candidates.first()) {
            (Some(t), Some(nearest)) => {
                let gap = match timing_of(nearest) {
                    Some(Timing::Outside(d)) => format!(", {d} days from it"),
                    _ => String::new(),
                };
                let when = nearest
                    .observed_unix
                    .map(day_of_unix)
                    .unwrap_or_else(|| "an unrecorded date".into());
                format!(
                    "Cannot tell for {}: emem holds no reading of {} at {place} from that date. \
                     The nearest it holds is from {when}{gap}, so it does not describe {}.",
                    t.phrase, qty.noun, t.phrase
                )
            }
            (Some(t), None) => format!(
                "Cannot tell for {}: emem holds no reading of {} ({bands}) at {place} from that \
                 date, and could not fetch one for it.",
                t.phrase, qty.noun
            ),
            (None, _) => format!(
                "Cannot tell: emem holds no reading of {} ({bands}) at {place}.",
                qty.noun
            ),
        };
        let mut out = base(Some("cannot_tell"), s, limits);
        if let (Some(m), Some(n)) = (out.as_object_mut(), candidates.first()) {
            m.insert(
                "nearest_reading".into(),
                json!({
                    "band": n.band, "value": n.value, "unit": n.unit, "fact_cid": n.fact_cid,
                    "observed_at": n.observed_unix.map(day_of_unix),
                }),
            );
        }
        return Some(out);
    };

    let v = lead.value.unwrap_or(f64::NAN);
    let when = when_phrase(lead, now);
    if let (Some(Timing::Near(d)), Some(t)) = (timing_of(lead), dated) {
        // A yes or no about one day is not answered from another day. For
        // Victoria Park on 14 December 2022 the nearest clear pass at or
        // before the date was the 9th, and snow fell on London between the
        // two. The reading is stated; the verdict is not drawn from it.
        if *inp.form != AnswerForm::Value {
            let reading = reading_phrase(lead);
            let s = format!(
                "Cannot tell for {}: no pass fell on that day. The nearest, {d} day(s) away, \
                 is not evidence for it: at {place}, {reading} {when}.",
                t.phrase
            );
            let mut out = base(Some("cannot_tell"), s, limits);
            if let Some(m) = out.as_object_mut() {
                m.insert(
                    "nearest_reading".into(),
                    json!({
                        "band": lead.band, "value": lead.value, "unit": lead.unit,
                        "fact_cid": lead.fact_cid,
                        "observed_at": lead.observed_unix.map(day_of_unix),
                        "days_from_date": d,
                    }),
                );
            }
            return Some(out);
        }
        limits.push(format!(
            "No pass fell on {} itself; this is the nearest, {d} day(s) away.",
            t.phrase
        ));
    }
    if lead.is_static() && inp.time.is_some() {
        if let Some(src) = static_layer_source(&lead.band) {
            limits.push(format!(
                "Static layer: {src}. The value does not change with the date asked, and it \
                 was not measured at the time of the question."
            ));
        }
    }
    let is_elevation = lead.band.contains("elevation") || lead.band.starts_with("gmrt.");
    if is_elevation && v < 0.0 {
        limits.push(format!(
            "The model reads {} m, below mean sea level. That is real in a depression or a \
             polder, but on a coast or reclaimed land it more often means the model saw water \
             or a surface that changed after it was acquired. Treat it as unverified.",
            fmt_num(v)
        ));
    }
    let rule = qty.rules.iter().find(|r| r.band == lead.band);
    if let Some(r) = rule {
        if r.near_line(v) {
            limits.push(format!(
                "{} is close to the rule's line, so it is weak evidence either way.",
                reading_phrase(lead)
            ));
        }
    }
    match lead_caution(&lead.band) {
        Some(c) => limits.push(c.to_string()),
        None if !qty.uncertainty.is_empty() => limits.push(qty.uncertainty.to_string()),
        None => {}
    }

    let reading = reading_phrase(lead);
    let (verdict, sentence): (Option<String>, String) = match (inp.form, rule) {
        (AnswerForm::Choice(a, b), Some(r)) if qty.key == "surface_water" && is_tide_pair(a, b) => {
            let occ = inp
                .readings
                .iter()
                .find(|x| x.band == "surface_water.occurrence")
                .and_then(|x| x.value);
            let (verdict, s) = tide_verdict(r.judge(v), occ, &reading, &when, place, a, b);
            (Some(verdict), s)
        }
        (AnswerForm::YesNo, Some(r)) | (AnswerForm::Choice(..), Some(r)) => {
            let j = r.judge(v);
            let what = match (r.void_at(v), j) {
                (Some(t), _) => t,
                (None, Judgement::Yes) => r.yes,
                (None, Judgement::No) => r.no,
                (None, Judgement::Between) => r.between,
            };
            let lead_word = match j {
                Judgement::Yes => "Yes",
                Judgement::No => "No",
                Judgement::Between => "Cannot tell",
            };
            let time_clause = dated
                .map(|t| format!(" for {}", t.phrase))
                .unwrap_or_default();
            (
                Some(verdict_word(j).to_string()),
                format!(
                    "{lead_word}{time_clause}: {what}. At {place}, {reading} {when}. Rule: {}.",
                    r.basis
                ),
            )
        }
        (AnswerForm::YesNo, None) | (AnswerForm::Choice(..), None) => (
            Some("cannot_tell".into()),
            format!(
                "Cannot tell as a yes or no: no rule here turns {} into one. At {place}, \
                 {reading} {when}.",
                qty.noun
            ),
        ),
        (AnswerForm::Value, _) => {
            let time_clause = dated
                .map(|t| format!(" for {}", t.phrase))
                .unwrap_or_default();
            (
                None,
                format!(
                    "{} at {place}{time_clause}: {reading}, {when}.",
                    capitalise(qty.noun)
                ),
            )
        }
    };

    let mut out = base(verdict.as_deref(), sentence, limits);
    if let Some(m) = out.as_object_mut() {
        m.insert("value".into(), json!(lead.value));
        m.insert("unit".into(), json!(lead.unit));
        m.insert("band".into(), json!(lead.band));
        m.insert("fact_cid".into(), json!(lead.fact_cid));
        m.insert(
            "observed".into(),
            if lead.is_static() {
                json!({
                    "static_layer": true,
                    "source": static_layer_source(&lead.band),
                    "released": lead.static_release,
                })
            } else {
                json!({
                    "at": lead.observed_unix.map(day_of_unix),
                    "basis": lead.observed_basis,
                    "age_days": lead.observed_unix.map(|t| (now - t).max(0) / DAY),
                })
            },
        );
        m.insert(
            "method".into(),
            json!({
                "band": lead.band,
                "rule": rule.map(|r| r.basis),
                "why": format!(
                    "the question asks about {}, and {} is the reading that measures it here",
                    qty.noun,
                    crate::band_display_label(&lead.band)
                ),
            }),
        );
    }
    Some(out)
}

fn is_tide_pair(a: &str, b: &str) -> bool {
    let set = [a, b];
    set.contains(&"high") && set.contains(&"low")
}

/// High or low water from one pixel: only a pixel that is sometimes dry can
/// tell. JRC occurrence says how often the pixel was water from 1984 to 2021.
fn tide_verdict(
    water: Judgement,
    occurrence: Option<f64>,
    reading: &str,
    when: &str,
    place: &str,
    a: &str,
    b: &str,
) -> (String, String) {
    let high = if a == "high" { a } else { b };
    let low = if a == "low" { a } else { b };
    let seen = format!("At {place}, {reading} {when}");
    let Some(occ) = occurrence else {
        return (
            "cannot_tell".into(),
            format!(
                "Cannot tell {high} from {low} water: {seen}, but with no record of how often \
                 this pixel is water, one reading cannot say which state of the tide it shows."
            ),
        );
    };
    let occ_s = fmt_num(occ);
    match water {
        Judgement::Yes if occ >= 90.0 => (
            "cannot_tell".into(),
            format!(
                "Cannot tell {high} from {low} water: {seen}, and this pixel was water in \
                 {occ_s}% of observations from 1984 to 2021. It is water at every state of \
                 the tide, so it cannot show which one this is. Read the foreshore instead: a \
                 water index over the whole place, against JRC occurrence."
            ),
        ),
        Judgement::Yes if occ >= 5.0 => (
            high.to_string(),
            format!(
                "{}: {seen}, on a pixel that was water in only {occ_s}% of observations from \
                 1984 to 2021. Covered when it is usually dry fits {high} water.",
                capitalise(high)
            ),
        ),
        Judgement::No if occ >= 90.0 => (
            "cannot_tell".into(),
            format!(
                "Cannot tell {high} from {low} water: {seen} reads dry on a pixel that was \
                 water in {occ_s}% of observations. That is unusual and more likely a misread \
                 than a tide."
            ),
        ),
        Judgement::No if occ >= 5.0 => (
            low.to_string(),
            format!(
                "{}: {seen} reads dry, on a pixel that was water in {occ_s}% of observations \
                 from 1984 to 2021. Uncovered when it is sometimes water fits {low} water.",
                capitalise(low)
            ),
        ),
        _ => (
            "cannot_tell".into(),
            format!(
                "Cannot tell {high} from {low} water: {seen}, on a pixel that was water in \
                 {occ_s}% of observations. One pixel that is almost never water, or a reading \
                 on the line, does not show the tide."
            ),
        ),
    }
}

// ── Threads ─────────────────────────────────────────────────────────────

/// What a conversation has established, so a follow-up can lean on it.
#[derive(Debug, Clone)]
pub(crate) struct Thread {
    pub(crate) cell: String,
    pub(crate) place_resolved: JsonValue,
    /// The question whose subject a follow-up inherits.
    pub(crate) question: String,
    pub(crate) turn: u32,
    at_ms: u64,
}

const THREAD_TTL_MS: u64 = 30 * 60 * 1000;
const THREAD_CAP: usize = 4096;
/// The most of a question a thread keeps. A request body may be 16 MB, and
/// THREAD_CAP of those would hold gigabytes; a follow-up needs the subject,
/// which is in the first words.
const THREAD_QUESTION_MAX: usize = 1024;

type ThreadMap = std::collections::HashMap<String, Thread>;

static THREADS: std::sync::Mutex<Option<ThreadMap>> = std::sync::Mutex::new(None);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A context id a caller may send: short, and only characters that cannot
/// carry markup or a path.
pub(crate) fn valid_context_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

pub(crate) fn mint_context_id(q: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(q.as_bytes());
    h.update(&now_ms().to_le_bytes());
    format!("ask-{}", &h.finalize().to_hex().to_string()[..20])
}

pub(crate) fn thread_get(id: &str) -> Option<Thread> {
    let now = now_ms();
    let g = THREADS.lock().ok()?;
    let t = g.as_ref()?.get(id)?;
    (now.saturating_sub(t.at_ms) < THREAD_TTL_MS).then(|| t.clone())
}

/// `id` is the caller's thread key: its context id, scoped to the caller.
pub(crate) fn thread_put(id: &str, cell: &str, place_resolved: &JsonValue, question: &str) -> u32 {
    if id.is_empty() || id.len() > 256 || cell.is_empty() {
        return 0;
    }
    let now = now_ms();
    let Ok(mut g) = THREADS.lock() else {
        return 0;
    };
    let map = g.get_or_insert_with(ThreadMap::new);
    map.retain(|_, t| now.saturating_sub(t.at_ms) < THREAD_TTL_MS);
    if map.len() >= THREAD_CAP {
        map.clear();
    }
    let turn = map.get(id).map_or(1, |t| t.turn + 1);
    map.insert(
        id.to_string(),
        Thread {
            cell: cell.to_string(),
            place_resolved: place_resolved.clone(),
            question: question[..cut_at_char(question, THREAD_QUESTION_MAX)].to_string(),
            turn,
            at_ms: now,
        },
    );
    turn
}

/// The longest prefix of `s` that is at most `max` bytes and ends on a char.
fn cut_at_char(s: &str, max: usize) -> usize {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// A follow-up that names no subject of its own ("and on 1 February 2023?")
/// continues the question before it.
pub(crate) fn is_continuation(q: &str, routed_topics: usize) -> bool {
    routed_topics == 0 && asked_quantity(q).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> i64 {
        crate::days_from_civil(y, m, d) * DAY
    }

    // 2026-11-12T18:40:00Z, when the three failures were measured.
    const NOW: i64 = 1_794_441_600 + 18 * 3600 + 40 * 60;

    #[test]
    fn the_test_clock_is_the_measurement_day() {
        assert_eq!(NOW - 18 * 3600 - 40 * 60, day(2026, 11, 12));
    }

    #[test]
    fn reads_a_day_in_three_spellings() {
        for q in [
            "Was there snow on the ground on 14 December 2022?",
            "was there snow on December 14, 2022",
            "snow on 2022-12-14?",
            "snow on the 14th of December 2022",
        ] {
            let t = asked_time(q, NOW).unwrap_or_else(|| panic!("no date in {q:?}"));
            assert_eq!(t.start_unix, day(2022, 12, 14), "{q}");
            assert_eq!(t.end_unix, day(2022, 12, 15), "{q}");
            assert_eq!(t.precision, TimePrecision::Day, "{q}");
            assert!(!t.change_window, "{q}");
            assert!(t.is_past(NOW));
        }
    }

    #[test]
    fn reads_month_and_year_and_spans() {
        let m = asked_time("NDVI in March 2021 at Ludhiana", NOW).unwrap();
        assert_eq!(m.precision, TimePrecision::Month);
        assert_eq!(m.start_unix, day(2021, 3, 1));
        assert_eq!(m.end_unix, day(2021, 4, 1));
        let y = asked_time("was the lake full in 2019", NOW).unwrap();
        assert_eq!(y.precision, TimePrecision::Year);
        assert!(!y.change_window);
        let s = asked_time("how has the forest changed since 2020", NOW).unwrap();
        assert!(s.change_window);
        let b = asked_time("NDVI between 2019 and 2021", NOW).unwrap();
        assert!(b.change_window);
    }

    #[test]
    fn a_count_is_not_a_year_and_no_date_is_none() {
        assert!(asked_time("are there 2000 buildings here", NOW).is_none());
        assert!(asked_time("what is the elevation of Mount Fuji", NOW).is_none());
        assert!(asked_time("may I ask the NDVI here", NOW).is_none());
        // An impossible day falls back to the month it names.
        let feb = asked_time("snow on 31 February 2022", NOW).unwrap();
        assert_eq!(feb.precision, TimePrecision::Month);
    }

    #[test]
    fn yesterday_and_future() {
        let y = asked_time("did it rain yesterday", NOW).unwrap();
        assert_eq!(y.start_unix, day(2026, 11, 11));
        let f = asked_time("will there be snow on 1 January 2030", NOW).unwrap();
        assert!(f.is_future(NOW));
    }

    #[test]
    fn the_date_is_taken_out_before_the_place_is_looked_for() {
        for q in [
            "and on 1 February 2023?",
            "snow on December 14, 2022 in Zermatt",
        ] {
            let t = asked_time(q, NOW).unwrap();
            let rest = without_time_phrase(q, &t);
            assert!(
                !rest.contains("February") && !rest.contains("December"),
                "{rest}"
            );
        }
    }

    #[test]
    fn date_field_forms() {
        assert_eq!(
            asked_time_field("2022-12-14").unwrap().precision,
            TimePrecision::Day
        );
        assert_eq!(
            asked_time_field("2022-12").unwrap().precision,
            TimePrecision::Month
        );
        assert_eq!(
            asked_time_field("2022").unwrap().precision,
            TimePrecision::Year
        );
        assert!(asked_time_field("14/12/2022").is_none());
    }

    #[test]
    fn word_boundaries_keep_snow_from_meaning_now() {
        assert!(!has_term("was there snow on the ground", "now"));
        assert!(has_term("is there snow now?", "now"));
        assert!(!says_now(
            "Was there snow on the ground on 14 December 2022?"
        ));
        assert!(!says_now("is the olive grove alive"));
        assert!(says_now("how busy is it right now"));
    }

    #[test]
    fn forms() {
        assert_eq!(
            answer_form(
                "Is the Thames at high or low water in the latest clear satellite picture?"
            ),
            AnswerForm::Choice("high".into(), "low".into())
        );
        assert_eq!(
            answer_form("Was there snow on the ground on 14 December 2022?"),
            AnswerForm::YesNo
        );
        assert_eq!(
            answer_form("What is the ground elevation here?"),
            AnswerForm::Value
        );
        assert_eq!(answer_form("is it raining or not"), AnswerForm::YesNo);
    }

    #[test]
    fn quantities() {
        assert_eq!(
            asked_quantity("Was there snow on the ground?").map(|q| q.key),
            Some("snow_cover")
        );
        assert_eq!(
            asked_quantity("Is the Thames at high or low water?").map(|q| q.key),
            Some("surface_water")
        );
        assert_eq!(
            asked_quantity("What is the ground elevation here?").map(|q| q.key),
            Some("ground_elevation")
        );
        assert_eq!(
            asked_quantity("is this neighbourhood flood-prone").map(|q| q.key),
            None
        );
        assert_eq!(
            asked_quantity("how high is the building").map(|q| q.key),
            None
        );
    }

    fn reading(
        band: &str,
        v: f64,
        unit: Option<&str>,
        observed: Option<i64>,
        basis: &str,
    ) -> Reading {
        Reading {
            band: band.into(),
            value: Some(v),
            unit: unit.map(String::from),
            fact_cid: Some("cid".into()),
            observed_unix: observed,
            observed_basis: basis.into(),
            static_release: None,
            slot_s: if basis == "static_layer" { 0 } else { DAY },
        }
    }

    #[test]
    fn a_dated_question_is_refused_when_the_reading_is_from_another_day() {
        let t = asked_time("Was there snow on the ground on 14 December 2022?", NOW).unwrap();
        let r = [reading(
            "indices.ndsi",
            -0.33,
            None,
            Some(day(2026, 10, 30)),
            "source_capture_time",
        )];
        let out = direct_answer(&AnswerInputs {
            form: &AnswerForm::YesNo,
            quantity: asked_quantity("snow"),
            time: Some(&t),
            readings: &r,
            place: "Victoria Park",
            place_notes: &[],
            now_unix: NOW,
        })
        .unwrap();
        assert_eq!(out["verdict"], "cannot_tell");
        let s = out["sentence"].as_str().unwrap();
        assert!(
            s.contains("14 December 2022") && s.contains("2026-10-30"),
            "{s}"
        );
    }

    #[test]
    fn in_the_polar_night_the_reanalysis_answers_and_says_it_is_a_model() {
        // Santa Claus Village, 3 December 2024: the nearest NDSI is 31 October,
        // so the observation cannot answer. ERA5-Land read 0.08 m that day.
        let t = asked_time("Was there snow on the ground on 3 December 2024?", NOW).unwrap();
        let mut hour = reading(
            "era5.snow_depth",
            0.08,
            Some("m"),
            Some(day(2024, 12, 3) + 12 * 3600),
            "source_capture_time",
        );
        hour.slot_s = 3600;
        let r = [
            reading(
                "indices.ndsi",
                0.71,
                None,
                Some(day(2024, 10, 31) + 36_000),
                "source_capture_time",
            ),
            hour,
        ];
        let out = direct_answer(&AnswerInputs {
            form: &AnswerForm::YesNo,
            quantity: asked_quantity("snow"),
            time: Some(&t),
            readings: &r,
            place: "Santa Claus Village",
            place_notes: &[],
            now_unix: NOW,
        })
        .unwrap();
        assert_eq!(out["verdict"], "yes", "{out}");
        assert_eq!(out["band"], "era5.snow_depth");
        let s = out["sentence"].as_str().unwrap();
        assert!(s.contains("in the ERA5-Land reanalysis"), "{s}");
        let limits = out["limits"].to_string();
        assert!(
            limits.contains("A model value, not an observation"),
            "{limits}"
        );
        assert!(!limits.contains("One pixel on one scene"), "{limits}");
    }

    #[test]
    fn a_glacier_pack_in_the_reanalysis_cannot_tell() {
        // Everest Base Camp, 15 July 2025: ERA5-Land holds 26.67 m all year.
        // On emem.dev that read "Yes ... snow depth 26.67 m" until this rule.
        let t = asked_time("Was there snow on the ground on 15 July 2025?", NOW).unwrap();
        let mut hour = reading(
            "era5.snow_depth",
            26.67,
            Some("m"),
            Some(day(2025, 7, 15) + 12 * 3600),
            "source_capture_time",
        );
        hour.slot_s = 3600;
        let r = [
            reading(
                "indices.ndsi",
                0.12,
                None,
                Some(day(2025, 7, 7) + 18_000),
                "source_capture_time",
            ),
            hour,
        ];
        let out = direct_answer(&AnswerInputs {
            form: &AnswerForm::YesNo,
            quantity: asked_quantity("snow"),
            time: Some(&t),
            readings: &r,
            place: "Everest Base Camp",
            place_notes: &[],
            now_unix: NOW,
        })
        .unwrap();
        assert_eq!(out["verdict"], "cannot_tell", "{out}");
        let s = out["sentence"].as_str().unwrap();
        assert!(s.contains("holds on a glacier all year"), "{s}");
        assert!(!s.starts_with("Yes"), "{s}");
    }

    #[test]
    fn a_dated_question_answers_from_a_pass_that_day() {
        let t = asked_time("Was there snow on the ground on 14 December 2022?", NOW).unwrap();
        let r = [reading(
            "indices.ndsi",
            0.61,
            None,
            Some(day(2022, 12, 14) + 39_600),
            "source_capture_time",
        )];
        let out = direct_answer(&AnswerInputs {
            form: &AnswerForm::YesNo,
            quantity: asked_quantity("snow"),
            time: Some(&t),
            readings: &r,
            place: "Victoria Park",
            place_notes: &[],
            now_unix: NOW,
        })
        .unwrap();
        assert_eq!(out["verdict"], "yes");
        assert!(out["sentence"]
            .as_str()
            .unwrap()
            .starts_with("Yes for 14 December 2022"));
    }

    #[test]
    fn a_yes_or_no_about_a_day_is_not_drawn_from_another_day() {
        let t = asked_time("Was there snow on the ground on 14 December 2022?", NOW).unwrap();
        let r = [reading(
            "indices.ndsi",
            -0.27,
            None,
            Some(day(2022, 12, 9) + 39_600),
            "source_capture_time",
        )];
        let out = direct_answer(&AnswerInputs {
            form: &AnswerForm::YesNo,
            quantity: asked_quantity("snow"),
            time: Some(&t),
            readings: &r,
            place: "Victoria Park",
            place_notes: &[],
            now_unix: NOW,
        })
        .unwrap();
        assert_eq!(out["verdict"], "cannot_tell");
        assert_eq!(out["nearest_reading"]["observed_at"], "2022-12-09");
        // A value question takes the nearest pass and says how far it is.
        let v = direct_answer(&AnswerInputs {
            form: &AnswerForm::Value,
            quantity: asked_quantity("snow"),
            time: Some(&t),
            readings: &r,
            place: "Victoria Park",
            place_notes: &[],
            now_unix: NOW,
        })
        .unwrap();
        assert_eq!(v["band"], "indices.ndsi");
        assert!(v["limits"].to_string().contains("No pass fell on"), "{v}");
    }

    #[test]
    fn tide_on_a_channel_pixel_cannot_be_told() {
        let form = answer_form(
            "Is the Thames at high or low water in the latest clear satellite picture?",
        );
        let r = [
            reading(
                "indices.mndwi",
                0.49,
                None,
                Some(day(2026, 10, 30)),
                "source_capture_time",
            ),
            reading(
                "surface_water.occurrence",
                98.0,
                Some("%"),
                None,
                "static_layer",
            ),
        ];
        let out = direct_answer(&AnswerInputs {
            form: &form,
            quantity: asked_quantity("high or low water"),
            time: None,
            readings: &r,
            place: "Thames Barrier",
            place_notes: &[],
            now_unix: NOW,
        })
        .unwrap();
        assert_eq!(out["verdict"], "cannot_tell");
        assert_eq!(out["band"], "indices.mndwi");
        let s = out["sentence"].as_str().unwrap();
        assert!(s.contains("98%") && !s.contains("B02"), "{s}");
        // On the foreshore the same reading answers.
        let r2 = [
            r[0].clone(),
            reading(
                "surface_water.occurrence",
                40.0,
                Some("%"),
                None,
                "static_layer",
            ),
        ];
        let out2 = direct_answer(&AnswerInputs {
            form: &form,
            quantity: asked_quantity("high or low water"),
            time: None,
            readings: &r2,
            place: "Thames Barrier",
            place_notes: &[],
            now_unix: NOW,
        })
        .unwrap();
        assert_eq!(out2["verdict"], "high");
    }

    #[test]
    fn a_static_negative_elevation_says_so() {
        let r = [reading(
            "copdem30m.elevation_mean",
            -5.51,
            Some("m"),
            None,
            "static_layer",
        )];
        let notes = ["`Palm Jumeirah, Dubai` resolved to a point of interest".to_string()];
        let out = direct_answer(&AnswerInputs {
            form: &AnswerForm::Value,
            quantity: asked_quantity("What is the ground elevation here?"),
            time: None,
            readings: &r,
            place: "Waldorf Astoria",
            place_notes: &notes,
            now_unix: NOW,
        })
        .unwrap();
        let s = out["sentence"].as_str().unwrap();
        assert!(s.contains("static layer") && !s.contains("days ago"), "{s}");
        let limits = out["limits"].to_string();
        assert!(limits.contains("below mean sea level"), "{limits}");
        assert!(limits.contains("point of interest"), "{limits}");
        assert_eq!(out["observed"]["static_layer"], true);
    }

    /// A dated follow-up continues the thread whatever the embedding router
    /// says. The keyword pass decides; "ndvi" names a subject and does not.
    #[test]
    fn a_dated_follow_up_names_no_topic_in_its_words() {
        let r = crate::topic_router::TopicRouter::global();
        let q = "and on 1 February 2023?";
        assert!(r.keyword_topics(q).is_empty());
        assert!(is_continuation(q, r.keyword_topics(q).len()));
        let q = "and what is the NDVI?";
        assert!(!r.keyword_topics(q).is_empty());
        assert!(!is_continuation(q, r.keyword_topics(q).len()));
    }

    #[test]
    fn threads_inherit_and_expire_by_id() {
        let id = mint_context_id("q");
        assert!(valid_context_id(&id));
        assert!(!valid_context_id("a b"));
        assert_eq!(
            thread_put(&id, "defi.zb64a.quno.zfe73", &json!({}), "snow?"),
            1
        );
        assert_eq!(
            thread_put(&id, "defi.zb64a.quno.zfe73", &json!({}), "snow?"),
            2
        );
        assert_eq!(thread_get(&id).unwrap().cell, "defi.zb64a.quno.zfe73");
        assert!(thread_get("ask-unknown").is_none());

        // A thread keeps at most THREAD_QUESTION_MAX bytes of the question,
        // cut on a char: a 16 MB body must not live on in the thread map.
        let long = format!(
            "{}é{}",
            "a".repeat(THREAD_QUESTION_MAX - 1),
            "b".repeat(4096)
        );
        let id2 = mint_context_id("long");
        thread_put(&id2, "defi.zb64a.quno.zfe73", &json!({}), &long);
        let kept = thread_get(&id2).unwrap().question;
        assert_eq!(kept.len(), THREAD_QUESTION_MAX - 1);
        assert!(long.starts_with(&kept));
        assert!(is_continuation("and on 1 February 2023?", 0));
        assert!(!is_continuation("and the elevation?", 0));
    }

    /// The fixed question set every change to /v1/ask is checked against.
    /// Offline: it exercises the classifier, not the network.
    #[test]
    fn question_set_classifies_as_expected() {
        let cases: JsonValue =
            serde_json::from_str(include_str!("testdata/ask_cases.json")).unwrap();
        let mut failures = Vec::new();
        for c in cases["cases"].as_array().unwrap() {
            let id = c["id"].as_str().unwrap();
            let q = c["q"].as_str().unwrap();
            let want = &c["expect"];
            if let Some(f) = want.get("form").and_then(|v| v.as_str()) {
                if answer_form(q).as_str() != f {
                    failures.push(format!("{id}: form {} != {f}", answer_form(q).as_str()));
                }
            }
            if let Some(k) = want.get("quantity") {
                let got = asked_quantity(q).map(|x| x.key);
                if got != k.as_str() {
                    failures.push(format!("{id}: quantity {got:?} != {k}"));
                }
            }
            if let Some(d) = want.get("time_from") {
                let got = asked_time(q, NOW).map(|t| day_of_unix(t.start_unix));
                if got.as_deref() != d.as_str() {
                    failures.push(format!("{id}: time {got:?} != {d}"));
                }
            }
            if let Some(cw) = want.get("change_window").and_then(|v| v.as_bool()) {
                let got = asked_time(q, NOW).is_some_and(|t| t.change_window);
                if got != cw {
                    failures.push(format!("{id}: change_window {got} != {cw}"));
                }
            }
            if let Some(ls) = want.get("live_scene").and_then(|v| v.as_bool()) {
                if crate::question_wants_live_scene(q) != ls {
                    failures.push(format!("{id}: live_scene != {ls}"));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
