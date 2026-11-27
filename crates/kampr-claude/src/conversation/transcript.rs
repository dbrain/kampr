use kampr_core::conversation::Update;
use kampr_journal::Journal;

use super::machine::Machine;

/// How many turns one page of the transcript holds, opening on and paging back through it.
const PAGE: usize = 200;

/// The session's transcript as a journal: the history the pane opens on, the pages it pages
/// back through and the finds it searches, all through the existing `ClaudeParser`. The file
/// is looked for each time, by the id the session reported last: a new conversation has none
/// until its first turn, and a `/clear` moves it.
fn journal(machine: &Machine) -> Option<kampr_journal::FileJournal> {
    let path = machine.settings.store.find(&machine.st.session_id)?;
    let mut journal = kampr_journal::FileJournal::new(
        path,
        Box::new(kampr_journal::claude::ClaudeParser::default()),
        None,
    );
    let _ = journal.poll();
    Some(journal)
}

pub(super) fn history(machine: &Machine) -> Option<kampr_journal::Page> {
    Some(journal(machine)?.page_before(None, PAGE))
}

pub(super) fn older(machine: &mut Machine, before: Option<&str>) {
    if let Some(page) = journal(machine).map(|j| j.page_before(before, PAGE)) {
        machine.tell(Update::Older(page));
    }
}

pub(super) fn find(machine: &mut Machine, query: String, cap: usize) {
    if let Some(found) = journal(machine).map(|j| j.search(&query, cap)) {
        machine.tell(Update::Found { query, found });
    }
}
