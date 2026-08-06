//! Resolving what to open, and building the [`Source`] behind it.
//!
//! Everything between "here is an argv" and "here is a `Box<dyn Source>`:
//! where the bytes come from (a file, a pipe, the corpus index), which format
//! they are in (SPEC.md §Multi-format reading) and which module reads them.
//! Split out of `main.rs` to keep both under the size limit; this is the only
//! place in the crate that knows the list of formats.
//!
//! A file on disk is *not* read here. Only its first bytes are, and only when
//! the extension says nothing: a multi-GB CSV must reach [`CsvSource`] as a
//! path so it can be indexed lazily (SPEC.md §CSV).
#![deny(unsafe_code)]

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::source::detect::{self, Format};
use crate::source::{csv::CsvSource, markdown::MarkdownSource, Source};
use crate::{cli, md, sys};

/// A fatal error plus the process exit code it maps to.
pub struct Fail {
    pub msg: String,
    pub code: i32,
}

impl Fail {
    pub fn runtime(msg: impl Into<String>) -> Self {
        Fail {
            msg: msg.into(),
            code: crate::EXIT_ERROR,
        }
    }
    pub fn usage(msg: impl Into<String>) -> Self {
        Fail {
            msg: msg.into(),
            code: crate::EXIT_USAGE,
        }
    }
}

/// The document to display, plus where keyboard input comes from.
pub struct Input {
    pub label: String,
    /// The bytes, when the input arrived on a pipe and cannot be re-read.
    /// `None` for a file on disk, which every format reads for itself — a
    /// multi-GB CSV must never be slurped (SPEC.md §CSV).
    pub bytes: Option<Vec<u8>>,
    /// The file on disk, when there is one (`None` for stdin).
    pub path: Option<PathBuf>,
    pub format: Format,
    /// Descriptor the pager reads keys from, and whether we own it. `None`
    /// means "use stdin" (it is already a terminal).
    pub tty: Option<(sys::Fd, bool)>,
}

impl Drop for Input {
    fn drop(&mut self) {
        if let Some((fd, owned)) = self.tty {
            if owned {
                sys::close_fd(fd);
            }
        }
    }
}


/// The document behind the format seam. Which format is this wiring's decision
/// and nothing else's: everything above takes a `Box<dyn Source>`
/// (SPEC.md §The `Source` seam).
pub fn build_source(input: &Input, args: &cli::Args) -> Result<Box<dyn Source>, Fail> {
    match input.format {
        Format::Csv => Ok(Box::new(csv_source(input, args)?)),
        Format::Markdown => Ok(Box::new(MarkdownSource::new(markdown_document(input)?))),
    }
}

/// The CSV source for this input: from the path when there is one, so the file
/// is never read whole, and from the piped bytes when there is not.
fn csv_source(input: &Input, args: &cli::Args) -> Result<CsvSource, Fail> {
    match &input.path {
        Some(path) => CsvSource::open(path, args.delim)
            .map_err(|e| Fail::runtime(format!("{}: {e}", path.display()))),
        None => Ok(CsvSource::from_bytes(
            input.bytes.clone().unwrap_or_default(),
            args.delim,
        )),
    }
}

/// Parse markdown, reading the file now if it was not read already.
pub fn markdown_document(input: &Input) -> Result<md::Document, Fail> {
    let text = match (&input.bytes, &input.path) {
        (Some(raw), _) => md::sanitize::decode(raw.clone()),
        (None, Some(path)) => md::sanitize::read_file(path)
            .map_err(|e| Fail::runtime(format!("{}: {}", path.display(), e)))?,
        (None, None) => String::new(),
    };
    Ok(md::parse(&text))
}

/// `--toc`: the heading outline for markdown, the column names for CSV. One
/// entry per line either way, so the output stays a list a script can read.
pub fn toc_text(input: &Input, args: &cli::Args) -> Result<String, Fail> {
    if input.format == Format::Markdown {
        return Ok(render_outline(&outline(&markdown_document(input)?)));
    }
    let mut src = csv_source(input, args)?;
    src.set_width(crate::dump_width(args));
    Ok(src.columns().iter().map(|c| format!("{c}\n")).collect())
}


