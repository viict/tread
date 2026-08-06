//! On-demand byte access to a CSV file (SPEC.md §CSV).
//!
//! The point of CSV support is files too big to load, so this module never
//! holds file *contents*: it owns an open [`File`] and one sliding read window,
//! and every row is fetched by seeking to its byte offset and reading to the
//! next one. There is no memmap (no crate, and `unsafe` is confined to
//! `src/sys/`), so everything here is `seek` + `read` through `std::fs`.
//!
//! Buffering is deliberate. A sequential scroll must not cost one syscall per
//! row: reads are served out of a [`WINDOW`]-sized window over the region last
//! touched, so scrolling a screenful of short rows is one `read(2)`, and the
//! row scan in [`super::index`] walks the file a window at a time. Row bodies
//! larger than the window are read directly and never enter the window, so one
//! 50MB cell cannot evict the region the viewport is sitting on.
//!
//! Nothing here panics on a hostile or moving file: a truncated file reads
//! short, a grown file is picked up by [`Reader::refresh_size`], an offset past
//! the end yields an empty span, and a row longer than [`MAX_ROW_BYTES`] is
//! truncated with the flag set rather than allocated in full.
#![deny(unsafe_code)]

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Size of the sliding read window, and the largest single buffered read.
///
/// 256 KiB holds a few thousand typical CSV rows, which is many screenfuls, so
/// a page of scrolling is normally zero syscalls and a long scroll is one
/// syscall per 256 KiB rather than one per row.
pub const WINDOW: usize = 256 * 1024;

/// Hard cap on the bytes returned for a single row.
///
/// A row longer than this is handed back truncated with [`Span::truncated`]
/// set. Nothing above needs more: the widest terminal cannot show a megabyte,
/// and the cap is what keeps a pathological 50MB single cell (SPEC.md §CSV,
/// "malformed input never panics") from turning a keypress into a 50MB alloc.
pub const MAX_ROW_BYTES: usize = 1 << 20;

/// Bytes read for a byte range, and whether the range was clipped to fit
/// [`MAX_ROW_BYTES`] or ran off the end of a file that shrank under us.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub data: Vec<u8>,
    pub truncated: bool,
}

impl Span {
    fn empty() -> Span {
        Span {
            data: Vec::new(),
            truncated: false,
        }
    }
}

/// Where a [`Reader`]'s bytes come from.
///
/// Piped input has no path to seek in, and stdin cannot be re-read, so it is
/// held whole — the one case where the "never load the file" rule cannot apply,
/// because the bytes only exist in the pipe. Everything above is unaffected:
/// the row index, the window and the source all see the same [`Reader`] API.
enum Backing {
    File(File),
    Mem(Vec<u8>),
}

impl Backing {
    /// Current size, or `None` when the file cannot be stat'ed (it may have
    /// been unlinked), which the caller answers by keeping the size it had.
    fn len(&self) -> Option<u64> {
        match self {
            Backing::File(f) => f.metadata().map(|m| m.len()).ok(),
            Backing::Mem(b) => Some(b.len() as u64),
        }
    }

    /// Read into `buf` from offset `at`, returning how many bytes landed.
    fn read_at(&mut self, at: u64, buf: &mut [u8], reads: &mut u64) -> usize {
        match self {
            Backing::File(f) => read_into(f, at, buf, reads),
            Backing::Mem(data) => {
                *reads += 1;
                let at = at.min(data.len() as u64) as usize;
                let n = buf.len().min(data.len() - at);
                buf[..n].copy_from_slice(&data[at..at + n]);
                n
            }
        }
    }
}

/// A file plus one sliding read window over it.
pub struct Reader {
    file: Backing,
    /// The window's bytes. Never larger than [`WINDOW`].
    win: Vec<u8>,
    /// File offset of `win[0]`.
    win_at: u64,
    /// The last window fill read short, i.e. it reached end-of-file.
    win_eof: bool,
    /// File size as of the last stat. Advisory: the file may change under us.
    size: u64,
    /// `read(2)` calls issued, for the tests that assert scrolling does not
    /// cost a syscall per row.
    reads: u64,
}

impl Reader {
    /// Open `path` for reading. Stats it once; reads nothing.
    pub fn open(path: &Path) -> io::Result<Reader> {
        let file = Backing::File(File::open(path)?);
        let size = file.len().unwrap_or(0);
        Ok(Reader {
            file,
            win: Vec::new(),
            win_at: 0,
            win_eof: false,
            size,
            reads: 0,
        })
    }

