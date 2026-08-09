//! Finding JS/TS declarations.
#![deny(unsafe_code)]

use super::*;

fn syms(src: &str) -> Vec<Symbol> {
    symbols(src).expect("a balanced file")
}

fn named(src: &str) -> Vec<(String, Kind)> {
    syms(src).into_iter().map(|s| (s.path, s.kind)).collect()
}

#[test]
fn functions_classes_and_types_are_found() {
    let src = "\
export function run(x: number): void {
}
export default class Widget {
}
interface Props {
}
type Alias = string;
enum Colour { Red }
namespace N {
}
";
    assert_eq!(
        named(src),
        vec![
            ("run".into(), Kind::Func),
            ("Widget".into(), Kind::Class),
            ("Props".into(), Kind::Interface),
            ("Alias".into(), Kind::Alias),
            ("Colour".into(), Kind::Type),
            ("N".into(), Kind::Mod),
        ]
    );
}

/// The decision that makes JS awkward: `const` is a value or a function
/// depending on what follows the `=`.
#[test]
fn a_const_bound_to_an_arrow_is_a_function() {
    let src = "\
const total = 42;
const add = (a, b) => a + b;
const run = async () => {
};
const old = function () {
};
const shape = { a: 1 };
export const Comp = ({ x }: Props) => <div>{x}</div>;
";
    assert_eq!(
        named(src),
        vec![
            ("total".into(), Kind::Const),
            ("add".into(), Kind::Func),
            ("run".into(), Kind::Func),
            ("old".into(), Kind::Func),
            ("shape".into(), Kind::Const),
            ("Comp".into(), Kind::Func),
        ]
    );
}

#[test]
fn class_members_are_qualified_by_their_class() {
    let src = "\
export class Store {
  private items = [];
  constructor(x) {
  }
  async load(id: string) {
  }
  get size() {
    return 1;
  }
  static make() {
  }
}
";
    let got = named(src);
    assert_eq!(got[0], ("Store".into(), Kind::Class));
    let names: Vec<&str> = got[1..].iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Store::items",
            "Store::constructor",
            "Store::load",
            "Store::size",
            "Store::make"
        ]
    );
    let depths: Vec<u8> = syms(src)[1..].iter().map(|s| s.depth).collect();
    assert!(depths.iter().all(|d| *d == 1), "members nest under the class");
}

/// The module path lives inside a string literal, which the blanked source
/// replaces with spaces — so this only works because the recognizer is handed
/// the raw line as well. Without that the outline reads `from "        "`.
#[test]
fn an_import_names_the_module_it_pulls_from() {
    let src = "\
import { a, b } from './thing';
import React from 'react';
import '@/styles/global.css';
import { X } from \"@/components/ui/select\";
";
    let got = syms(src);
    assert!(got.iter().all(|s| s.kind == Kind::Import), "{got:?}");
    let names: Vec<&str> = got.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["./thing", "react", "@/styles/global.css", "@/components/ui/select"]
    );
}

#[test]
fn a_jsdoc_block_above_a_declaration_belongs_to_it() {
    let src = "\
/**
 * Does the thing.
 */
export function go() {
}
";
    let s = &syms(src)[0];
    assert_eq!(s.doc, (0, 3), "the whole block");
    assert_eq!(s.sig, (3, 4));
}

#[test]
fn a_nested_function_is_not_listed() {
    let src = "function outer() {\n  function helper() {}\n  const x = () => 1;\n}\n";
    assert_eq!(named(src), vec![("outer".into(), Kind::Func)]);
}

#[test]
fn a_file_that_does_not_lex_has_no_symbols() {
    assert!(symbols("function f() {").is_none());
    assert!(symbols("const s = `open").is_none());
    assert!(symbols("").is_some());
}

/// A keyword inside a string, a comment or a regex is not a declaration.
#[test]
fn keywords_that_are_not_code_are_not_declarations() {
    let src = "\
// function commented() {}
/** function documented() {} */
const re = /class Fake/;
const s = 'function quoted() {}';
function real() {
}
";
    assert_eq!(named(src), vec![("re".into(), Kind::Const), ("s".into(), Kind::Const), ("real".into(), Kind::Func)]);
}
