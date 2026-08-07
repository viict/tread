//! Block parsing: the leaf blocks that are not lists, tables or frontmatter —
//! footnote definitions, HTML blocks, soft breaks, source-line provenance and
//! the empty document.
//!
//! Split out of `tests.rs` to keep both files under the size limit; the helpers
//! come from the parent module.
#![deny(unsafe_code)]

use super::super::super::ast::{inline_text, Block, Inline};
use super::super::parse_document;

fn text_of(b: &Block) -> String {
    match b {
        Block::Heading { content, .. } | Block::Paragraph { content, .. } => inline_text(content),
        _ => String::new(),
    }
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
