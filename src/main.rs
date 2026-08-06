//! `mdr` — a terminal markdown reader. Entry point and wiring.
//!
//! Argument parsing, input resolution, the panic guard, the interactive event
//! loop and the non-interactive dump path all hang off here.
//!
//! There is deliberately no crate-wide `allow(dead_code)`: an unused item is
//! either drift or a missing keybinding, and both should fail the build's
//! warning check rather than hide.

// `sys` is the only module allowed to contain `unsafe`. Every other module
// carries its own `#![deny(unsafe_code)]` inner attribute (repeating it here
// as an outer attribute is redundant), and this file contains none itself.
mod sys;

mod cli;
mod dump;
mod key;
mod md;
mod nav;
mod pager;
mod render;
mod select;
mod term;
mod theme;

use std::env;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const EXIT_OK: i32 = 0;
const EXIT_ERROR: i32 = 1;
const EXIT_USAGE: i32 = 2;

/// A fatal error plus the process exit code it maps to.
struct Fail {
    msg: String,
    code: i32,
}

impl Fail {
    fn runtime(msg: impl Into<String>) -> Self {
        Fail {
            msg: msg.into(),
            code: EXIT_ERROR,
        }
    }
    fn usage(msg: impl Into<String>) -> Self {
        Fail {
            msg: msg.into(),
            code: EXIT_USAGE,
        }
    }
}

/// The document to display, plus where keyboard input comes from.
struct Input {
    label: String,
    text: String,
    /// The file on disk, when there is one (`None` for stdin).
    path: Option<PathBuf>,
    /// Descriptor the pager reads keys from, and whether we own it. `None`
    /// means "use stdin" (it is already a terminal).
    tty: Option<(sys::Fd, bool)>,
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

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args = match cli::parse(env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            fatal(&e.to_string());
            return EXIT_USAGE;
        }
    };
    if args.help {
        return emit(&cli::help_text());
    }
    if args.version {
        return emit(&cli::version_text());
    }
    install_panic_hook();
    match start(&args) {
        Ok(()) => EXIT_OK,
        Err(f) => {
            fatal(&f.msg);
            f.code
        }
    }
}

fn start(args: &cli::Args) -> Result<(), Fail> {
    let input = resolve_input(args)?;
    let doc = md::parse(&input.text);
    let outline = outline(&doc);
    if args.toc {
        emit(&render_outline(&outline));
        return Ok(());
    }
    if sys::is_tty(sys::STDOUT) {
        // Interactive: hand the document to the pager. A missing controlling
        // terminal is not fatal — fall through to the dump path.
        match interactive(args, &input, doc) {
            Ok(()) => return Ok(()),
            Err(PagerExit::NoTty(doc)) => return dump_document(args, doc),
            Err(PagerExit::Fatal(f)) => return Err(f),
        }
    }
    dump_document(args, doc)
}

/// Non-interactive rendering: one full-fidelity pass to stdout.
fn dump_document(args: &cli::Args, doc: md::Document) -> Result<(), Fail> {
    let plain = plain_mode(
        args.plain,
        env::var("NO_COLOR").ok(),
        sys::is_tty(sys::STDOUT),
    );
    let cols = sys::winsize_of(sys::STDOUT).or_else(sys::winsize).map(|(c, _)| c as usize);
    let width = dump::layout_width(args.width, cols);
    // A terminal has a viewport, so overflowing rows are clipped the way the
    // pager clips them; a file or pipe keeps the full row.
    let clip = sys::is_tty(sys::STDOUT);
    emit(&dump::dump(&doc, width, plain, clip));
    Ok(())
}

/// Why the interactive path did not run to completion. `NoTty` hands the
/// document back so the caller can fall back to dumping it.
enum PagerExit {
    NoTty(md::Document),
    Fatal(Fail),
}

/// Enter raw mode and run the event loop until the pager asks to quit.
fn interactive(args: &cli::Args, input: &Input, doc: md::Document) -> Result<(), PagerExit> {
    // One NO_COLOR rule for both paths: `Term` reads no environment itself.
    let opts = term::TermOptions {
        alt_screen: !args.no_alt,
        plain: plain_mode(args.plain, env::var("NO_COLOR").ok(), true),
    };
    let mut term = match term::Term::new(opts) {
        Ok(t) => t,
        Err(term::TermError::NoTty) => return Err(PagerExit::NoTty(doc)),
        Err(e) => return Err(PagerExit::Fatal(Fail::runtime(format!("terminal: {e:?}")))),
    };
    let (cols, rows) = term.size();
    let label = status_label(input);
    let mut pager = pager::Pager::new(doc, label, cols as usize, rows as usize, args.width);
    // A document read from a real file gets a corpus: relative links, history
    // and the index all hang off it. Piped stdin has no location, so it does
    // not (SPEC.md §Navigation).
    if let Some(path) = &input.path {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        pager.attach_nav(nav::Navigator::new(path, args.index.as_deref(), &cwd));
    }
    let result = event_loop(&mut term, &mut pager);
    // Runs on the error path too: `result` is only unwrapped afterwards.
    term.restore();
    result.map_err(PagerExit::Fatal)
}

