//! Hand-rolled command line parser. Zero dependencies, no panics on bad input.
#![deny(unsafe_code)]

use std::fmt;
use std::path::PathBuf;

use crate::csv::delim;
use crate::lens;
use crate::source::detect::{parse_format, Format};

pub const BIN: &str = "tread";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Fully parsed command line. `parse` never reads the environment, so this is
/// unit-testable without a real argv.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Args {
    /// Positional FILE. `Some("-")` means "read stdin explicitly".
    pub file: Option<PathBuf>,
    /// `--index <PATH>`: corpus index file, or a directory holding README.md.
    pub index: Option<PathBuf>,
    pub no_alt: bool,
    pub plain: bool,
    /// `--no-browser`: never hand an external link to the system opener; show
    /// the URL and refuse, as the reader did before SPEC.md §"Opening a link
    /// outside the reader".
    pub no_browser: bool,
    pub width: Option<usize>,
    /// `--format <md|csv|json|jsonl|text>`: overrides the extension and the sniff.
    pub format: Option<Format>,
    /// `--delim <char|tab|comma|semicolon|pipe>`: overrides the CSV sniff.
    pub delim: Option<u8>,
    /// `--lens <name>`: a semantic view over a record file (SPEC.md §Lenses).
    /// Validated here, so an unknown name never reaches the reader.
    pub lens: Option<String>,
    /// `--lens list`: print the lenses and exit 2.
    pub lens_list: bool,
    pub toc: bool,
    /// `--to-jsonl`: stream a top-level JSON array to stdout, one element per
    /// line, and exit (SPEC.md §JSON).
    pub to_jsonl: bool,
    pub help: bool,
    pub version: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    UnknownFlag(String),
    MissingValue(String),
    BadValue {
        flag: String,
        value: String,
        why: String,
    },
    UnexpectedPositional(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::UnknownFlag(flag) => {
                write!(f, "unknown option `{flag}`; try `{BIN} --help`")
            }
            CliError::MissingValue(flag) => {
                write!(f, "option `{flag}` needs a value; try `{BIN} --help`")
            }
            CliError::BadValue { flag, value, why } => {
                write!(f, "invalid value `{value}` for `{flag}`: {why}")
            }
            CliError::UnexpectedPositional(arg) => write!(
                f,
                "unexpected argument `{arg}`; {BIN} takes at most one FILE"
            ),
        }
    }
}

pub fn help_text() -> String {
    format!(
        "\
{BIN} {VERSION} — a terminal reader for markdown, CSV, JSON and code

USAGE
    {BIN} [OPTIONS] [FILE]

    With no FILE, {BIN} reads piped stdin, or opens the corpus index when
    stdin is a terminal. `-` as FILE forces reading stdin.

OPTIONS
    --index <PATH>  Treat PATH as the corpus index. PATH may be a markdown
                    file or a directory containing README.md. Relative links
                    in the corpus resolve against it.
    --no-alt        Render into the scrollback instead of the alternate
                    screen, so the output stays after quitting.
    --plain         Disable color and styling. Implied by NO_COLOR or by a
                    non-terminal stdout.
    --no-browser    Never open an external link. Enter on an http, https or
                    mailto link normally hands the URL to the system opener
                    (one process, never a shell); this shows the URL and
                    refuses instead. Any other scheme is always refused, by
                    name.
    --width <N>     Force the wrap width to N columns instead of detecting
                    the terminal size.
    --format <FMT>  Force the format: `md`, `csv`, `json`, `jsonl`
                    (`ndjson`) or `text`. By default the file extension
                    decides — a name it does not know is plain text — and
                    unnamed input (a pipe) is sniffed.
    --delim <D>     CSV field delimiter: one character, or `tab`, `comma`,
                    `semicolon`, `pipe`. Sniffed among , TAB ; | by default.
    --lens <NAME>   Read a record file through a semantic view: `agent` for
                    Claude Code session logs. `--lens list` prints them all.
                    Without it, records render as the generic JSON tree.
    --toc           Print the heading outline of the document and exit.
    --to-jsonl      Write a JSON document's top-level array to stdout as one
                    element per line, and exit. Streams: the document is never
                    held in memory. Anything but an array is refused.
    -h, --help      Show this help and exit.
    -V, --version   Show the version and exit.

KEYS
    j/k                   one row       h/l         scroll horizontally
    d/u space/b g/G       scroll        Tab/S-Tab   next/prev heading, or
    za Enter zM zR        collapse                  block under a lens
    n \u{2190}/\u{2192}                 pick a link   Enter    follow / open
    Backspace             back
    o  i                  outline/index  / ?        search
    v y Y c               select & yank  q  Ctrl-C   quit

JSON
    A .json document is a foldable tree: root open, everything under it
    folded. Nothing reads the whole file, at any size — containers are
    indexed by byte range as you open them, and a member is parsed only when
    it is shown. y copies the value under the cursor, Y the subtree as valid
    JSON, and the status bar names its path (.users[3].name).

CSV
    Large files open instantly: the row index is built lazily, the header
    stays pinned, h/l move a whole column and w widens the column under the
    cursor. y copies the cell, Y the row and c the column, always as valid
    CSV rather than as the padded display form.

LENSES
    --lens turns a .jsonl trajectory back into the conversation it recorded:
    messages stay on screen, and runs of tool calls and their results fold
    into one row — \u{27e8}6 steps \u{b7} 4 tool calls\u{27e9} — that opens with Enter or za.
    Under a row is what was said, or what the step was thinking, clipped to
    six lines that say how many they hid. Enter or za toggles the record's two
    levels: the whole of that text with the record's tool calls listed as
    calls, and back to the clip. Enter on a call row shows the arguments it
    was made with and the output it returned. r shows the raw record from
    either level, and shuts it again. A record the lens does not recognise
    renders as the generic tree, whole: a lens adds interpretation and never
    hides data.

The mouse is never captured, so terminal-native drag-select always works.
"
    )
}