pub fn resolve_input(args: &cli::Args) -> Result<Input, Fail> {
    let from_stdin = args.file.as_deref() == Some(Path::new("-"))
        || (args.file.is_none() && !sys::is_tty(sys::STDIN));
    if from_stdin {
        let mut raw = Vec::new();
        std::io::stdin()
            .read_to_end(&mut raw)
            .map_err(|e| Fail::runtime(format!("reading stdin: {e}")))?;
        check_encoding("<stdin>", sniff_head(&raw))?;
        // Unnamed input: the content decides (SPEC.md §Multi-format reading).
        let format = detect::decide(args.format, None, sniff_head(&raw));
        // stdin is a pipe, so keys have to come from the controlling terminal.
        return Ok(Input {
            label: "<stdin>".to_string(),
            bytes: Some(raw),
            path: None,
            format,
            tty: sys::tty_fd(),
        });
    }
    let path = match &args.file {
        Some(p) => p.clone(),
        None => index_path(args.index.as_deref())?,
    };
    read_document(args, &path)
}

/// Describe a file without reading it: the extension decides, and only a file
/// whose extension says nothing is sniffed — from its first bytes, never from
/// all of them.
fn read_document(args: &cli::Args, path: &Path) -> Result<Input, Fail> {
    // A named pipe, a device or anything else that is not a regular file has no
    // size to stat and no offset to seek to, so the lazy row index has nothing
    // to work with — and opening it twice (once to peek, once to read) blocks
    // forever on a fifo whose writer has already gone. It is a *stream*, so it
    // takes the same path piped stdin does: read once, keep the bytes.
    if !is_regular(path) {
        return stream_document(args, path);
    }
    // Peeked at even when the extension already decided: four bytes are enough
    // to refuse an encoding that would only render as mojibake, and a multi-GB
    // CSV must not be read one byte further than that.
    let head = match args.format.is_some() || detect::from_path(path).is_some() {
        true => head_bytes(path, BOM_BYTES)?,
        false => head_bytes(path, SNIFF_BYTES)?,
    };
    check_encoding(&path.display().to_string(), &head)?;
    let format = detect::decide(args.format, Some(path), &head);
    Ok(Input {
        label: path.display().to_string(),
        bytes: None,
        path: Some(path.to_path_buf()),
        format,
        tty: None,
    })
}

/// True when `path` is a regular file: something with a size and an offset,
/// which is what the lazy row index needs. A fifo, a character device
/// (`/dev/null`, `/dev/stdin`) or a directory is not.
///
/// `metadata` follows symlinks, which is right: a symlink to a regular file is
/// one. A path that cannot be stat'ed at all is left to the reader below, which
/// reports the open error with the same wording it always did.
fn is_regular(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(m) => m.is_file(),
        Err(_) => true,
    }
}

/// The most a non-seekable input is read into memory.
///
/// A stream has to be held whole — the bytes exist nowhere else — so unlike a
/// file on disk its size is a memory cost, and `tread /dev/zero` must stop
/// somewhere rather than take the machine down with it.
const STREAM_CAP: usize = 256 * 1024 * 1024;

/// Read a non-seekable path — a named pipe, a device — the way stdin is read.
fn stream_document(args: &cli::Args, path: &Path) -> Result<Input, Fail> {
    let label = path.display().to_string();
    let mut f = std::fs::File::open(path).map_err(|e| Fail::runtime(format!("{label}: {e}")))?;
    let mut raw = Vec::new();
    // One byte past the cap distinguishes "exactly the cap" from "more".
    f.by_ref()
        .take(STREAM_CAP as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| Fail::runtime(format!("{label}: {e}")))?;
    if raw.len() > STREAM_CAP {
        return Err(Fail::runtime(format!(
            "{label}: not a regular file, and larger than {}MiB \u{2014} tread reads a \
             stream into memory; redirect it to a file first",
            STREAM_CAP / (1024 * 1024)
        )));
    }
    check_encoding(&label, sniff_head(&raw))?;
    let format = detect::decide(args.format, Some(path), sniff_head(&raw));
    Ok(Input {
        label,
        bytes: Some(raw),
        // No path: there is nothing to re-open, and a device is not part of a
        // corpus. Everything downstream reads the bytes we already have.
        path: None,
        format,
        tty: None,
    })
}

/// Bytes a content sniff looks at.
const SNIFF_BYTES: usize = 8 * 1024;

/// Bytes an encoding check looks at: the longest byte-order mark there is.
const BOM_BYTES: usize = 4;

fn sniff_head(raw: &[u8]) -> &[u8] {
    &raw[..raw.len().min(SNIFF_BYTES)]
}

