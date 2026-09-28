//! Documents a compliance check asks for by name, read out of OCR text:
//!
//! - `lab_report_parse@1`: a pesticide-residue lab report, row by row
//!   (analyte, result or not-detected, unit, LOQ, the MRL the report prints).
//! - `land_record_parse@1`: a land record's owner, parcel id, area and place.
//!
//! Deterministic: no model guesses a value. Every field carries the line it
//! was read from and that line's byte offset in the text, and the text is
//! bound by its blake3 to the signed `/v1/ocr` receipt that produced it, so
//! a reader can check each value against the image. A field the parser
//! cannot place is reported missing, never filled.

use serde::Serialize;

/// One value, where it came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Located<T> {
    pub value: T,
    pub line: String,
    pub line_no: usize,
    pub byte_offset: usize,
}

// ───────────────────────────── lab reports ─────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Measured {
    Value {
        raw: String,
        number: f64,
    },
    NotDetected {
        raw: String,
    },
    BelowLimit {
        raw: String,
        limit: Option<f64>,
    },
    /// "1,000": a decimal comma or a thousands separator. Read either way it
    /// could be 1 or 1000, so it never concludes a row is within a limit.
    Ambiguous {
        raw: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResidueRow {
    pub analyte: String,
    pub result: Measured,
    pub unit: Option<String>,
    pub result_mg_kg: Option<f64>,
    pub loq_mg_kg: Option<f64>,
    pub mrl_mg_kg: Option<f64>,
    /// Against the MRL the report prints, when it prints one.
    pub exceeds_printed_mrl: Option<bool>,
    /// Values on the line the header did not name.
    pub unmapped: Vec<String>,
    pub line: String,
    pub line_no: usize,
    pub byte_offset: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LabReport {
    pub rows: Vec<ResidueRow>,
    pub columns: Vec<&'static str>,
    pub header_line_no: Option<usize>,
    pub sample_id: Option<Located<String>>,
    pub dates: Vec<Located<String>>,
    pub accreditation: Vec<Located<String>>,
    pub methods: Vec<Located<String>>,
    pub verdict: &'static str,
    pub exceedances: usize,
}

/// A unit token and its factor to mg/kg.
fn unit_factor(tok: &str) -> Option<(&'static str, f64)> {
    let t = tok
        .trim_matches(|c: char| c == '(' || c == ')' || c == '[' || c == ']' || c == ',')
        .to_lowercase()
        .replace('μ', "µ");
    Some(match t.as_str() {
        "mg/kg" | "mgkg" | "mg/kg." | "ppm" | "mg·kg-1" | "mg/kg-1" => ("mg/kg", 1.0),
        "µg/kg" | "ug/kg" | "mcg/kg" | "ppb" | "µgkg" => ("µg/kg", 0.001),
        "g/kg" => ("g/kg", 1000.0),
        _ => return None,
    })
}

/// A token read as a result: a number, not-detected, or below a limit.
fn measured(tok: &str, next: Option<&str>) -> Option<(Measured, bool)> {
    let raw = tok.trim_matches(|c: char| c == '|' || c == ';');
    let l = raw.to_lowercase();
    if matches!(
        l.as_str(),
        "nd" | "n.d." | "n.d" | "bdl" | "bql" | "nq" | "n.q." | "absent" | "nil"
    ) {
        return Some((
            Measured::NotDetected {
                raw: raw.to_string(),
            },
            false,
        ));
    }
    if l == "not" && next.is_some_and(|n| n.to_lowercase().starts_with("detect")) {
        return Some((
            Measured::NotDetected {
                raw: "not detected".into(),
            },
            true,
        ));
    }
    if let Some(rest) = l.strip_prefix(['<', '≤']) {
        let rest = rest.trim();
        if rest.is_empty() {
            // "< 0.01" split in two tokens.
            let n = next.and_then(number);
            return Some((
                Measured::BelowLimit {
                    raw: format!("< {}", next.unwrap_or("")),
                    limit: n,
                },
                n.is_some()
                    || next.is_some_and(|x| {
                        matches!(x.to_lowercase().as_str(), "loq" | "lod" | "lor")
                    }),
            ));
        }
        if matches!(rest, "loq" | "lod" | "lor" | "rl") {
            return Some((
                Measured::BelowLimit {
                    raw: raw.to_string(),
                    limit: None,
                },
                false,
            ));
        }
        if let Some(n) = number(rest) {
            return Some((
                Measured::BelowLimit {
                    raw: raw.to_string(),
                    limit: Some(n),
                },
                false,
            ));
        }
        return None;
    }
    if ambiguous_separator(raw) {
        return Some((
            Measured::Ambiguous {
                raw: raw.to_string(),
            },
            false,
        ));
    }
    number(raw).map(|n| {
        (
            Measured::Value {
                raw: raw.to_string(),
                number: n,
            },
            false,
        )
    })
}

/// "1,000": one comma, no dot, three digits after it.
fn ambiguous_separator(tok: &str) -> bool {
    let t = tok.trim_matches(|c: char| c == '(' || c == ')' || c == '*');
    let Some((a, b)) = t.split_once(',') else {
        return false;
    };
    !t.contains('.')
        && !b.contains(',')
        && (1..=3).contains(&a.len())
        && b.len() == 3
        && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
}

/// A plain decimal, with `.` or `,` as the separator.
fn number(tok: &str) -> Option<f64> {
    if ambiguous_separator(tok) {
        return None;
    }
    let t = tok.trim_matches(|c: char| c == '(' || c == ')' || c == '*');
    if t.is_empty() || !t.chars().next()?.is_ascii_digit() {
        return None;
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
    {
        return None;
    }
    t.replace(',', ".").parse().ok()
}

fn is_header(l: &str) -> bool {
    let l = l.to_lowercase();
    let result = [
        "result",
        "found",
        "residue",
        "concentration",
        "value",
        "content",
    ]
    .iter()
    .any(|w| l.contains(w));
    let limit = ["loq", "lod", "mrl", "limit", "unit", "lor", "tolerance"]
        .iter()
        .any(|w| l.contains(w));
    result && limit
}

/// The header's value columns, in the order they appear.
fn header_columns(l: &str) -> Vec<&'static str> {
    let l = l.to_lowercase();
    let mut cols: Vec<(usize, &'static str)> = Vec::new();
    let mut find = |words: &[&str], name: &'static str| {
        if let Some(p) = words.iter().filter_map(|w| l.find(w)).min() {
            cols.push((p, name));
        }
    };
    find(
        &[
            "result",
            "found",
            "residue level",
            "concentration",
            "value",
            "content",
        ],
        "result",
    );
    find(
        &[
            "loq",
            "lod",
            "lor",
            "limit of quant",
            "limit of det",
            "reporting limit",
        ],
        "loq",
    );
    find(
        &[
            "mrl",
            "maximum residue",
            "max. limit",
            "permissible",
            "tolerance",
        ],
        "mrl",
    );
    cols.sort_by_key(|c| c.0);
    cols.into_iter().map(|c| c.1).collect()
}

/// Words that begin a line of a report without naming an analyte.
const NOT_ANALYTES: &[&str] = &[
    "sample",
    "date",
    "page",
    "method",
    "report",
    "total",
    "client",
    "customer",
    "address",
    "tel",
    "phone",
    "email",
    "lab",
    "remarks",
    "note",
    "signature",
    "authori",
    "received",
    "analys",
    "parameter",
    "test",
    "batch",
    "lot",
    "order",
    "reference",
    "ref",
    "product",
    "matrix",
    "commodity",
    "weight",
    "quantity",
    "moisture",
];

/// Lines with their byte offsets in `text`, computed once. `str::lines`
/// drops `\r\n` and `\n` alike, so a running `len + 1` fell a byte short
/// per earlier line on CRLF text; and recomputing it per hit was quadratic.
fn split_lines(text: &str) -> (Vec<&str>, Vec<usize>) {
    let mut lines = Vec::new();
    let mut offs = Vec::new();
    let mut at = 0;
    for raw in text.split_inclusive('\n') {
        offs.push(at);
        at += raw.len();
        lines.push(raw.trim_end_matches('\n').trim_end_matches('\r'));
    }
    (lines, offs)
}

fn locate<T>(value: T, offs: &[usize], line_no: usize, line: &str) -> Located<T> {
    Located {
        value,
        line: line.to_string(),
        line_no,
        byte_offset: offs.get(line_no).copied().unwrap_or(0),
    }
}

/// Case-insensitive find over `hay`, returning byte positions in `hay`
/// itself: lowercasing can change a string's byte length, so positions in
/// a lowercased copy do not slice the original.
fn find_ci(hay: &str, needle: &str) -> Option<(usize, usize)> {
    // ASCII on both sides: lowercasing keeps byte positions.
    if hay.is_ascii() && needle.is_ascii() {
        let p = hay
            .to_ascii_lowercase()
            .find(&needle.to_ascii_lowercase())?;
        return Some((p, p + needle.len()));
    }
    let n: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if n.is_empty() {
        return None;
    }
    let idx: Vec<(usize, char)> = hay.char_indices().collect();
    for start in 0..idx.len() {
        let (mut k, mut j) = (0, start);
        while k < n.len() && j < idx.len() {
            let low: Vec<char> = idx[j].1.to_lowercase().collect();
            if n[k..].starts_with(&low) {
                k += low.len();
                j += 1;
            } else {
                break;
            }
        }
        if k == n.len() {
            return Some((idx[start].0, idx.get(j).map_or(hay.len(), |x| x.0)));
        }
    }
    None
}

pub fn parse_lab_report(text: &str) -> LabReport {
    let (lines, offs) = split_lines(text);
    let header_line_no = lines.iter().position(|l| is_header(l));
    let columns = header_line_no
        .map(|i| header_columns(lines[i]))
        .unwrap_or_default();
    let header_unit =
        header_line_no.and_then(|i| lines[i].split_whitespace().find_map(unit_factor));
    let doc_unit = {
        let mut counts: std::collections::BTreeMap<&'static str, (usize, f64)> = Default::default();
        for t in text.split_whitespace() {
            if let Some((u, f)) = unit_factor(t) {
                counts.entry(u).or_insert((0, f)).0 += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, (n, _))| *n)
            .map(|(u, (_, f))| (u, f))
    };
    let mut rows = Vec::new();
    let start = header_line_no.map_or(0, |i| i + 1);
    for (no, line) in lines.iter().enumerate().skip(start) {
        let toks: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == '|' || c == '\t')
            .filter(|t| !t.is_empty())
            .collect();
        let Some(first_val) =
            (0..toks.len()).find(|&i| measured(toks[i], toks.get(i + 1).copied()).is_some())
        else {
            continue;
        };
        let analyte = toks[..first_val].join(" ");
        let analyte =
            analyte.trim_matches(|c: char| c == ':' || c == '-' || c == '.' || c.is_whitespace());
        let letters = analyte.chars().filter(|c| c.is_alphabetic()).count();
        let lower = analyte.to_lowercase();
        if letters < 3 || NOT_ANALYTES.iter().any(|w| lower.starts_with(w)) {
            continue;
        }
        let mut values: Vec<Measured> = Vec::new();
        let mut unit: Option<(&'static str, f64)> = None;
        let mut i = first_val;
        while i < toks.len() {
            if let Some(u) = unit_factor(toks[i]) {
                unit = unit.or(Some(u));
                i += 1;
                continue;
            }
            match measured(toks[i], toks.get(i + 1).copied()) {
                Some((m, used_next)) => {
                    values.push(m);
                    i += if used_next { 2 } else { 1 };
                }
                None => i += 1,
            }
        }
        let unit = unit.or(header_unit).or(doc_unit);
        let to_mg = |x: f64| unit.map(|(_, f)| x * f);
        let num = |m: &Measured| match m {
            Measured::Value { number, .. } => Some(*number),
            Measured::BelowLimit { limit, .. } => *limit,
            Measured::NotDetected { .. } | Measured::Ambiguous { .. } => None,
        };
        let order: Vec<&'static str> = if columns.is_empty() {
            vec!["result"]
        } else {
            columns.clone()
        };
        let mut result = None;
        let (mut loq, mut mrl) = (None, None);
        let mut unmapped = Vec::new();
        for (k, v) in values.into_iter().enumerate() {
            match order.get(k).copied() {
                Some("result") if result.is_none() => result = Some(v),
                Some("loq") => loq = num(&v).and_then(to_mg),
                Some("mrl") => mrl = num(&v).and_then(to_mg),
                _ => unmapped.push(match &v {
                    Measured::Value { raw, .. }
                    | Measured::NotDetected { raw }
                    | Measured::Ambiguous { raw }
                    | Measured::BelowLimit { raw, .. } => raw.clone(),
                }),
            }
        }
        let Some(result) = result else { continue };
        let result_mg_kg = match &result {
            Measured::Value { number, .. } => to_mg(*number),
            _ => None,
        };
        // A result under a limit is within the MRL only when that limit is:
        // "<0.05" says nothing about an MRL of 0.01.
        let under = match &result {
            Measured::BelowLimit { limit, .. } => limit.and_then(to_mg).or(loq),
            Measured::NotDetected { .. } => loq,
            _ => None,
        };
        let exceeds_printed_mrl = match (&result, result_mg_kg, mrl) {
            (_, Some(r), Some(m)) => Some(r > m),
            (Measured::NotDetected { .. } | Measured::BelowLimit { .. }, None, Some(m)) => {
                under.filter(|l| *l <= m).map(|_| false)
            }
            _ => None,
        };
        let at = locate((), &offs, no, line);
        rows.push(ResidueRow {
            analyte: analyte.to_string(),
            result,
            unit: unit.map(|(u, _)| u.to_string()),
            result_mg_kg,
            loq_mg_kg: loq,
            mrl_mg_kg: mrl,
            exceeds_printed_mrl,
            unmapped,
            line: at.line,
            line_no: no,
            byte_offset: at.byte_offset,
        });
    }
    let find_all = |pred: &dyn Fn(&str) -> Option<String>| -> Vec<Located<String>> {
        lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| pred(l).map(|v| locate(v, &offs, i, l)))
            .collect()
    };
    // Labels in order of preference, each over the whole document: a
    // "Test Report No" on an earlier line is not the sample's id.
    let sample_id = [
        "sample id",
        "sample no",
        "sample number",
        "sample code",
        "lab no",
        "report no",
    ]
    .iter()
    .find_map(|w| {
        lines.iter().enumerate().find_map(|(i, l)| {
            let (_, end) = find_ci(l, w)?;
            let v = l[end..].trim_start_matches(|c: char| {
                c == ':' || c == '.' || c == '#' || c.is_whitespace()
            });
            let v = v.split_whitespace().next()?;
            v.chars()
                .any(|c| c.is_ascii_digit())
                .then(|| locate(v.to_string(), &offs, i, l))
        })
    });
    let dates = find_all(&|l: &str| date_in(l));
    let accreditation = find_all(&|l: &str| {
        let low = l.to_lowercase();
        [
            "17025", "nabl", "dakks", "ukas", "cofrac", "enac", "accredia", "a2la", "iso/iec",
        ]
        .iter()
        .find(|w| low.contains(**w))
        .map(|w| w.to_uppercase())
    });
    let methods = find_all(&|l: &str| {
        let low = l.to_lowercase();
        [
            "quechers", "en 15662", "lc-ms/ms", "gc-ms/ms", "lc-ms", "gc-ms", "aoac", "hplc",
        ]
        .iter()
        .find(|w| low.contains(**w))
        .map(|w| w.to_uppercase())
    });
    let exceedances = rows
        .iter()
        .filter(|r| r.exceeds_printed_mrl == Some(true))
        .count();
    let verdict = if rows.is_empty() {
        "no_rows_found"
    } else if exceedances > 0 {
        "exceeds_printed_mrl"
    } else if rows
        .iter()
        .any(|r| r.mrl_mg_kg.is_some() && r.exceeds_printed_mrl.is_none())
    {
        // A row the report gives a limit for but that cannot be decided
        // (a "<LOQ" above the MRL, an ambiguous separator) blocks "within".
        "undetermined_rows"
    } else if rows.iter().any(|r| r.mrl_mg_kg.is_some()) {
        "within_printed_mrls"
    } else {
        "no_mrl_printed"
    };
    LabReport {
        rows,
        columns,
        header_line_no,
        sample_id,
        dates,
        accreditation,
        methods,
        verdict,
        exceedances,
    }
}

