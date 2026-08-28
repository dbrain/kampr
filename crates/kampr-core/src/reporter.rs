//! Writing the name Kampr computed back into herdr, so it draws on the pane's border at the desk.
//!
//! **This is a side effect of viewing and it is off unless an operator turned it on.** ADR 0002's
//! invariant is that looking at a pane changes nothing for the person sitting in front of it, and
//! probe #298 is what that invariant looks like when it is broken: a viewer reshaped somebody's
//! PTY and their screen went wrong with no error anywhere. A title is a far smaller mark than a
//! resize, but it is still a mark, so it is opt-in per node.
//!
//! Everything here exists because of probe #295. `ok` on `pane.report_metadata` means the request
//! was well-formed, not that it landed: a stale `seq` is dropped silently and answered `ok` all
//! the same. So every report is confirmed by reading `pane.get` back, and nothing else counts.

use crate::naming::{Fields, Template};
use crate::provider::PaneInfo;
use kampr_herdr::Herdr;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, warn};

/// The `source` every report goes in under. herdr keeps one record per source and the most recent
/// write wins across all of them (probe #295), so this names Kampr's own row rather than claiming
/// the field.
pub const SOURCE: &str = "kampr";

/// The token the name is reported on as well as the title.
///
/// A title is only ever a pane border; a **token** is the field herdr's own agents sidebar can be
/// told to sort and filter on (probe #296), and the sortable builtins are `agent` and `status`
/// alone. So this is the whole of what makes [`crate::agent_view`] possible, it costs nothing —
/// same call, same record, same `seq` — and it stays invisible at the desk unless the operator's
/// own sidebar `rows` ask for `$kampr`.
pub const TOKEN: &str = "kampr";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reported {
    Applied,
    /// herdr answered `ok` and the pane is showing something else. Either the report was dropped
    /// as stale, or another source wrote after it — both are last-writer-wins, and neither is an
    /// error to retry into a loop.
    NotApplied,
}

#[derive(Debug, Default)]
pub struct Reporter {
    state: Mutex<HashMap<String, PaneState>>,
}

#[derive(Debug, Default)]
struct PaneState {
    seq: u64,
    sent: Option<String>,
    warned: bool,
}

impl Reporter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reports every pane whose name has moved since this node last sent one.
    pub async fn sweep(&self, herdr: &Herdr, template: &Template, panes: &[PaneInfo]) {
        for pane in panes {
            let title = template.render(&Fields::from_info(pane));
            if self.already_sent(&pane.pane_id, &title) {
                continue;
            }
            match self.report(herdr, &pane.pane_id, &title).await {
                Ok(Reported::Applied) => {
                    self.settled(&pane.pane_id, &title, true);
                }
                Ok(Reported::NotApplied) => {
                    if self.settled(&pane.pane_id, &title, false) {
                        warn!(
                            pane = %pane.pane_id,
                            %title,
                            "herdr accepted this pane's name and is showing another source's; \
                             Kampr will not overwrite it"
                        );
                    }
                }
                Err(e) => debug!(pane = %pane.pane_id, error = %e, "could not report this pane's name"),
            }
        }
        self.forget(panes);
    }

    /// One pane, reported and then **read back**, because `ok` is not confirmation (probe #295).
    pub async fn report(&self, herdr: &Herdr, pane_id: &str, title: &str) -> anyhow::Result<Reported> {
        let seq = self.next_seq(pane_id);
        let tokens = BTreeMap::from([(TOKEN.to_string(), title.to_string())]);
        herdr
            .report_metadata(pane_id, SOURCE, title, &tokens, seq)
            .await?;
        let showing = herdr.pane_title(pane_id).await?;
        Ok(match showing.as_deref() == Some(title) {
            true => Reported::Applied,
            false => Reported::NotApplied,
        })
    }

    /// Monotonic per pane, and **seeded from the clock** rather than from zero.
    ///
    /// herdr remembers the last `seq` this source sent for as long as the pane lives, and a node
    /// that restarts does not (probe #295). Counting from one after a restart is therefore a run
    /// of reports that are all silently dropped and all answered `ok` — the exact failure the
    /// read-back exists to catch, arriving on every pane at once. Microseconds since the epoch
    /// cannot go backwards across a restart and are finer than the socket round trip that
    /// separates two reports; the `+1` covers two inside one tick within a single run.
    fn next_seq(&self, pane_id: &str) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_micros() as u64);
        let mut state = self.state.lock().unwrap();
        let pane = state.entry(pane_id.to_string()).or_default();
        pane.seq = now.max(pane.seq + 1);
        pane.seq
    }

    fn already_sent(&self, pane_id: &str, title: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .get(pane_id)
            .is_some_and(|pane| pane.sent.as_deref() == Some(title))
    }

    /// Records what was sent whether or not it landed — a report that lost the field must not be
    /// re-sent every sweep, because that is two sources overwriting each other for ever.
    ///
    /// Returns whether this is the first time this pane has failed, so the warning is said once.
    fn settled(&self, pane_id: &str, title: &str, applied: bool) -> bool {
        let mut state = self.state.lock().unwrap();
        let pane = state.entry(pane_id.to_string()).or_default();
        pane.sent = Some(title.to_string());
        let first = !applied && !pane.warned;
        pane.warned = !applied;
        first
    }

    fn forget(&self, panes: &[PaneInfo]) {
        let live: std::collections::HashSet<&str> = panes.iter().map(|p| p.pane_id.as_str()).collect();
        self.state
            .lock()
            .unwrap()
            .retain(|id, _| live.contains(id.as_str()));
    }
}
