//! Python: finding the declarations.
//!
//! The one language here that cannot use the shared walk. Python delimits
//! blocks by **indentation**, so brace depth says nothing: every `def` in a
//! file sits at brace depth zero, and a method would be indistinguishable from
//! a free function. Depth and extent are therefore measured in columns.
//!
//! The other inversion is the docstring. Every other language puts a
//! declaration's documentation *above* it, where the shared `doc_above` finds
//! it; Python puts it as the first statement of the body — which is inside the
//! part that folds away. So a docstring is pulled up into the signature rows,
//! and folding a function leaves `def f(x):` with its docstring under it, which
//! is the whole point of the collapsed view.
#![deny(unsafe_code)]

use super::decl::{doc_above, ident, word};
use super::py::{balanced, lex};
use super::scan::blank;
use super::{disambiguate, Kind, Symbol};

/// Tab stop used when measuring indentation.
const TAB: usize = 4;

/// The symbols in `src`, or `None` when it does not lex cleanly.
pub fn symbols(src: &str) -> Option<Vec<Symbol>> {
    let toks = lex(src);
    if !balanced(src, &toks) {
        return None;
    }
    let blanked = blank(src, &toks);
    let lines: Vec<&str> = blanked.lines().collect();
    let raw: Vec<&str> = src.lines().collect();

    let mut out: Vec<Symbol> = Vec::new();
    // The class we are inside: its name, the column it was declared at, and the
    // column its members sit at once the first one has been seen.
    //
    // That third field is what stops a class declared *inside a method* — a
    // `class Joke(BaseModel)` in a test, say — from becoming the container. It
    // sits deeper than the member level, and taking it would end the real class
    // and silently drop every method after it.
    let mut container: Option<(String, usize, Option<usize>)> = None;
    for i in 0..lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            continue;
        }
        let col = indent(line);
        if let Some((_, at, _)) = &container {
            if col <= *at {
                container = None;
            }
        }
        let Some((kind, name)) = recognise(line.trim_start(), raw[i].trim_start()) else {
            continue;
        };
        // Which level is this? Top level, a member of the class we are in, or
        // something nested deeper — which an outline does not want.
        let member = match &mut container {
            Some((_, _, member_col)) => match member_col {
                // The first declaration inside the class sets the level.
                None => {
                    *member_col = Some(col);
                    true
                }
                Some(m) if col == *m => true,
                _ => continue,
            },
            None => {
                if col > 0 {
                    continue; // declared inside a function
                }
                false
            }
        };
        let (sig_end, body_end) = extent(&lines, &raw, i, col, kind);
        let path = match &container {
            Some((c, _, _)) => format!("{c}::{name}"),
            None => name.clone(),
        };
        if kind == Kind::Class {
            container = Some((name.clone(), col, None));
        }
        out.push(Symbol {
            kind,
            name,
            path,
            depth: member as u8,
            doc: doc_above(&raw, i),
            sig: (i, sig_end),
            body: (sig_end, body_end),
        });
    }
    disambiguate(&mut out);
    Some(out)
}

/// Columns of leading whitespace, tabs to the next stop.
fn indent(line: &str) -> usize {
    let mut col = 0usize;
    for c in line.chars() {
        match c {
            ' ' => col += 1,
            '\t' => col += TAB - (col % TAB),
            _ => break,
        }
    }
    col
}

/// What, if anything, this line declares.
fn recognise(line: &str, raw: &str) -> Option<(Kind, String)> {
    // `import a.b` / `from a.b import c` — the module is what a reader follows.
    if let Some(rest) = word(line, "import") {
        return Some((Kind::Import, rest.split(' ').next()?.trim().to_string()));
    }
    if let Some(rest) = word(line, "from") {
        return Some((Kind::Import, rest.split(' ').next()?.trim().to_string()));
    }
    let after_async = word(line, "async").unwrap_or(line);
    if let Some(rest) = word(after_async, "def") {
        return Some((Kind::Func, ident(rest)));
    }
    if let Some(rest) = word(line, "class") {
        return Some((Kind::Class, ident(rest)));
    }
    // A module-level constant: `NAME = …`, upper case by convention. Anything
    // else assigned at the top level is a value a reader scrolls past.
    let name = ident(line);
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()) {
        let rest = raw[name.len()..].trim_start();
        if rest.starts_with('=') || rest.starts_with(':') {
            return Some((Kind::Const, name));
        }
    }
    None
}

