//! `cli.rs` tests. Beside the code, one file over, so both stay under the
//! 500-line limit (`src/main_tests.rs` and `src/plat/path_tests.rs` do the same).
#![deny(unsafe_code)]

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

/// `--no-browser` is off by default — SPEC.md §"Opening a link outside the
/// reader" makes opening the behaviour and refusing the opt-in — takes no
/// value, and mixes with a FILE like any other flag.
#[test]
fn no_browser_is_a_plain_flag_and_off_by_default() {
    assert!(!p(&[]).unwrap().no_browser);
    assert!(p(&["--no-browser"]).unwrap().no_browser);
    let a = p(&["--no-browser", "notes.md"]).unwrap();
    assert!(a.no_browser);
    assert_eq!(a.file, Some(PathBuf::from("notes.md")));
    assert!(p(&["--no-browser=yes"]).is_err(), "it takes no value");
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
    assert_eq!(p(&["--format=jsonl"]).unwrap().format, Some(Format::Jsonl));
    assert_eq!(p(&["--format", "ndjson"]).unwrap().format, Some(Format::Jsonl));
    assert_eq!(p(&["--format=json"]).unwrap().format, Some(Format::Json));
    assert_eq!(p(&["--format=text"]).unwrap().format, Some(Format::Text));
    assert_eq!(p(&["--format", "TXT"]).unwrap().format, Some(Format::Text));
    assert_eq!(p(&["--delim", "tab"]).unwrap().delim, Some(b'\t'));
    assert_eq!(p(&["--delim=;"]).unwrap().delim, Some(b';'));
    assert_eq!(p(&[]).unwrap().format, None);
    for bad in ["", "yaml"] {
        assert!(p(&["--format", bad]).is_err(), "{bad} should be rejected");
    }
    for bad in ["", "abc", "\""] {
        assert!(p(&["--delim", bad]).is_err(), "{bad} should be rejected");
    }
}

/// `--to-jsonl` is an export, so it is a flag and not a value: it takes
/// none, it is off by default, and it survives being mixed with a FILE.
#[test]
fn to_jsonl_is_a_plain_flag() {
    assert!(!p(&[]).unwrap().to_jsonl);
    assert!(p(&["--to-jsonl"]).unwrap().to_jsonl);
    let a = p(&["--to-jsonl", "big.json"]).unwrap();
    assert!(a.to_jsonl);
    assert_eq!(a.file, Some(PathBuf::from("big.json")));
    assert!(p(&["--to-jsonl=yes"]).is_err());
}

/// `--lens` is validated here, against the same table the reader resolves
/// it from: an unknown name can never reach a file.
#[test]
fn lens_names_are_checked_against_the_registry() {
    let a = p(&["--lens", "agent", "run.jsonl"]).unwrap();
    assert_eq!(a.lens.as_deref(), Some("agent"));
    assert!(!a.lens_list);
    assert_eq!(p(&["--lens=agent"]).unwrap().lens.as_deref(), Some("agent"));
    assert_eq!(p(&[]).unwrap().lens, None);

    let err = p(&["--lens", "opencode"]).unwrap_err();
    assert!(matches!(err, CliError::BadValue { .. }));
    let text = err.to_string();
    assert!(text.contains("opencode"), "{text}");
    assert!(text.contains("agent"), "the error lists what there is: {text}");
    assert!(text.contains("list"), "{text}");
    assert!(p(&["--lens"]).is_err(), "the flag takes a value");
}

/// `--lens list` is a question the caller answers by printing and exiting
/// 2; the parser only records that it was asked.
#[test]
fn lens_list_is_carried_not_printed() {
    let a = p(&["--lens", "list"]).unwrap();
    assert!(a.lens_list);
    assert_eq!(a.lens, None);
    assert!(p(&["--lens=list"]).unwrap().lens_list);
}

#[test]
fn help_and_version_text_are_useful() {
    let h = help_text();
    for needle in [
        "--index", "--no-alt", "--plain", "--width", "--toc", "--to-jsonl", "-V", "--format",
        "--delim", "--lens", "agent", "--no-browser", "http", "mailto",
    ] {
        assert!(h.contains(needle), "help missing {needle}");
    }
    assert!(version_text().starts_with("tread "));
}
