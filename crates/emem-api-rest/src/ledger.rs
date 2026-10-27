//! A view of the memory ledger, kept current from the ledger's own writes.
//!
//! The inbox, the channel's geo feed and the agent roster each answered by
//! walking every path in the store and reading each note's metadata, and the
//! geo feed read every body: 79k notes and about a second a call on
//! 2026-10-27, with 669 inbox calls a day paying it in full. The view is built
//! by that same walk once; after it, every write queues the path it touched and
//! the next reader re-reads only those. A store the view was not built for, an
//! overflowed queue, or ten minutes of age sends the next reader through a full
//! rebuild, which also catches the internal cleanups that publish no event.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{
    inbox_note_head, memory_db, now_unix_s_local, parse_skill_declaration, ApiError, AppState,
    ErrorBody, ErrorCode, InboxNoteHead, MemoryFileMeta, StatusCode,
};
use emem_primitives::memory_events::MemoryEvent;

/// A view older than this is rebuilt by the next reader.
const MAX_AGE: Duration = Duration::from_secs(600);

const PREFIX: &str = "/memories/by_attester/";

/// Queued paths past this many mean nobody has read in a long time.
const PENDING_CAP: usize = 50_000;

/// One note under `/memories/by_attester/`, as the three readers need it.
pub(crate) struct NoteRec {
    pub(crate) path: String,
    pub(crate) cid: String,
    /// The namespace owner, which the store guarantees.
    pub(crate) from: String,
    pub(crate) signed_at: String,
    pub(crate) signed_at_unix_s: i64,
    /// A caller signature is stored.
    pub(crate) has_sig: bool,
    /// The key, when key, signature and body hash are all present.
    pub(crate) proven_key: Option<String>,
    pub(crate) attester_pubkey_b32: Option<String>,
    pub(crate) head: Arc<InboxNoteHead>,
    /// The declaration, for an `agent-skills.md`.
    pub(crate) skills: Option<serde_json::Value>,
}

pub(crate) struct Ledger {
    /// The store this view describes: `Arc::as_ptr` of its `AppState`.
    owner: usize,
    built_at: Instant,
    pub(crate) notes: HashMap<String, Arc<NoteRec>>,
    /// file_cid -> author, for threading.
    pub(crate) cid_author: HashMap<String, String>,
}

static LEDGER: Mutex<Option<Ledger>> = Mutex::new(None);

/// Paths the write events named since the view last caught up.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Set when the queue overflowed: the view cannot know what it dropped.
static STALE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn owner_of(s: &AppState) -> usize {
    Arc::as_ptr(s) as usize
}

fn internal(what: &str, e: impl std::fmt::Display) -> ApiError {
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        ErrorBody {
            code: ErrorCode::CacheError,
            message: format!("{what}: {e}"),
            details: None,
        },
    )
}

