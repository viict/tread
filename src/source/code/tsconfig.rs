//! `tsconfig.json` path aliases.
//!
//! Without this the reader is useless on real TypeScript: an application's
//! imports are overwhelmingly `@/components/…`, not `../../components/…`, and
//! an alias that resolves to nothing is a link that is not there. The mapping
//! lives in `compilerOptions.paths`, so following an import means reading the
//! project's own configuration rather than guessing a convention.
//!
//! Two things make the file awkward and both are handled here: it is JSON
//! *with comments and trailing commas*, which the RFC 8259 parser rejects, and
//! it may `extends` another config — routinely in a monorepo, where the aliases
//! live in the shared base.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use crate::code::scan::Tok;
use crate::code::ts;
use crate::json::parse::parse_str;
use crate::json::value::Value;

/// How deep an `extends` chain is followed before giving up. Configs in the
/// wild are two or three deep; a cycle would otherwise not terminate.
const MAX_EXTENDS: usize = 8;

/// The alias table for one file: patterns and the absolute templates they map
/// to, both possibly containing a single `*`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Aliases {
    entries: Vec<(String, Vec<String>)>,
}

impl Aliases {
    pub fn none() -> Aliases {
        Aliases::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Load the nearest config above `file`.
    ///
    /// `read` returns a file's text, so the whole search is testable without a
    /// tree on disk.
    pub fn load(file: &Path, read: &dyn Fn(&Path) -> Option<String>) -> Aliases {
        let mut at = file.parent();
        while let Some(dir) = at {
            // Never climb out through a package boundary: a dependency's own
            // config says nothing about this file's aliases.
            if dir.file_name().map(|n| n == "node_modules").unwrap_or(false) {
                return Aliases::none();
            }
            for name in ["tsconfig.json", "jsconfig.json"] {
                let p = dir.join(name);
                if let Some(text) = read(&p) {
                    let a = from_config(&p, &text, read, 0);
                    if !a.is_empty() {
                        return a;
                    }
                }
            }
            at = dir.parent();
        }
        Aliases::none()
    }

    /// The paths `spec` could name, most specific pattern first.
    ///
    /// The caller probes them: this cannot know which extension exists, and a
    /// pattern may legitimately offer several candidates.
    pub fn candidates(&self, spec: &str) -> Vec<PathBuf> {
        let mut out: Vec<(usize, PathBuf)> = Vec::new();
        for (pattern, targets) in &self.entries {
            let Some(star) = matched(pattern, spec) else {
                continue;
            };
            // Specificity is how much of the pattern is *literal*: `@/ui/*`
            // beats `@/*`, and an exact pattern beats both. Measuring the whole
            // pattern instead would underflow the moment the captured text is
            // longer than the pattern, which `@/*` matching `@/a/b/c` always is.
            let rank = pattern.chars().filter(|c| *c != '*').count();
            for t in targets {
                out.push((rank, PathBuf::from(t.replacen('*', &star, 1))));
            }
        }
        // Most specific first.
        out.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));
        out.into_iter().map(|(_, p)| p).collect()
    }
}

/// What `*` captured when `pattern` matches `spec`, or `None`.
fn matched(pattern: &str, spec: &str) -> Option<String> {
    match pattern.split_once('*') {
        None => (pattern == spec).then(String::new),
        Some((head, tail)) => {
            if !spec.starts_with(head) || !spec.ends_with(tail) {
                return None;
            }
            let rest = &spec[head.len()..];
            (rest.len() >= tail.len()).then(|| rest[..rest.len() - tail.len()].to_string())
        }
    }
}

/// Read one config, following `extends` first so the child's own `paths` win.
fn from_config(
    path: &Path,
    text: &str,
    read: &dyn Fn(&Path) -> Option<String>,
    depth: usize,
) -> Aliases {
    let Ok(doc) = parse_str(&sanitise(text)) else {
        return Aliases::none();
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    let opts = doc.get("compilerOptions");

    // `extends` first: anything the child declares replaces it.
    let mut out = Aliases::none();
    if depth < MAX_EXTENDS {
        if let Some(rel) = doc.get("extends").and_then(Value::as_str) {
            // Only a relative config is followed. `extends: "@tsconfig/next"`
            // names a package, which lives in `node_modules` and is not the
            // reader's code.
            if rel.starts_with('.') {
                let p = normalise(dir, rel);
                let p = match p.extension().is_some() {
                    true => p,
                    false => p.with_extension("json"),
                };
                if let Some(t) = read(&p) {
                    out = from_config(&p, &t, read, depth + 1);
                }
            }
        }
    }

    // `paths` are relative to `baseUrl` when it is set, and to the config's own
    // directory when it is not — which is what modern TypeScript does and what
    // every config in sight relies on.
    let base = match opts.and_then(|o| o.get("baseUrl")).and_then(Value::as_str) {
        Some(b) => normalise(dir, b),
        None => dir.to_path_buf(),
    };
    let Some(paths) = opts.and_then(|o| o.get("paths")).and_then(Value::as_object) else {
        return out;
    };
    let mut entries: Vec<(String, Vec<String>)> = Vec::new();
    for m in paths {
        let targets: Vec<String> = m
            .value
            .as_array()
            .unwrap_or(&[])
            .iter()
            .filter_map(Value::as_str)
            .map(|t| normalise(&base, t).to_string_lossy().to_string())
            .collect();
        if !targets.is_empty() {
            entries.push((m.key.clone(), targets));
        }
    }
    if !entries.is_empty() {
        out.entries = entries;
    }
    out
}

/// Join a config-relative path, resolving `.` and `..`, keeping any `*`.
fn normalise(dir: &Path, rel: &str) -> PathBuf {
    let mut out = dir.to_path_buf();
    for part in rel.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                out.pop();
            }
            p => out.push(p),
        }
    }
    out
}

/// Turn JSON-with-comments into JSON.
///
/// The *JavaScript* lexer classifies the file — the one that already knows a
/// `//` inside a string is not a comment — and only comment tokens are blanked;
/// strings are copied through, since they are the patterns we came to read.
/// Then trailing commas go. Both are illegal in RFC 8259 and both are
/// everywhere in a real `tsconfig.json`.
fn sanitise(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for s in ts::lex(text) {
        let chunk = &text[s.start..s.end];
        match s.tok {
            Tok::Line { .. } | Tok::Block { .. } => {
                // Newlines survive so a `//` comment does not swallow the line
                // that follows it.
                out.extend(chunk.chars().map(|c| match c {
                    '\n' => '\n',
                    _ => ' ',
                }));
            }
            _ => out.push_str(chunk),
        }
    }
    drop_trailing_commas(&out)
}

/// `[1, 2, ]` -> `[1, 2 ]`. A comma before `}` or `]` is legal in tsconfig and
/// not in JSON.
fn drop_trailing_commas(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        if c == ',' {
            let next = b[i + 1..]
                .iter()
                .find(|c| !c.is_ascii_whitespace())
                .copied();
            if matches!(next, Some(b'}') | Some(b']')) {
                out.push(' ');
                continue;
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
#[path = "tsconfig_tests.rs"]
mod tests;
