//! A segment walk that checks every frame and resyncs past a tear.
//!
//! The tree's walk (`AttestationLog::leaf_hashes_from`) is length-driven and
//! never hashes a record. At a torn frame it takes whatever 32 bytes the torn
//! length points at as a leaf, and it usually stops reading the segment soon
//! after. That walk must not change: every signed head commits to what it
//! read, and a walk that resyncs would put leaves into the middle of the
//! tree. This walk is for readers that must see what is on disk, the log
//! repair and the orphan pass. It checks each frame (`blake3(cbor)` equals
//! the stored hash). At a frame that does not check, it looks forward for
//! the next `[u32 n][CBOR map][blake3]` frame that does. It also replays the
//! tree's walk from the first tear, so a caller can say which entries are
//! leaves and which leaves the tree took from a torn frame.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::os::unix::fs::FileExt;
use std::path::Path;

/// No entry this responder writes comes near this, so a longer declared
/// length is a torn one.
pub(crate) const MAX_ENTRY: u64 = 64 << 20;
/// How far past a tear the walk looks for the next frame that checks.
const RESYNC_WINDOW: u64 = 64 << 20;
const SCAN_CHUNK: u64 = 1 << 20;
/// The smallest frame: a 4-byte length, a 1-byte map and a 32-byte hash.
const MIN_FRAME: u64 = 37;

/// Segment indices on disk, ascending.
pub(crate) fn segment_indices(root: &Path) -> std::io::Result<Vec<u64>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root)? {
        if let Some(n) = entry?
            .file_name()
            .to_str()
            .and_then(|n| n.strip_prefix("merkle.log."))
            .and_then(|r| r.parse::<u64>().ok())
        {
            out.push(n);
        }
    }
    out.sort_unstable();
    Ok(out)
}

/// A frame the tree's walk took at or after the first tear of a segment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TreeFrame {
    pub offset: u64,
    pub cbor_len: u64,
    /// The 32 bytes the tree committed as this leaf.
    pub leaf: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TearKind {
    /// The declared frame fits in the file, and its stored hash is not the
    /// hash of its bytes.
    BadHash,
    /// The declared frame runs past the end of the file.
    PastEnd,
    /// The declared length is longer than any entry.
    TooLong,
    /// Fewer than 4 bytes are left, so there is no length.
    Short,
}

impl TearKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TearKind::BadHash => "bad_hash",
            TearKind::PastEnd => "runs_past_end",
            TearKind::TooLong => "too_long",
            TearKind::Short => "short",
        }
    }
}

/// A place where a frame does not check.
#[derive(Debug, Clone)]
pub(crate) struct Tear {
    pub offset: u64,
    /// What its first 4 bytes declare; `None` with fewer than 4 bytes left.
    pub declared_len: Option<u64>,
    pub kind: TearKind,
    /// Where the next frame that checks starts; `None` when none does.
    pub resume: Option<u64>,
    /// The tree's walk took a frame here: its ordinal in the segment.
    pub tree_ordinal: Option<u64>,
    /// The tree's walk of this segment ends here.
    pub tree_stops_here: bool,
}

pub(crate) enum Step<'a> {
    /// A frame whose stored hash is the hash of its bytes.
    Entry {
        offset: u64,
        cbor: &'a [u8],
        hash: [u8; 32],
        /// Its ordinal among the tree's leaves of this segment, when the
        /// tree's walk took this very frame.
        tree_ordinal: Option<u64>,
    },
    /// A frame that does not check. `partial` is the part of the torn
    /// record that is on disk: from its CBOR start to the resume point or
    /// the declared end, whichever comes first.
    Tear { tear: Tear, partial: &'a [u8] },
}

pub(crate) struct SegmentWalk {
    file: File,
    rd: BufReader<File>,
    len: u64,
    pos: u64,
    buf: Vec<u8>,
    partial: Vec<u8>,
    entries: u64,
    /// Entries before the first tear. The tree took exactly these first.
    before_tear: Option<u64>,
    /// The tree's frames from the first tear on, ascending by offset.
    tree_tail: Vec<TreeFrame>,
    tail_matched: Vec<bool>,
    tree_end: u64,
    trailer: bool,
}