/// The first date on a line: dd.mm.yyyy, dd/mm/yyyy, dd-mm-yyyy or yyyy-mm-dd.
fn date_in(l: &str) -> Option<String> {
    let b = l.as_bytes();
    for (i, _) in l.char_indices() {
        let rest = &l[i..];
        let mut parts = Vec::new();
        let mut cur = String::new();
        let mut seps = 0;
        for c in rest.chars().take(10) {
            if c.is_ascii_digit() {
                cur.push(c);
            } else if matches!(c, '.' | '/' | '-') && !cur.is_empty() && seps < 2 {
                parts.push(std::mem::take(&mut cur));
                seps += 1;
            } else {
                break;
            }
        }
        if !cur.is_empty() {
            parts.push(cur);
        }
        if parts.len() == 3 {
            let lens: Vec<usize> = parts.iter().map(|p| p.len()).collect();
            let ok = matches!(lens[..], [1..=2, 1..=2, 4] | [4, 1..=2, 1..=2]);
            if ok && (i == 0 || !b[i - 1].is_ascii_digit()) {
                return Some(parts.join("-"));
            }
        }
    }
    None
}

// ───────────────────────────── land records ─────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LandRecord {
    pub owners: Vec<Located<String>>,
    pub parcel_ids: Vec<Located<String>>,
    pub areas: Vec<Located<Area>>,
    pub places: Vec<Located<String>>,
    pub dates: Vec<Located<String>>,
    pub fields_found: Vec<&'static str>,
    pub fields_missing: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Area {
    pub raw: String,
    pub unit: Option<String>,
    /// Only for units with one meaning everywhere; a bigha is not one.
    pub hectares: Option<f64>,
}

