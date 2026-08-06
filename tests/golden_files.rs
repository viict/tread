//! Checked-in golden renders (SPEC.md §Testing).
//!
//! Each fixture in `tests/fixtures/` is rendered through the real binary at a
//! set of widths and compared byte-for-byte against `tests/golden/<name>.w<N>
//! .txt`. Goldens hold the **ANSI-stripped** text only, so a palette change
//! cannot break them; style spans are asserted separately in `render::tests`.
//!
//! Regenerate after an intentional layout change:
//!
//! ```sh
//! UPDATE_GOLDEN=1 cargo test --test golden_files
//! ```

mod harness;

use harness::{fixtures_dir, render, strip};
use std::path::PathBuf;

const WIDTHS: [usize; 4] = [40, 80, 120, 200];

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn check(name: &str, width: usize) {
    let src = fixtures_dir().join(format!("{name}.md"));
    let got = strip(&render(&src, &["--width", &width.to_string()]));
    let want_path = golden_dir().join(format!("{name}.w{width}.txt"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden_dir()).expect("create golden dir");
        std::fs::write(&want_path, &got).expect("write golden");
        return;
    }
    let want = std::fs::read_to_string(&want_path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}\nrun `UPDATE_GOLDEN=1 cargo test --test golden_files`",
            want_path.display()
        )
    });
    if got != want {
        panic!(
            "{name} @ width {width} drifted\n--- want ---\n{}\n--- got ---\n{}",
            first_diff(&want, &got),
            preview(&got)
        );
    }
}

/// The first differing line, with its number, so failures are readable.
fn first_diff(want: &str, got: &str) -> String {
    for (i, (w, g)) in want.lines().zip(got.lines()).enumerate() {
        if w != g {
            return format!("line {}: {w:?}\n  got: {g:?}", i + 1);
        }
    }
    format!(
        "line count {} vs {}",
        want.lines().count(),
        got.lines().count()
    )
}

fn preview(s: &str) -> String {
    s.lines().take(40).collect::<Vec<_>>().join("\n")
}

macro_rules! golden {
    ($fn_name:ident, $file:literal) => {
        #[test]
        fn $fn_name() {
            for w in WIDTHS {
                check($file, w);
            }
        }
    };
}

golden!(kitchen_sink, "kitchen-sink");
golden!(unicode, "unicode");
golden!(hostile, "hostile");
golden!(unclosed_fence, "unclosed-fence");
golden!(no_newline, "no-newline");
golden!(bom_invalid_utf8, "bom-invalid-utf8");
golden!(deep_nesting, "deep-nesting");

/// Every fixture on disk must have a golden at every width -- otherwise a new
/// fixture could sit untested because someone forgot the `golden!` line.
#[test]
fn every_fixture_is_covered() {
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(fixtures_dir()).expect("read fixtures") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        for w in WIDTHS {
            let g = golden_dir().join(format!("{stem}.w{w}.txt"));
            if !g.exists() {
                missing.push(g.display().to_string());
            }
        }
    }
    assert!(missing.is_empty(), "uncovered fixtures: {missing:?}");
}