impl SegmentWalk {
    /// Walk `path` up to its length now; bytes appended later are not read.
    pub(crate) fn open(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        let rd = BufReader::with_capacity(1 << 20, file.try_clone()?);
        Ok(Self {
            file,
            rd,
            len,
            pos: 0,
            buf: Vec::new(),
            partial: Vec::new(),
            entries: 0,
            before_tear: None,
            tree_tail: Vec::new(),
            tail_matched: Vec::new(),
            tree_end: len,
            trailer: false,
        })
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    /// The segment ended with 32 bytes after the last frame: a rotation's
    /// segment hash. It is not checked here; `AttestationLog::verify` does.
    pub(crate) fn has_trailer(&self) -> bool {
        self.trailer
    }

    pub(crate) fn next_step(&mut self) -> std::io::Result<Option<Step<'_>>> {
        let rest = self.len - self.pos;
        if rest == 0 {
            return Ok(None);
        }
        if rest == 32 {
            self.trailer = true;
            if self.before_tear.is_none() {
                self.tree_end = self.pos;
            }
            self.pos = self.len;
            return Ok(None);
        }
        let at = self.pos;
        if rest < 4 {
            return self.tear(at, None, TearKind::Short);
        }
        let mut lb = [0u8; 4];
        self.rd.read_exact(&mut lb)?;
        let n = u64::from(u32::from_le_bytes(lb));
        if n > MAX_ENTRY {
            return self.tear(at, Some(n), TearKind::TooLong);
        }
        if 4 + n + 32 > rest {
            return self.tear(at, Some(n), TearKind::PastEnd);
        }
        self.buf.resize(n as usize + 32, 0);
        self.rd.read_exact(&mut self.buf)?;
        let (cbor, stored) = self.buf.split_at(n as usize);
        if blake3::hash(cbor).as_bytes() != stored {
            return self.tear(at, Some(n), TearKind::BadHash);
        }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(stored);
        self.pos = at + 4 + n + 32;
        let tree_ordinal = match self.before_tear {
            None => Some(self.entries),
            Some(b) => self.tail_ordinal(at, n).map(|j| {
                self.tail_matched[j] = true;
                b + j as u64
            }),
        };
        self.entries += 1;
        Ok(Some(Step::Entry {
            offset: at,
            cbor: &self.buf[..n as usize],
            hash,
            tree_ordinal,
        }))
    }

    fn tail_ordinal(&self, offset: u64, cbor_len: u64) -> Option<usize> {
        let j = self
            .tree_tail
            .binary_search_by_key(&offset, |f| f.offset)
            .ok()?;
        (self.tree_tail[j].cbor_len == cbor_len).then_some(j)
    }

