//! Placeholder `sys` backend for targets with no real one yet.
//!
//! Keeps the crate compiling — and the public surface of [`crate::sys`]
//! identical — until a Windows backend lands (see WINDOWS.md). Every entry
//! point reports "no terminal here", which `main.rs` already handles: it is the
//! same path taken when stdout is redirected, so the reader degrades to the
//! non-interactive dump instead of failing. Contains no `unsafe` code.
#![deny(unsafe_code)]

use super::{Fd, ReadOutcome};

/// Nothing to save, because nothing is ever changed.
#[derive(Clone, Copy)]
pub struct SavedTermios;

pub fn install_signal_handlers() {}
pub fn is_tty(_fd: Fd) -> bool {
    false
}
pub fn open_tty() -> Option<Fd> {
    None
}
pub fn tty_fd() -> Option<(Fd, bool)> {
    None
}
pub fn close_fd(_fd: Fd) {}
pub fn winsize() -> Option<(u16, u16)> {
    None
}
pub fn winsize_of(_fd: Fd) -> Option<(u16, u16)> {
    None
}
pub fn set_raw(_fd: Fd) -> Option<SavedTermios> {
    None
}
pub fn restore(_fd: Fd, _saved: &SavedTermios) -> bool {
    false
}
pub fn read_input(_fd: Fd, _buf: &mut [u8]) -> ReadOutcome {
    ReadOutcome::Eof
}
pub fn write_all(_fd: Fd, _buf: &[u8]) -> Result<(), i32> {
    Err(0)
}
