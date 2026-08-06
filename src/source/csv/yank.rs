//! `y` / `Y` / `c` for CSV: source-faithful text, correctly re-quoted
//! (SPEC.md §CSV, "Yank").
//!
//! What must never come back is the *display* form: the padded cell, the
//! `\u{2026}` of a truncated value, the box-drawing bars, the `\u{b7}` standing
//! in for a newline. Everything here therefore works from the parsed fields —
//! the values [`crate::csv::parse`] produced — and re-encodes them, so a cell
//! holding `a,b` or `say "hi"` comes back as something a CSV parser accepts
//! rather than as something that would silently re-split.
//!
//! Quoting is minimal and RFC 4180: a field is quoted only when it has to be,
//! and a literal quote inside a quoted field is doubled.
#![deny(unsafe_code)]

use crate::csv::parse::QUOTE;

/// Encode one field, quoting it only when the delimiter, a quote, a newline or
/// edge whitespace would otherwise change what it means.
pub fn field(value: &str, delim: u8) -> String {
    let d = delim as char;
    let needs = value.chars().any(|c| c == d || c == QUOTE as char || c == '\n' || c == '\r')
        || value.starts_with(' ')
        || value.ends_with(' ');
    if !needs {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push(QUOTE as char);
    for c in value.chars() {
        if c == QUOTE as char {
            out.push(QUOTE as char);
        }
        out.push(c);
    }
    out.push(QUOTE as char);
    out
}

/// Encode one record as a CSV line, terminator included.
pub fn record(fields: &[String], delim: u8) -> String {
    let mut out = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(delim as char);
        }
        out.push_str(&field(f, delim));
    }
    out.push('\n');
    out
}

/// Encode several records.
pub fn records(rows: &[Vec<String>], delim: u8) -> String {
    rows.iter().map(|r| record(r, delim)).collect()
}

/// Encode a single column as a one-field-per-line CSV document.
pub fn column(name: &str, values: &[String], delim: u8) -> String {
    let mut out = record(std::slice::from_ref(&name.to_string()), delim);
    for v in values {
        out.push_str(&field(v, delim));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csv::parse::records as parse_records;

    #[test]
    fn plain_values_are_left_alone() {
        assert_eq!(field("abc", b','), "abc");
        assert_eq!(field("", b','), "");
        assert_eq!(field("a b", b','), "a b");
    }

    #[test]
    fn anything_structural_is_quoted() {
        assert_eq!(field("a,b", b','), "\"a,b\"");
        assert_eq!(field("a,b", b'\t'), "a,b", "not the delimiter here");
        assert_eq!(field("a\tb", b'\t'), "\"a\tb\"");
        assert_eq!(field("say \"hi\"", b','), "\"say \"\"hi\"\"\"");
        assert_eq!(field("two\nlines", b','), "\"two\nlines\"");
        assert_eq!(field(" pad ", b','), "\" pad \"");
    }

    #[test]
    fn a_yanked_row_reparses_to_the_same_fields() {
        let fields: Vec<String> = ["a,b", "say \"hi\"", "two\nlines", " pad ", ""]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let text = record(&fields, b',');
        let back = parse_records(text.as_bytes(), b',');
        assert_eq!(back, vec![fields]);
    }

    #[test]
    fn a_yanked_column_reparses_one_field_per_row() {
        let values: Vec<String> = ["x", "a,b", "q\"q"].iter().map(|s| s.to_string()).collect();
        let text = column("name", &values, b',');
        let back = parse_records(text.as_bytes(), b',');
        assert_eq!(back.len(), 4);
        assert_eq!(back[0], vec!["name".to_string()]);
        assert_eq!(back[2], vec!["a,b".to_string()]);
        assert_eq!(back[3], vec!["q\"q".to_string()]);
    }

    #[test]
    fn several_records_round_trip() {
        let rows = vec![
            vec!["1".to_string(), "a,b".to_string()],
            vec!["2".to_string(), "c".to_string()],
        ];
        assert_eq!(parse_records(records(&rows, b',').as_bytes(), b','), rows);
    }
}
