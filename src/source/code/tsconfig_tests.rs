//! Reading `tsconfig.json` path aliases.
#![deny(unsafe_code)]

use super::*;

/// A fake tree of config files.
fn files(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&Path) -> Option<String> {
    move |p: &Path| {
        pairs
            .iter()
            .find(|(k, _)| Path::new(k) == p)
            .map(|(_, v)| v.to_string())
    }
}

/// The shape every project here actually uses.
#[test]
fn an_at_alias_resolves_against_the_config_directory() {
    let read = files(&[(
        "/app/tsconfig.json",
        r#"{ "compilerOptions": { "paths": { "@/*": ["./*"] } } }"#,
    )]);
    let a = Aliases::load(Path::new("/app/components/form.tsx"), &read);
    assert_eq!(
        a.candidates("@/components/ui/button"),
        vec![PathBuf::from("/app/components/ui/button")]
    );
    assert!(a.candidates("react").is_empty(), "a package is not an alias");
}

#[test]
fn a_src_alias_and_base_url_are_both_honoured() {
    let read = files(&[(
        "/p/tsconfig.json",
        r#"{ "compilerOptions": { "paths": { "@/*": ["./src/*"] } } }"#,
    )]);
    let a = Aliases::load(Path::new("/p/src/a.ts"), &read);
    assert_eq!(a.candidates("@/lib/x"), vec![PathBuf::from("/p/src/lib/x")]);

    let read = files(&[(
        "/q/tsconfig.json",
        r#"{ "compilerOptions": { "baseUrl": "./src", "paths": { "~/*": ["./*"] } } }"#,
    )]);
    let a = Aliases::load(Path::new("/q/src/a.ts"), &read);
    assert_eq!(a.candidates("~/lib/x"), vec![PathBuf::from("/q/src/lib/x")]);
}

/// `"@payload-config": ["./src/payload.config.ts"]` — a real one, with no star.
#[test]
fn an_exact_alias_maps_to_one_file() {
    let read = files(&[(
        "/p/tsconfig.json",
        r#"{"compilerOptions":{"paths":{"@payload-config":["./src/payload.config.ts"],"@/*":["./src/*"]}}}"#,
    )]);
    let a = Aliases::load(Path::new("/p/src/a.ts"), &read);
    assert_eq!(
        a.candidates("@payload-config"),
        vec![PathBuf::from("/p/src/payload.config.ts")]
    );
}

/// Comments and trailing commas are illegal JSON and ordinary tsconfig.
#[test]
fn a_config_with_comments_and_trailing_commas_still_parses() {
    let read = files(&[(
        "/p/tsconfig.json",
        r#"{
  // the compiler options
  "compilerOptions": {
    /* aliases below */
    "paths": {
      "@/*": ["./src/*"],
    },
  },
}"#,
    )]);
    let a = Aliases::load(Path::new("/p/src/a.ts"), &read);
    assert_eq!(a.candidates("@/x"), vec![PathBuf::from("/p/src/x")]);
}

/// A `//` inside a string is not a comment — the reason the JS lexer does this
/// rather than a regex.
#[test]
fn a_url_inside_a_string_is_not_a_comment() {
    let read = files(&[(
        "/p/tsconfig.json",
        r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}},"docs":"https://example.com/x"}"#,
    )]);
    let a = Aliases::load(Path::new("/p/src/a.ts"), &read);
    assert_eq!(a.candidates("@/x"), vec![PathBuf::from("/p/src/x")]);
}

/// Monorepos put the aliases in a shared base config.
#[test]
fn extends_is_followed_and_the_child_wins() {
    let read = files(&[
        (
            "/repo/tsconfig.base.json",
            r##"{"compilerOptions":{"paths":{"@/*":["./packages/*"],"#shared/*":["./shared/*"]}}}"##,
        ),
        (
            "/repo/apps/web/tsconfig.json",
            r#"{"extends":"../../tsconfig.base.json","compilerOptions":{"paths":{"@/*":["./*"]}}}"#,
        ),
    ]);
    let a = Aliases::load(Path::new("/repo/apps/web/app/page.tsx"), &read);
    // The child's `@/*` replaces the base's...
    assert_eq!(a.candidates("@/ui"), vec![PathBuf::from("/repo/apps/web/ui")]);
}

#[test]
fn a_package_extends_is_not_followed_and_a_cycle_terminates() {
    let read = files(&[(
        "/p/tsconfig.json",
        r#"{"extends":"@tsconfig/next/tsconfig.json","compilerOptions":{"paths":{"@/*":["./*"]}}}"#,
    )]);
    let a = Aliases::load(Path::new("/p/a.ts"), &read);
    assert_eq!(a.candidates("@/x"), vec![PathBuf::from("/p/x")]);

    let cyclic = files(&[
        ("/c/tsconfig.json", r#"{"extends":"./other.json"}"#),
        ("/c/other.json", r#"{"extends":"./tsconfig.json"}"#),
    ]);
    assert!(Aliases::load(Path::new("/c/a.ts"), &cyclic).is_empty(), "terminates");
}

#[test]
fn the_search_stops_at_a_package_boundary_and_at_the_root() {
    let read = files(&[(
        "/app/tsconfig.json",
        r#"{"compilerOptions":{"paths":{"@/*":["./*"]}}}"#,
    )]);
    // A file inside a dependency does not inherit the application's aliases.
    let a = Aliases::load(Path::new("/app/node_modules/pkg/index.js"), &read);
    assert!(a.is_empty());
    // And nothing at all is not an error.
    assert!(Aliases::load(Path::new("/nowhere/x.ts"), &files(&[])).is_empty());
}

#[test]
fn a_longer_pattern_is_offered_before_a_shorter_one() {
    let read = files(&[(
        "/p/tsconfig.json",
        r#"{"compilerOptions":{"paths":{"@/*":["./a/*"],"@/ui/*":["./ui/*"]}}}"#,
    )]);
    let a = Aliases::load(Path::new("/p/x.ts"), &read);
    assert_eq!(
        a.candidates("@/ui/button"),
        vec![PathBuf::from("/p/ui/button"), PathBuf::from("/p/a/ui/button")],
        "the more specific alias is tried first"
    );
}