/// Devanagari, Bengali, Gujarati, Gurmukhi, Tamil, Telugu, Kannada and
/// Malayalam digits to ASCII.
pub fn ascii_digits(s: &str) -> String {
    const ZEROS: [u32; 8] = [
        0x0966, 0x09E6, 0x0AE6, 0x0A66, 0x0BE6, 0x0C66, 0x0CE6, 0x0D66,
    ];
    s.chars()
        .map(|c| {
            let u = c as u32;
            ZEROS
                .iter()
                .find(|&&z| (z..z + 10).contains(&u))
                .and_then(|z| char::from_digit(u - z, 10))
                .unwrap_or(c)
        })
        .collect()
}

const OWNER_LABELS: &[&str] = &[
    "name of owner",
    "owner's name",
    "owner name",
    "owner",
    "occupant",
    "khatedar",
    "pattadar",
    "खातेदाराचे नाव",
    "भोगवटादाराचे नाव",
    "खातेदार का नाम",
    "खातेदार",
    "भूमिस्वामी",
    "proprietário",
    "proprietario",
    "possuidor",
    "titular",
    "propietario",
    "propriétaire",
    "pemegang hak",
    "nama pemilik",
];
const PARCEL_LABELS: &[&str] = &[
    "survey no",
    "survey number",
    "sy. no",
    "sy no",
    "gat no",
    "gut no",
    "khasra no",
    "khasra number",
    "plot no",
    "parcel no",
    "parcel id",
    "dag no",
    "patta no",
    "भूमापन क्रमांक",
    "गट क्रमांक",
    "सर्वे नंबर",
    "खसरा संख्या",
    "खसरा नं",
    "खसरा",
    "matrícula",
    "matricula",
    "código do imóvel",
    "codigo do imovel",
    "número do recibo",
    "recibo de inscrição",
    "número de predio",
    "numero de predio",
    "cédula catastral",
    "nomor hak",
    "nomor sertipikat",
    "nib",
];
const AREA_LABELS: &[&str] = &[
    "total area",
    "area",
    "extent",
    "क्षेत्रफल",
    "क्षेत्र",
    "रकबा",
    "área total",
    "área",
    "area total",
    "superficie",
    "luas",
];
const PLACE_LABELS: &[&str] = &[
    "village",
    "tehsil",
    "taluka",
    "mandal",
    "district",
    "गाव",
    "गांव",
    "तालुका",
    "तहसील",
    "जिल्हा",
    "जिला",
    "município",
    "municipio",
    "comune",
    "commune",
    "kecamatan",
    "desa",
    "kabupaten",
];

