//! Shared helpers for the integration tests.
//!
//! Integration tests cannot reach into a binary crate, so everything here
//! drives the built executable (`CARGO_BIN_EXE_tread`) as a subprocess.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Run `tread <args> <path>` and return stdout. Panics on a non-zero exit.
pub fn render(path: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_tread"))
        .args(args)
        .arg(path)
        .env_remove("NO_COLOR")
        .output()
        .expect("run tread");
    assert!(
        out.status.success(),
        "tread {:?} {} exited {:?}: {}",
        args,
        path.display(),
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    // Output is lossy-decoded on purpose: a render must never be able to emit
    // bytes that are not valid UTF-8, and this asserts it by construction.
    String::from_utf8(out.stdout).expect("render produced invalid UTF-8")
}

/// Feed bytes on stdin instead of naming a file.
pub fn render_stdin(body: &[u8], args: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tread"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tread");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(body)
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait tread");
    assert!(out.status.success(), "tread exited {:?}", out.status);
    String::from_utf8(out.stdout).expect("render produced invalid UTF-8")
}

/// Write `body` to a uniquely named temp file and return the path.
pub fn temp_doc(name: &str, body: &[u8]) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "tread-{}-{}-{name}.md",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&p, body).expect("write temp doc");
    p
}

/// Remove CSI and OSC sequences, leaving the visible text.
pub fn strip(s: &str) -> String {
    let mut out = String::new();
    let mut cs = s.chars().peekable();
    while let Some(c) = cs.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match cs.next() {
            // CSI: parameters then a final alphabetic byte.
            Some('[') => {
                for c in cs.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            // OSC: terminated by BEL or ST (ESC \).
            Some(']') => {
                while let Some(c) = cs.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        cs.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip;

    #[test]
    fn strip_removes_csi_and_osc_but_keeps_text() {
        assert_eq!(strip("\x1b[1mbold\x1b[0m"), "bold");
        assert_eq!(strip("\x1b]8;;http://x\x07link\x1b]8;;\x07"), "link");
        assert_eq!(strip("plain"), "plain");
        assert_eq!(strip("\x1b]0;t\x1b\\after"), "after");
    }
}
