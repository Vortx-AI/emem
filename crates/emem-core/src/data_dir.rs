//! Where this node keeps its data, described once.
//!
//! `EMEM_DATA` was read in twelve places across five crates, with THREE
//! different defaults: `./var/emem` in the server binary, `/var/emem` in the
//! JRC fetcher, and `/home/ubuntu/emem/var/emem` in five more. In production
//! the variable is set, so all twelve agreed and nothing was visibly wrong.
//!
//! Unset -- which is what a fresh clone gets -- they disagree. The server would
//! write under `./var/emem` while the Lance index looked under an absolute path
//! that exists on exactly one machine in the world, and the JRC fetcher looked
//! under `/var/emem`, which exists on none. The node would split its own data
//! across three directories, two of them absent, and report no error doing it.
//!
//! The absolute paths were also a developer's home directory committed to a
//! public repository, which made the defect invisible to us and only to us.
//!
//! One function, one default. The default is relative because the node's data
//! belongs beside the node, and because a relative path cannot silently be
//! right for one machine and wrong for every other.

use std::path::PathBuf;

/// The default when `EMEM_DATA` is unset. Relative to the working directory,
/// which for a checkout is the repository root.
pub const DEFAULT_DATA_DIR: &str = "./var/emem";

/// This node's data directory: `EMEM_DATA`, or [`DEFAULT_DATA_DIR`].
///
/// Every caller that needs the data root goes through here. Reading the
/// variable directly is how the three defaults happened.
pub fn data_dir() -> PathBuf {
    std::env::var("EMEM_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_DATA_DIR))
}

/// Two callers deliberately do NOT use this: the topic router and the text
/// embedder ask `if let Ok(d) = env::var("EMEM_DATA")` and, when it is unset,
/// return "neither EMEM_TOPIC_MODEL_DIR nor EMEM_DATA is set -- set one to
/// point at a model". That is not a second default; it is a question about
/// whether the node was configured, and its answer names the variables to set.
/// Routing them through here would trade that for a vaguer "model missing"
/// after a look in a directory nobody asked for.
///
/// A path under the data directory: `data_dir().join(tail)`.
pub fn data_path(tail: &str) -> PathBuf {
    data_dir().join(tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default must be relative.
    ///
    /// An absolute default is the defect this module exists to remove: it is
    /// correct on the machine it was written on and wrong everywhere else, and
    /// it fails by silently reading an empty directory rather than by erroring.
    #[test]
    fn the_default_is_relative_and_names_no_particular_machine() {
        let p = PathBuf::from(DEFAULT_DATA_DIR);
        assert!(
            p.is_relative(),
            "the default data dir must be relative, got {DEFAULT_DATA_DIR}"
        );
        assert!(
            !DEFAULT_DATA_DIR.contains("/home/"),
            "a home directory in the default is one machine's truth: {DEFAULT_DATA_DIR}"
        );
    }

    /// `data_path` must be `data_dir` plus the tail, so that a caller who uses
    /// it cannot end up somewhere `data_dir` does not cover.
    #[test]
    fn data_path_stays_under_data_dir() {
        let root = data_dir();
        let p = data_path("lance");
        assert!(p.starts_with(&root), "{p:?} escaped {root:?}");
        assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("lance"));
    }
}