    /// A reader over bytes already in memory, for input that arrived on a pipe.
    ///
    /// It has no name of its own: what the status bar shows is the label
    /// `open.rs` resolved for the *input*, which is above this seam.
    pub fn memory(data: Vec<u8>) -> Reader {
        let size = data.len() as u64;
        Reader {
            file: Backing::Mem(data),
            win: Vec::new(),
            win_at: 0,
            win_eof: false,
            size,
            reads: 0,
        }
    }

    /// File size as of the last stat.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Re-stat the file, picking up growth or truncation. Keeps the previous
    /// value if the stat fails (the file may have been unlinked).
    pub fn refresh_size(&mut self) -> u64 {
        if let Some(n) = self.file.len() {
            self.size = n;
        }
        self.size
    }

    /// Number of `read(2)` calls issued so far. Nothing in the binary counts
    /// its own syscalls; the tests that pin "a screenful of rows is one read"
    /// do.
    #[cfg(test)]
    pub fn reads(&self) -> u64 {
        self.reads
    }

    /// Up to `want` bytes at file offset `at`, served from the window,
    /// refilling it if the range is not covered. Short at end-of-file, empty
    /// past it. `want` is clamped to [`WINDOW`].
    pub fn chunk(&mut self, at: u64, want: usize) -> &[u8] {
        let want = want.min(WINDOW);
        if !self.covers(at, want) {
            self.fill(at);
        }
        let off = at.saturating_sub(self.win_at) as usize;
        let off = off.min(self.win.len());
        let end = off.saturating_add(want).min(self.win.len());
        &self.win[off..end]
    }

    /// True when `at..at+want` is already in the window.
    ///
    /// A window that hit end-of-file also covers a range running past its end,
    /// *unless* the file has since grown — which is checked by re-stating, so
    /// a file appended to while open starts yielding its new bytes.
    fn covers(&mut self, at: u64, want: usize) -> bool {
        if at < self.win_at {
            return false;
        }
        let off = (at - self.win_at) as usize;
        if off > self.win.len() {
            return false;
        }
        // The file shrank under us since the last stat: the window is holding
        // bytes that no longer exist, so it has to be re-read rather than
        // served. The cached size is enough — no extra syscall here.
        if self.win_eof && self.size < self.win_at + self.win.len() as u64 {
            return false;
        }
        if off + want <= self.win.len() {
            return true;
        }
        if !self.win_eof {
            return false;
        }
        let end = self.win_at + self.win.len() as u64;
        self.refresh_size() <= end
    }

    /// Refill the window so it starts at `at`.
    fn fill(&mut self, at: u64) {
        self.win.clear();
        self.win.resize(WINDOW, 0);
        let Reader { file, win, reads, .. } = self;
        let got = file.read_at(at, win, reads);
        self.win.truncate(got);
        self.win_at = at;
        self.win_eof = got < WINDOW;
    }

    /// The bytes of `start..end`, capped at [`MAX_ROW_BYTES`].
    ///
    /// Ranges up to a window go through the window (so a screenful of rows is
    /// one read); larger ones are read directly and left out of the window so
    /// that one huge row does not evict the viewport's neighbourhood.
    pub fn bytes(&mut self, start: u64, end: u64) -> Span {
        if end <= start {
            return Span::empty();
        }
        let full = end - start;
        let want = full.min(MAX_ROW_BYTES as u64) as usize;
        let mut truncated = full > want as u64;
        let data = if want <= WINDOW {
            self.chunk(start, want).to_vec()
        } else {
            let mut buf = vec![0u8; want];
            let Reader { file, reads, .. } = self;
            let got = file.read_at(start, &mut buf, reads);
            buf.truncate(got);
            buf
        };
        // A file that shrank under us reads short: report it rather than
        // pretending the row is intact.
        truncated |= data.len() < want;
        Span { data, truncated }
    }
}

/// Read at most `buf.len()` bytes at offset `at`, returning how many landed.
///
/// Errors are not propagated: a mid-scroll read failure on a file that was
/// truncated or unlinked degrades to a short row, never to a panic or a dead
/// viewport. `Interrupted` is retried, every other error stops the read.
fn read_into(file: &mut File, at: u64, buf: &mut [u8], reads: &mut u64) -> usize {
    if file.seek(SeekFrom::Start(at)).is_err() {
        return 0;
    }
    let mut got = 0;
    while got < buf.len() {
        *reads += 1;
        match file.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    got
}

#[cfg(test)]
#[path = "read_tests.rs"]
pub(crate) mod tests;
