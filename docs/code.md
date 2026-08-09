# Reading code

`tread` opens a source file as its comments and declarations, with every body
folded behind its signature (SPEC.md §Code). This document is the contract for
the parts a change touches: how a language is added, and what each one has
actually been proven against.

## The shape

Four layers, and only the first is per-language.

| Layer | Where | What it does |
| --- | --- | --- |
| Lexer | `src/code/<lang>.rs` | Classifies bytes as code, comment or literal, and says whether the file is whole |
| Declarations | `src/code/<lang>_decl.rs` | Recognises what a line declares |
| Line arithmetic | `src/code/decl.rs` | Depth, extents, doc comments, blocks — shared by every brace language |
| The document | `src/source/code/` | Rows, folding, colouring, links |

Everything downstream of the lexer is brace counting, so the lexer is the part
that must be right. A brace inside a string or a comment that is read as code
does not produce a slightly-off outline — it produces a body that swallows the
rest of the file.

## Adding a language

1. `src/code/<lang>.rs` — `lex` and `balanced`, over `scan::Cursor`.
2. `src/code/<lang>_decl.rs` — a `recognise(blanked, raw)` and a `symbols`
   that hands it to `decl::symbols`, unless the language is indentation-
   structured (see Python below).
3. One line in `LANGS` in `src/source/code/mod.rs`, and the extensions in
   `src/source/detect.rs`.
4. Keywords in `src/source/code/paint.rs`, if it should colour.

Nothing else changes: folding, the outline, search, yank and the fold counts
come from the `Source` seam.

## The rules that are not obvious

- **A file that does not lex cleanly gets no outline at all** and opens as raw
  source. A wrong outline hides code; no outline only fails to help.
- **Detection runs on the blanked source**, where every comment and literal is
  spaces. A name that lives *inside* a literal — an import path — must come
  from the raw line, which is why recognisers are handed both.
- **Regions are stated, not inferred.** A block ends where it closes; the prose
  model of "until the next heading" would fold a branch over the statements
  after it (`src/source/fold.rs`).
- **Python is measured in columns.** Blocks are indentation, so it has its own
  walker, and its docstring — the first statement of the body — is pulled up
  into the signature so folding does not hide it.

## What each language has been proven against

Fixtures prove the shapes someone thought of. These are the corpora, and each
is a test that skips unless its environment variable is set.

| Language | Corpus | Result |
| --- | --- | --- |
| Rust | this repository, every `.rs` | all lex balanced; all symbols in range |
| JavaScript / TypeScript | 20,000 real files (`TREAD_JS_CORPUS`) | 19,995 balanced; the 5 are generated bundles |
| Python | the 3.11 standard library, and LangChain (`TREAD_PY_CORPUS`) | 667/667 and 2,539/2,539 parse |
| Java | Spring Framework (`TREAD_JAVA_CORPUS`) | 9,458/9,458 parse, 177,806 symbols |

Import resolution is measured separately with `TREAD_TS_PROJECT`, which reports
what fraction of a project's own imports resolve. It is what caught three
resolution bugs that unit tests could not: an absolute path where a relative one
was needed, a corpus rooted at the wrong directory, and an extension substituted
rather than appended.

## Known gaps

- **Java and Python imports are not followed.** Only Rust (`crate::`, `super::`,
  `self::`, `mod`) and TypeScript (relative, `tsconfig` aliases, workspace
  packages) resolve. Java would need `com.example.Foo` mapped onto a source
  root; Python `from a.b import c` onto the package tree.
- **Nothing is semantic.** Macro-generated items do not exist, `cfg`-ed out code
  is listed, and an identifier is never a link.
- A language's *recognition quality* is not measured by the corpus checks, which
  only prove that nothing was refused and every span is in range. Comparing the
  symbols found against the declarations in the file is a separate, manual
  check — and it is what found the Python bug where a class declared inside a
  method ended its enclosing class, dropping every method after it.