/// The value after a label on the same line.
fn after_label(line: &str, labels: &[&str]) -> Option<String> {
    // The earliest label, and of labels starting there the longest.
    // Short labels must be whole words: "desa" is inside "Desai".
    let ascii_low = line.is_ascii().then(|| line.to_ascii_lowercase());
    let bounded = |w: &str| {
        let (p, e) = match &ascii_low {
            Some(low) if w.is_ascii() => {
                let p = low.find(&w.to_ascii_lowercase())?;
                (p, p + w.len())
            }
            // An ASCII line cannot hold a non-ASCII label.
            Some(_) => return None,
            None => find_ci(line, w)?,
        };
        let short = w.chars().count() <= 5;
        let before = line[..p]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        let after = line[e..].chars().next().is_some_and(char::is_alphanumeric);
        (!short || (!before && !after)).then_some((p, e))
    };
    let (_, end) = labels
        .iter()
        .filter_map(|w| bounded(w))
        .min_by_key(|(p, e)| (*p, usize::MAX - e))?;
    let rest = &line[end..];
    let v = rest
        .trim_start_matches(|c: char| {
            matches!(c, ':' | '-' | '.' | '#' | '|' | '=') || c.is_whitespace()
        })
        .trim();
    (!v.is_empty()).then(|| v.to_string())
}

