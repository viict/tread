#![deny(unsafe_code)]
use super::super::ast::{inline_text, Align, Block, FieldValue, Inline, ListKind};
use super::parse_document;

fn text_of(b: &Block) -> String {
    match b {
        Block::Heading { content, .. } | Block::Paragraph { content, .. } => inline_text(content),
        _ => String::new(),
    }
}

fn heading(b: &Block) -> (u8, String, String) {
    match b {
        Block::Heading {
            level, id, content, ..
        } => (*level, inline_text(content), id.clone()),
        other => panic!("not a heading: {:?}", other),
    }
}

#[test]
fn atx_headings_levels_and_closing_hashes() {
    let d = parse_document("# One\n\n### Three ###\n\n####### seven\n\n#no-space");
    assert_eq!(d.blocks.len(), 4);
    assert_eq!(heading(&d.blocks[0]), (1, "One".into(), "one".into()));
    assert_eq!(heading(&d.blocks[1]), (3, "Three".into(), "three".into()));
    assert_eq!(text_of(&d.blocks[2]), "####### seven");
    assert_eq!(text_of(&d.blocks[3]), "#no-space");
}

#[test]
fn setext_beats_thematic_break_after_paragraph() {
    let d = parse_document("Title\n---\n\nBody\n\n---\n");
    assert_eq!(heading(&d.blocks[0]), (2, "Title".into(), "title".into()));
    assert_eq!(text_of(&d.blocks[1]), "Body");
    assert!(matches!(d.blocks[2], Block::ThematicBreak { .. }));
}

#[test]
fn setext_h1_and_source_lines() {
    let d = parse_document("intro\n\nTitle\n===\n");
    assert_eq!(heading(&d.blocks[1]), (1, "Title".into(), "title".into()));
    assert_eq!(d.blocks[0].source_line(), 1);
    assert_eq!(d.blocks[1].source_line(), 3);
}

#[test]
fn duplicate_heading_slugs_are_suffixed() {
    let d = parse_document("## References\n\n## References\n");
    assert_eq!(heading(&d.blocks[0]).2, "references");
    assert_eq!(heading(&d.blocks[1]).2, "references-1");
}

#[test]
fn yaml_frontmatter_is_kept_as_a_block() {
    let src = "---\nstatus: Active\nowner: alice\n---\n\n# Codex Conventions\n";
    let d = parse_document(src);
    assert_eq!(d.blocks.len(), 2, "the metadata and the heading");
    let Block::FrontMatter { fields, .. } = &d.blocks[0] else {
        panic!("expected frontmatter, got {:?}", d.blocks[0]);
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].key, "status");
    assert_eq!(fields[0].value, FieldValue::Scalar("Active".into()));
    assert_eq!(
        heading(&d.blocks[1]),
        (1, "Codex Conventions".into(), "codex-conventions".into())
    );
    assert_eq!(d.blocks[1].source_line(), 6);
}

/// The shapes the corpus actually uses: scalars, `-` lists, and an item long
/// enough to be wrapped across lines.
#[test]
fn frontmatter_reads_scalars_lists_and_wrapped_items() {
    let src = concat!(
        "---\n",
        "status: Draft\n",
        "deciders:\n",
        "  - alice\n",
        "  - bo\n",
        "notes:\n",
        "  - A note that runs on\n",
        "    across two lines.\n",
        "empty:\n",
        "---\n\nbody\n"
    );
    let d = parse_document(src);
    let Block::FrontMatter { fields, .. } = &d.blocks[0] else {
        panic!("expected frontmatter");
    };
    assert_eq!(fields[0].value, FieldValue::Scalar("Draft".into()));
    assert_eq!(
        fields[1].value,
        FieldValue::List(vec!["alice".into(), "bo".into()])
    );
    assert_eq!(
        fields[2].value,
        FieldValue::List(vec!["A note that runs on across two lines.".into()]),
        "a wrapped item is one value, not two"
    );
    assert_eq!(fields[3].value, FieldValue::List(Vec::new()), "`key:` alone");
}

