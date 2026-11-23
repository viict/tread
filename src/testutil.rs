//! Test-only helpers shared by the unit tests.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

/// A unique path under the temp directory, removed when it goes out of scope,
/// also when the test panics. Nothing is created: the test makes a file or a
/// directory there, and either one is removed.
pub struct Scratch(PathBuf);

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if std::fs::remove_dir_all(&self.0).is_err() {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

/// A fresh [`Scratch`]: `tread-<name>-<pid>-<nanos>`, so parallel tests and
/// parallel runs never share one.
pub fn scratch(name: &str) -> Scratch {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = std::env::temp_dir().join(format!("tread-{name}-{}-{nanos}", std::process::id()));
    Scratch(p)
}

#[test]
fn a_scratch_path_removes_a_file_or_a_directory() {
    let file = scratch("t-file");
    std::fs::write(&*file, b"x").unwrap();
    let dir = scratch("t-dir");
    std::fs::create_dir_all(dir.join("a/b")).unwrap();
    let (f, d) = (file.to_path_buf(), dir.to_path_buf());
    drop((file, dir));
    assert!(!f.exists() && !d.exists(), "{f:?} {d:?}");
}