fn parse_area(v: &str) -> Area {
    let a = ascii_digits(v);
    let low = a.to_lowercase();
    let n: Option<f64> = {
        let t: String = a
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
            .collect();
        // "1.20.00" is hectare.are.square-metre on a 7/12 extract.
        let parts: Vec<&str> = t.split('.').collect();
        if parts.len() == 3 {
            let (h, ar, sq) = (
                parts[0].parse::<f64>().ok(),
                parts[1].parse::<f64>().ok(),
                parts[2].parse::<f64>().ok(),
            );
            match (h, ar, sq) {
                (Some(h), Some(ar), Some(sq)) => Some(h + ar / 100.0 + sq / 10_000.0),
                _ => None,
            }
        } else {
            t.replace(',', ".").trim_end_matches('.').parse().ok()
        }
    };
    let unit_table: &[(&[&str], &str, Option<f64>)] = &[
        (
            &["hectare", "hect", "ha", "हेक्टर", "हे.", "हे", "hektar"],
            "ha",
            Some(1.0),
        ),
        (&["acre", "acres", "एकर", "एकड़"], "acre", Some(0.404_685_6)),
        (&["guntha", "गुंठे", "गुंठा"], "guntha", Some(0.010_117_1)),
        (&["are", "आर"], "are", Some(0.01)),
        (
            &[
                "m²",
                "m2",
                "sq m",
                "sq. m",
                "sqm",
                "metros quadrados",
                "मीटर",
            ],
            "m2",
            Some(0.0001),
        ),
        (&["bigha", "बीघा", "बिघा"], "bigha", None),
        (&["kanal", "कनाल"], "kanal", None),
        (&["cent", "सेंट"], "cent", Some(0.004_046_86)),
    ];
    // Whole words: "ha" is inside "bigha", and a bigha is not a hectare.
    let words: Vec<&str> = low
        .split(|c: char| {
            c.is_whitespace() || c.is_ascii_digit() || matches!(c, ',' | ';' | '(' | ')' | '/')
        })
        .filter(|w| !w.is_empty())
        .collect();
    let unit = unit_table.iter().find(|(names, _, _)| {
        names.iter().any(|n| {
            let multi = n.contains(' ');
            (multi && low.contains(n))
                || words.iter().any(|w| {
                    let w = w.trim_matches('.');
                    w == n.trim_matches('.') || (n.chars().count() >= 5 && w.starts_with(n))
                })
        })
    });
    // A three-part 7/12 area is already hectares; count the dots in the
    // number itself, so a written "ha.are.m2" beside it does not hide it.
    let numeric: String = a
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let three_part = numeric.trim_end_matches('.').matches('.').count() == 2 && unit.is_none();
    Area {
        raw: v.to_string(),
        unit: unit
            .map(|(_, u, _)| u.to_string())
            .or(three_part.then(|| "ha.are.m2".into())),
        hectares: match (n, unit) {
            (Some(n), Some((_, _, Some(f)))) => Some(n * f),
            (Some(n), None) if three_part => Some(n),
            _ => None,
        },
    }
}

