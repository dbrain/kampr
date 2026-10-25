//! What the live suites need before they can assert anything, and how they fail without it.
//!
//! Four test binaries used to carry their own copy of this with three different wordings, each
//! printing to stderr and returning — which libtest scores as a **pass**. Under the command
//! `CLAUDE.md` prescribes (`cargo test --workspace`) stderr is captured, so a machine with no
//! herdr printed 112 green tests and not one word about herdr.
//!
//! Both functions here panic instead, and they panic *differently*, because the two reasons a
//! live suite cannot start are not the same reason. A herdr that is installed and broken is
//! [#233](../../../docs/03-probe-log.md) — the defect where every socket answer is correct and
//! every pane is blank — and reporting it as "not installed" is how that defect hides.

pub mod alloc;
mod keeper;

pub use keeper::{kampr, keeper};

use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn on_path(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    })
}

/// A process that runs until killed and is named after `path`, ignoring its arguments. A script
/// rather than a copy of `sleep`: ubuntu-26.04's coreutils is one multi-call binary that picks its
/// tool from argv[0], so a copy named `claude` exits at once.
pub fn named_sleeper(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, "#!/bin/sh\nwhile :; do sleep 1; done\n").expect("a named sleeper");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("executable");
}

pub fn herdr_on_path() -> PathBuf {
    match on_path("herdr") {
        Some(path) => path,
        None => panic!(
            "no `herdr` on PATH. This suite drives a real herdr end to end and asserts nothing \
             without one, so it fails rather than passing quietly. Install herdr, or put the \
             build you want to test first on PATH."
        ),
    }
}

pub fn herdr_never_listened(session: &str, socket: &Path, waited: Duration) -> ! {
    panic!(
        "`herdr server --session {session}` was spawned but never opened {} within {waited:?}. \
         herdr is present and not working, which is a different fact from herdr being absent — \
         answering both with the same message is the shape of #233.",
        socket.display()
    )
}
