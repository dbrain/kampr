use crate::sessions::Sessions;
use kampr_core::desk::same_socket;
use kampr_native::NativeProvider;
use std::path::Path;

/// Ops about where a native session sits at the desk, as opposed to the program in it. They go to
/// the herdr pane showing it, under every restriction a herdr pane has (ADR 0015).
pub fn about_the_desk(op: &str) -> bool {
    matches!(op, "focus" | "pane.zoom" | "pane.split")
}

/// The global id of the herdr pane showing `local`, in whichever served herdr session holds it.
pub fn host_pane(sessions: &Sessions, native: &NativeProvider, local: &str) -> Option<String> {
    let session = native.session(local)?;
    session.hosts().into_iter().find_map(|host| {
        let socket = host.herdr_socket?;
        let pane = host.herdr_pane?;
        let node = sessions
            .all()
            .into_iter()
            .find(|s| same_socket(s.herdr.socket(), Path::new(&socket)))?;
        Some(node.global_pane(&pane))
    })
}
