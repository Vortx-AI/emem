//! `emem:tree`: per-row audit paths into a note's Merkle table.
//!
//! A pointer note (`emem: pointer.v1`) names a large object by the BLAKE3 hash
//! of every chunk it read and one Merkle root over them; a directory note
//! (`emem: directory.v1`) does the same over a listing. The root is signed,
//! inside the author's note. Checking one chunk used to mean fetching the
//! whole table and rebuilding the tree, which stops working when the table is
//! a bucket of millions of chunks. Given a row, this returns the log2(n)
//! sibling hashes from its leaf to the note's root, so a reader checks one
//! chunk against the signed root and nothing else.
//!
//! The tree is the one those notes already commit to, reproduced exactly
//! rather than redefined, so every root already published verifies:
//!
//!   leaf  = blake3(url || u64_be offset || u64_be length || hash)
//!   node  = blake3(left || right); an odd node is promoted unchanged
//!
//! A pointer row at the note's own source is written `·` and hashed with an
//! empty url. A directory row's `hash` is `blake3(path "\n" size "\n"
//! publisher_hash)` and its leaf is over `(url, 0, size, that hash)`.
//!
//! This tree has no leaf/node domain separation (RFC 6962 prefixes 0x00 and
//! 0x01; this responder's own transparency log does). A leaf here hashes
//! at least 16 bytes plus a url and an interior node exactly 64, so a leaf
//! that is also a valid node would need a url of the right length whose bytes
//! spell two hashes; the path also carries each step's side. That is stated
//! rather than assumed away: a verifier that wants the stronger property
//! should also check the row's content against the source.
//!
//! No signature is minted here. The path is checkable against the root in the
//! author-signed note, and adding ours would only say that we computed it.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use crate::{ApiError, AppState, ErrorBody, ErrorCode};

pub(crate) type Hash = [u8; 32];

/// One row of a note's Merkle table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub label: String,
    pub leaf: Hash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NoteTree {
    pub kind: String,
    pub stated_root: Option<String>,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TreeError {
    /// The note is not a kind this route knows how to read as a tree.
    NotATree(String),
    /// A kind that should carry rows carried none.
    NoRows,
}

pub(crate) fn chunk_leaf(url: &str, offset: u64, length: u64, hash: &Hash) -> Hash {
    let mut h = blake3::Hasher::new();
    h.update(url.as_bytes());
    h.update(&offset.to_be_bytes());
    h.update(&length.to_be_bytes());
    h.update(hash);
    *h.finalize().as_bytes()
}

fn node(l: &Hash, r: &Hash) -> Hash {
    let mut h = blake3::Hasher::new();
    h.update(l);
    h.update(r);
    *h.finalize().as_bytes()
}

pub(crate) fn root(leaves: &[Hash]) -> Option<Hash> {
    let mut level: Vec<Hash> = leaves.to_vec();
    if level.is_empty() {
        return None;
    }
    while level.len() > 1 {
        level = level
            .chunks(2)
            .map(|p| {
                if p.len() == 2 {
                    node(&p[0], &p[1])
                } else {
                    p[0]
                }
            })
            .collect();
    }
    Some(level[0])
}

/// Which side of the running hash a sibling goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Left,
    Right,
}

/// Sibling hashes from `leaves[index]` to the root. A level where the node is
/// the promoted odd one contributes no step.
pub(crate) fn audit_path(leaves: &[Hash], index: usize) -> Option<Vec<(Side, Hash)>> {
    if index >= leaves.len() {
        return None;
    }
    let mut level: Vec<Hash> = leaves.to_vec();
    let mut i = index;
    let mut path = Vec::new();
    while level.len() > 1 {
        if i % 2 == 1 {
            path.push((Side::Left, level[i - 1]));
        } else if i + 1 < level.len() {
            path.push((Side::Right, level[i + 1]));
        }
        level = level
            .chunks(2)
            .map(|p| {
                if p.len() == 2 {
                    node(&p[0], &p[1])
                } else {
                    p[0]
                }
            })
            .collect();
        i /= 2;
    }
    Some(path)
}

pub(crate) fn fold_path(leaf: &Hash, path: &[(Side, Hash)]) -> Hash {
    path.iter().fold(*leaf, |acc, (side, sib)| match side {
        Side::Left => node(sib, &acc),
        Side::Right => node(&acc, sib),
    })
}

fn b32(h: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(h).to_ascii_lowercase()
}

fn unb32_32(s: &str) -> Option<Hash> {
    let v = data_encoding::BASE32_NOPAD
        .decode(s.to_ascii_uppercase().as_bytes())
        .ok()?;
    <Hash>::try_from(v.as_slice()).ok()
}

