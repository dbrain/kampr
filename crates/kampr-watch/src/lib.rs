//! What the kernel will say without being asked again: a file or directory changed, a process
//! ended. inotify, eventfd and pidfd on Linux; kqueue on macOS. Blocking and thread-free, so the
//! keeper can use it as well as the node; `kampr_core::push` turns it into async change signals.

mod exit;
mod files;

pub use exit::Exit;
pub use files::{Event, Key, Stop, Waited, Watch};