/// One buffered frame per iteration; blocks in `read` between frames.
fn event_loop(term: &mut term::Term, pager: &mut pager::Pager) -> Result<(), Fail> {
    let mut decoder = key::Decoder::new();
    let mut buf = [0u8; 4096];
    loop {
        if pager.take_dirty() {
            let mut frame = term.frame();
            pager.paint(&mut frame);
            term.flush(&frame)
                .map_err(|e| Fail::runtime(format!("write: {e:?}")))?;
        }
        match term.read(&mut buf) {
            sys::ReadOutcome::Bytes(n) => {
                for ev in decoder.feed(&buf[..n]) {
                    pager.handle(ev);
                }
            }
            sys::ReadOutcome::Timeout => {}
            sys::ReadOutcome::Eof => return Ok(()),
            sys::ReadOutcome::Error(e) => return Err(Fail::runtime(format!("read: errno {e}"))),
        }
        if let Some(y) = pager.take_yank() {
            let msg = deliver_yank(term, &y);
            pager.notify(msg);
        }
        // SIGWINCH is polled, never handled inline: the handler only sets a flag.
        if term.resize_pending() {
            let (c, r) = term.refresh_size();
            pager.resize(c as usize, r as usize);
        }
        pager.tick();
        // SIGTERM/SIGHUP/SIGQUIT are caught (sys::install_signal_handlers) so
        // that they land here and the caller's `term.restore()` still runs,
        // instead of the default disposition killing us in raw mode.
        if pager.should_quit() || term.interrupt_pending() || term.terminate_pending() {
            return Ok(());
        }
    }
}

/// Put a yank on the system clipboard and on disk, and describe the outcome
/// for the status bar.
///
/// OSC 52 is wrapped for whichever multiplexer is in the way, and the full text
/// always goes to `~/.cache/mdr/last-yank.txt` as well, so a terminal that
/// refuses the escape never loses the copy silently (SPEC.md §Keybindings).
fn deliver_yank(term: &mut term::Term, yank: &select::Yank) -> String {
    let saved = select::clip::write_fallback(&yank.text);
    let home = env::var_os("HOME").map(PathBuf::from);
    let shown = saved
        .as_ref()
        .map(|p| select::clip::display_path(p, home.as_deref()));
    let (frame, report) = clipboard_frame(term.frame(), &yank.text, select::clip::mux_from_env());
    let report = match term.flush(&frame) {
        Ok(()) => Some(report),
        Err(_) => None,
    };
    select::clip::yank_message(&yank.what, report, shown.as_deref())
}

/// Put the OSC 52 sequence for `text` into `frame`.
///
/// The sequence is not part of the next frame's layout, so it goes out on its
/// own single write — but still through the frame buffer, never `print!`
/// (SPEC.md §7). Split out of [`deliver_yank`] so the delivery path can be
/// tested without a terminal. `raw` is used rather than `span` because the
/// bytes are a control sequence, not styled text: plain mode must not strip it.
fn clipboard_frame(
    mut frame: term::Frame,
    text: &str,
    mux: select::clip::Mux,
) -> (term::Frame, term::ClipReport) {
    let (seq, report) = select::clip::clipboard_sequence(text, mux);
    frame.raw(&seq);
    (frame, report)
}

/// Status-bar label: the file name as given, or `<stdin>`.
fn status_label(input: &Input) -> String {
    Path::new(&input.label)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| input.label.clone())
}

/// Colour is off when asked for, when `NO_COLOR` is set non-empty, or when
/// stdout is not a terminal.
///
/// This is the *only* NO_COLOR rule in the crate — `Term` reads no environment
/// of its own — so `mdr doc.md` and `mdr doc.md | cat` can never disagree. An
/// empty `NO_COLOR=` does not count, per the no-color.org spec.
fn plain_mode(flag: bool, no_color: Option<String>, stdout_tty: bool) -> bool {
    flag || no_color.map(|v| !v.is_empty()).unwrap_or(false) || !stdout_tty
}

fn resolve_input(args: &cli::Args) -> Result<Input, Fail> {
    let from_stdin = args.file.as_deref() == Some(Path::new("-"))
        || (args.file.is_none() && !sys::is_tty(sys::STDIN));
    if from_stdin {
        let mut raw = Vec::new();
        std::io::stdin()
            .read_to_end(&mut raw)
            .map_err(|e| Fail::runtime(format!("reading stdin: {e}")))?;
        let text = md::sanitize::decode(raw);
        // stdin is a pipe, so keys have to come from the controlling terminal.
        return Ok(Input {
            label: "<stdin>".to_string(),
            text,
            path: None,
            tty: sys::tty_fd(),
        });
    }
    let path = match &args.file {
        Some(p) => p.clone(),
        None => index_path(args.index.as_deref())?,
    };
    read_document(&path)
}

fn read_document(path: &Path) -> Result<Input, Fail> {
    let text = md::sanitize::read_file(path)
        .map_err(|e| Fail::runtime(format!("{}: {}", path.display(), e)))?;
    Ok(Input {
        label: path.display().to_string(),
        text,
        path: Some(path.to_path_buf()),
        tty: None,
    })
}

/// Where to start when no FILE was given: the `--index` target, or a README.md
/// in the working directory.
fn index_path(index: Option<&Path>) -> Result<PathBuf, Fail> {
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
fn outline(doc: &md::Document) -> Vec<(u8, String)> {
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

fn render_outline(outline: &[(u8, String)]) -> String {
    let mut s = String::new();
    for (level, text) in outline {
        s.push_str(&" ".repeat((*level as usize - 1) * 2));
        s.push_str(text);
        s.push('\n');
    }
    s
}

/// All non-TUI stdout goes through here; a closed pipe is not an error.
fn emit(s: &str) -> i32 {
    let stdout = std::io::stdout();
    let _ = stdout.lock().write_all(s.as_bytes());
    let _ = std::io::stdout().flush();
    EXIT_OK
}

fn fatal(msg: &str) {
    // Permitted by SPEC §7: startup/teardown errors, never UI.
    eprintln!("{}: {}", cli::BIN, msg);
}

/// Restore the terminal before the default hook prints the panic message,
/// otherwise the message lands on the alternate screen in raw mode.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        term::emergency_restore();
        previous(info);
    }));
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
