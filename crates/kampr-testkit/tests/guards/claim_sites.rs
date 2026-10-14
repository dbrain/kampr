use crate::repo;
use std::collections::BTreeMap;

/// Every path that may take a pane's PTY, and why it is allowed to (AGENTS.md rule 3,
/// ADRs 0012, 0013 and 0015). A new entry here is a decision, not a refactor.
const SANCTIONED: &[(&str, usize)] = &[
    ("crates/kampr-herdr/src/control.rs", 1),
    ("crates/kampr-node/src/manage.rs", 1),
    ("crates/kampr-node/src/restore.rs", 1),
    ("crates/kampr-cli/src/resize.rs", 1),
];

fn claims(text: &str) -> usize {
    text.matches("Controller::claim(").count() + text.matches("Controller as ").count()
}

fn controls(text: &str) -> usize {
    text.matches("\"session\", \"control\"").count()
}

#[test]
fn only_the_sanctioned_paths_claim_a_pane() {
    let root = repo::root();
    let found: BTreeMap<String, usize> = repo::files(&root.join("crates"), &["rs"])
        .into_iter()
        .filter(|path| path.components().any(|c| c.as_os_str() == "src"))
        .map(|path| (repo::relative(&path), claims(&repo::read(&path))))
        .filter(|(_, n)| *n > 0)
        .collect();
    let expected: BTreeMap<String, usize> = SANCTIONED
        .iter()
        .map(|(path, n)| (path.to_string(), *n))
        .collect();
    assert_eq!(
        found, expected,
        "a new caller of Controller::claim reshapes a pane"
    );
}

#[test]
fn nothing_but_the_controller_spawns_a_control_child() {
    let root = repo::root();
    let spawners: Vec<String> = repo::files(&root.join("crates"), &["rs"])
        .into_iter()
        .filter(|path| path.components().any(|c| c.as_os_str() == "src"))
        .filter(|path| controls(&repo::read(path)) > 0)
        .map(|path| repo::relative(&path))
        .collect();
    assert_eq!(spawners, vec!["crates/kampr-herdr/src/control.rs"]);
}

#[test]
fn the_scan_sees_a_call_an_alias_and_a_raw_spawn() {
    assert_eq!(
        claims("let c = kampr_herdr::Controller::claim(bin, &s, p, 80, 24).await?;"),
        1
    );
    assert_eq!(claims("use kampr_herdr::Controller as Pty;"), 1);
    assert_eq!(controls(r#".args(["terminal", "session", "control", pane])"#), 1);
}
