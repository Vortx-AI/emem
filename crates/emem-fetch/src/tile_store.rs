//! Whole local copies of the static rasters the forest checks read.
//!
//! Hansen GFC, JRC GFC2020, the JRC TMF mirror and ESA WorldCover publish each
//! version once, under a name that carries the version. Reading them by HTTP
//! range paid the publisher's latency on every statement: over 24-27 October
//! 2026 JRC's tile host took 43 s on average to answer a header read and, from
//! 18:00 UTC on the 27th, answered nothing, while the same plots' Hansen reads
//! took 47 ms. The first range read of such a file now starts a download of
//! the whole file in the background; once it is on disk at its published
//! length, every read of that url is served from the copy. The bytes are the
//! publisher's, so a fact read from the copy is the fact read from the url,
//! and the url stays its source.
//!
//! Off until the server calls [`enable`]: a library user or a test run never
//! downloads a raster it did not ask for.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use reqwest::Client;
use tokio::io::AsyncWriteExt;

/// Published files whose name carries their version. Never a whole-world
/// mosaic: JRC's 41 GB single COG sits beside its tiles and is left out.
const STATIC_PREFIXES: &[&str] = &[
    "https://storage.googleapis.com/earthenginepartners-hansen/GFC-",
    "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/tiles/",
    "https://data.source.coop/epoch/jrc-tmf/",
    "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/",
];

/// A file larger than this is read by range, never copied.
const MAX_FILE: u64 = 8 << 30;
/// Space the disk keeps free after a copy lands, whatever the budget says.
const FREE_FLOOR: u64 = 30 << 30;
/// Copies downloading at once.
const CONCURRENT: usize = 2;
/// Downloads waiting for a slot, past which a new want is dropped.
const QUEUE_CAP: usize = 32;
/// A download that has sent nothing for this long is abandoned.
const STALL: Duration = Duration::from_secs(60);
/// A url whose copy failed is not tried again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(1800);
/// How often a copy in use has its mtime moved forward, which is the
/// last-use time eviction reads.
const TOUCH_EVERY: Duration = Duration::from_secs(6 * 3600);
/// How long a copy is served before its publisher is asked whether the file
/// changed under its name. The names carry versions, but `LATEST/` is a
/// directory a publisher can rewrite, and a copy is a promise that its bytes
/// are the publisher's.
const REVALIDATE_EVERY: Duration = Duration::from_secs(7 * 86_400);

struct Store {
    dir: PathBuf,
    budget: u64,
}

static STORE: OnceLock<Store> = OnceLock::new();

/// Urls with a complete copy on disk, and the copy's length.
static PRESENT: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);

enum Job {
    Running,
    Failed(Instant),
}

static JOBS: LazyLock<Mutex<HashMap<String, Job>>> = LazyLock::new(Default::default);

static SLOTS: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(CONCURRENT));

static TOUCHED: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

/// When this process last considered a copy for a check.
static CHECKED: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

/// Bytes the copies in flight will add to the disk. Each copy checks the
/// budget and the free floor against the others' too: two copies checked
/// alone could each fit and together not.
static RESERVED: Mutex<u64> = Mutex::new(0);

/// Gives back a copy's reservation however the copy ends.
struct Reservation(u64);

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut r = lock(&RESERVED);
        *r = r.saturating_sub(self.0);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The copy's identity: the url without its query string.