/// Where the signature ends and the body does.
///
/// The signature runs to the line whose brackets close and which ends in `:`,
/// so a parameter list spread over five lines is one signature. A docstring
/// immediately after is pulled into the signature rows so it survives folding.
fn extent(lines: &[&str], raw: &[&str], start: usize, col: usize, kind: Kind) -> (usize, usize) {
    // An import has no body at all.
    if kind == Kind::Import || kind == Kind::Const {
        return (start + 1, start + 1);
    }
    let mut depth = 0i32;
    let mut sig_end = start + 1;
    for (n, line) in lines.iter().enumerate().skip(start) {
        for b in line.bytes() {
            match b {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 && line.trim_end().ends_with(':') {
            sig_end = n + 1;
            break;
        }
        sig_end = n + 1;
    }
    let sig_end = docstring_end(raw, sig_end);
    // The body is every following line indented past the declaration; blank
    // lines belong to it, but only if something indented follows them.
    let mut end = sig_end;
    for (n, line) in lines.iter().enumerate().skip(sig_end) {
        if line.trim().is_empty() {
            continue;
        }
        if indent(line) <= col {
            break;
        }
        end = n + 1;
    }
    (sig_end, end.max(sig_end))
}

/// Extend the signature over a docstring beginning at `from`.
///
/// Read from the *raw* lines: blanking replaced the docstring's own text with
/// spaces, so the blanked source cannot tell a docstring from an empty line.
/// Without this the docstring is the first thing folding hides, which is
/// exactly backwards for the one language that documents itself there.
fn docstring_end(raw: &[&str], from: usize) -> usize {
    let Some(line) = raw.get(from) else {
        return from;
    };
    let text = line.trim_start();
    // A docstring may carry the same prefixes any other literal can.
    let body = text.trim_start_matches(|c: char| {
        matches!(c.to_ascii_lowercase(), 'r' | 'b' | 'f' | 'u')
    });
    let Some(quote) = ["\"\"\"", "'''"].into_iter().find(|q| body.starts_with(q)) else {
        return from;
    };
    // A one-line docstring closes on its own line. The opening quote must not
    // be mistaken for the closing one, hence the skip past it.
    if body[quote.len()..].contains(quote) {
        return from + 1;
    }
    for (n, l) in raw.iter().enumerate().skip(from + 1) {
        if l.contains(quote) {
            return n + 1;
        }
    }
    from
}

/// Every foldable suite in `lines`: an `if`, a `for`, a `with`, a `try`.
///
/// Python has no braces to count, so a block is a line ending in `:` and the
/// indented lines under it. `def` and `class` are excluded — a declaration
/// already owns its body, and a second region over the same lines would be two
/// folds for one thing.
///
/// `lines` must be the blanked source, or a `:` inside a string opens a block.
pub fn blocks(lines: &[&str], min: usize) -> Vec<super::decl::Block> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    for (n, line) in lines.iter().enumerate() {
        let opened_at = depth;
        for b in line.bytes() {
            match b {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
        }
        // A suite opens on a line that ends in `:` with nothing left open.
        // What matters is the depth *after* the line: a condition spread over
        // several lines closes its brackets on the `):` that opens the suite,
        // and judging by the depth it started at would skip exactly that line.
        let _ = opened_at;
        if depth > 0 || !line.trim_end().ends_with(':') {
            continue;
        }
        let head = line.trim_start();
        if word(head, "def").is_some()
            || word(head, "class").is_some()
            || word(word(head, "async").unwrap_or(head), "def").is_some()
        {
            continue;
        }
        let col = indent(line);
        let mut end = n;
        for (k, l) in lines.iter().enumerate().skip(n + 1) {
            if l.trim().is_empty() {
                continue;
            }
            if indent(l) <= col {
                break;
            }
            end = k;
        }
        if end > n && end - n >= min {
            out.push((n, n + 1, end + 1));
        }
    }
    out
}

#[cfg(test)]
#[path = "py_decl_tests.rs"]
mod tests;
