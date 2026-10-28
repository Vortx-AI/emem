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
    /// The store this view describes. A `Weak`, not the address: a test
    /// binary drops one store and allocates the next, and an allocator that
    /// hands the next store the freed address (macOS does, readily) made a
    /// view of the dropped store pass for the new one's, broadcasts and all.
    /// A live `Weak` keeps the allocation, so no other store can share it.
    owner: std::sync::Weak<emem_storage::Server>,
    built_at: Instant,
    pub(crate) notes: HashMap<String, Arc<NoteRec>>,
    /// Every path under the prefix, `.md` or not, and the file_cid it holds:
    /// a reply threads to any note it names, as the walk's map did.
    cids: HashMap<String, String>,
    /// file_cid -> the paths that hold it.
    by_cid: HashMap<String, std::collections::BTreeSet<String>>,
}

static LEDGER: Mutex<Option<Ledger>> = Mutex::new(None);

/// Paths the write events named since the view last caught up.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Set when the queue overflowed: the view cannot know what it dropped.
static STALE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    note_paths(touched);
}

/// Queue `paths` for the next reader to re-read.
pub(crate) fn note_paths<'a>(paths: impl IntoIterator<Item = &'a str>) {
    let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    p.extend(
        paths
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

/// Queues its paths when dropped. A write publishes its event after its last
/// await, so a write cancelled at an await, or returning early with `?`,
/// after it changed the path index left the view without it until a
/// rebuild. Made right after the index changes, this reaches the view
/// either way; on a normal finish it only repeats a path the event queued.
pub(crate) struct Requeue(pub(crate) Vec<String>);

impl Drop for Requeue {
    fn drop(&mut self) {
        note_paths(self.0.iter().map(String::as_str));
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
    fn empty(owner: &AppState) -> Self {
        Ledger {
            owner: Arc::downgrade(owner),
            built_at: Instant::now(),
            notes: HashMap::new(),
            cids: HashMap::new(),
            by_cid: HashMap::new(),
        }
    }

    /// Who wrote the note with this file_cid: the namespace owner of the
    /// last path in key order that holds it, as the walk's map answered when
    /// two paths hold the same bytes. One delete no longer forgets a cid
    /// another path still holds.
    pub(crate) fn author_of_cid(&self, cid: &str) -> Option<&str> {
        let path = self.by_cid.get(cid)?.iter().next_back()?;
        path.strip_prefix(PREFIX)?.split('/').next()
    }

    /// Record which file_cid `path` holds now, `None` when it is gone.
    fn index_path(&mut self, path: &str, cid: Option<String>) {
        if let Some(old) = self.cids.remove(path) {
            if let Some(set) = self.by_cid.get_mut(&old) {
                set.remove(path);
                if set.is_empty() {
                    self.by_cid.remove(&old);
                }
            }
        }
        if let Some(cid) = cid {
            self.by_cid
                .entry(cid.clone())
                .or_default()
                .insert(path.to_string());
            self.cids.insert(path.to_string(), cid);
        }
    }

    /// Bring `path` up to what the store holds now.
    fn refresh(
        &mut self,
        paths: &sled::Tree,
        blobs: &sled::Tree,
        metas: &sled::Tree,
        path: &str,
    ) -> Result<(), ApiError> {
        match read_rec(paths, blobs, metas, path)? {
            Some(rec) => {
                self.index_path(path, Some(rec.cid.clone()));
                self.notes.insert(rec.path.clone(), Arc::new(rec));
            }
            None => {
                self.notes.remove(path);
                let cid = paths
                    .get(path.as_bytes())
                    .map_err(|e| internal("read memory_files", e))?
                    .map(|c| String::from_utf8_lossy(&c).into_owned());
                self.index_path(path, cid);
            }
        }
        Ok(())
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
    let rebuild = match guard.as_ref() {
        None => true,
        Some(l) => {
            !std::ptr::eq(l.owner.as_ptr(), Arc::as_ptr(s))
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
        // The old view goes first: a walk that fails part way must leave no
        // view to serve, or the next reader would take the old one as current
        // with the queue it was owed already cleared.
        *guard = None;
        let mut l = Ledger::empty(s);
        for kv in paths.scan_prefix(PREFIX.as_bytes()).flatten() {
            let key = String::from_utf8_lossy(&kv.0).into_owned();
            l.index_path(&key, Some(String::from_utf8_lossy(&kv.1).into_owned()));
            if let Some(rec) = read_rec(&paths, &blobs, &metas, &key)? {
                l.notes.insert(rec.path.clone(), Arc::new(rec));
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
        if let Err(e) = l.refresh(&paths, &blobs, &metas, &path) {
            // The rest of the drained queue is lost with this read: the next
            // reader rebuilds rather than trust a view that missed it.
            STALE.store(true, std::sync::atomic::Ordering::SeqCst);
            return Err(e);
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