/// Refuse a document whose byte-order mark says it is in an encoding tread
/// cannot read, and say what to do about it. A reader that renders UTF-16 as
/// half a screen of replacement characters has told the user nothing; this is
/// the one case where opening the file is worse than not (see
/// [`detect::unreadable_encoding`]).
fn check_encoding(label: &str, head: &[u8]) -> Result<(), Fail> {
    match detect::unreadable_encoding(head) {
        None => Ok(()),
        Some(enc) => Err(Fail::runtime(format!(
            "{label}: {enc} text (byte-order mark); tread reads UTF-8 \
             \u{2014} convert it first, e.g. iconv -f {enc} -t UTF-8"
        ))),
    }
}

/// The first `want` bytes of a file, for the sniffer and the encoding check. A
/// file that cannot be opened is reported here rather than by whichever format
/// got it.
fn head_bytes(path: &Path, want: usize) -> Result<Vec<u8>, Fail> {
    let mut f = std::fs::File::open(path)
        .map_err(|e| Fail::runtime(format!("{}: {}", path.display(), e)))?;
    let mut buf = vec![0u8; want];
    let mut got = 0;
    while got < buf.len() {
        match f.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Fail::runtime(format!("{}: {}", path.display(), e))),
        }
    }
    buf.truncate(got);
    Ok(buf)
}

/// Where to start when no FILE was given: the `--index` target, or a README.md
/// in the working directory.
pub fn index_path(index: Option<&Path>) -> Result<PathBuf, Fail> {
    let candidate = match index {
        Some(p) if p.is_dir() => p.join("README.md"),
        Some(p) => p.to_path_buf(),
        None => PathBuf::from("README.md"),
    };
    if candidate.exists() {
        return Ok(candidate);
    }
    if index.is_some() {
        return Err(Fail::usage(format!(
            "index not found: {}",
            candidate.display()
        )));
    }
    Err(Fail::usage(format!(
        "no input: give a FILE, pipe markdown in, or use --index; try `{} --help`",
        cli::BIN
    )))
}


/// Heading outline, taken from the parsed document so `--toc` and the pager's
/// outline overlay always agree with the renderer.
pub fn outline(doc: &md::Document) -> Vec<(u8, String)> {
    let mut out = Vec::new();
    collect_headings(&doc.blocks, &mut out);
    out
}

fn collect_headings(blocks: &[md::Block], out: &mut Vec<(u8, String)>) {
    for b in blocks {
        match b {
            md::Block::Heading { level, content, .. } => {
                out.push((*level, md::ast::inline_text(content)));
            }
            md::Block::Quote { blocks, .. } => collect_headings(blocks, out),
            _ => {}
        }
    }
}

pub fn render_outline(outline: &[(u8, String)]) -> String {
    let mut s = String::new();
    for (level, text) in outline {
        s.push_str(&" ".repeat((*level as usize - 1) * 2));
        s.push_str(text);
        s.push('\n');
    }
    s
}


#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!("tread-open-{}-{nanos}-{name}", std::process::id()));
        p
    }

    #[test]
    fn only_a_regular_file_can_be_read_lazily() {
        let file = tmp_path("regular");
        std::fs::write(&file, b"id,name\n1,a\n").expect("write");
        assert!(is_regular(&file));
        std::fs::remove_file(&file).ok();

        let dir = tmp_path("dir");
        std::fs::create_dir(&dir).expect("mkdir");
        assert!(!is_regular(&dir), "a directory is not a regular file");
        std::fs::remove_dir(&dir).ok();

        // A path that cannot be stat'ed is left to the reader, which reports
        // the open error with its own wording rather than calling it a stream.
        assert!(is_regular(Path::new("/definitely/not/here")));
    }

    /// A character device has no size to stat and no offset to seek to, so it
    /// takes the stream path — the bug this catches is a fifo being opened
    /// twice (peek, then read), which blocks forever once the writer has gone.
    #[test]
    #[cfg(unix)]
    fn a_device_is_a_stream_not_a_file() {
        assert!(!is_regular(Path::new("/dev/null")));
    }

    #[test]
    fn an_encoding_we_cannot_read_is_refused_with_the_fix() {
        let err = check_encoding("x.csv", b"\xff\xfei\x00").expect_err("refused");
        assert_eq!(err.code, crate::EXIT_ERROR);
        assert!(err.msg.contains("x.csv"), "{}", err.msg);
        assert!(err.msg.contains("UTF-16"), "{}", err.msg);
        assert!(err.msg.contains("iconv -f UTF-16 -t UTF-8"), "{}", err.msg);
        // UTF-8, with or without a BOM, is not refused.
        assert!(check_encoding("x.csv", b"\xef\xbb\xbfid,name\n").is_ok());
        assert!(check_encoding("x.csv", b"id,name\n").is_ok());
        assert!(check_encoding("x.csv", b"").is_ok());
    }
}
