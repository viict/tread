//! Hand-rolled command line parser. Zero dependencies, no panics on bad input.
#![deny(unsafe_code)]

use std::fmt;
use std::path::PathBuf;

use crate::csv::delim;
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
    pub width: Option<usize>,
    /// `--format <md|csv>`: overrides the extension and the content sniff.
    pub format: Option<Format>,
    /// `--delim <char|tab|comma|semicolon|pipe>`: overrides the CSV sniff.
    pub delim: Option<u8>,
    pub toc: bool,
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
{BIN} {VERSION} — a terminal reader for markdown and CSV

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
    --width <N>     Force the wrap width to N columns instead of detecting
                    the terminal size.
    --format <FMT>  Force the format: `md` or `csv`. By default the file
                    extension decides, and unnamed input (a pipe) is sniffed.
    --delim <D>     CSV field delimiter: one character, or `tab`, `comma`,
                    `semicolon`, `pipe`. Sniffed among , TAB ; | by default.
    --toc           Print the heading outline of the document and exit.
    -h, --help      Show this help and exit.
    -V, --version   Show the version and exit.

KEYS
    j/k d/u space/b g/G   scroll        h/l         scroll horizontally
    za Enter zM zR        collapse      Tab/S-Tab   next/prev heading
    n Enter               follow link   Backspace   back
    o  i                  outline/index  / ?        search
    v y Y c               select & yank  q  Ctrl-C   quit

CSV
    Large files open instantly: the row index is built lazily, the header
    stays pinned, h/l move a whole column and w widens the column under the
    cursor. y copies the cell, Y the row and c the column, always as valid
    CSV rather than as the padded display form.

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
    let wants_value = matches!(name, "index" | "width" | "format" | "delim");
    if let (false, Some(v)) = (wants_value, inline.as_ref()) {
        if matches!(name, "no-alt" | "plain" | "toc" | "help" | "version") {
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
            out.format = Some(parsed(parse_format, take()?, &flag, "expected `md` or `csv`")?)
        }
        "delim" => out.delim = Some(parsed(delim::parse_delim, take()?, &flag, DELIM_WHY)?),
        "no-alt" => out.no_alt = true,
        "plain" => out.plain = true,
        "toc" => out.toc = true,
        "help" => out.help = true,
        "version" => out.version = true,
        _ => return Err(CliError::UnknownFlag(flag.clone())),
    }
    Ok(())
}

/// Why a `--delim` value was rejected. A constant so the message and the
/// `--help` text cannot drift apart.
const DELIM_WHY: &str = "expected one character, or tab/comma/semicolon/pipe";

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
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Args, CliError> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn empty_is_all_defaults() {
        assert_eq!(p(&[]).unwrap(), Args::default());
    }

    #[test]
    fn positional_file() {
        assert_eq!(p(&["a.md"]).unwrap().file, Some(PathBuf::from("a.md")));
        assert_eq!(p(&["-"]).unwrap().file, Some(PathBuf::from("-")));
    }

    #[test]
    fn every_boolean_flag() {
        let a = p(&["--no-alt", "--plain", "--toc", "--help", "--version"]).unwrap();
        assert!(a.no_alt && a.plain && a.toc && a.help && a.version);
    }

    #[test]
    fn short_flags_and_bundles() {
        assert!(p(&["-h"]).unwrap().help);
        assert!(p(&["-V"]).unwrap().version);
        let a = p(&["-hV"]).unwrap();
        assert!(a.help && a.version);
    }

    #[test]
    fn value_flags_both_forms() {
        for a in [p(&["--index", "c/"]).unwrap(), p(&["--index=c/"]).unwrap()] {
            assert_eq!(a.index, Some(PathBuf::from("c/")));
        }
        for a in [p(&["--width", "72"]).unwrap(), p(&["--width=72"]).unwrap()] {
            assert_eq!(a.width, Some(72));
        }
    }

    #[test]
    fn mixed_order_with_file() {
        let a = p(&["--width=40", "doc.md", "--plain"]).unwrap();
        assert_eq!(a.width, Some(40));
        assert!(a.plain);
        assert_eq!(a.file, Some(PathBuf::from("doc.md")));
    }

    #[test]
    fn end_of_options_marker() {
        let a = p(&["--", "--weird-name.md"]).unwrap();
        assert_eq!(a.file, Some(PathBuf::from("--weird-name.md")));
        assert!(!a.help);
    }

    #[test]
    fn unknown_flags_error_by_name() {
        assert_eq!(
            p(&["--nope"]),
            Err(CliError::UnknownFlag("--nope".to_string()))
        );
        assert_eq!(p(&["-x"]), Err(CliError::UnknownFlag("-x".to_string())));
        assert!(p(&["--nope"]).unwrap_err().to_string().contains("--help"));
    }

    #[test]
    fn value_on_boolean_flag_is_rejected() {
        assert!(p(&["--plain=yes"]).is_err());
    }

    #[test]
    fn missing_values() {
        assert_eq!(
            p(&["--index"]),
            Err(CliError::MissingValue("--index".to_string()))
        );
        assert_eq!(
            p(&["--width="]),
            Err(CliError::MissingValue("--width".to_string()))
        );
    }

    #[test]
    fn bad_width_values() {
        for bad in ["x", "0", "-3", "12.5", "99999"] {
            assert!(p(&["--width", bad]).is_err(), "{bad} should be rejected");
        }
        assert!(p(&["--width", "0"]).unwrap_err().to_string().contains("0"));
    }

    #[test]
    fn two_positionals_error() {
        assert_eq!(
            p(&["a.md", "b.md"]),
            Err(CliError::UnexpectedPositional("b.md".to_string()))
        );
    }

    #[test]
    fn flag_looking_value_is_consumed_as_value() {
        let a = p(&["--index", "--odd"]).unwrap();
        assert_eq!(a.index, Some(PathBuf::from("--odd")));
    }

    #[test]
    fn format_and_delimiter_overrides() {
        for a in [p(&["--format", "csv"]).unwrap(), p(&["--format=CSV"]).unwrap()] {
            assert_eq!(a.format, Some(Format::Csv));
        }
        assert_eq!(p(&["--format=md"]).unwrap().format, Some(Format::Markdown));
        assert_eq!(p(&["--delim", "tab"]).unwrap().delim, Some(b'\t'));
        assert_eq!(p(&["--delim=;"]).unwrap().delim, Some(b';'));
        assert_eq!(p(&[]).unwrap().format, None);
        for bad in ["json", "", "yaml"] {
            assert!(p(&["--format", bad]).is_err(), "{bad} should be rejected");
        }
        for bad in ["", "abc", "\""] {
            assert!(p(&["--delim", bad]).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn help_and_version_text_are_useful() {
        let h = help_text();
        for needle in [
            "--index", "--no-alt", "--plain", "--width", "--toc", "-V", "--format", "--delim",
        ] {
            assert!(h.contains(needle), "help missing {needle}");
        }
        assert!(version_text().starts_with("tread "));
    }
}
