//! Resolving an import of a workspace package.
#![deny(unsafe_code)]

use super::*;

/// A fake tree: files with contents, directories inferred from the paths.
struct Tree(&'static [(&'static str, &'static str)]);

impl Files for Tree {
    /// Compared as `Path` values, not as strings: `join` uses the platform
    /// separator, and a string comparison would pass on unix and fail on
    /// Windows for the same tree.
    fn read(&self, path: &Path) -> Option<String> {
        self.0
            .iter()
            .find(|(k, _)| Path::new(k) == path)
            .map(|(_, v)| v.to_string())
    }

    fn list(&self, dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = self
            .0
            .iter()
            .filter_map(|(k, _)| Path::new(k).strip_prefix(dir).ok())
            .filter_map(|rest| rest.components().next())
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

/// The shape this was written for, from a real monorepo.
const PNPM: Tree = Tree(&[
    ("/repo/pnpm-workspace.yaml", "packages:\n  - \"apps/*\"\n  - \"packages/*\"\n"),
    ("/repo/apps/client/package.json", r#"{"name":"@ww/client"}"#),
    (
        "/repo/packages/ui/package.json",
        r#"{"name":"@ww/ui","exports":{"./utils/locale-slugs":"./src/utils/locale-slugs.ts","./title":"./src/title/index.tsx"}}"#,
    ),
    ("/repo/packages/ui/src/utils/locale-slugs.ts", ""),
]);

#[test]
fn a_workspace_package_resolves_through_its_exports() {
    let w = Workspace::load(Path::new("/repo/apps/client/src/lib/locales.ts"), &PNPM);

    assert_eq!(
        w.resolve("@ww/ui/utils/locale-slugs", &PNPM),
        Some(PathBuf::from("/repo/packages/ui/src/utils/locale-slugs.ts")),
        "the import that started this"
    );
    assert_eq!(
        w.resolve("@ww/ui/title", &PNPM),
        Some(PathBuf::from("/repo/packages/ui/src/title/index.tsx"))
    );
    // A package that is not a member is not ours to follow.
    assert_eq!(w.resolve("react", &PNPM), None);
    assert_eq!(w.resolve("@ww/nope/x", &PNPM), None);
}

#[test]
fn npm_yarn_and_bun_declare_members_in_the_manifest() {
    let t = Tree(&[
        ("/r/package.json", r#"{"workspaces":["packages/*"]}"#),
        ("/r/packages/core/package.json", r#"{"name":"core","main":"./index.js"}"#),
    ]);
    let w = Workspace::load(Path::new("/r/packages/app/a.ts"), &t);
    assert_eq!(w.resolve("core", &t), Some(PathBuf::from("/r/packages/core/index.js")));

    // yarn's object form.
    let t2 = Tree(&[
        ("/r/package.json", r#"{"workspaces":{"packages":["libs/*"]}}"#),
        ("/r/libs/x/package.json", r#"{"name":"x","types":"./x.d.ts"}"#),
    ]);
    let w2 = Workspace::load(Path::new("/r/libs/y/a.ts"), &t2);
    assert_eq!(w2.resolve("x", &t2), Some(PathBuf::from("/r/libs/x/x.d.ts")));
}

/// `exports` may map a wildcard, and may hold conditions rather than a string.
#[test]
fn exports_wildcards_and_conditions_are_honoured() {
    let t = Tree(&[
        ("/r/pnpm-workspace.yaml", "packages:\n  - \"pkgs/*\"\n"),
        (
            "/r/pkgs/ui/package.json",
            r#"{"name":"ui","exports":{"./*":"./src/*.ts","./special":{"types":"./src/special.d.ts","default":"./dist/special.js"}}}"#,
        ),
    ]);
    let w = Workspace::load(Path::new("/r/pkgs/app/a.ts"), &t);
    assert_eq!(w.resolve("ui/button", &t), Some(PathBuf::from("/r/pkgs/ui/src/button.ts")));
    // `types` wins: it points at the source, which is what a reader wants —
    // a build output is the last thing worth opening.
    assert_eq!(
        w.resolve("ui/special", &t),
        Some(PathBuf::from("/r/pkgs/ui/src/special.d.ts"))
    );
}

#[test]
fn an_exact_export_beats_a_wildcard() {
    let t = Tree(&[
        ("/r/package.json", r#"{"workspaces":["p/*"]}"#),
        (
            "/r/p/ui/package.json",
            r#"{"name":"ui","exports":{"./*":"./src/*.ts","./sb":"./src/sb/index.ts"}}"#,
        ),
    ]);
    let w = Workspace::load(Path::new("/r/p/app/a.ts"), &t);
    assert_eq!(w.resolve("ui/sb", &t), Some(PathBuf::from("/r/p/ui/src/sb/index.ts")));
}

/// A package with no `exports` at all: the subpath is the path.
#[test]
fn a_package_without_exports_falls_back_to_its_layout() {
    let t = Tree(&[
        ("/r/package.json", r#"{"workspaces":["p/*"]}"#),
        ("/r/p/lib/package.json", r#"{"name":"lib"}"#),
    ]);
    let w = Workspace::load(Path::new("/r/p/app/a.ts"), &t);
    assert_eq!(w.resolve("lib/util", &t), Some(PathBuf::from("/r/p/lib/util")));
}

#[test]
fn a_file_inside_a_dependency_has_no_workspace() {
    let t = Tree(&[
        ("/r/package.json", r#"{"workspaces":["p/*"]}"#),
        ("/r/p/ui/package.json", r#"{"name":"ui"}"#),
    ]);
    // Climbing out of `node_modules` would hand a dependency the application's
    // package map.
    let w = Workspace::load(Path::new("/r/node_modules/dep/index.js"), &t);
    assert_eq!(w.resolve("ui", &t), None, "a dependency gets no package map");
    assert_eq!(Workspace::load(Path::new("/nowhere/a.ts"), &t).resolve("ui", &t), None);
}

#[test]
fn the_yaml_reader_takes_only_the_packages_list() {
    let t = Tree(&[
        (
            "/r/pnpm-workspace.yaml",
            "# a comment\npackages:\n  - \"apps/*\"\n  - 'packages/*'\nonlyBuiltDependencies:\n  - esbuild\n",
        ),
        ("/r/apps/a/package.json", r#"{"name":"a"}"#),
        ("/r/packages/b/package.json", r#"{"name":"b"}"#),
    ]);
    let w = Workspace::load(Path::new("/r/apps/a/x.ts"), &t);
    // `esbuild` belongs to the next key, not to `packages`.
    assert_eq!(w.resolve("b", &t), Some(PathBuf::from("/r/packages/b")));
    assert_eq!(w.resolve("esbuild", &t), None);
}
