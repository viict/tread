//! Unit tests for the inline parser.
#![deny(unsafe_code)]
use super::*;
use crate::md::ast::inline_text;

fn refs() -> LinkRefs {
    LinkRefs::new()
}

fn with(pairs: &[(&str, &str)]) -> LinkRefs {
    let mut m = LinkRefs::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), ((*v).to_string(), None));
    }
    m
}

fn p(s: &str) -> Vec<Inline> {
    parse_inlines(s, &refs())
}

fn t(s: &str) -> Inline {
    Inline::Text(s.to_string())
}

fn code(s: &str) -> Inline {
    Inline::Code(s.to_string())
}

fn link(text: Vec<Inline>, url: &str) -> Inline {
    Inline::Link {
        text,
        url: url.to_string(),
        title: None,
    }
}

// ---------------------------------------------------------------- plain text

#[test]
fn plain_text_is_one_run() {
    assert_eq!(p("hello world"), vec![t("hello world")]);
}

#[test]
fn empty_input_yields_nothing() {
    assert!(p("").is_empty());
}

#[test]
fn newlines_become_soft_breaks() {
    assert_eq!(p("one\ntwo"), vec![t("one"), Inline::SoftBreak, t("two")]);
}

#[test]
fn two_trailing_spaces_make_a_hard_break() {
    assert_eq!(p("one  \ntwo"), vec![t("one"), Inline::HardBreak, t("two")]);
    assert_eq!(p("one\\\ntwo"), vec![t("one"), Inline::HardBreak, t("two")]);
    assert_eq!(p("one \ntwo"), vec![t("one"), Inline::SoftBreak, t("two")]);
}

#[test]
fn soft_break_eats_the_next_lines_indent() {
    assert_eq!(
        p("one\n   two"),
        vec![t("one"), Inline::SoftBreak, t("two")]
    );
}

// ---------------------------------------------------------------- code spans

#[test]
fn code_span_wins_over_emphasis() {
    assert_eq!(p("a `**b**` c"), vec![t("a "), code("**b**"), t(" c")]);
}

#[test]
fn code_span_may_contain_pipes_and_brackets() {
    assert_eq!(
        p("`a | b [c](d)`"),
        vec![Inline::Code("a | b [c](d)".into())]
    );
}

#[test]
fn code_span_run_lengths_must_match() {
    assert_eq!(p("``a ` b``"), vec![code("a ` b")]);
    assert_eq!(p("`` ` ``"), vec![code("`")]);
    assert_eq!(p("a ` b"), vec![t("a ` b")]);
    assert_eq!(p("``unclosed"), vec![t("``unclosed")]);
}

#[test]
fn code_span_strips_one_space_each_side_only() {
    assert_eq!(p("` a `"), vec![code("a")]);
    assert_eq!(p("`  a  `"), vec![code(" a ")]);
    assert_eq!(p("` `"), vec![code(" ")]);
    assert_eq!(p("`a `"), vec![code("a ")]);
}

#[test]
fn code_span_folds_newlines_to_spaces() {
    assert_eq!(p("`a\nb`"), vec![code("a b")]);
}

// ----------------------------------------------------------------- emphasis

#[test]
fn simple_emphasis_and_strong() {
    assert_eq!(p("*a*"), vec![Inline::Emph(vec![t("a")])]);
    assert_eq!(p("_a_"), vec![Inline::Emph(vec![t("a")])]);
    assert_eq!(p("**a**"), vec![Inline::Strong(vec![t("a")])]);
    assert_eq!(p("__a__"), vec![Inline::Strong(vec![t("a")])]);
    assert_eq!(
        p("***a***"),
        vec![Inline::Emph(vec![Inline::Strong(vec![t("a")])])]
    );
}

#[test]
fn strong_containing_emphasis_nests() {
    assert_eq!(
        p("**bold with *italic* inside**"),
        vec![Inline::Strong(vec![
            t("bold with "),
            Inline::Emph(vec![t("italic")]),
            t(" inside"),
        ])]
    );
}

#[test]
fn emphasis_containing_code_and_links() {
    assert_eq!(
        p("*see `x` now*"),
        vec![Inline::Emph(vec![t("see "), code("x"), t(" now")])]
    );
}

#[test]
fn snake_case_identifiers_never_emphasize() {
    for s in [
        "snake_case_identifier",
        "a_b_c",
        "platform_org and waf_event",
        "rollout_in_progress",
        "superseded_by: frontmatter",
    ] {
        assert_eq!(p(s), vec![t(s)], "{s} must stay literal");
    }
}

#[test]
fn intraword_asterisks_do_emphasize() {
    assert_eq!(p("a*b*c"), vec![t("a"), Inline::Emph(vec![t("b")]), t("c")]);
}

#[test]
fn unclosed_emphasis_degrades_to_text() {
    assert_eq!(p("*a"), vec![t("*a")]);
    assert_eq!(p("**a"), vec![t("**a")]);
    // Space on both sides: neither run can open or close, so both stay literal.
    assert_eq!(p("a * b * c"), vec![t("a * b * c")]);
    assert_eq!(inline_text(&p("**a *b")), "**a *b");
}

#[test]
fn leftover_delimiters_spill_as_text() {
    assert_eq!(p("***a*"), vec![t("**"), Inline::Emph(vec![t("a")])]);
    assert_eq!(inline_text(&p("*a***")), "a**");
}

#[test]
fn rule_of_three_keeps_inner_strong() {
    assert_eq!(
        p("*foo**bar**baz*"),
        vec![Inline::Emph(vec![
            t("foo"),
            Inline::Strong(vec![t("bar")]),
            t("baz"),
        ])]
    );
}

#[test]
fn mismatched_delimiter_kinds_do_not_pair() {
    assert_eq!(p("*a_"), vec![t("*a_")]);
    assert_eq!(inline_text(&p("_a*")), "_a*");
}

#[test]
fn strikethrough() {
    assert_eq!(p("~~gone~~"), vec![Inline::Strike(vec![t("gone")])]);
    assert_eq!(
        p("a ~~b **c**~~ d"),
        vec![
            t("a "),
            Inline::Strike(vec![t("b "), Inline::Strong(vec![t("c")])]),
            t(" d"),
        ]
    );
    assert_eq!(p("~~~nope~~~"), vec![t("~~~nope~~~")]);
    assert_eq!(p("a ~~ b"), vec![t("a ~~ b")]);
}

mod ext;
