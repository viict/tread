//! Inline parser tests: links, autolinks, escapes, HTML, adversarial input
//! and lines lifted verbatim from the target corpus.
#![deny(unsafe_code)]
use super::*;

// -------------------------------------------------------------------- links

#[test]
fn inline_link_with_title_and_nested_code() {
    assert_eq!(
        p("[`CONVENTIONS.md`](CONVENTIONS.md)"),
        vec![link(vec![code("CONVENTIONS.md")], "CONVENTIONS.md")]
    );
    assert_eq!(
        p(r#"[a](/u "T")"#),
        vec![Inline::Link {
            text: vec![t("a")],
            url: "/u".into(),
            title: Some("T".into()),
        }]
    );
}

#[test]
fn link_text_may_contain_brackets_and_emphasis() {
    assert_eq!(
        p("[see [inner] **b**](u)"),
        vec![link(
            vec![t("see [inner] "), Inline::Strong(vec![t("b")])],
            "u"
        )]
    );
}

#[test]
fn link_destinations_balance_parens_and_accept_angles() {
    assert_eq!(p("[a](b(c)d)"), vec![link(vec![t("a")], "b(c)d")]);
    assert_eq!(p("[a](<u v>)"), vec![link(vec![t("a")], "u v")]);
    assert_eq!(p("[a]()"), vec![link(vec![t("a")], "")]);
}

#[test]
fn links_do_not_nest() {
    let got = p("[a [b](c) d](e)");
    assert_eq!(inline_text(&got), "a [b](c) d");
    match &got[0] {
        Inline::Link { text, url, .. } => {
            assert_eq!(url, "e");
            assert!(text.iter().all(|i| matches!(i, Inline::Text(_))));
        }
        other => panic!("expected link, got {other:?}"),
    }
}

#[test]
fn reference_links_full_collapsed_and_shortcut() {
    let r = with(&[("codex", "CODEX.md"), ("a b", "/ab")]);
    assert_eq!(
        parse_inlines("[the docs][Codex]", &r),
        vec![link(vec![t("the docs")], "CODEX.md")]
    );
    assert_eq!(
        parse_inlines("[codex][]", &r),
        vec![link(vec![t("codex")], "CODEX.md")]
    );
    assert_eq!(
        parse_inlines("[CODEX]", &r),
        vec![link(vec![t("CODEX")], "CODEX.md")]
    );
    assert_eq!(
        parse_inlines("[A   B]", &r),
        vec![link(vec![t("A   B")], "/ab")]
    );
}

#[test]
fn unknown_reference_stays_literal_but_inner_markup_parses() {
    assert_eq!(
        p("[**x**] and [y][z]"),
        vec![t("["), Inline::Strong(vec![t("x")]), t("] and [y][z]"),]
    );
}

#[test]
fn images() {
    assert_eq!(
        p("![a *b*](i.png)"),
        vec![Inline::Image {
            alt: "a b".into(),
            url: "i.png".into()
        }]
    );
    let r = with(&[("logo", "l.png")]);
    assert_eq!(
        parse_inlines("![logo][]", &r),
        vec![Inline::Image {
            alt: "logo".into(),
            url: "l.png".into()
        }]
    );
    assert_eq!(p("![oops"), vec![t("![oops")]);
}

#[test]
fn footnote_references() {
    assert_eq!(
        p("text[^1] more[^note-a]"),
        vec![
            t("text"),
            Inline::FootnoteRef("1".into()),
            t(" more"),
            Inline::FootnoteRef("note-a".into()),
        ]
    );
    assert_eq!(p("[^ bad]"), vec![t("[^ bad]")]);
}

// ---------------------------------------------------------------- autolinks

#[test]
fn angle_autolinks() {
    assert_eq!(
        p("<https://x.dev/a>"),
        vec![Inline::Autolink("https://x.dev/a".into())]
    );
    assert_eq!(
        p("<mailto:a@b.co>"),
        vec![Inline::Autolink("mailto:a@b.co".into())]
    );
    assert_eq!(
        p("<a@b.co>"),
        vec![Inline::Link {
            text: vec![t("a@b.co")],
            url: "mailto:a@b.co".into(),
            title: None
        }]
    );
}

#[test]
fn bare_urls_are_detected_and_trimmed() {
    assert_eq!(
        p("see https://www.cedarpolicy.com (engine)."),
        vec![
            t("see "),
            Inline::Autolink("https://www.cedarpolicy.com".into()),
            t(" (engine)."),
        ]
    );
    assert_eq!(
        p("(https://stalw.art/docs/ref/object/dkim-signature/)."),
        vec![
            t("("),
            Inline::Autolink("https://stalw.art/docs/ref/object/dkim-signature/".into()),
            t(")."),
        ]
    );
    assert_eq!(
        p("http://10.0.30.10:8081, next"),
        vec![
            Inline::Autolink("http://10.0.30.10:8081".into()),
            t(", next"),
        ]
    );
    assert_eq!(p("`http://x.dev`"), vec![code("http://x.dev")]);
    assert_eq!(
        p("[u](http://x.dev)"),
        vec![link(vec![t("u")], "http://x.dev")]
    );
}

#[test]
fn bare_url_keeps_balanced_parens() {
    assert_eq!(
        p("https://en.wikipedia.org/wiki/A_(b)"),
        vec![Inline::Autolink(
            "https://en.wikipedia.org/wiki/A_(b)".into()
        )]
    );
}

// ------------------------------------------------------------------- escapes

#[test]
fn backslash_escapes_all_ascii_punctuation() {
    assert_eq!(p(r"\*not emph\*"), vec![t("*not emph*")]);
    assert_eq!(p(r"\[not a link\]"), vec![t("[not a link]")]);
    assert_eq!(p(r"a \\ b"), vec![t(r"a \ b")]);
    assert_eq!(p(r"\`x\`"), vec![t("`x`")]);
    assert_eq!(p(r"c:\path"), vec![t(r"c:\path")]);
}

// ---------------------------------------------------------------- inline html

#[test]
fn inline_html_passes_through() {
    assert_eq!(
        p("a <br/> b"),
        vec![t("a "), Inline::Html("<br/>".into()), t(" b")]
    );
    assert_eq!(
        p("<span class=\"x>y\">z</span>"),
        vec![
            Inline::Html("<span class=\"x>y\">".into()),
            t("z"),
            Inline::Html("</span>".into()),
        ]
    );
    assert_eq!(p("<!-- hi -->"), vec![Inline::Html("<!-- hi -->".into())]);
    assert_eq!(p("a < b and 3<4"), vec![t("a < b and 3<4")]);
}

// -------------------------------------------------------------- adversarial

#[test]
fn malformed_input_never_panics() {
    let cases = [
        "[",
        "]",
        "[]",
        "[]()",
        "![",
        "[a](",
        "[a](<",
        "[a][",
        "[a][b",
        "`",
        "```",
        "*",
        "**",
        "***",
        "~",
        "~~",
        "<",
        "<>",
        "<!--",
        "\\",
        "[^",
        "[^]",
        "http://",
        "https://",
        "*[a](b*",
        "**[`x`](y)**",
        "[[[[[[a]]]]]]",
        "*_*_*_*_*_",
        "~~*~~*",
        "a\\",
        "  \n  ",
    ];
    for c in cases {
        let out = parse_inlines(c, &with(&[("a", "u")]));
        let _ = inline_text(&out);
    }
}

#[test]
fn multibyte_input_is_never_sliced_mid_char() {
    let s = "héllo — *wörld* 日本語 🎉 `código` [ünï](ü.md)";
    let got = p(s);
    assert!(got.iter().any(|i| matches!(i, Inline::Link { .. })));
    assert_eq!(inline_text(&p("日本語**太字**です")), "日本語太字です");
    assert_eq!(p("🎉*a*🎉").len(), 3);
}

#[test]
fn deeply_nested_delimiters_terminate() {
    let s = "*".repeat(200);
    assert!(!parse_inlines(&s, &refs()).is_empty());
    let s2 = "*a".repeat(300);
    let _ = parse_inlines(&s2, &refs());
}

// ------------------------------------------------------- real corpus lines

#[test]
fn corpus_readme_blockquote_line() {
    let got = p("knowledge base. Every doc here has a known **status**, a known");
    assert_eq!(
        got,
        vec![
            t("knowledge base. Every doc here has a known "),
            Inline::Strong(vec![t("status")]),
            t(", a known"),
        ]
    );
}

#[test]
fn corpus_readme_table_cell_link() {
    let got = p("[models/SAMPLE_MODEL.md](models/SAMPLE_MODEL.md) — sample architecture (tiers, addressing, sample canonical)");
    assert_eq!(
        got,
        vec![
            link(vec![t("models/SAMPLE_MODEL.md")], "models/SAMPLE_MODEL.md"),
            t(" — sample architecture (tiers, addressing, sample canonical)"),
        ]
    );
}

#[test]
fn corpus_underscore_heavy_link_text_is_not_emphasis() {
    let got = p("[plans/SAMPLE_LONG_PLAN_NAME.md](plans/SAMPLE_LONG_PLAN_NAME.md)");
    assert_eq!(inline_text(&got), "plans/SAMPLE_LONG_PLAN_NAME.md");
}

#[test]
fn corpus_mixed_emphasis_and_code() {
    let got = p("- **Decisions** — dated ADRs, immutable *while Accepted*; superseded");
    assert_eq!(
        got,
        vec![
            t("- "),
            Inline::Strong(vec![t("Decisions")]),
            t(" — dated ADRs, immutable "),
            Inline::Emph(vec![t("while Accepted")]),
            t("; superseded"),
        ]
    );
    let got2 = p("Unpins `overlay`/`block` from a single host");
    assert_eq!(
        got2,
        vec![
            t("Unpins "),
            code("overlay"),
            t("/"),
            code("block"),
            t(" from a single host"),
        ]
    );
}

#[test]
fn corpus_postmortem_link_with_code_in_text() {
    let got = p("[Pulse deployments postmortem — stuck fleet surge / `rollout_in_progress`](decisions/2026-06-21-pulse-deployments-postmortem.md)");
    assert_eq!(
        got,
        vec![link(
            vec![
                t("Pulse deployments postmortem — stuck fleet surge / "),
                code("rollout_in_progress"),
            ],
            "decisions/2026-06-21-pulse-deployments-postmortem.md"
        )]
    );
}

#[test]
fn corpus_code_span_with_angle_placeholder() {
    assert_eq!(
        p("cache_url      = `http://10.0.20.<dnsdist>:8083`"),
        vec![
            t("cache_url      = "),
            code("http://10.0.20.<dnsdist>:8083"),
        ]
    );
}

#[test]
fn corpus_rust_snippet_in_prose() {
    assert_eq!(
        p("`vec![var(Variable::RequestUri)]` (or whichever variable), the right"),
        vec![
            code("vec![var(Variable::RequestUri)]"),
            t(" (or whichever variable), the right"),
        ]
    );
}