    fn tear(
        &mut self,
        at: u64,
        declared_len: Option<u64>,
        kind: TearKind,
    ) -> std::io::Result<Option<Step<'_>>> {
        if self.before_tear.is_none() {
            self.before_tear = Some(self.entries);
            self.replay_tree_from(at)?;
        }
        let resume = self.find_frame(at + 1)?;
        let end = resume.unwrap_or(self.len);
        let from = (at + 4).min(end);
        let to = declared_len
            .map_or(end, |n| (at + 4).saturating_add(n).min(end))
            .min(from + MAX_ENTRY);
        self.partial.resize((to.max(from) - from) as usize, 0);
        self.file.read_exact_at(&mut self.partial, from)?;
        let tree_ordinal = self.tail_ordinal_any(at);
        let tear = Tear {
            offset: at,
            declared_len,
            kind,
            resume,
            tree_ordinal: tree_ordinal.map(|j| self.before_tear.unwrap_or(0) + j as u64),
            tree_stops_here: tree_ordinal.is_none() && self.tree_end == at,
        };
        match resume {
            Some(q) => {
                self.rd.seek(SeekFrom::Start(q))?;
                self.pos = q;
            }
            None => self.pos = self.len,
        }
        Ok(Some(Step::Tear {
            tear,
            partial: &self.partial,
        }))
    }

    fn tail_ordinal_any(&self, offset: u64) -> Option<usize> {
        self.tree_tail
            .binary_search_by_key(&offset, |f| f.offset)
            .ok()
    }

    /// The tree's own walk from `from`: read a length, take the 32 bytes
    /// after the declared CBOR as the leaf, move on, and stop when a frame
    /// runs past the end. The same steps as `leaf_hashes_from`.
    fn replay_tree_from(&mut self, from: u64) -> std::io::Result<()> {
        let mut i = from;
        while i + 4 <= self.len {
            let mut lb = [0u8; 4];
            self.file.read_exact_at(&mut lb, i)?;
            let n = u64::from(u32::from_le_bytes(lb));
            let needed = 4 + n + 32;
            if i + needed > self.len {
                break;
            }
            let mut leaf = [0u8; 32];
            self.file.read_exact_at(&mut leaf, i + 4 + n)?;
            self.tree_tail.push(TreeFrame {
                offset: i,
                cbor_len: n,
                leaf,
            });
            i += needed;
        }
        self.tree_end = i;
        self.tail_matched = vec![false; self.tree_tail.len()];
        Ok(())
    }

    /// Whether a `[u32 n][CBOR map][blake3]` frame that checks starts at `p`.
    fn frame_at(&self, p: u64, scratch: &mut Vec<u8>) -> std::io::Result<bool> {
        if p + MIN_FRAME > self.len {
            return Ok(false);
        }
        let mut head = [0u8; 5];
        self.file.read_exact_at(&mut head, p)?;
        let n = u64::from(u32::from_le_bytes([head[0], head[1], head[2], head[3]]));
        if n == 0 || n > MAX_ENTRY || p + 4 + n + 32 > self.len || !(0xa0..=0xbb).contains(&head[4])
        {
            return Ok(false);
        }
        scratch.resize(n as usize + 32, 0);
        self.file.read_exact_at(scratch, p + 4)?;
        let (cbor, stored) = scratch.split_at(n as usize);
        Ok(blake3::hash(cbor).as_bytes() == stored)
    }

    /// The first offset at or after `from` where a frame checks, looking at
    /// most [`RESYNC_WINDOW`] bytes ahead.
    fn find_frame(&self, from: u64) -> std::io::Result<Option<u64>> {
        let stop = self.len.min(from.saturating_add(RESYNC_WINDOW));
        let mut scratch = Vec::new();
        let mut q = from;
        while q < stop && q + MIN_FRAME <= self.len {
            let win_end = (q + SCAN_CHUNK + 5).min(self.len);
            let mut win = vec![0u8; (win_end - q) as usize];
            self.file.read_exact_at(&mut win, q)?;
            let last = win.len().saturating_sub(5);
            for k in 0..=last {
                let p = q + k as u64;
                if p >= stop || p + MIN_FRAME > self.len {
                    return Ok(None);
                }
                if !(0xa0..=0xbb).contains(&win[k + 4]) {
                    continue;
                }
                let n = u64::from(u32::from_le_bytes([
                    win[k],
                    win[k + 1],
                    win[k + 2],
                    win[k + 3],
                ]));
                if n == 0 || n > MAX_ENTRY || p + 4 + n + 32 > self.len {
                    continue;
                }
                if self.frame_at(p, &mut scratch)? {
                    return Ok(Some(p));
                }
            }
            q += last as u64 + 1;
        }
        Ok(None)
    }

    /// Bytes walked so far, for a read-rate cap.
    pub(crate) fn pos(&self) -> u64 {
        self.pos
    }

    /// How many leaves the tree takes from this segment. Valid after the walk.
    pub(crate) fn tree_leaves(&self) -> u64 {
        match self.before_tear {
            Some(b) => b + self.tree_tail.len() as u64,
            None => self.entries,
        }
    }

    /// The tree's frames that are not a frame that checks, with their
    /// ordinal in the segment: each is a leaf that is not the hash of an
    /// entry. Valid after the walk.
    pub(crate) fn misframed(&self) -> Vec<(u64, TreeFrame)> {
        let base = self.before_tear.unwrap_or(0);
        let mut scratch = Vec::new();
        self.tree_tail
            .iter()
            .zip(&self.tail_matched)
            .enumerate()
            .filter(|(_, (f, matched))| {
                !**matched && !self.frame_at(f.offset, &mut scratch).unwrap_or(false)
            })
            .map(|(j, (f, _))| (base + j as u64, *f))
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn frame(cbor: &[u8]) -> Vec<u8> {
        let mut v = (cbor.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(cbor);
        v.extend_from_slice(blake3::hash(cbor).as_bytes());
        v
    }

    /// A CBOR map of about `pad` bytes.
    pub(crate) fn map_entry(i: u64, pad: usize) -> Vec<u8> {
        let mut buf = Vec::new();
        let v = serde_json::json!({"kind": "test.entry", "i": i, "pad": "y".repeat(pad)});
        ciborium::ser::into_writer(&v, &mut buf).unwrap();
        buf
    }

    /// The way the production tears were made: a write stopped at a 4 KiB
    /// boundary, nothing cut it back, and the next records were appended
    /// from that boundary. The cut record is longer than the room left, so
    /// it is always cut. Returns the segment, its intact entries in order,
    /// and the offset of the tear.
    pub(crate) fn torn_segment(
        before: usize,
        after: usize,
        base: u64,
    ) -> (Vec<u8>, Vec<Vec<u8>>, u64) {
        let mut seg = Vec::new();
        let mut entries = Vec::new();
        for i in 0..before {
            let e = map_entry(base + i as u64, 900);
            seg.extend(frame(&e));
            entries.push(e);
        }
        let tear_at = seg.len() as u64;
        let cut = frame(&map_entry(999, 5000));
        let block = (seg.len() / 4096 + 1) * 4096;
        seg.extend_from_slice(&cut[..block - seg.len()]);
        for i in 0..after {
            let e = map_entry(base + 100 + i as u64, 900);
            seg.extend(frame(&e));
            entries.push(e);
        }
        (seg, entries, tear_at)
    }

    type Walked = (Vec<(u64, [u8; 32], Option<u64>)>, Vec<Tear>, SegmentWalk);

    fn walk_all(path: &Path) -> Walked {
        let mut w = SegmentWalk::open(path).unwrap();
        let (mut entries, mut tears) = (Vec::new(), Vec::new());
        while let Some(s) = w.next_step().unwrap() {
            match s {
                Step::Entry {
                    offset,
                    hash,
                    tree_ordinal,
                    ..
                } => entries.push((offset, hash, tree_ordinal)),
                Step::Tear { tear, .. } => tears.push(tear),
            }
        }
        (entries, tears, w)
    }

    /// Past a cut frame the walk finds every record, says which ones the
    /// tree holds, and names the leaf the tree took from the torn frame.
    /// With ten records after the tear, the torn length fits in the file and
    /// the tree takes a wrong leaf; with one, the tree stops at the tear.
    #[test]
    fn the_walk_resyncs_past_a_cut_frame_and_replays_the_tree() {
        for (before, after, misframes) in [(3, 10, true), (2, 1, false)] {
            let tmp = tempfile::tempdir().unwrap();
            let (seg, entries, tear_at) = torn_segment(before, after, 0);
            let path = tmp.path().join("merkle.log.0");
            std::fs::write(&path, &seg).unwrap();
            let log = crate::AttestationLog::open(tmp.path()).unwrap();
            let tree = log.leaf_hashes().unwrap();

            let (got, tears, w) = walk_all(&path);
            assert_eq!(got.len(), entries.len(), "every record is found");
            for ((_, h, _), e) in got.iter().zip(&entries) {
                assert_eq!(h, blake3::hash(e).as_bytes());
            }
            assert_eq!(tears.len(), 1);
            let t = &tears[0];
            assert_eq!(t.offset, tear_at);
            assert_eq!(t.resume, Some(4096));
            assert_eq!(w.tree_leaves(), tree.len() as u64);
            for (i, (_, h, ord)) in got.iter().enumerate() {
                match ord {
                    Some(k) => assert_eq!(&tree[*k as usize], h, "entry {i}"),
                    None => assert!(i >= before, "only records after the tear"),
                }
            }
            let wrong = w.misframed();
            for (ord, f) in &wrong {
                assert_eq!(tree[*ord as usize], f.leaf);
                assert!(!entries
                    .iter()
                    .any(|e| blake3::hash(e).as_bytes() == &f.leaf));
            }
            if misframes {
                assert_eq!(t.kind, TearKind::BadHash);
                assert_eq!(t.tree_ordinal, Some(before as u64));
                assert_eq!(wrong.len(), 1);
                assert_eq!(wrong[0].0, before as u64, "the tree took the torn frame");
            } else {
                assert_eq!(t.kind, TearKind::PastEnd);
                assert!(t.tree_stops_here);
                assert!(wrong.is_empty());
                assert_eq!(tree.len(), before);
            }
        }
    }

    /// A sealed segment's trailer is not a tear, and a clean segment's
    /// entries are all leaves in order.
    #[test]
    fn a_clean_segment_with_a_trailer_walks_like_the_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let entries: Vec<Vec<u8>> = (0..4).map(|i| map_entry(i, 200)).collect();
        let mut seg: Vec<u8> = entries.iter().flat_map(|e| frame(e)).collect();
        let trailer = *blake3::hash(&seg).as_bytes();
        seg.extend_from_slice(&trailer);
        let path = tmp.path().join("merkle.log.0");
        std::fs::write(&path, &seg).unwrap();
        let (got, tears, w) = walk_all(&path);
        assert!(tears.is_empty());
        assert!(w.has_trailer());
        assert_eq!(w.tree_leaves(), 4);
        for (k, (_, h, ord)) in got.iter().enumerate() {
            assert_eq!(*ord, Some(k as u64));
            assert_eq!(h, blake3::hash(&entries[k]).as_bytes());
        }
    }
}