fn key_of(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

fn eligible(key: &str) -> bool {
    key.ends_with(".tif") && STATIC_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// `<file name>-<hash of the url>.tif`: readable on disk, and two urls that
/// end in the same file name do not share a copy.
fn file_for(dir: &Path, key: &str) -> PathBuf {
    let name: String = key
        .rsplit('/')
        .next()
        .unwrap_or("tile.tif")
        .trim_end_matches(".tif")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(96)
        .collect();
    let h = blake3::hash(key.as_bytes()).to_hex();
    dir.join(format!("{name}-{}.tif", &h[..16]))
}

fn sidecar_for(file: &Path) -> PathBuf {
    file.with_extension("json")
}

/// Turn the store on at `dir`, holding at most `budget_bytes` of copies.
/// Copies already in `dir` from an earlier run are served at once. A second
/// call is ignored.
pub fn enable(dir: PathBuf, budget_bytes: u64) -> std::io::Result<usize> {
    std::fs::create_dir_all(&dir)?;
    let mut found = 0;
    for e in std::fs::read_dir(&dir)?.flatten() {
        let p = e.path();
        match p.extension().and_then(|x| x.to_str()) {
            // An interrupted download.
            Some("part") => {
                let _ = std::fs::remove_file(&p);
            }
            Some("json") => {
                let Some(meta) = std::fs::read(&p)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                else {
                    continue;
                };
                let (Some(url), Some(bytes)) = (meta["url"].as_str(), meta["bytes"].as_u64())
                else {
                    continue;
                };
                let tif = p.with_extension("tif");
                if std::fs::metadata(&tif).is_ok_and(|m| m.len() == bytes) {
                    lock(&PRESENT).insert(url.to_string(), bytes);
                    found += 1;
                }
            }
            _ => {}
        }
    }
    let _ = STORE.set(Store {
        dir,
        budget: budget_bytes,
    });
    Ok(found)
}

/// Keep a small file beside the copies, such as a publisher's tile listing,
/// so a restart while the publisher is down still knows what it published.
/// Does nothing while the store is off. `name` must not end in `.tif`,
/// `.json` or `.part`, which are the copies' own.
pub async fn keep(name: &str, bytes: Vec<u8>) {
    let Some(st) = STORE.get() else {
        return;
    };
    let path = st.dir.join(name);
    let tmp = st.dir.join(format!("{name}.part"));
    if tokio::fs::write(&tmp, &bytes).await.is_ok() {
        let _ = tokio::fs::rename(&tmp, &path).await;
    }
}

/// A file [`keep`] kept.
pub async fn kept(name: &str) -> Option<Vec<u8>> {
    let st = STORE.get()?;
    tokio::fs::read(st.dir.join(name)).await.ok()
}

/// The local copy of `url`, when one is complete.
pub(crate) fn local_copy(url: &str) -> Option<PathBuf> {
    let st = STORE.get()?;
    let key = key_of(url);
    if !lock(&PRESENT).contains_key(key) {
        return None;
    }
    let file = file_for(&st.dir, key);
    touch(key, &file);
    revalidate_if_due(key, &file);
    Some(file)
}

/// Ask the publisher, in the background, whether a copy served for
/// [`REVALIDATE_EVERY`] still matches its file; drop it when it does not, so
/// the next read fetches the file again. A check that fails says nothing
/// and keeps the copy, and so does a 404: the copy is still the version that
/// was published.
fn revalidate_if_due(key: &str, file: &Path) {
    {
        let mut c = lock(&CHECKED);
        if c.get(key)
            .is_some_and(|at| at.elapsed() < Duration::from_secs(86_400))
        {
            return;
        }
        c.insert(key.to_string(), Instant::now());
    }
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let (key, side) = (key.to_string(), sidecar_for(file));
    tokio::spawn(async move {
        let Some(mut meta) = tokio::fs::read(&side)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        else {
            return;
        };
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let last = meta["checked_at_unix_s"]
            .as_u64()
            .or_else(|| meta["copied_at_unix_s"].as_u64())
            .unwrap_or(0);
        if now.saturating_sub(last) < REVALIDATE_EVERY.as_secs()
            || crate::cog::host_cooling_for(&key).is_some()
        {
            return;
        }
        let head = download_client()
            .head(&key)
            .header("user-agent", emem_core::outbound::user_agent())
            .send();
        let Ok(Ok(resp)) = tokio::time::timeout(Duration::from_secs(30), head).await else {
            return;
        };
        if !resp.status().is_success() {
            return;
        }
        let len = header(&resp, "content-length").and_then(|v| v.parse::<u64>().ok());
        if copy_changed(
            &meta,
            len,
            header(&resp, "etag").as_deref(),
            header(&resp, "last-modified").as_deref(),
        ) {
            tracing::warn!(target: "emem::tile_store", url = %key, "the publisher's file changed; the copy is dropped and fetched again");
            forget(&key);
            return;
        }
        meta["checked_at_unix_s"] = serde_json::json!(now);
        let _ = tokio::fs::write(&side, meta.to_string()).await;
    });
}

/// Whether a publisher's answer describes a different file than the copy
/// recorded: a length, or an ETag or Last-Modified that both sides state,
/// that differs.
fn copy_changed(
    recorded: &serde_json::Value,
    len: Option<u64>,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> bool {
    let differs =
        |now: Option<&str>, then: Option<&str>| matches!((now, then), (Some(a), Some(b)) if a != b);
    len.is_some_and(|n| recorded["bytes"].as_u64().is_some_and(|b| b != n))
        || differs(etag, recorded["etag"].as_str())
        || differs(last_modified, recorded["last_modified"].as_str())
}

/// Whether the copy of `url` is gone or no longer its recorded length. A
/// read can fail for reasons that are not the copy's (too many open files, a
/// range that starts past the end), and a verified copy must not be thrown
/// away and downloaded again for them.
pub(crate) fn copy_is_broken(url: &str) -> bool {
    let Some(st) = STORE.get() else {
        return false;
    };
    let key = key_of(url);
    let Some(bytes) = lock(&PRESENT).get(key).copied() else {
        return false;
    };
    match std::fs::metadata(file_for(&st.dir, key)) {
        Ok(m) => m.len() != bytes,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// A broken copy: stop serving it, and let the next read fetch it again.
pub(crate) fn forget(url: &str) {
    let key = key_of(url);
    lock(&PRESENT).remove(key);
    {
        let mut jobs = lock(&JOBS);
        if !matches!(jobs.get(key), Some(Job::Running)) {
            jobs.remove(key);
        }
    }
    if let Some(st) = STORE.get() {
        let file = file_for(&st.dir, key);
        let _ = std::fs::remove_file(sidecar_for(&file));
        let _ = std::fs::remove_file(file);
    }
}

fn touch(key: &str, file: &Path) {
    {
        let mut t = lock(&TOUCHED);
        if t.get(key).is_some_and(|at| at.elapsed() < TOUCH_EVERY) {
            return;
        }
        t.insert(key.to_string(), Instant::now());
    }
    let file = file.to_path_buf();
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::spawn_blocking(move || {
            if let Ok(f) = std::fs::File::options().append(true).open(&file) {
                let _ = f.set_modified(SystemTime::now());
            }
        });
    }
}

/// Start a background copy of `url` if it is a static raster this store
/// keeps and no copy is on disk, in flight or recently failed. Returns at
/// once; the read that asked goes to the network as before.
pub(crate) fn want(url: &str) {
    let Some(st) = STORE.get() else {
        return;
    };
    let key = key_of(url);
    if !eligible(key) || lock(&PRESENT).contains_key(key) {
        return;
    }
    // A host left alone after failing cannot serve a copy either; asking it
    // would only mark the copy failed for half an hour past its recovery.
    if crate::cog::host_cooling_for(key).is_some() {
        return;
    }
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    {
        let mut jobs = lock(&JOBS);
        match jobs.get(key) {
            Some(Job::Running) => return,
            Some(Job::Failed(at)) if at.elapsed() < RETRY_AFTER => return,
            _ => {}
        }
        if jobs.values().filter(|j| matches!(j, Job::Running)).count() >= QUEUE_CAP {
            return;
        }
        jobs.insert(key.to_string(), Job::Running);
    }
    let key = key.to_string();
    let dir = st.dir.clone();
    let budget = st.budget;
    tokio::spawn(async move {
        let Ok(_slot) = SLOTS.acquire().await else {
            return;
        };
        let t0 = Instant::now();
        match copy(&key, &dir, budget).await {
            Ok(bytes) => {
                lock(&PRESENT).insert(key.clone(), bytes);
                lock(&JOBS).remove(&key);
                tracing::info!(
                    target: "emem::tile_store",
                    url = %key,
                    mb = bytes >> 20,
                    elapsed_s = t0.elapsed().as_secs(),
                    "local copy ready; reads of this url are now served from disk"
                );
            }
            Err(CopyFailed::Later(why)) => {
                lock(&JOBS).remove(&key);
                tracing::info!(target: "emem::tile_store", url = %key, why = %why, "local copy put off");
            }
            Err(CopyFailed::Now(why)) => {
                lock(&JOBS).insert(key.clone(), Job::Failed(Instant::now()));
                tracing::warn!(
                    target: "emem::tile_store",
                    url = %key,
                    error = %why,
                    retry_after_s = RETRY_AFTER.as_secs(),
                    "local copy failed; reads stay on the network"
                );
            }
        }
    });
}

fn download_client() -> &'static Client {
    static C: OnceLock<Client> = OnceLock::new();
    C.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new())
    })
}

