use std::path::Path;

/// Kills every keeper under a node's native root, and its fleet runs' roots, with its child's
/// process group. A detached keeper outlives the node by design, and a tempdir removed under it
/// does not stop it, so a harness that started one must end it.
pub fn stop_keepers(root: &Path) {
    let fleets = std::fs::read_dir(root.join("fleet"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path());
    for dir in std::iter::once(root.to_path_buf()).chain(fleets) {
        for session in kampr_keep::list(&dir).unwrap_or_default() {
            if let Some(keeper) = session.keeper() {
                unsafe {
                    libc::kill(-(keeper.child as i32), libc::SIGKILL);
                    libc::kill(keeper.keeper as i32, libc::SIGKILL);
                }
            }
        }
    }
}
