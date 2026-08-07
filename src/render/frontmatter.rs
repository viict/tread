//! The `---` metadata block, rendered as the document's masthead.
//!
//! In a documentation corpus this is where the status, the owner and the
//! cross-references live, so it is the first thing a reader wants and the last
//! thing that should be thrown away. Laid out as an aligned key/value column:
//! dim labels, values in a column of their own, list values stacked under
//! their key, and a rule closing it off from the document.
//!
//! Two values are treated as more than text. A `status` is coloured by what it
//! says — live, in flight, or historical — because it is the one field a reader
//! checks before anything else. A value that looks like a path to a markdown
//! document becomes a real link, so the cross-references a corpus keeps here
//! are reachable with `n` and `Enter` rather than being something to copy out
//! and open by hand.
#![deny(unsafe_code)]

use super::block::{Ctx, Pfx};
use super::{str_width, HeadingLine, LineKind, Span};
use crate::term::Style;
use crate::md::ast::{Field, FieldValue};
use crate::theme;

/// Longest label we will align to. A pathological key should indent the values
/// off the screen no more than a pathological list nesting should.
const MAX_LABEL: usize = 18;

/// Columns between the label and its value.
const GAP: usize = 2;

/// Between summary parts.
const SEP: &str = "  ·  ";

/// Fold id of the metadata block. Shared with the source, which starts it
/// closed.
pub const METADATA_ID: &str = "metadata";

pub fn render(ctx: &mut Ctx, fields: &[Field], source_line: usize, pfx: &Pfx) {
    if fields.is_empty() {
        return;
    }
    let label_w = fields
        .iter()
        .map(|f| str_width(&f.key))
        .max()
        .unwrap_or(0)
        .min(MAX_LABEL);

    // The block leads with a one-line summary, and that row is the fold handle.
    // Closed — the default — it is all you see: the status, the short scalars,
    // and a count for each list. That is the orientation a reader wants before
    // reading, in one line rather than the ten a `related:` list can run to.
    let (summary, plain) = summary_row(fields);
    ctx.emit(
        summary,
        LineKind::Paragraph,
        source_line,
        false,
        Some(HeadingLine {
            level: 1,
            id: METADATA_ID.to_string(),
            text: plain,
            summarised: true,
        }),
    );

    for field in fields {
        match &field.value {
            FieldValue::Scalar(v) => {
                let spans = row(&field.key, v, label_w, field.key == "status");
                ctx.line(spans, LineKind::Paragraph, source_line);
            }
            FieldValue::List(items) => {
                for (i, item) in items.iter().enumerate() {
                    // Only the first line of a list carries the label; the rest
                    // align under it, so the key reads as one thing with many
                    // values rather than as many repeated keys.
                    let key = if i == 0 { field.key.as_str() } else { "" };
                    let spans = row(key, item, label_w, false);
                    ctx.line(spans, LineKind::Paragraph, source_line);
                }
                if items.is_empty() {
                    let spans = row(&field.key, "—", label_w, false);
                    ctx.line(spans, LineKind::Paragraph, source_line);
                }
            }
        }
    }
    rule(ctx, source_line, pfx);
}

/// The fold handle: `Active · viict · 2026-07-07 · 7 related`.
///
/// Scalars appear as their values — a status, an owner and a date say what they
/// are without being labelled. Lists appear as `N key`, because their contents
/// are what the expanded block is for and their *number* is what you want at a
/// glance. The key is used verbatim rather than pluralised: the corpus already
/// names them in the plural (`deciders`, `notes`), and "1 deciders" is a
/// smaller wrong than a pluraliser guessing at English.
///
/// Returns the styled spans and the same text unstyled, for the outline.
fn summary_row(fields: &[Field]) -> (Vec<Span>, String) {
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    for field in fields {
        let (text, style) = match &field.value {
            FieldValue::Scalar(v) if v.is_empty() => continue,
            FieldValue::Scalar(v) => match field.key == "status" {
                true => (v.clone(), theme::status_of(v)),
                false => (v.clone(), theme::muted()),
            },
            FieldValue::List(items) if items.is_empty() => continue,
            FieldValue::List(items) => {
                (format!("{} {}", items.len(), count_label(&field.key, items.len())), theme::muted())
            }
        };
        if !plain.is_empty() {
            spans.push(Span::new(SEP, theme::rule()));
            plain.push_str(SEP);
        }
        plain.push_str(&text);
        spans.push(Span::new(text, style));
    }
    if spans.is_empty() {
        spans.push(Span::new("metadata", theme::muted()));
        plain.push_str("metadata");
    }
    (spans, plain)
}

/// `deciders` for many, `decider` for one.
///
/// Only ever drops a trailing `s`, which is all the corpus needs and all a
/// reader would notice. A key that is not a plural (`related`) is left alone,
/// because "1 relate" would be worse than "1 related".
fn count_label(key: &str, n: usize) -> &str {
    match n == 1 {
        true => key.strip_suffix('s').unwrap_or(key),
        false => key,
    }
}