/// Why a copy did not land: `Later` is worth asking again on the next read
/// (the host is cooling), `Now` waits [`RETRY_AFTER`].
enum CopyFailed {
    Later(String),
    Now(String),
}

impl From<String> for CopyFailed {
    fn from(s: String) -> Self {
        CopyFailed::Now(s)
    }
}

impl From<&str> for CopyFailed {
    fn from(s: &str) -> Self {
        CopyFailed::Now(s.to_string())
    }
}

async fn copy(key: &str, dir: &Path, budget: u64) -> Result<u64, CopyFailed> {
    if let Some((left, why)) = crate::cog::host_cooling_for(key) {
        return Err(CopyFailed::Later(format!(
            "host is cooling for {left}s more: {why}"
        )));
    }
    // The connect timeout does not cover a host that accepts and never
    // answers, which is what JRC's did on 2026-10-27; without this a copy
    // slot waited on it for good.
    let mut resp = tokio::time::timeout(
        STALL,
        download_client()
            .get(key)
            .header("user-agent", emem_core::outbound::user_agent())
            .send(),
    )
    .await
    .map_err(|_| format!("no answer in {}s", STALL.as_secs()))?
    .map_err(|e| e.to_string())?;
    if resp.status() != reqwest::StatusCode::OK {
        return Err(format!("HTTP {}", resp.status()).into());
    }
    let len = resp
        .content_length()
        .ok_or("no content-length, so a complete copy could not be told from a cut one")?;
    if len > MAX_FILE || len > budget {
        return Err(format!(
            "{} MB is over the {} MB a copy may take",
            len >> 20,
            MAX_FILE.min(budget) >> 20
        )
        .into());
    }
    let etag = header(&resp, "etag");
    let last_modified = header(&resp, "last-modified");

    let dir_c = dir.to_path_buf();
    let _reservation = tokio::task::spawn_blocking(move || {
        // Checked and reserved under one lock, so two copies cannot both
        // see room only one of them has.
        let mut reserved = lock(&RESERVED);
        make_room(&dir_c, len, budget, *reserved)?;
        *reserved += len;
        Ok::<_, String>(Reservation(len))
    })
    .await
    .map_err(|e| e.to_string())??;

    // A part file of its own: two copies of one url (one started after a
    // broken copy was forgotten) must not write into the same file.
    static PARTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let file = file_for(dir, key);
    let part = file.with_extension(format!(
        "{}-{}.part",
        std::process::id(),
        PARTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut out = tokio::fs::File::create(&part)
        .await
        .map_err(|e| format!("create {}: {e}", part.display()))?;
    let mut written: u64 = 0;
    let mut head = [0u8; 4];
    let result: Result<(), String> = async {
        loop {
            let chunk = match tokio::time::timeout(STALL, resp.chunk()).await {
                Err(_) => return Err(format!("stalled for {}s", STALL.as_secs())),
                Ok(Err(e)) => return Err(format!("body: {e}")),
                Ok(Ok(None)) => break,
                Ok(Ok(Some(c))) => c,
            };
            for (i, b) in chunk.iter().take(4).enumerate() {
                if written as usize + i < 4 {
                    head[written as usize + i] = *b;
                }
            }
            written += chunk.len() as u64;
            if written > len {
                return Err(format!("sent more than its {len} bytes"));
            }
            out.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        out.sync_all().await.map_err(|e| e.to_string())?;
        if written != len {
            return Err(format!("sent {written} of {len} bytes"));
        }
        // Little- or big-endian TIFF, classic or BigTIFF.
        if !matches!(&head, b"II*\0" | b"MM\0*" | b"II+\0" | b"MM\0+") {
            return Err("not a TIFF".into());
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e.into());
    }
    let meta = serde_json::json!({
        "url": key,
        "bytes": len,
        "etag": etag,
        "last_modified": last_modified,
        "copied_at_unix_s": SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    tokio::fs::write(sidecar_for(&file), meta.to_string())
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(&part, &file)
        .await
        .map_err(|e| e.to_string())?;
    Ok(len)
}

fn header(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Evict the least recently used copies until `incoming` more bytes fit the
/// budget beside the `reserved` bytes of the copies in flight, then refuse
/// unless the disk keeps [`FREE_FLOOR`] free after all of them land.
fn make_room(dir: &Path, incoming: u64, budget: u64, reserved: u64) -> Result<(), String> {
    let mut copies: Vec<(SystemTime, u64, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|x| x.to_str()) == Some("tif")).then_some(())?;
            let m = e.metadata().ok()?;
            Some((m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), p))
        })
        .collect();
    let mut held: u64 = copies.iter().map(|c| c.1).sum::<u64>() + reserved;
    copies.sort_by_key(|c| c.0);
    let mut oldest = copies.into_iter();
    while held + incoming > budget {
        let Some((_, n, p)) = oldest.next() else {
            break;
        };
        if let Some(url) = std::fs::read(sidecar_for(&p))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|m| m["url"].as_str().map(str::to_string))
        {
            lock(&PRESENT).remove(&url);
        }
        let _ = std::fs::remove_file(sidecar_for(&p));
        let _ = std::fs::remove_file(&p);
        held = held.saturating_sub(n);
        tracing::info!(target: "emem::tile_store", file = %p.display(), "evicted the least recently used copy");
    }
    if held + incoming > budget {
        return Err("does not fit the store's budget".into());
    }
    let free = fs2::available_space(dir).map_err(|e| e.to_string())?;
    if free < incoming + reserved + FREE_FLOOR {
        return Err(format!(
            "{} GB free; a copy must leave {} GB",
            free >> 30,
            FREE_FLOOR >> 30
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_is_dropped_only_when_the_publisher_says_its_file_changed() {
        let rec = serde_json::json!({"bytes": 100, "etag": "\"a\"", "last_modified": "Mon"});
        assert!(!copy_changed(&rec, Some(100), Some("\"a\""), Some("Mon")));
        assert!(
            !copy_changed(&rec, None, None, None),
            "an answer that states nothing"
        );
        assert!(copy_changed(&rec, Some(101), Some("\"a\""), Some("Mon")));
        assert!(copy_changed(&rec, Some(100), Some("\"b\""), None));
        assert!(copy_changed(&rec, None, None, Some("Tue")));
        let bare = serde_json::json!({"bytes": 100});
        assert!(
            !copy_changed(&bare, Some(100), Some("\"z\""), Some("Wed")),
            "nothing recorded to differ from"
        );
    }

    #[test]
    fn only_versioned_tiles_are_kept() {
        assert!(eligible(
            "https://storage.googleapis.com/earthenginepartners-hansen/GFC-2025-v1.13/Hansen_GFC-2025-v1.13_lossyear_10N_010W.tif"
        ));
        assert!(eligible(
            "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/tiles/JRC_GFC2020_V4_N10_W10.tif"
        ));
        assert!(!eligible(
            "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/single-cog/JRC_GFC2020_V4_COG.tif"
        ));
        assert!(!eligible(
            "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/32/T/NS/2026/10/01/B04.tif"
        ));
    }

    #[test]
    fn a_copy_is_named_for_its_file_and_its_url() {
        let d = Path::new("/x");
        let a = file_for(d, "https://a/GFC-1/t_10N_010W.tif");
        let b = file_for(d, "https://b/GFC-1/t_10N_010W.tif");
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("t_10N_010W-") && name.ends_with(".tif"));
        assert_eq!(key_of("https://a/x.tif?sig=1"), "https://a/x.tif");
    }

    /// The only test that turns the store on: [`enable`] holds for the whole
    /// test binary. The budget is too small for any real raster, so a read
    /// elsewhere in the binary that would start a copy is refused before a
    /// byte of body is written.
    #[tokio::test]
    async fn a_local_copy_answers_its_url_without_the_network() {
        let dir =
            std::env::temp_dir().join(format!("emem-tile-store-serve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A host that cannot answer: a read that reached the network fails.
        let url =
            "https://storage.googleapis.com/earthenginepartners-hansen/GFC-test/t_00N_000E.tif";
        let body: Vec<u8> = b"II*\0"
            .iter()
            .copied()
            .chain((0u8..=255).cycle().take(4092))
            .collect();
        let file = file_for(&dir, url);
        std::fs::write(&file, &body).unwrap();
        std::fs::write(
            sidecar_for(&file),
            serde_json::json!({"url": url, "bytes": body.len()}).to_string(),
        )
        .unwrap();
        std::fs::write(dir.join("cut-0.part"), b"II").unwrap();
        assert_eq!(enable(dir.clone(), 1 << 20).unwrap(), 1);
        assert!(
            !dir.join("cut-0.part").exists(),
            "an interrupted copy is cleared"
        );
        let got = crate::cog::http_range(&Client::new(), &format!("{url}?sig=rotates"), 4, 11)
            .await
            .unwrap();
        assert_eq!(&got[..], &body[4..12]);
        // A range past the end is cut there, as HTTP answers it, and the
        // copy stays.
        let tail = crate::cog::http_range(&Client::new(), url, 4000, 4999)
            .await
            .unwrap();
        assert_eq!(&tail[..], &body[4000..]);
        assert!(local_copy(url).is_some() && !copy_is_broken(url));
        // Cut short on disk, it is broken.
        std::fs::write(&file, &body[..100]).unwrap();
        assert!(copy_is_broken(url));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn eviction_takes_the_least_recently_used_copy_first() {
        let dir = std::env::temp_dir().join(format!("emem-tile-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old-0.tif");
        let new = dir.join("new-0.tif");
        std::fs::write(&old, vec![0u8; 1000]).unwrap();
        std::fs::write(&new, vec![0u8; 1000]).unwrap();
        std::fs::File::options()
            .append(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        // Room for 1500 more with a 2500-byte budget: one of the two must go.
        let r = make_room(&dir, 1500, 2500, 0);
        assert!(!old.exists(), "the older copy is evicted");
        assert!(new.exists(), "the newer copy stays");
        // Whether the floor passes depends on this disk; the eviction is what
        // is under test.
        let _ = r;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
