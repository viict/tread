//! Where `--lens` meets the input (SPEC.md §Lenses).
//!
//! Split out of `open.rs` so both stay under the size limit, and because this
//! is genuinely one decision: a lens applies to *records*, and this is the only
//! place that pairs a lens with the format its records come in. Nothing here
//! knows a dialect — `crate::lens` owns the registry and each dialect declares
//! [`RecordsAt`] for itself, `crate::source::record` owns the rows.
//!
//! # Two shapes of record file, and a refusal for each
//!
//! A `RecordsAt::Lines` dialect reads a `.jsonl`; a `RecordsAt::Root` or
//! `RecordsAt::Member` dialect reads one JSON *document*. Pointing either at
//! the other is a usage error rather than a silent no-op or a wrong render: a
//! `.json` read as records would be one enormous record, and a `.jsonl` read as
//! a document would find no top-level array at all. Both refusals name the
//! lens, name what the file is, and say what to do.
#![deny(unsafe_code)]

use std::path::Path;

use super::{Fail, Input};
use crate::lens::RecordsAt;
use crate::source::detect::{self, Format};
use crate::source::jsonarray::{ArraySource, At};
use crate::source::jsonl::JsonlSource;
use crate::{cli, lens};

/// The `.jsonl` source, read through `--lens` when one was asked for.
pub(super) fn jsonl_source_with(input: &Input, args: &cli::Args) -> Result<JsonlSource, Fail> {
    let mut src = super::jsonl_source(input)?;
    if let Some(name) = args.lens.as_deref() {
        src.set_lens(with_lens(name)?);
    }
    Ok(src)
}

/// The document source for a lens whose records live *inside* a document.
///
/// Only ever reached with a lens: without one a `.json` is the document tree,
/// which is what [`crate::source::json`] is for.
pub(super) fn array_source_with(input: &Input, args: &cli::Args) -> Result<ArraySource, Fail> {
    let name = match args.lens.as_deref() {
        Some(name) => name,
        None => return Err(needs_lens()),
    };
    let at = match lens::records_at(name) {
        Some(RecordsAt::Member(key)) => At::Key(key),
        Some(RecordsAt::Root) => At::Root,
        // Unreachable: `build_source` refuses a `Lines` lens on a document
        // before it gets here, with a message that says what to do.
        _ => return Err(needs_records(name, Format::Json)),
    };
    let mut src = match &input.path {
        Some(path) => ArraySource::open(path, at)
            .map_err(|e| Fail::runtime(format!("{}: {e}", path.display())))?,
        None => ArraySource::from_bytes(input.bytes.clone().unwrap_or_default(), at),
    };
    src.set_lens(with_lens(name)?);
    Ok(src)
}

/// Does `--lens <name>` want the records-per-line reader, or the document one?
///
/// The one question the format wiring asks about a dialect. An unknown name
/// cannot reach here — `cli` validates it against the same table — and reads as
/// the old behaviour rather than as a panic.
pub(super) fn wants_document(name: &str) -> bool {
    !matches!(lens::records_at(name), None | Some(RecordsAt::Lines))
}

/// Whether `--lens <name>` can read a file of this format at all.
pub(super) fn accepts(name: &str, format: Format) -> bool {
    match wants_document(name) {
        true => format == Format::Json,
        false => format == Format::Jsonl,
    }
}

fn with_lens(name: &str) -> Result<Box<dyn lens::Lens>, Fail> {
    // Unreachable: `cli` validates the name against the same table.
    lens::find(name).ok_or_else(|| Fail::usage(unknown_lens(name)))
}

/// A `--lens` on something whose records it cannot find.
///
/// A lens is a transform over records, so naming one for a markdown or CSV
/// document is a usage error rather than a silent no-op: a flag that quietly
/// does nothing is worse than one that says why. Which file it *would* have
/// read depends on the dialect, so the message says which one this is.
pub(super) fn needs_records(name: &str, format: Format) -> Fail {
    let is = detect::name_of(format);
    match wants_document(name) {
        true => Fail::usage(format!(
            "--lens {name} reads records inside a JSON document; this is {is}. \
             Try `--format json` if that is what it is."
        )),
        false => Fail::usage(format!(
            "--lens {name} reads a record file (.jsonl / .ndjson); this is {is}. \
             Try `--format jsonl` if that is what it is."
        )),
    }
}

fn needs_lens() -> Fail {
    Fail::usage(String::from("a JSON document is only read as records under --lens"))
}