pub fn parse_land_record(text: &str) -> LandRecord {
    let (lines, offs) = split_lines(text);
    let mut rec = LandRecord {
        owners: vec![],
        parcel_ids: vec![],
        areas: vec![],
        places: vec![],
        dates: vec![],
        fields_found: vec![],
        fields_missing: vec![],
    };
    for (i, l) in lines.iter().enumerate() {
        if let Some(v) = after_label(l, PARCEL_LABELS) {
            let id = ascii_digits(&v);
            let id = id
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches([',', ';']);
            if id.chars().any(|c| c.is_ascii_digit()) {
                rec.parcel_ids.push(locate(id.to_string(), &offs, i, l));
            }
        }
        if let Some(v) = after_label(l, AREA_LABELS) {
            let a = parse_area(&v);
            if ascii_digits(&v).chars().any(|c| c.is_ascii_digit()) {
                rec.areas.push(locate(a, &offs, i, l));
            }
        }
        if let Some(v) = after_label(l, OWNER_LABELS) {
            if v.chars().filter(|c| c.is_alphabetic()).count() >= 3 {
                rec.owners.push(locate(v, &offs, i, l));
            }
        }
        if let Some(v) = after_label(l, PLACE_LABELS) {
            if v.chars().filter(|c| c.is_alphabetic()).count() >= 2 {
                rec.places.push(locate(v, &offs, i, l));
            }
        }
        if let Some(d) = date_in(&ascii_digits(l)) {
            rec.dates.push(locate(d, &offs, i, l));
        }
    }
    for (name, found) in [
        ("owner", !rec.owners.is_empty()),
        ("parcel_id", !rec.parcel_ids.is_empty()),
        ("area", !rec.areas.is_empty()),
        ("place", !rec.places.is_empty()),
    ] {
        if found {
            rec.fields_found.push(name);
        } else {
            rec.fields_missing.push(name);
        }
    }
    rec
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "Eurofins Food Testing Lab  ISO/IEC 17025 accredited (DAkkS D-PL-14602)\n\
Sample ID: EF-2026-00417   Received: 12.09.2026\n\
Method: QuEChERS EN 15662, LC-MS/MS and GC-MS/MS\n\
Analyte            Result   Unit    LOQ     MRL\n\
Chlorpyrifos       0.023    mg/kg   0.01    0.01\n\
Lambda-cyhalothrin <0.01    mg/kg   0.01    0.2\n\
Imidacloprid       n.d.     mg/kg   0.01    0.5\n\
Carbendazim        0,08     mg/kg   0.01    0.1\n\
Total                                          \n";

    #[test]
    fn a_residue_table_reads_row_by_row_against_its_printed_mrl() {
        let r = parse_lab_report(REPORT);
        assert_eq!(r.columns, vec!["result", "loq", "mrl"]);
        assert_eq!(r.rows.len(), 4, "{:#?}", r.rows);
        let chlor = &r.rows[0];
        assert_eq!(chlor.analyte, "Chlorpyrifos");
        assert_eq!(chlor.result_mg_kg, Some(0.023));
        assert_eq!(chlor.mrl_mg_kg, Some(0.01));
        assert_eq!(chlor.exceeds_printed_mrl, Some(true));
        assert!(
            matches!(r.rows[1].result, Measured::BelowLimit { limit: Some(l), .. } if (l - 0.01).abs() < 1e-12)
        );
        assert_eq!(r.rows[1].exceeds_printed_mrl, Some(false));
        assert!(matches!(r.rows[2].result, Measured::NotDetected { .. }));
        assert_eq!(r.rows[3].result_mg_kg, Some(0.08), "comma decimals");
        assert_eq!(r.verdict, "exceeds_printed_mrl");
        assert_eq!(r.exceedances, 1);
        assert_eq!(
            r.sample_id.as_ref().map(|s| s.value.as_str()),
            Some("EF-2026-00417")
        );
        assert!(r
            .accreditation
            .iter()
            .any(|a| a.value == "17025" || a.value == "ISO/IEC"));
        assert!(r.methods.iter().any(|m| m.value == "QUECHERS"));
        assert_eq!(r.dates[0].value, "12-09-2026");
        // The offset points at the line in the text.
        let row = &r.rows[0];
        assert!(REPORT[row.byte_offset..].starts_with("Chlorpyrifos"));
    }

    #[test]
    fn a_limit_above_the_mrl_proves_nothing_and_a_thousands_comma_is_ambiguous() {
        let t = "Analyte   Result   Unit   LOQ   MRL\r\nChlorpyrifos   <0.05   mg/kg   0.05   0.01\r\nImidacloprid   1,000   µg/kg   10   500\r\nAcetamiprid   n.d.   mg/kg   0.01   0.3\r\n";
        let r = parse_lab_report(t);
        assert_eq!(r.rows.len(), 3, "{:#?}", r.rows);
        assert_eq!(
            r.rows[0].exceeds_printed_mrl, None,
            "<0.05 cannot show it is under 0.01"
        );
        assert!(matches!(r.rows[1].result, Measured::Ambiguous { .. }));
        assert_eq!(r.rows[1].exceeds_printed_mrl, None);
        assert_eq!(
            r.rows[2].exceeds_printed_mrl,
            Some(false),
            "n.d. at LOQ 0.01 is under 0.3"
        );
        // CRLF offsets point at the line itself.
        for row in &r.rows {
            assert!(
                t[row.byte_offset..].starts_with(row.line.as_str()),
                "{}",
                row.line
            );
        }
        assert_ne!(r.verdict, "within_printed_mrls");
    }

    #[test]
    fn the_sample_id_beats_a_report_number_and_a_labelled_seven_twelve_area_reads() {
        let r = parse_lab_report("Test Report No: TR-88121\nSample ID: S-2026-17\n");
        assert_eq!(r.sample_id.map(|s| s.value).as_deref(), Some("S-2026-17"));
        let a = parse_area("1.20.50 ha.are.m2");
        assert!((a.hectares.unwrap() - 1.205).abs() < 1e-9, "{a:?}");
    }

    #[test]
    fn short_labels_are_whole_words_and_cent_is_not_centiare() {
        let r = parse_land_record("Owner: Ravi Desai\nDesa: Sukamaju\n");
        assert_eq!(r.places.len(), 1, "{:?}", r.places);
        assert_eq!(r.places[0].value, "Sukamaju");
        assert!(parse_area("12 centiare").hectares.is_none());
    }

    #[test]
    fn many_lines_parse_in_linear_time() {
        let time = |n: usize| {
            let t = "01.01.2020 Village: X\n".repeat(n);
            let start = std::time::Instant::now();
            let r = parse_land_record(&t);
            assert_eq!(r.dates.len(), n);
            start.elapsed().as_secs_f64()
        };
        let (a, b) = (time(10_000), time(40_000));
        // Four times the lines: about four times the work, not sixteen.
        assert!(b < a * 8.0 + 0.05, "10k lines {a:.3}s, 40k lines {b:.3}s");
    }

    #[test]
    fn micrograms_convert_and_a_report_without_mrls_says_so() {
        let t =
            "Pesticide   Result (µg/kg)   LOQ\nAcetamiprid   12   10\nThiamethoxam   <10   10\n";
        let r = parse_lab_report(t);
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.rows[0].result_mg_kg, Some(0.012));
        assert_eq!(r.rows[0].loq_mg_kg, Some(0.01));
        assert_eq!(r.verdict, "no_mrl_printed");
        assert_eq!(
            parse_lab_report("nothing tabular here").verdict,
            "no_rows_found"
        );
    }

    #[test]
    fn a_seven_twelve_extract_gives_owner_parcel_and_area() {
        let t = "गाव : वडगाव   तालुका : हवेली   जिल्हा : पुणे\n\
भूमापन क्रमांक : १२३/२\n\
क्षेत्र : १.२०.००\n\
भोगवटादाराचे नाव : रमेश पाटील\n\
दिनांक 05/08/2024\n";
        let r = parse_land_record(t);
        assert_eq!(r.parcel_ids[0].value, "123/2");
        assert!((r.areas[0].value.hectares.unwrap() - 1.2).abs() < 1e-9);
        assert_eq!(r.owners[0].value, "रमेश पाटील");
        assert!(!r.places.is_empty());
        assert_eq!(r.dates[0].value, "05-08-2024");
        assert!(r.fields_missing.is_empty(), "{:?}", r.fields_missing);
    }

    #[test]
    fn a_car_receipt_in_portuguese_and_an_ambiguous_unit() {
        let t = "Número do Recibo: MT-5107925-1A2B3C4D5E6F\nÁrea total: 48,5 ha\nMunicípio: Sorriso\nProprietário: Fazenda Boa Vista Ltda\n";
        let r = parse_land_record(t);
        assert!(r.parcel_ids[0].value.starts_with("MT-5107925"));
        assert!((r.areas[0].value.hectares.unwrap() - 48.5).abs() < 1e-9);
        let b = parse_area("3 bigha 5 biswa");
        assert_eq!(b.unit.as_deref(), Some("bigha"));
        assert!(b.hectares.is_none(), "a bigha differs by state");
    }
}
