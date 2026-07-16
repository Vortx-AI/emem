//! Content-addressed artifact store for field responses.
//!
//! docs/plans/field-tokens.md, section 5: a field's pixel bytes live
//! here, keyed by `artifact_cid = base32_nopad_lower(blake3(bytes))`,
//! the full 32-byte digest (52 characters, the `fact_cid` convention).
//! Blobs are EVICTABLE by design: the signed derivation record persists
//! like any fact and carries everything needed to rebuild the identical
//! bytes, so evicting an artifact never breaks a citation, it only turns
//! a dereference into a recompute. That is the cell-build-graph promise
//! ("derived layers are evictable without breaking citations") applied
//! to the first artifact-typed value.
//!
//! Mechanics, deliberately boring:
//!   * layout `<root>/<cid[..2]>/<cid>`, two-hex-ish fanout so one
//!     directory never holds the whole corpus;
//!   * `put` writes to a temp file in the same directory and renames,
//!     so a crash never leaves a half blob under a valid cid;
//!   * `get` and a repeated `put` touch the blob's mtime, and eviction
//!     removes oldest-mtime blobs until the store fits `max_bytes`:
//!     LRU without a database, from metadata the filesystem already
//!     keeps;
//!   * every cid is validated against the 52-character base32 alphabet
//!     before it touches a path, so a token cannot traverse.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One content-addressed blob directory with a byte cap.
#[derive(Debug, Clone)]
pub struct ArtifactStore {
    root: PathBuf,
    max_bytes: u64,
}

/// 32-byte blake3, base32-nopad-lowercase: always 52 characters.
const CID_LEN: usize = 52;

fn cid_is_valid(cid: &str) -> bool {
    cid.len() == CID_LEN
        && cid
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

fn cid_of(bytes: &[u8]) -> String {
    data_encoding::BASE32_NOPAD
        .encode(blake3::hash(bytes).as_bytes())
        .to_lowercase()
}

impl ArtifactStore {
    /// Open (creating the root if needed). `max_bytes` caps the total
    /// blob size; eviction runs after every `put`.
    pub fn open(root: impl Into<PathBuf>, max_bytes: u64) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root, max_bytes })
    }

    fn blob_path(&self, cid: &str) -> PathBuf {
        self.root.join(&cid[..2]).join(cid)
    }

    /// Store bytes, return their cid. Idempotent: an existing blob is
    /// touched (freshened for LRU) and its cid returned without a write.
    pub fn put(&self, bytes: &[u8]) -> io::Result<String> {
        let cid = cid_of(bytes);
        let path = self.blob_path(&cid);
        if path.exists() {
            touch(&path);
            return Ok(cid);
        }
        let dir = path.parent().expect("blob path always has a parent");
        fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".tmp-{cid}"));
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &path)?;
        self.evict_to_cap()?;
        Ok(cid)
    }

    /// Fetch a blob by cid. `Ok(None)` when absent or evicted; an
    /// invalid cid is refused before any path is formed.
    pub fn get(&self, cid: &str) -> io::Result<Option<Vec<u8>>> {
        if !cid_is_valid(cid) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "not an artifact cid: {} chars, want {CID_LEN} base32",
                    cid.len()
                ),
            ));
        }
        let path = self.blob_path(cid);
        match fs::read(&path) {
            Ok(bytes) => {
                touch(&path);
                Ok(Some(bytes))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Total stored bytes right now.
    pub fn total_bytes(&self) -> io::Result<u64> {
        Ok(self.walk()?.into_iter().map(|(_, len, _)| len).sum())
    }

    fn walk(&self) -> io::Result<Vec<(PathBuf, u64, SystemTime)>> {
        let mut out = Vec::new();
        for fan in fs::read_dir(&self.root)? {
            let fan = fan?;
            if !fan.file_type()?.is_dir() {
                continue;
            }
            for blob in fs::read_dir(fan.path())? {
                let blob = blob?;
                let meta = blob.metadata()?;
                if meta.is_file() {
                    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    out.push((blob.path(), meta.len(), mtime));
                }
            }
        }
        Ok(out)
    }

    /// Remove oldest-mtime blobs until the store fits its cap. Never
    /// removes the newest blob, so a single artifact larger than the
    /// cap still serves (the cap bounds the tail, not the head).
    fn evict_to_cap(&self) -> io::Result<()> {
        let mut blobs = self.walk()?;
        let mut total: u64 = blobs.iter().map(|(_, len, _)| len).sum();
        if total <= self.max_bytes {
            return Ok(());
        }
        blobs.sort_by_key(|(_, _, mtime)| *mtime);
        let keep_newest = blobs.len().saturating_sub(1);
        for (path, len, _) in blobs.into_iter().take(keep_newest) {
            if total <= self.max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
        Ok(())
    }
}

fn touch(path: &Path) {
    // Freshen mtime for LRU. Best-effort: a failed touch only ages the
    // blob early, it never corrupts anything.
    if let Ok(f) = fs::File::options().append(true).open(path) {
        let _ = f.set_times(fs::FileTimes::new().set_modified(SystemTime::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(max: u64) -> (ArtifactStore, PathBuf) {
        // Unique per call: two tests sharing a pid+cap keyed directory
        // raced each other's cleanup and one saw NotFound mid-put.
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "emem-artifacts-test-{}-{max}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        (ArtifactStore::open(&dir, max).unwrap(), dir)
    }

    #[test]
    fn put_get_roundtrip_and_cid_shape() {
        let (s, dir) = store(1 << 20);
        let cid = s.put(b"field bytes").unwrap();
        assert_eq!(cid.len(), CID_LEN);
        assert!(cid_is_valid(&cid));
        assert_eq!(s.get(&cid).unwrap().unwrap(), b"field bytes");
        // Same bytes, same cid: content addressing, not a counter.
        assert_eq!(s.put(b"field bytes").unwrap(), cid);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_cids_are_refused_before_any_path_forms() {
        let (s, dir) = store(1 << 20);
        assert!(s.get("../../etc/passwd").is_err());
        assert!(s.get("SHOUTING").is_err());
        assert!(s.get("short").is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn eviction_removes_oldest_first_and_never_the_newest() {
        let (s, dir) = store(8);
        let a = s.put(b"aaaa").unwrap();
        // Age blob a explicitly so the LRU order does not depend on clock
        // resolution.
        let old = SystemTime::now() - std::time::Duration::from_secs(3600);
        let f = fs::File::options()
            .append(true)
            .open(s.blob_path(&a))
            .unwrap();
        f.set_times(fs::FileTimes::new().set_modified(old)).unwrap();
        let b = s.put(b"bbbb").unwrap();
        // cap 8 holds both (4+4). A third 4-byte blob overflows: the
        // oldest (a) must go, the newest must stay.
        let c = s.put(b"cccc").unwrap();
        assert!(s.get(&a).unwrap().is_none(), "oldest blob must be evicted");
        assert!(s.get(&c).unwrap().is_some(), "newest blob must survive");
        let _ = b;
        let _ = fs::remove_dir_all(dir);
    }
}