fn unknown_lens(name: &str) -> String {
    format!("unknown lens `{name}`; try `{} --lens list`", cli::BIN)
}

/// A `--lens` says what the input is, which is stronger evidence than a content
/// sniff — and *which* thing it says now depends on the dialect.
///
/// The extension is evidence too, and a better one: `tread --lens agent
/// notes.md` should say so rather than read prose as records. But unnamed input
/// — a pipe — is only ever *guessed* at, and one JSON object on a line looks
/// exactly like a JSON document to a sniffer, so `cat run.jsonl | tread --lens
/// agent` would otherwise be refused for being what it is. A document lens
/// wants the opposite answer for the same reason: `cat trajectory.json | tread
/// --lens atif` must not be forced into one enormous single record. An explicit
/// `--format` always wins over both.
pub(super) fn format_for(args: &cli::Args, path: Option<&Path>, sniffed: Format) -> Format {
    let Some(name) = args.lens.as_deref() else {
        return sniffed;
    };
    if args.format.is_some() {
        return sniffed;
    }
    match path.and_then(detect::from_path) {
        Some(_) => sniffed,
        None => match wants_document(name) {
            true => Format::Json,
            false => Format::Jsonl,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(lens: &str) -> cli::Args {
        cli::Args { lens: Some(lens.to_string()), ..cli::Args::default() }
    }

    /// A lens is defined over records, so it settles a *guess* about what the
    /// input is — and never overrules the extension or an explicit `--format`.
    #[test]
    fn a_lens_decides_what_unnamed_input_is() {
        let mut a = args("agent");
        // A pipe: one JSON object on a line sniffs as a document, and the lens
        // says it is a record.
        assert_eq!(format_for(&a, None, Format::Json), Format::Jsonl);
        // A file whose extension said nothing is a guess too.
        assert_eq!(format_for(&a, Some(Path::new("run.log")), Format::Csv), Format::Jsonl);
        // A named extension is evidence, and keeps its answer.
        for (name, format) in [("a.md", Format::Markdown), ("a.csv", Format::Csv)] {
            assert_eq!(format_for(&a, Some(Path::new(name)), format), format);
        }
        // An explicit --format always wins.
        a.format = Some(Format::Json);
        assert_eq!(format_for(&a, None, Format::Json), Format::Json);
        // And with no lens, nothing changes at all.
        let plain = cli::Args::default();
        assert_eq!(format_for(&plain, None, Format::Json), Format::Json);
        assert_eq!(format_for(&plain, None, Format::Markdown), Format::Markdown);
    }

    /// The dialect decides which way a *pipe* is read: the same bytes are a
    /// record stream under `agent` and one document under `atif`.
    #[test]
    fn a_document_lens_reads_a_pipe_as_a_document() {
        assert_eq!(format_for(&args("atif"), None, Format::Json), Format::Json);
        assert_eq!(format_for(&args("atif"), None, Format::Csv), Format::Json);
        assert!(wants_document("atif") && !wants_document("agent"));
        // An unknown name never reaches here, and reads as the old behaviour.
        assert!(!wants_document("nope"));
    }

    /// Each lens accepts exactly the file its records live in, and refuses the
    /// other one by name rather than rendering it wrongly.
    #[test]
    fn a_lens_accepts_the_shape_its_records_live_in() {
        assert!(accepts("agent", Format::Jsonl) && !accepts("agent", Format::Json));
        assert!(accepts("atif", Format::Json) && !accepts("atif", Format::Jsonl));
        for f in [Format::Markdown, Format::Csv, Format::Text, Format::Code] {
            assert!(!accepts("agent", f) && !accepts("atif", f));
        }
    }

    /// The refusals name the flag, the lens, what the file is, and the fix.
    #[test]
    fn the_refusals_say_what_to_do() {
        let f = needs_records("agent", Format::Markdown);
        assert_eq!(f.code, crate::EXIT_USAGE);
        assert!(f.msg.contains("--lens agent") && f.msg.contains("markdown"), "{}", f.msg);
        assert!(f.msg.contains("--format jsonl"), "{}", f.msg);

        let d = needs_records("atif", Format::Jsonl);
        assert_eq!(d.code, crate::EXIT_USAGE);
        assert!(d.msg.contains("--lens atif") && d.msg.contains("JSON document"), "{}", d.msg);
        assert!(d.msg.contains("--format json"), "{}", d.msg);

        assert!(unknown_lens("opencode").contains("--lens list"));
    }
}