/// An unterminated `---` is a thematic break, not a metadata block that eats
/// the rest of the file.
#[test]
fn an_unclosed_frontmatter_fence_is_not_frontmatter() {
    let d = parse_document("---\nstatus: Active\n\n# Title\n");
    assert!(
        !matches!(d.blocks.first(), Some(Block::FrontMatter { .. })),
        "{:?}",
        d.blocks.first()
    );
    assert!(d.blocks.iter().any(|b| matches!(b, Block::Heading { .. })));
}

/// A `---` that is not at the very top is a thematic break as it always was.
#[test]
fn frontmatter_must_lead_the_document() {
    let d = parse_document("# Title\n\n---\nstatus: Active\n---\n");
    assert!(!matches!(d.blocks.first(), Some(Block::FrontMatter { .. })));
}

#[test]
fn fenced_code_with_info_and_longer_close() {
    let src = "```yaml\nstatus: Draft\nowner: x\n```\n";
    let d = parse_document(src);
    match &d.blocks[0] {
        Block::CodeBlock {
            lang,
            lines,
            fenced,
            source_line,
        } => {
            assert_eq!(lang.as_deref(), Some("yaml"));
            assert_eq!(lines, &["status: Draft", "owner: x"]);
            assert!(fenced);
            assert_eq!(*source_line, 1);
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn nested_fence_inside_longer_fence() {
    let src = "````markdown\n```\ninner\n```\n````\n";
    let d = parse_document(src);
    match &d.blocks[0] {
        Block::CodeBlock { lines, lang, .. } => {
            assert_eq!(lang.as_deref(), Some("markdown"));
            assert_eq!(lines, &["```", "inner", "```"]);
        }
        other => panic!("{:?}", other),
    }
    assert_eq!(d.blocks.len(), 1);
}

#[test]
fn tilde_fence_and_unterminated_fence() {
    let d = parse_document("~~~\na ``` b\n~~~\n");
    match &d.blocks[0] {
        Block::CodeBlock { lines, .. } => assert_eq!(lines, &["a ``` b"]),
        other => panic!("{:?}", other),
    }
    let d = parse_document("```\nno close\n");
    match &d.blocks[0] {
        Block::CodeBlock { lines, .. } => assert_eq!(lines, &["no close"]),
        other => panic!("{:?}", other),
    }
}

#[test]
fn indented_code_block() {
    let d = parse_document("para\n\n    let x = 1;\n    let y = 2;\n\nafter\n");
    match &d.blocks[1] {
        Block::CodeBlock { lines, fenced, .. } => {
            assert!(!fenced);
            assert_eq!(lines, &["let x = 1;", "let y = 2;"]);
        }
        other => panic!("{:?}", other),
    }
    assert_eq!(text_of(&d.blocks[2]), "after");
}

#[test]
fn thematic_breaks_variants() {
    let d = parse_document("---\n\n***\n\n___\n\n- - -\n");
    assert_eq!(d.blocks.len(), 4);
    assert!(d
        .blocks
        .iter()
        .all(|b| matches!(b, Block::ThematicBreak { .. })));
}

#[test]
fn codex_readme_table() {
    let src = "\
| Doc | Status | Owner | Last reviewed |
|---|---|---|---|
| [foundations/BASICS.md](foundations/BASICS.md) — catalog | Active | alice | 2026-06-13 |
| [models/SAMPLE_MODEL.md](models/SAMPLE_MODEL.md) | Active | alice | 2026-06-18 |
";
    let d = parse_document(src);
    match &d.blocks[0] {
        Block::Table {
            align,
            head,
            rows,
            source_line,
        } => {
            assert_eq!(align, &vec![Align::None; 4]);
            assert_eq!(inline_text(&head[0]), "Doc");
            assert_eq!(inline_text(&head[3]), "Last reviewed");
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].len(), 4);
            assert_eq!(inline_text(&rows[1][3]), "2026-06-18");
            assert_eq!(*source_line, 1);
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn table_alignment_ragged_rows_and_escaped_pipes() {
    let src = "\
| Cmd | Desc |
| :--- | ---: |
| `river create [--waf-tier free\\|pro]` |
| a | b | c |
";
    let d = parse_document(src);
    match &d.blocks[0] {
        Block::Table { align, rows, .. } => {
            assert_eq!(align, &vec![Align::Left, Align::Right]);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].len(), 2, "short row padded");
            assert_eq!(inline_text(&rows[0][1]), "");
            assert_eq!(rows[1].len(), 2, "long row truncated");
            assert!(
                inline_text(&rows[0][0]).contains("--waf-tier free|pro"),
                "escaped pipe is unescaped for the inline parser: {:?}",
                inline_text(&rows[0][0])
            );
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn table_interrupts_paragraph_and_ends_at_blank() {
    let src = "lead in\n| a | b |\n|---|---|\n| 1 | 2 |\n\ntail\n";
    let d = parse_document(src);
    assert_eq!(text_of(&d.blocks[0]), "lead in");
    assert!(matches!(d.blocks[1], Block::Table { .. }));
    assert_eq!(text_of(&d.blocks[2]), "tail");
}

#[test]
fn not_a_table_when_arity_mismatches() {
    let d = parse_document("| a | b |\n|---|\n");
    assert!(matches!(d.blocks[0], Block::Paragraph { .. }));
}

#[test]
fn bullet_list_tight_with_nesting() {
    let src = "\
- **Good:** one
- **Bad:** two
  - nested a
  - nested b
- three
";
    let d = parse_document(src);
    match &d.blocks[0] {
        Block::List {
            kind, tight, items, ..
        } => {
            assert_eq!(*kind, ListKind::Bullet);
            assert!(tight);
            assert_eq!(items.len(), 3);
            match &items[1].blocks[1] {
                Block::List { items: sub, .. } => assert_eq!(sub.len(), 2),
                other => panic!("expected nested list, got {:?}", other),
            }
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn loose_list_when_items_are_blank_separated() {
    let d = parse_document("- one\n\n- two\n");
    match &d.blocks[0] {
        Block::List { tight, items, .. } => {
            assert!(!tight);
            assert_eq!(items.len(), 2);
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn ordered_list_keeps_start_and_delimiter_family() {
    let d = parse_document("3. three\n4. four\n");
    match &d.blocks[0] {
        Block::List { kind, items, .. } => {
            assert_eq!(*kind, ListKind::Ordered { start: 3 });
            assert_eq!(items.len(), 2);
        }
        other => panic!("{:?}", other),
    }
    let d = parse_document("1) a\n1) b\n");
    assert!(matches!(
        d.blocks[0],
        Block::List {
            kind: ListKind::Ordered { start: 1 },
            ..
        }
    ));
}

#[test]
fn task_list_checkboxes() {
    let src = "- [x] F1 — resolve platform org id\n- [ ] F2 — rekey secrets\n- plain\n";
    let d = parse_document(src);
    match &d.blocks[0] {
        Block::List { items, .. } => {
            assert_eq!(items[0].task, Some(true));
            assert_eq!(items[1].task, Some(false));
            assert_eq!(items[2].task, None);
            assert_eq!(text_of(&items[1].blocks[0]), "F2 — rekey secrets");
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn code_fence_inside_list_item() {
    let src = "\
- run it:

  ```bash
  cargo test
  ```

- done
";
    let d = parse_document(src);
    match &d.blocks[0] {
        Block::List { items, tight, .. } => {
            assert!(!tight);
            assert_eq!(items.len(), 2);
            match &items[0].blocks[1] {
                Block::CodeBlock { lang, lines, .. } => {
                    assert_eq!(lang.as_deref(), Some("bash"));
                    assert_eq!(lines, &["cargo test"]);
                }
                other => panic!("{:?}", other),
            }
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn list_item_lazy_continuation_and_trailing_blank() {
    let src = "- **Domain crate per concern.** Each service is its own crate\n  with a self-contained build\nand an isolated graph\n\nAfter.\n";
    let d = parse_document(src);
    match &d.blocks[0] {
        Block::List { items, tight, .. } => {
            assert!(tight);
            assert_eq!(items.len(), 1);
            assert!(text_of(&items[0].blocks[0]).ends_with("an isolated graph"));
        }
        other => panic!("{:?}", other),
    }
    assert_eq!(text_of(&d.blocks[1]), "After.");
}

#[test]
fn block_quote_nested_and_lazy() {
    let src = "\
> Successor to `docs/`. The codex is the
> authoritative knowledge base.
lazy tail
>
> > nested
";
    let d = parse_document(src);
    match &d.blocks[0] {
        Block::Quote { blocks, .. } => {
            assert_eq!(blocks.len(), 2);
            assert!(text_of(&blocks[0]).ends_with("lazy tail"));
            match &blocks[1] {
                Block::Quote { blocks: inner, .. } => assert_eq!(text_of(&inner[0]), "nested"),
                other => panic!("{:?}", other),
            }
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn quote_containing_list_and_table() {
    let src = "> - a\n> - b\n>\n> | x | y |\n> |---|---|\n> | 1 | 2 |\n";
    let d = parse_document(src);
    match &d.blocks[0] {
        Block::Quote { blocks, .. } => {
            assert!(matches!(blocks[0], Block::List { .. }));
            assert!(matches!(blocks[1], Block::Table { .. }));
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn link_reference_definitions_are_collected() {
    let src =
        "[Codex]: https://example.com/codex \"The codex\"\n[b]: <models/SAMPLE_MODEL.md>\n\ntext\n";
    let d = parse_document(src);
    assert_eq!(
        d.link_refs.get("codex"),
        Some(&(
            "https://example.com/codex".to_string(),
            Some("The codex".to_string())
        ))
    );
    assert_eq!(
        d.link_refs.get("b"),
        Some(&("models/SAMPLE_MODEL.md".to_string(), None))
    );
    assert_eq!(d.blocks.len(), 1);
}

#[test]
fn ref_like_line_inside_code_fence_is_not_a_definition() {
    let d = parse_document("```\n[nope]: http://x\n```\n");
    assert!(d.link_refs.is_empty());
}

#[test]
fn footnote_definition() {
    let src = "text[^1]\n\n[^1]: The note body\n    continued here\n\nafter\n";
    let d = parse_document(src);
    match &d.blocks[1] {
        Block::FootnoteDef { label, blocks, .. } => {
            assert_eq!(label, "1");
            assert_eq!(text_of(&blocks[0]), "The note body continued here");
        }
        other => panic!("{:?}", other),
    }
    assert_eq!(text_of(&d.blocks[2]), "after");
}

#[test]
fn html_block_and_comment() {
    let d = parse_document("<div class=\"x\">\nbody\n</div>\n\npara\n");
    match &d.blocks[0] {
        Block::Html { lines, .. } => assert_eq!(lines.len(), 3),
        other => panic!("{:?}", other),
    }
    let d = parse_document("<!-- hidden\nnote -->\npara\n");
    match &d.blocks[0] {
        Block::Html { lines, .. } => assert_eq!(lines.len(), 2),
        other => panic!("{:?}", other),
    }
    assert_eq!(text_of(&d.blocks[1]), "para");
}

#[test]
fn paragraph_soft_breaks_preserved_for_inline_parser() {
    let d = parse_document("one\ntwo\n");
    match &d.blocks[0] {
        Block::Paragraph { content, .. } => {
            assert!(content.contains(&Inline::SoftBreak), "{:?}", content);
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn source_lines_survive_containers() {
    let src = "# H\n\n> quoted\n\n- item\n\n  nested para\n";
    let d = parse_document(src);
    assert_eq!(d.blocks[0].source_line(), 1);
    assert_eq!(d.blocks[1].source_line(), 3);
    assert_eq!(d.blocks[2].source_line(), 5);
    match &d.blocks[2] {
        Block::List { items, .. } => {
            assert_eq!(items[0].blocks[0].source_line(), 5);
            assert_eq!(items[0].blocks[1].source_line(), 7);
        }
        other => panic!("{:?}", other),
    }
}

#[test]
fn empty_and_whitespace_documents() {
    assert!(parse_document("").blocks.is_empty());
    assert!(parse_document("\n\n   \n").blocks.is_empty());
    assert!(!parse_document("---\nno close\n").blocks.is_empty());
}
