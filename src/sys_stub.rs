//! Placeholder `sys` backend for non-unix targets.
//!
//! Keeps the crate compiling — and the public surface of [`crate::sys`]
//! identical — until `sys_windows.rs` lands (see WINDOWS.md). Contains no
//! `unsafe` code, which is why it lives outside `sys.rs`.
#![deny(unsafe_code)]

use super::{Fd, ReadOutcome};

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
