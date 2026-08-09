//! Workspace packages: `@ww/ui/utils/locale-slugs` and its kind.
//!
//! In a monorepo a package's own code is imported by *name*, not by path, and
//! nothing in `tsconfig.json` says where that name lives — the workspace does.
//! Without this, an import of a sibling package resolves to nothing and the
//! reader looks broken in exactly the place a reader is most useful: the seam
//! between two packages.
//!
//! Three managers, one idea. pnpm lists members in `pnpm-workspace.yaml`, npm,
//! yarn and bun in `package.json`'s `workspaces`; turbo declares none of its
//! own and rides on whichever is there. The member list is globs, each
//! expanding to directories whose `package.json` supplies the name.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use crate::json::parse::parse_str;
use crate::json::value::Value;

/// What a workspace knows: where each package by name lives.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    packages: Vec<(String, PathBuf)>,
}

/// The filesystem, as this module needs it — so every rule below is testable
/// without a tree on disk.
pub trait Files {
    fn read(&self, path: &Path) -> Option<String>;
    /// Directory entries, names only.
    fn list(&self, dir: &Path) -> Vec<String>;
}

impl Workspace {
    pub fn none() -> Workspace {
        Workspace::default()
    }

    /// Find the workspace above `file` and index its members.
    pub fn load(file: &Path, fs: &dyn Files) -> Workspace {
        let mut at = file.parent();
        while let Some(dir) = at {
            if dir.file_name().map(|n| n == "node_modules").unwrap_or(false) {
                return Workspace::none();
            }
            if let Some(globs) = members(dir, fs) {
                return Workspace {
                    packages: index(dir, &globs, fs),
                };
            }
            at = dir.parent();
        }
        Workspace::none()
    }

    /// Where `spec` points, given the file doing the importing.
    ///
    /// `@scope/pkg/sub/path` splits into the package — two segments when
    /// scoped, one otherwise — and a subpath the package's `exports` map is
    /// asked about.
    pub fn resolve(&self, spec: &str, fs: &dyn Files) -> Option<PathBuf> {
        let (name, sub) = split(spec);
        let dir = self
            .packages
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, d)| d.clone())?;
        let manifest = parse_str(&fs.read(&dir.join("package.json"))?).ok()?;
        let key = match sub.is_empty() {
            true => String::from("."),
            false => format!("./{sub}"),
        };
        if let Some(target) = exports(&manifest, &key) {
            return Some(join(&dir, &target));
        }
        // No `exports`, or nothing matched: fall back to the entry fields for
        // the package itself, and to the plain path for a subpath.
        if sub.is_empty() {
            for field in ["types", "module", "main"] {
                if let Some(t) = manifest.get(field).and_then(Value::as_str) {
                    return Some(join(&dir, t));
                }
            }
            // A package that names no entry point at all: its directory is the
            // honest answer, and the reader can list a directory.
            return Some(dir);
        }
        Some(dir.join(sub))
    }
}

/// `@scope/pkg/a/b` -> `("@scope/pkg", "a/b")`; `pkg/a` -> `("pkg", "a")`.
fn split(spec: &str) -> (String, String) {
    let parts: Vec<&str> = spec.splitn(4, '/').collect();
    let take = match spec.starts_with('@') {
        true => 2,
        false => 1,
    };
    let name = parts.iter().take(take).copied().collect::<Vec<_>>().join("/");
    let sub = parts
        .iter()
        .skip(take)
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    (name, sub)
}

