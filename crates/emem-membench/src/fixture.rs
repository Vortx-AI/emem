//! Built-in synthetic fixture corpus.
//!
//! Everything here is deliberately tiny and self-contained so the
//! benchmark produces NON-EMPTY, *computed* scores fully offline. None
//! of these numbers are baked-in results — they are the ground truth the
//! scoring functions in [`crate::score`] grade a responder against.
//!
//! The corpus is shaped to exercise the four MemoryAgentBench-style axes
//! over an emem-flavoured "memory" of Earth-observation facts:
//!
//! * **Retrieval accuracy** — does a recall on a known cell+band return
//!   the expected fact value?
//! * **Test-time learning** — after a sequence of session "writes", can a
//!   later query recall the most-recently-written value (not a stale one)?
//! * **Long-range understanding** — across a long interleaved transcript,
//!   can a query that depends on an item near the *start* still be
//!   answered correctly (no recency-only collapse)?
//! * **Conflict resolution** — when two facts about the same subject
//!   disagree, does the system surface the contradiction rather than
//!   silently picking one?

/// A single stored memory item the responder is expected to "know".
#[derive(Clone, Debug)]
pub struct MemoryItem {
    /// Opaque key the query addresses (an emem cell64-ish handle).
    pub cell: &'static str,
    /// Band / attribute name.
    pub band: &'static str,
    /// Canonical truth value as a short string (we compare normalised).
    pub value: &'static str,
}

/// A retrieval-accuracy question: recall `cell`+`band`, expect `expected`.
#[derive(Clone, Debug)]
pub struct RetrievalQuery {
    pub cell: &'static str,
    pub band: &'static str,
    pub expected: &'static str,
}

/// A test-time-learning episode: a series of writes to one cell+band over
/// a session, then the value that should win (the last write).
#[derive(Clone, Debug)]
pub struct LearningEpisode {
    pub cell: &'static str,
    pub band: &'static str,
    /// Writes in session order; the LAST is the ground-truth answer.
    pub writes: &'static [&'static str],
}

/// A long-range item: a fact planted at `position` in a long transcript,
/// queried at the end. `position` is informational (smaller = earlier =
/// harder for recency-biased systems).
#[derive(Clone, Debug)]
pub struct LongRangeItem {
    pub cell: &'static str,
    pub band: &'static str,
    pub expected: &'static str,
    pub position: usize,
}

/// A conflict case: two attestations disagree about one subject. A correct
/// system reports a contradiction (`should_conflict == true`).
#[derive(Clone, Debug)]
pub struct ConflictCase {
    pub cell: &'static str,
    pub band: &'static str,
    pub value_a: &'static str,
    pub value_b: &'static str,
    pub should_conflict: bool,
}

/// The base "memory" the stub responder is seeded with for the retrieval
/// and long-range axes.
pub const ITEMS: &[MemoryItem] = &[
    MemoryItem { cell: "defi.aa111.bbbb.cccc", band: "copdem30m.elevation_mean", value: "812.0" },
    MemoryItem { cell: "defi.aa111.bbbb.cccc", band: "esa_worldcover.class",      value: "tree_cover" },
    MemoryItem { cell: "defi.dd222.eeee.ffff", band: "s2.ndvi",                   value: "0.71" },
    MemoryItem { cell: "defi.dd222.eeee.ffff", band: "jrc_gsw.occurrence",        value: "3" },
    MemoryItem { cell: "defi.gg333.hhhh.iiii", band: "s2.ndvi",                   value: "0.18" },
    MemoryItem { cell: "defi.gg333.hhhh.iiii", band: "copdem30m.elevation_mean", value: "4.0" },
];

pub const RETRIEVAL_QUERIES: &[RetrievalQuery] = &[
    RetrievalQuery { cell: "defi.aa111.bbbb.cccc", band: "copdem30m.elevation_mean", expected: "812.0" },
    RetrievalQuery { cell: "defi.aa111.bbbb.cccc", band: "esa_worldcover.class",      expected: "tree_cover" },
    RetrievalQuery { cell: "defi.dd222.eeee.ffff", band: "s2.ndvi",                   expected: "0.71" },
    RetrievalQuery { cell: "defi.dd222.eeee.ffff", band: "jrc_gsw.occurrence",        expected: "3" },
    RetrievalQuery { cell: "defi.gg333.hhhh.iiii", band: "s2.ndvi",                   expected: "0.18" },
];

pub const LEARNING_EPISODES: &[LearningEpisode] = &[
    // NDVI updated three times in one growing-season session; latest wins.
    LearningEpisode {
        cell: "defi.dd222.eeee.ffff",
        band: "s2.ndvi",
        writes: &["0.41", "0.58", "0.71"],
    },
    // Water occurrence revised as new scenes land.
    LearningEpisode {
        cell: "defi.dd222.eeee.ffff",
        band: "jrc_gsw.occurrence",
        writes: &["1", "2", "3"],
    },
    // Land-cover reclassified after a clearing event.
    LearningEpisode {
        cell: "defi.aa111.bbbb.cccc",
        band: "esa_worldcover.class",
        writes: &["tree_cover", "shrubland", "tree_cover"],
    },
    // Elevation is a one-shot write (single-element session is valid).
    LearningEpisode {
        cell: "defi.aa111.bbbb.cccc",
        band: "copdem30m.elevation_mean",
        writes: &["812.0"],
    },
];

pub const LONG_RANGE_ITEMS: &[LongRangeItem] = &[
    // Planted early, queried late — the classic "needle near the front".
    LongRangeItem { cell: "defi.aa111.bbbb.cccc", band: "copdem30m.elevation_mean", expected: "812.0", position: 0 },
    LongRangeItem { cell: "defi.aa111.bbbb.cccc", band: "esa_worldcover.class",      expected: "tree_cover", position: 1 },
    LongRangeItem { cell: "defi.gg333.hhhh.iiii", band: "copdem30m.elevation_mean", expected: "4.0", position: 5 },
    LongRangeItem { cell: "defi.gg333.hhhh.iiii", band: "s2.ndvi",                   expected: "0.18", position: 12 },
];

pub const CONFLICT_CASES: &[ConflictCase] = &[
    // Two sources disagree on NDVI for the same cell → must flag conflict.
    ConflictCase {
        cell: "defi.dd222.eeee.ffff",
        band: "s2.ndvi",
        value_a: "0.71",
        value_b: "0.33",
        should_conflict: true,
    },
    // Disagreeing land-cover labels → conflict.
    ConflictCase {
        cell: "defi.aa111.bbbb.cccc",
        band: "esa_worldcover.class",
        value_a: "tree_cover",
        value_b: "cropland",
        should_conflict: true,
    },
    // Identical values from two sources → NOT a conflict (must not flag).
    ConflictCase {
        cell: "defi.gg333.hhhh.iiii",
        band: "copdem30m.elevation_mean",
        value_a: "4.0",
        value_b: "4.0",
        should_conflict: false,
    },
];