/// Record the paths a write touched. Called inside `publish_memory_event`, so
/// a write's path is queued before the write returns: a reader that follows a
/// writer always sees it, with no task and no lag in between.
pub(crate) fn note_event(ev: &MemoryEvent) {
    let touched: Vec<&str> = match ev {
        MemoryEvent::Created { path, .. }
        | MemoryEvent::Modified { path, .. }
        | MemoryEvent::Deleted { path, .. }
        | MemoryEvent::Expired { path, .. } => vec![path.as_str()],
        MemoryEvent::Renamed {
            old_path, new_path, ..
        } => vec![old_path.as_str(), new_path.as_str()],
        MemoryEvent::Consolidated { paths, .. } => paths.iter().map(String::as_str).collect(),
    };
    let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    p.extend(
        touched
            .into_iter()
            .filter(|t| t.starts_with(PREFIX))
            .map(str::to_string),
    );
    // Nobody has read for a long while: a rebuild is cheaper than holding
    // every path written since.
    if p.len() > PENDING_CAP {
        p.clear();
        STALE.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The record for `path` as the store holds it now, or `None` when the path
/// is gone or names a blob that is.
fn read_rec(
    paths: &sled::Tree,
    blobs: &sled::Tree,
    metas: &sled::Tree,
    path: &str,
) -> Result<Option<NoteRec>, ApiError> {
    if !path.ends_with(".md") {
        return Ok(None);
    }
    let Some(from) = path
        .strip_prefix(PREFIX)
        .and_then(|r| r.split('/').next())
        .filter(|f| !f.is_empty())
    else {
        return Ok(None);
    };
    let Some(cid) = paths
        .get(path.as_bytes())
        .map_err(|e| internal("read memory_files", e))?
    else {
        return Ok(None);
    };
    let cid = String::from_utf8_lossy(&cid).into_owned();
    let Some(head) = inbox_note_head(blobs, &cid)? else {
        // The path names a blob that is gone; drop the dangling index entry,
        // as `read_memory_file` does.
        let _ = paths.remove(path.as_bytes());
        return Ok(None);
    };
    let meta = metas
        .get(cid.as_bytes())
        .ok()
        .flatten()
        .and_then(|b| ciborium::de::from_reader::<MemoryFileMeta, _>(&b[..]).ok());
    let (signed_at, signed_at_unix_s, has_sig, proven_key, attester_pubkey_b32) = match meta {
        Some(m) => {
            let proven = match (
                &m.attester_pubkey_b32,
                &m.attester_sig_b32,
                &m.attester_body_hash_hex,
            ) {
                (Some(pk), Some(_), Some(_)) => Some(pk.clone()),
                _ => None,
            };
            (
                m.signed_at,
                m.signed_at_unix_s,
                m.attester_sig_b32.is_some(),
                proven,
                m.attester_pubkey_b32,
            )
        }
        None => (
            emem_storage::server::iso8601_now(),
            now_unix_s_local(),
            false,
            None,
            None,
        ),
    };
    let skills = if path.ends_with("/agent-skills.md") {
        blobs
            .get(cid.as_bytes())
            .ok()
            .flatten()
            .and_then(|b| parse_skill_declaration(&String::from_utf8_lossy(&b)))
    } else {
        None
    };
    Ok(Some(NoteRec {
        path: path.to_string(),
        cid,
        from: from.to_string(),
        signed_at,
        signed_at_unix_s,
        has_sig,
        proven_key,
        attester_pubkey_b32,
        head,
        skills,
    }))
}

impl Ledger {
    fn insert(&mut self, rec: NoteRec) {
        self.remove(&rec.path.clone());
        self.cid_author.insert(rec.cid.clone(), rec.from.clone());
        self.notes.insert(rec.path.clone(), Arc::new(rec));
    }

    fn remove(&mut self, path: &str) {
        if let Some(old) = self.notes.remove(path) {
            if self.cid_author.get(&old.cid) == Some(&old.from) {
                self.cid_author.remove(&old.cid);
            }
        }
    }
}

/// Run `f` over a view of `s`'s ledger that has caught up with every write
/// event, rebuilding it first when it is missing, belongs to another store,
/// is past [`MAX_AGE`], or missed events. Blocking: call it on the blocking
/// pool, as the scans it replaces were.
pub(crate) fn with_ledger<T>(s: &AppState, f: impl FnOnce(&Ledger) -> T) -> Result<T, ApiError> {
    let db = memory_db(s)?;
    let paths = db
        .open_tree(emem_storage::TREE_MEMORY_FILES)
        .map_err(|e| internal("open memory_files", e))?;
    let blobs = db
        .open_tree(emem_storage::TREE_MEMORY_FILE_BLOBS)
        .map_err(|e| internal("open memory_file_blobs", e))?;
    let metas = db
        .open_tree(emem_storage::TREE_MEMORY_FILE_META)
        .map_err(|e| internal("open memory_file_meta", e))?;

    let mut guard = LEDGER.lock().unwrap_or_else(|e| e.into_inner());
    let owner = owner_of(s);
    let rebuild = match guard.as_ref() {
        None => true,
        Some(l) => {
            l.owner != owner
                || l.built_at.elapsed() > MAX_AGE
                || STALE.load(std::sync::atomic::Ordering::SeqCst)
        }
    };
    if rebuild {
        // Clear the flag and the queue first: anything written during the
        // walk lands in the queue again and is re-read below, and re-reading
        // a path only ever yields its current state.
        STALE.store(false, std::sync::atomic::Ordering::SeqCst);
        PENDING.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let mut l = Ledger {
            owner,
            built_at: Instant::now(),
            notes: HashMap::new(),
            cid_author: HashMap::new(),
        };
        for kv in paths.scan_prefix(PREFIX.as_bytes()).flatten() {
            let key = String::from_utf8_lossy(&kv.0).into_owned();
            if let Some(rec) = read_rec(&paths, &blobs, &metas, &key)? {
                l.insert(rec);
            }
        }
        *guard = Some(l);
    }
    let Some(l) = guard.as_mut() else {
        return Err(internal("ledger view", "missing after rebuild"));
    };
    let touched: Vec<String> =
        std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
    let mut seen = std::collections::HashSet::new();
    for path in touched {
        if !seen.insert(path.clone()) {
            continue;
        }
        match read_rec(&paths, &blobs, &metas, &path)? {
            Some(rec) => l.insert(rec),
            None => l.remove(&path),
        }
    }
    Ok(f(l))
}

/// Build the view for `state` at boot, so the first inbox call is not the one
/// that pays for the walk.
pub(crate) fn spawn_warm(state: &AppState) {
    // A unit test builds hundreds of routers; each builds its view on first read.
    if cfg!(test)
        || state.storage.hot_sled_db().is_none()
        || tokio::runtime::Handle::try_current().is_err()
    {
        return;
    }
    let s = state.clone();
    tokio::task::spawn_blocking(move || {
        let t0 = Instant::now();
        match with_ledger(&s, |l| l.notes.len()) {
            Ok(n) => tracing::info!(
                target: "emem::ledger",
                notes = n,
                elapsed_ms = t0.elapsed().as_millis() as u64,
                "ledger view built"
            ),
            Err(e) => tracing::warn!(
                target: "emem::ledger",
                error = %e.1.message,
                "ledger view did not build; the first reader builds it"
            ),
        }
    });
}