/// The `exports` entry for `key`, resolved through conditions and wildcards.
fn exports(manifest: &Value, key: &str) -> Option<String> {
    let map = manifest.get("exports")?;
    // `"exports": "./index.js"` — the whole package, one file.
    if let Some(s) = map.as_str() {
        return (key == ".").then(|| s.to_string());
    }
    let members = map.as_object()?;
    // An exact key wins over a pattern, as it does for TypeScript's `paths`.
    if let Some(m) = members.iter().find(|m| m.key == key) {
        return condition(&m.value);
    }
    let mut best: Option<(usize, String)> = None;
    for m in members {
        let Some((head, tail)) = m.key.split_once('*') else {
            continue;
        };
        if !key.starts_with(head) || !key.ends_with(tail) || key.len() < head.len() + tail.len() {
            continue;
        }
        let star = &key[head.len()..key.len() - tail.len()];
        let Some(target) = condition(&m.value) else {
            continue;
        };
        let rank = m.key.len() - 1;
        if best.as_ref().map(|(r, _)| rank > *r).unwrap_or(true) {
            best = Some((rank, target.replacen('*', star, 1)));
        }
    }
    best.map(|(_, t)| t)
}

/// A target that may be a string or a conditions object.
///
/// Preference is `types` first — it points at the TypeScript source, which is
/// what a reader wants — then the module forms, then whatever `default` says.
/// A build output is the last thing worth opening.
fn condition(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    let members = value.as_object()?;
    for want in ["types", "source", "import", "require", "node", "default"] {
        if let Some(m) = members.iter().find(|m| m.key == want) {
            if let Some(found) = condition(&m.value) {
                return Some(found);
            }
        }
    }
    None
}

/// The member globs declared in `dir`, if it is a workspace root.
fn members(dir: &Path, fs: &dyn Files) -> Option<Vec<String>> {
    // pnpm keeps them in YAML. Only the one shape is read — a `packages:` key
    // and a list of quoted globs — which is all the file ever holds; a YAML
    // parser for this would be a dependency to read six lines.
    if let Some(text) = fs.read(&dir.join("pnpm-workspace.yaml")) {
        let globs = yaml_list(&text, "packages");
        if !globs.is_empty() {
            return Some(globs);
        }
    }
    let manifest = parse_str(&fs.read(&dir.join("package.json"))?).ok()?;
    let ws = manifest.get("workspaces")?;
    // `workspaces` is an array, or an object holding one.
    let array = ws
        .as_array()
        .or_else(|| ws.get("packages").and_then(Value::as_array))?;
    let globs: Vec<String> = array
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    (!globs.is_empty()).then_some(globs)
}

/// The quoted or bare entries of a `key:` list in a small YAML file.
fn yaml_list(text: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('-') {
            if inside {
                let v = rest.trim().trim_matches(|c| c == '"' || c == '\'');
                if !v.is_empty() {
                    out.push(v.to_string());
                }
            }
            continue;
        }
        // Any other unindented key ends the list.
        inside = trimmed.starts_with(key) && trimmed[key.len()..].starts_with(':');
    }
    out
}

/// Expand the globs and read each member's name.
fn index(root: &Path, globs: &[String], fs: &dyn Files) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for g in globs {
        for dir in expand(root, g, fs) {
            let Some(text) = fs.read(&dir.join("package.json")) else {
                continue;
            };
            let Ok(manifest) = parse_str(&text) else {
                continue;
            };
            if let Some(name) = manifest.get("name").and_then(Value::as_str) {
                out.push((name.to_string(), dir));
            }
        }
    }
    out
}

/// Directories matching a workspace glob. `*` matches one path segment; `**`
/// is accepted and treated as `*`, which is what the shapes in the wild need.
fn expand(root: &Path, glob: &str, fs: &dyn Files) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    for part in glob.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        let mut next = Vec::new();
        for d in &dirs {
            match part.contains('*') {
                true => next.extend(
                    fs.list(d)
                        .into_iter()
                        // A dependency tree is not a workspace member.
                        .filter(|n| n != "node_modules" && !n.starts_with('.'))
                        .map(|n| d.join(n)),
                ),
                false => next.push(d.join(part)),
            }
        }
        dirs = next;
        // A workspace has tens of members, not thousands; this is a guard
        // against a glob like `**/**` walking a whole disk.
        if dirs.len() > 512 {
            dirs.truncate(512);
        }
    }
    dirs
}

/// Join a `./`-prefixed target onto a package directory.
fn join(dir: &Path, target: &str) -> PathBuf {
    let mut out = dir.to_path_buf();
    for part in target.split('/') {
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

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
