//! Where `--lens` meets the input (SPEC.md §Lenses).
//!
//! Split out of `open.rs` so both stay under the size limit, and because this
//! is genuinely one decision: a lens applies to a *record file*, so this is the
//! only place that pairs a lens with a format. Nothing here knows a dialect —
//! `crate::lens` owns the registry, and the record source owns the rows.
#![deny(unsafe_code)]

use std::path::Path;

use super::{Fail, Input};
use crate::source::detect::{self, Format};
use crate::source::jsonl::JsonlSource;
use crate::{cli, lens};

/// The record source, read through `--lens` when one was asked for.
pub(super) fn jsonl_source_with(input: &Input, args: &cli::Args) -> Result<JsonlSource, Fail> {
    let mut src = super::jsonl_source(input)?;
    if let Some(name) = args.lens.as_deref() {
        match lens::find(name) {
            Some(l) => src.set_lens(l),
            // Unreachable: `cli` validates the name against the same table.
            None => return Err(Fail::usage(unknown_lens(name))),
        }
    }
    Ok(src)
}

/// A `--lens` on something that is not a record file.
///
/// A lens is a transform over records, so naming one for a markdown or CSV
/// document is a usage error rather than a silent no-op: a flag that quietly
/// does nothing is worse than one that says why.
pub(super) fn needs_records(format: Format) -> Fail {
    Fail::usage(format!(
        "--lens reads a record file (.jsonl / .ndjson); this is {}. \
         Try `--format jsonl` if that is what it is.",
        detect::name_of(format)
    ))
}

fn unknown_lens(name: &str) -> String {
    format!("unknown lens `{name}`; try `{} --lens list`", cli::BIN)
}

/// A `--lens` says the input is records, which is stronger evidence than a
/// content sniff.
///
/// The extension is evidence too, and a better one: `tread --lens agent
/// notes.md` should say so rather than read prose as records. But unnamed input
/// — a pipe — is only ever *guessed* at, and one JSON object on a line looks
/// exactly like a JSON document to a sniffer, so `cat run.jsonl | tread --lens
/// agent` would otherwise be refused for being what it is. An explicit
/// `--format` always wins over both.
pub(super) fn format_for(args: &cli::Args, path: Option<&Path>, sniffed: Format) -> Format {
    if args.lens.is_none() || args.format.is_some() {
        return sniffed;
    }
    match path.and_then(detect::from_path) {
        Some(_) => sniffed,
        None => Format::Jsonl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lens is defined over records, so it settles a *guess* about what the
    /// input is — and never overrules the extension or an explicit `--format`.
    #[test]
    fn a_lens_decides_what_unnamed_input_is() {
        let mut args = cli::Args { lens: Some("agent".to_string()), ..cli::Args::default() };
        // A pipe: one JSON object on a line sniffs as a document, and the lens
        // says it is a record.
        assert_eq!(format_for(&args, None, Format::Json), Format::Jsonl);
        // A file whose extension said nothing is a guess too.
        assert_eq!(
            format_for(&args, Some(Path::new("run.log")), Format::Csv),
            Format::Jsonl
        );
        // A named extension is evidence, and keeps its answer.
        for (name, format) in [("a.md", Format::Markdown), ("a.csv", Format::Csv)] {
            assert_eq!(format_for(&args, Some(Path::new(name)), format), format);
        }
        // An explicit --format always wins.
        args.format = Some(Format::Json);
        assert_eq!(format_for(&args, None, Format::Json), Format::Json);
        // And with no lens, nothing changes at all.
        let plain = cli::Args::default();
        assert_eq!(format_for(&plain, None, Format::Json), Format::Json);
        assert_eq!(format_for(&plain, None, Format::Markdown), Format::Markdown);
    }

    /// The two errors this seam can produce name the flag, the lens and the fix.
    #[test]
    fn the_refusals_say_what_to_do() {
        let f = needs_records(Format::Markdown);
        assert_eq!(f.code, crate::EXIT_USAGE);
        assert!(f.msg.contains("--lens") && f.msg.contains("markdown"), "{}", f.msg);
        assert!(f.msg.contains("--format jsonl"), "{}", f.msg);
        assert!(unknown_lens("opencode").contains("--lens list"));
    }
}
