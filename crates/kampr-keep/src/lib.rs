mod ctl;
mod dir;
mod keeper;
mod launch;
mod log;
mod notify;
mod procs;
mod sizes;
mod sys;
mod upgrade;

pub use ctl::{borrow, claim_size, close, state};
pub use dir::{
    Ending, Exit, Host, HostGuard, How, Image, KeeperInfo, SessionDir, Size, Spec, attach_holder, list,
    list_tree, make_private_dir, new_id, register_host, root, view_holder,
};
pub use keeper::{HANDOVER, serve};
pub use launch::{Launch, UNIT_PREFIX, spawn, unit_name};
pub use log::{DEFAULT_CAP, SEGMENT, segment_name, segments};
pub use procs::{Proc, ProcState};
pub use sizes::{Replay, Resize, SIZES, SizedReader, size_at, sizes};
pub use upgrade::{stale, upgrade_stale, upgrade_while};

pub fn own_exe() -> std::io::Result<std::path::PathBuf> {
    sys::own_exe()
}
