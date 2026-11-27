use kampr_herdr::{Command, Pane};

// muffit's TUI holds the alternate screen for its whole run as a harness does, and herdr has no
// label for it and reports no modes for any pane (#686), so its name is the only signal. `muffit`
// execs into `muffit-tui`, so that is the only name herdr reports for it (#699); a bare `muffit`
// is its line-mode REPL or a one-shot turn, which never take the screen.
const HOLDS_THE_SCREEN: &[&str] = &["muffit-tui"];

/// A pane reporting no ring whose program holds the alternate screen for its whole run. An
/// unnamed pane reporting none is a pager, a pane that has not scrolled yet, or a harness herdr
/// lost the label for, none of which may cost the ring its rows.
pub(super) fn holds_the_screen(pane: &Pane, command: Option<&Command>) -> bool {
    !pane.scrollback_is_safe_to_read()
        && (pane.is_agent() || command.is_some_and(|c| HOLDS_THE_SCREEN.contains(&c.name.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(agent: Option<&str>, ring: u64) -> Pane {
        serde_json::from_value(serde_json::json!({
            "pane_id": "w1:p1",
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "cwd": null,
            "label": null,
            "agent": agent,
            "agent_status": "idle",
            "agent_session": null,
            "scroll": { "offset_from_bottom": 0, "max_offset_from_bottom": ring, "viewport_rows": 24 },
        }))
        .expect("a pane")
    }

    fn running(name: &str) -> Command {
        Command {
            name: name.to_string(),
            line: name.to_string(),
        }
    }

    #[test]
    fn a_harness_or_muffit_with_no_ring_holds_the_screen_and_nothing_else_does() {
        for (agent, command, ring, holds) in [
            (Some("claude"), None, 0, true),
            (Some("claude"), None, 40, false),
            (None, Some("muffit"), 0, false),
            (None, Some("muffit-tui"), 0, true),
            (None, Some("muffit"), 40, false),
            (None, Some("less"), 0, false),
            (None, None, 0, false),
        ] {
            assert_eq!(
                holds_the_screen(&pane(agent, ring), command.map(running).as_ref()),
                holds,
                "{agent:?} {command:?} with {ring} rows behind it"
            );
        }
    }
}