/// One `label   value` row.
fn row(key: &str, value: &str, label_w: usize, is_status: bool) -> Vec<Span> {
    let mut spans = vec![Span::new(
        format!("{:<w$}{}", key, " ".repeat(GAP), w = label_w),
        theme::muted(),
    )];
    spans.push(value_span(value, is_status));
    spans
}

/// The value itself: a link when it points at a document, a status colour when
/// it is one, plain text otherwise.
///
/// A linked value is coloured by [`crate::render::link_style`], the same function
/// the inline renderer paints `[text](url)` with, so a frontmatter value that
/// turns out to leave the reader (`docs: https://elsewhere/notes.md` satisfies
/// [`looks_like_doc`] as readily as a relative path does) is painted in the
/// external colour rather than in the internal blue. This used to be a hardcoded
/// `theme::link()`, which made the one field a reader would check before
/// following claim it stayed inside the corpus while `Enter` handed it to the
/// system opener — the exact "painted as one thing, opened as another" SPEC.md
/// §Navigation asks the colour to prevent.
fn value_span(value: &str, is_status: bool) -> Span {
    if is_status {
        return Span::new(value.to_string(), theme::status_of(value));
    }
    match looks_like_doc(value) {
        true => Span {
            text: value.to_string(),
            style: super::inline::link_style(Style::new(), value),
            link: Some(value.to_string()),
        },
        false => Span::new(value.to_string(), theme::text()),
    }
}

/// Is this value a path to a markdown document?
///
/// Deliberately narrow: a `.md` suffix and no whitespace. `related:` holds
/// paths, but `notes:` holds prose that may well mention a filename, and
/// turning a sentence into a link because it ends in `.md` would be worse than
/// leaving a path unlinked.
///
/// An `https://host/notes.md` passes, and is meant to: it is a cross-reference
/// like any other and is reachable with `n` and `Enter`. What matters is that it
/// is *painted* as leaving — see [`value_span`].
fn looks_like_doc(value: &str) -> bool {
    !value.contains(char::is_whitespace)
        && !value.contains(char::is_control)
        && value.len() > 3
        && value.to_ascii_lowercase().ends_with(".md")
}

/// The rule that closes the block off from the document.
fn rule(ctx: &mut Ctx, source_line: usize, pfx: &Pfx) {
    let width = ctx.width.saturating_sub(pfx.cont_width()).max(1);
    ctx.line(
        vec![Span::new(
            super::repeat('\u{2500}', width),
            theme::rule(),
        )],
        LineKind::Rule,
        source_line,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression, SPEC.md §Navigation ("coloured apart ... so it is visible
    /// before pressing which links leave"). `value_span` hardcoded
    /// `theme::link()`, so `docs: https://elsewhere/notes.md` — which
    /// `looks_like_doc` accepts as readily as a relative path — was painted in
    /// the *internal* blue while `Enter` handed it to the system opener. The one
    /// place SPEC.md asks for the distinction was the one place it was not made.
    #[test]
    fn a_frontmatter_value_that_leaves_the_reader_is_coloured_as_leaving() {
        let inside = value_span("models/A.md", false);
        let outside = value_span("https://evil.example.com/notes.md", false);
        assert_eq!(inside.link.as_deref(), Some("models/A.md"));
        assert_eq!(outside.link.as_deref(), Some("https://evil.example.com/notes.md"));
        assert_eq!(inside.style.fg, Some(theme::link_fg(false)));
        assert_eq!(outside.style.fg, Some(theme::link_fg(true)));
        assert_ne!(inside.style.fg, outside.style.fg, "the whole point");
        // Both are still links: underlined, and reachable with `n`.
        assert_eq!(inside.style.attrs, outside.style.attrs);
    }

    /// The same colour the inline renderer would give the same URL, because it is
    /// literally the same function — a second copy is how the two drift apart.
    #[test]
    fn a_frontmatter_link_is_coloured_like_an_inline_one() {
        for url in ["models/A.md", "https://x/a.md", "mailto:a@b.md", "javascript:x.md"] {
            assert_eq!(
                value_span(url, false).style,
                super::super::inline::link_style(Style::new(), url),
                "{url}"
            );
        }
    }

    #[test]
    fn only_a_document_path_becomes_a_link() {
        assert!(looks_like_doc("foundations/SOVEREIGNTY.md"));
        assert!(looks_like_doc("a.md"));
        assert!(!looks_like_doc("see notes.md for detail"), "prose is not a link");
        assert!(!looks_like_doc("2026-08-04"));
        assert!(!looks_like_doc(".md"), "a suffix alone is not a path");
        assert!(!looks_like_doc("Active"));
    }
}
