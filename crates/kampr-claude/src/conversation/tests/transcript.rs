//! History is the transcript on disk, found by the Claude session id the conversation reports.

use kampr_core::conversation::{Command, Update};

use super::*;

fn write_transcript(rig: &Rig, id: &str, lines: &[String]) {
    let project = rig.dir.path().join("projects").join("-w");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join(format!("{id}.jsonl")), lines.join("\n") + "\n").unwrap();
}

fn exchange(i: usize) -> [String; 2] {
    [
        format!(
            r#"{{"type":"user","uuid":"u{i}","timestamp":"2025-01-01T00:00:00Z","message":{{"content":"prompt number {i}"}}}}"#
        ),
        format!(
            r#"{{"type":"assistant","uuid":"a{i}","parentUuid":"u{i}","timestamp":"2025-01-01T00:00:01Z","message":{{"content":[{{"type":"text","text":"answer number {i}"}}]}}}}"#
        ),
    ]
}

fn exchanges(n: usize) -> Vec<String> {
    (1..=n).flat_map(exchange).collect()
}

fn text(turn: &kampr_journal::Turn) -> String {
    turn.blocks
        .iter()
        .filter_map(|b| match b {
            kampr_journal::Block::Md { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn page_of(rig: &Rig, f: fn(&Update) -> Option<&kampr_journal::Page>) -> kampr_journal::Page {
    rig.updates()
        .iter()
        .rev()
        .find_map(|u| f(u).cloned())
        .expect("a page for the pane")
}

fn first_page(u: &Update) -> Option<&kampr_journal::Page> {
    match u {
        Update::Page(p) => Some(p),
        _ => None,
    }
}

fn older_page(u: &Update) -> Option<&kampr_journal::Page> {
    match u {
        Update::Older(p) => Some(p),
        _ => None,
    }
}

#[test]
fn a_conversation_opens_on_its_sessions_transcript() {
    let mut rig = Rig::opened(Opts::default());
    write_transcript(&rig, "s1", &exchanges(2));
    rig.machine.opened();
    let page = page_of(&rig, first_page);
    assert!(
        page.turns.iter().any(|t| text(t).contains("answer number 2")),
        "{page:?}"
    );
}

#[test]
fn a_session_with_no_transcript_yet_opens_on_an_empty_page_not_none() {
    let rig = Rig::opened(Opts::default());
    let page = page_of(&rig, first_page);
    assert!(page.turns.is_empty() && !page.more);
}

#[test]
fn older_pages_back_from_the_cursor_and_find_searches() {
    let mut rig = Rig::allowed(Opts::default());
    write_transcript(&rig, "s1", &exchanges(210));
    rig.command(Command::Older { before: None });
    let first = page_of(&rig, older_page);
    assert!(first.more, "there is transcript above the first page");
    let cursor = first.cursor.clone().expect("a cursor");
    let oldest = text(first.turns.first().unwrap());

    rig.clear();
    rig.command(Command::Older { before: Some(cursor) });
    let second = page_of(&rig, older_page);
    assert!(
        second.turns.iter().all(|t| text(t) != oldest),
        "the page above repeats the one below"
    );

    rig.clear();
    rig.command(Command::Find {
        query: "number 105".into(),
        cap: 10,
    });
    let found = rig
        .updates()
        .into_iter()
        .find_map(|u| match u {
            Update::Found { found, .. } => Some(found),
            _ => None,
        })
        .expect("a find");
    assert!(
        found.total >= 1 && found.hits[0].text.contains("105"),
        "{found:?}"
    );
}

#[test]
fn a_transcript_that_appears_after_the_open_is_found_when_asked_for() {
    let mut rig = Rig::allowed(Opts::default());
    rig.command(Command::Older { before: None });
    assert!(
        !rig.updates().iter().any(|u| matches!(u, Update::Older(_))),
        "no file yet, no guess"
    );
    write_transcript(&rig, "s1", &exchanges(2));
    rig.command(Command::Older { before: None });
    assert!(rig.updates().iter().any(|u| matches!(u, Update::Older(_))));
}

// #677: `/clear` gives the next `system/init` a new session id, and the history is that file's.
#[test]
fn a_session_id_the_child_reports_is_the_one_the_transcript_is_looked_for_under() {
    let mut rig = Rig::allowed(Opts::default());
    write_transcript(&rig, "s2", &exchanges(1));
    rig.line(&TURN_INIT.replace("\"s1\"", "\"s2\""));
    rig.command(Command::Find {
        query: "answer number 1".into(),
        cap: 10,
    });
    assert!(
        rig.updates()
            .iter()
            .any(|u| matches!(u, Update::Found { found, .. } if found.total == 1))
    );
}