pub fn version_text() -> String {
    format!("{BIN} {VERSION}\n")
}

/// Parse an argument list that does *not* include the program name.
pub fn parse<I: Iterator<Item = String>>(args: I) -> Result<Args, CliError> {
    let mut out = Args::default();
    let mut it = args.peekable();
    let mut only_positional = false;

    while let Some(arg) = it.next() {
        if only_positional {
            set_positional(&mut out, arg)?;
        } else if arg == "--" {
            only_positional = true;
        } else if let Some(body) = arg.strip_prefix("--") {
            parse_long(&mut out, body, &mut it)?;
        } else if arg.len() > 1 && arg.starts_with('-') {
            parse_shorts(&mut out, &arg)?;
        } else {
            set_positional(&mut out, arg)?;
        }
    }
    Ok(out)
}

fn set_positional(out: &mut Args, arg: String) -> Result<(), CliError> {
    if out.file.is_some() {
        return Err(CliError::UnexpectedPositional(arg));
    }
    out.file = Some(PathBuf::from(arg));
    Ok(())
}

/// Handle one `--long`, `--long value` or `--long=value` argument.
fn parse_long<I: Iterator<Item = String>>(
    out: &mut Args,
    body: &str,
    it: &mut std::iter::Peekable<I>,
) -> Result<(), CliError> {
    let (name, inline) = match body.split_once('=') {
        Some((n, v)) => (n, Some(v.to_string())),
        None => (body, None),
    };
    let flag = format!("--{name}");
    let wants_value = matches!(name, "index" | "width" | "format" | "delim" | "lens");
    if let (false, Some(v)) = (wants_value, inline.as_ref()) {
        if matches!(
            name,
            "no-alt" | "plain" | "no-browser" | "toc" | "to-jsonl" | "help" | "version"
        ) {
            return Err(CliError::BadValue {
                flag,
                value: v.clone(),
                why: "this option takes no value".to_string(),
            });
        }
    }
    let mut take = || -> Result<String, CliError> {
        match inline.clone().or_else(|| it.next()) {
            Some(v) if !v.is_empty() => Ok(v),
            _ => Err(CliError::MissingValue(flag.clone())),
        }
    };
    match name {
        "index" => out.index = Some(PathBuf::from(take()?)),
        "width" => out.width = Some(parse_width(&take()?)?),
        "format" => {
            out.format = Some(parsed(parse_format, take()?, &flag, FORMAT_WHY)?)
        }
        "delim" => out.delim = Some(parsed(delim::parse_delim, take()?, &flag, DELIM_WHY)?),
        "lens" => set_lens(out, take()?, &flag)?,
        "no-alt" => out.no_alt = true,
        "plain" => out.plain = true,
        "no-browser" => out.no_browser = true,
        "toc" => out.toc = true,
        "to-jsonl" => out.to_jsonl = true,
        "help" => out.help = true,
        "version" => out.version = true,
        _ => return Err(CliError::UnknownFlag(flag.clone())),
    }
    Ok(())
}

/// `--lens <name>`: `list` asks for the catalogue, a known name selects it, and
/// anything else is a usage error naming what there is. Both of the first two
/// exit 2 (SPEC.md §Lenses is a flag, never a guess), which is why `list` is
/// carried on [`Args`] rather than printed from here — this function neither
/// writes nor exits.
fn set_lens(out: &mut Args, value: String, flag: &str) -> Result<(), CliError> {
    if value == "list" {
        out.lens_list = true;
        return Ok(());
    }
    if !lens::exists(&value) {
        return Err(CliError::BadValue {
            flag: flag.to_string(),
            value,
            why: lens_why(),
        });
    }
    out.lens = Some(value);
    Ok(())
}

/// The names there are, for the error and the help text.
fn lens_why() -> String {
    format!(
        "expected `list` or one of: {}",
        lens::names().join(", ")
    )
}

/// Why a `--delim` value was rejected. A constant so the message and the
/// `--help` text cannot drift apart.
const DELIM_WHY: &str = "expected one character, or tab/comma/semicolon/pipe";

/// Why a `--format` value was rejected, for the same reason.
const FORMAT_WHY: &str = "expected `md`, `csv`, `json`, `jsonl` or `text`";

/// Apply a value parser to a flag's argument, turning `None` into the standard
/// "invalid value" error.
fn parsed<T>(
    parse: impl Fn(&str) -> Option<T>,
    raw: String,
    flag: &str,
    why: &str,
) -> Result<T, CliError> {
    parse(&raw).ok_or_else(|| CliError::BadValue {
        flag: flag.to_string(),
        value: raw,
        why: why.to_string(),
    })
}

/// Handle a bundle of short flags such as `-hV`.
fn parse_shorts(out: &mut Args, arg: &str) -> Result<(), CliError> {
    for ch in arg.chars().skip(1) {
        match ch {
            'h' => out.help = true,
            'V' => out.version = true,
            _ => return Err(CliError::UnknownFlag(format!("-{ch}"))),
        }
    }
    Ok(())
}

fn parse_width(raw: &str) -> Result<usize, CliError> {
    let bad = |why: &str| CliError::BadValue {
        flag: "--width".to_string(),
        value: raw.to_string(),
        why: why.to_string(),
    };
    let n: usize = raw.parse().map_err(|_| bad("expected a whole number"))?;
    if n == 0 {
        return Err(bad("must be at least 1"));
    }
    if n > 10_000 {
        return Err(bad("must be at most 10000"));
    }
    Ok(n)
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