fn is_hash52(s: &str) -> bool {
    s.len() == 52
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

/// The frontmatter value for `key`, from the block between the first two
/// `---` lines.
fn front(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for l in lines {
        if l.trim() == "---" {
            return None;
        }
        if let Some(v) = l.strip_prefix(key).and_then(|r| r.strip_prefix(':')) {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// The cells of a markdown table line, `| a | b |` -> `["a", "b"]`, without
/// trimming, so the exact spacing the writers use can be required.
fn cells(line: &str) -> Option<Vec<&str>> {
    let inner = line.strip_prefix("| ")?.strip_suffix(" |")?;
    Some(inner.split(" | ").collect())
}

/// A pointer row: `| label | url-or-· | offset | length | hash |` with an
/// optional trailing stats cell, as ememdemo's `parseRows` reads it.
fn pointer_row(line: &str) -> Option<Row> {
    let c = cells(line)?;
    if !(c.len() == 5 || c.len() == 6) {
        return None;
    }
    let (label, url, off, len, hash) = (c[0], c[1], c[2], c[3], c[4].trim());
    if label.trim().is_empty() || label.contains('|') || label.trim() == "what" {
        return None;
    }
    if url.is_empty() || url.contains(char::is_whitespace) {
        return None;
    }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(off) || !digits(len) {
        return None;
    }
    let offset: u64 = off.parse().ok()?;
    let length: u64 = len.parse().ok()?;
    // A row without a hash (a fill value, not read) is committed as zeros.
    let h = if is_hash52(hash) {
        unb32_32(hash)?
    } else {
        [0u8; 32]
    };
    let url = if url == "·" { "" } else { url };
    Some(Row {
        label: label.trim().to_string(),
        leaf: chunk_leaf(url, offset, length, &h),
    })
}

/// A directory row: `| path | https:url | bytes | scheme:hash |`.
fn directory_row(line: &str) -> Option<Row> {
    let c = cells(line)?;
    if c.len() != 4 {
        return None;
    }
    let (path, url, size, hash) = (c[0], c[1], c[2], c[3]);
    if path.is_empty() || !url.starts_with("https:") || url.contains(char::is_whitespace) {
        return None;
    }
    if size.is_empty() || !size.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !hash.contains(':') || hash.contains(char::is_whitespace) {
        return None;
    }
    let path = path.replace("%7C", "|");
    let n: u64 = size.parse().ok()?;
    let row_hash = *blake3::hash(format!("{path}\n{n}\n{hash}").as_bytes()).as_bytes();
    Some(Row {
        label: path,
        leaf: chunk_leaf(url, 0, n, &row_hash),
    })
}

pub(crate) fn parse_note(text: &str) -> Result<NoteTree, TreeError> {
    let kind = front(text, "emem").unwrap_or_default();
    let parse: fn(&str) -> Option<Row> = match kind.as_str() {
        "pointer.v1" => pointer_row,
        "directory.v1" => directory_row,
        other => return Err(TreeError::NotATree(other.to_string())),
    };
    let rows: Vec<Row> = text.lines().filter_map(parse).collect();
    if rows.is_empty() {
        return Err(TreeError::NoRows);
    }
    Ok(NoteTree {
        kind,
        stated_root: front(text, "root"),
        rows,
    })
}

fn bad(
    status: StatusCode,
    code: ErrorCode,
    wire: &str,
    message: String,
    extra: JsonValue,
) -> ApiError {
    let mut details = json!({ "code": wire });
    if let (Some(d), Some(e)) = (details.as_object_mut(), extra.as_object()) {
        d.extend(e.clone());
    }
    ApiError(
        status,
        ErrorBody {
            code,
            message,
            details: Some(details),
        },
    )
}

fn path_json(path: &[(Side, Hash)]) -> JsonValue {
    json!(path
        .iter()
        .map(|(side, h)| json!({
            "side": if *side == Side::Left { "left" } else { "right" },
            "hash_b32": b32(h),
        }))
        .collect::<Vec<_>>())
}

const SCHEME: &str = "leaf = blake3(url || u64_be offset || u64_be length || hash); node = blake3(left || right); an odd node is promoted unchanged";
const VERIFY: &str = "h = leaf; for each step: h = side == left ? blake3(step.hash || h) : blake3(h || step.hash); accept if h == root_b32 and root_b32 is the `root:` of the author-signed note";

/// `GET /v1/tree/{file_cid}?row=<index|label>`.
pub(crate) async fn get_tree(
    State(s): State<AppState>,
    Path(raw): Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, ApiError> {
    let file_cid = raw
        .trim()
        .trim_start_matches("emem:tree:")
        .split('#')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let db = crate::memory_db(&s)?;
    let blob = db
        .open_tree(emem_storage::TREE_MEMORY_FILE_BLOBS)
        .ok()
        .and_then(|t| t.get(file_cid.as_bytes()).ok().flatten())
        .map(|b| b.to_vec())
        // Content-addressed: bytes that do not hash to the cid are not the note.
        .filter(|b| crate::compute_file_cid(b) == file_cid)
        .ok_or_else(|| {
            bad(
                StatusCode::NOT_FOUND,
                ErrorCode::CidNotFound,
                "cid_not_found",
                format!("no memory note with file_cid {file_cid}"),
                json!({ "file_cid": file_cid }),
            )
        })?;
    let text = String::from_utf8_lossy(&blob);
    let tree = parse_note(&text).map_err(|e| match e {
        TreeError::NotATree(kind) => bad(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::InvalidArgument,
            "not_a_tree",
            format!("note kind {kind:?} carries no Merkle table this route reads; pointer.v1 and directory.v1 do (a chained feed is a hash chain, not a tree)"),
            json!({ "kind": kind, "supported": ["pointer.v1", "directory.v1"] }),
        ),
        TreeError::NoRows => bad(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::InvalidArgument,
            "no_rows",
            "the note names a tree kind but no table row parsed".into(),
            json!({}),
        ),
    })?;
    let leaves: Vec<Hash> = tree.rows.iter().map(|r| r.leaf).collect();
    let computed = root(&leaves).map(|r| b32(&r)).unwrap_or_default();
    // A root this responder cannot reproduce from the note's own table gets no
    // path: a path to a root the note does not state proves nothing.
    if tree.stated_root.as_deref() != Some(computed.as_str()) {
        return Err(bad(
            StatusCode::CONFLICT,
            ErrorCode::InvalidArgument,
            "root_mismatch",
            "the note's stated root does not match the tree over its own rows".into(),
            json!({
                "file_cid": file_cid,
                "stated_root_b32": tree.stated_root,
                "computed_root_b32": computed,
                "rows": leaves.len(),
            }),
        ));
    }
    let base = json!({
        "file_cid": file_cid,
        "kind": tree.kind,
        "rows": leaves.len(),
        "root_b32": computed,
        "note_root_matches": true,
        "scheme": SCHEME,
        "verify": VERIFY,
    });
    let Some(want) = q.get("row") else {
        let mut out = base;
        out["token"] = json!(format!("emem:tree:{file_cid}"));
        out["labels"] = json!(tree
            .rows
            .iter()
            .take(50)
            .map(|r| &r.label)
            .collect::<Vec<_>>());
        out["note"] = json!("pass ?row=<index or label> for one row's audit path");
        return Ok(Json(out));
    };
    let index = want
        .parse::<usize>()
        .ok()
        .filter(|i| *i < leaves.len())
        .or_else(|| tree.rows.iter().position(|r| &r.label == want))
        .ok_or_else(|| {
            bad(
                StatusCode::NOT_FOUND,
                ErrorCode::InvalidArgument,
                "row_not_found",
                format!("no row {want:?} in this note ({} rows)", leaves.len()),
                json!({ "rows": leaves.len() }),
            )
        })?;
    let path = audit_path(&leaves, index).unwrap_or_default();
    // Never hand out a path that does not reach the root it is served with.
    if root(&leaves).is_some_and(|r| fold_path(&leaves[index], &path) != r) {
        return Err(bad(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            "path_self_check_failed",
            "the computed audit path does not fold to the root".into(),
            json!({}),
        ));
    }
    let mut out = base;
    out["token"] = json!(format!("emem:tree:{file_cid}#row={index}"));
    out["row"] = json!({ "index": index, "label": tree.rows[index].label });
    out["leaf_b32"] = json!(b32(&leaves[index]));
    out["path"] = path_json(&path);
    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
pub(crate) struct TreePathReq {
    /// Leaf hashes, base32-nopad of 32 bytes each, in tree order.
    #[serde(default)]
    leaves: Option<Vec<String>>,
    /// Or the rows themselves, and the leaf is computed here.
    #[serde(default)]
    chunks: Option<Vec<ChunkIn>>,
    index: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChunkIn {
    #[serde(default)]
    url: String,
    offset: u64,
    length: u64,
    hash: String,
}

const MAX_LEAVES: usize = 1 << 20;

/// `POST /v1/tree/path`: the same path over rows a caller holds, with no
/// note and no parser in between.
pub(crate) async fn post_tree_path(
    Json(req): Json<TreePathReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let arg = |m: String| {
        bad(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidArgument,
            "invalid_argument",
            m,
            json!({}),
        )
    };
    let leaves: Vec<Hash> = match (req.leaves, req.chunks) {
        (Some(ls), None) => ls
            .iter()
            .map(|l| {
                unb32_32(l).ok_or_else(|| arg(format!("leaf {l:?} is not base32 of 32 bytes")))
            })
            .collect::<Result<_, _>>()?,
        (None, Some(cs)) => cs
            .iter()
            .map(|c| {
                let h = if is_hash52(&c.hash) {
                    unb32_32(&c.hash)
                } else {
                    None
                }
                .ok_or_else(|| arg(format!("chunk hash {:?} is not base32 of 32 bytes", c.hash)))?;
                let url = if c.url == "·" { "" } else { c.url.as_str() };
                Ok::<Hash, ApiError>(chunk_leaf(url, c.offset, c.length, &h))
            })
            .collect::<Result<_, _>>()?,
        _ => return Err(arg("pass exactly one of `leaves` or `chunks`".into())),
    };
    if leaves.len() > MAX_LEAVES {
        return Err(arg(format!("at most {MAX_LEAVES} leaves")));
    }
    let path = audit_path(&leaves, req.index).ok_or_else(|| {
        arg(format!(
            "index {} out of range ({} leaves)",
            req.index,
            leaves.len()
        ))
    })?;
    Ok(Json(json!({
        "index": req.index,
        "leaves": leaves.len(),
        "leaf_b32": b32(&leaves[req.index]),
        "path": path_json(&path),
        "root_b32": root(&leaves).map(|r| b32(&r)),
        "scheme": SCHEME,
        "verify": VERIFY,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIRECTORY_NOTE: &str = include_str!("testdata/ememdemo-directory.v1.md");
    const POINTER_NOTE: &str = include_str!("testdata/ememdemo-pointer.v1.md");

    /// The roots two published ememdemo notes state, reproduced from their
    /// own tables: the parser reads what the writer wrote, not a fixture
    /// shaped to agree with it.
    #[test]
    fn published_notes_reproduce_their_stated_roots() {
        for (note, rows) in [(DIRECTORY_NOTE, 21), (POINTER_NOTE, 107)] {
            let t = parse_note(note).expect("parses");
            assert_eq!(t.rows.len(), rows, "{}", t.kind);
            let leaves: Vec<Hash> = t.rows.iter().map(|r| r.leaf).collect();
            assert_eq!(root(&leaves).map(|r| b32(&r)), t.stated_root, "{}", t.kind);
        }
    }

    /// Every row's path folds to the root, at every tree size including the
    /// odd ones where a node is promoted.
    #[test]
    fn every_path_folds_to_the_root() {
        for n in 1..=33usize {
            let leaves: Vec<Hash> = (0..n)
                .map(|i| *blake3::hash(&i.to_be_bytes()).as_bytes())
                .collect();
            let r = root(&leaves).unwrap();
            for i in 0..n {
                let p = audit_path(&leaves, i).unwrap();
                assert!(p.len() <= usize::BITS as usize - n.leading_zeros() as usize);
                assert_eq!(fold_path(&leaves[i], &p), r, "n={n} i={i}");
                // Control: the same path does not fold a different leaf to it.
                let other = *blake3::hash(b"not a row").as_bytes();
                assert_ne!(fold_path(&other, &p), r, "n={n} i={i}");
            }
        }
        assert!(audit_path(&[[0u8; 32]], 1).is_none());
    }

    #[test]
    fn a_tampered_row_or_an_unknown_kind_is_refused() {
        let t = parse_note(DIRECTORY_NOTE).unwrap();
        let first = t.rows[0].label.clone();
        let tampered = DIRECTORY_NOTE.replacen("| 557635 |", "| 557636 |", 1);
        assert_ne!(
            tampered, DIRECTORY_NOTE,
            "control: the edit applied to {first}"
        );
        let t2 = parse_note(&tampered).unwrap();
        let leaves: Vec<Hash> = t2.rows.iter().map(|r| r.leaf).collect();
        assert_ne!(root(&leaves).map(|r| b32(&r)), t2.stated_root);

        let other = DIRECTORY_NOTE.replacen("emem: directory.v1", "emem: world.v1", 1);
        assert_eq!(
            parse_note(&other),
            Err(TreeError::NotATree("world.v1".into()))
        );
    }
}
