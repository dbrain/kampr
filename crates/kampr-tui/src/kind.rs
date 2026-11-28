use kampr_core::wire::PaneEntry;

pub fn muffit(pane: &PaneEntry) -> bool {
    pane.provider.as_deref() == Some("muffit")
}

/// A headless Claude's pane id is `c:<keeper>` on its node (`kampr-claude`'s `provider::PREFIX`).
pub fn headless(pane: &PaneEntry) -> bool {
    pane.provider.is_none()
        && pane
            .id
            .split_once('/')
            .is_some_and(|(_, local)| local.starts_with("c:"))
}

/// A muffit session and a headless Claude have no grid: the conversation is the whole pane.
pub fn has_terminal(pane: &PaneEntry) -> bool {
    !muffit(pane) && !headless(pane)
}
