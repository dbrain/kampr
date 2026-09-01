//! The input router, at the level the operator's fingers meet it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kampr_tui::input::{Outcome, Router};
use kampr_tui::keymap::{Action, Mode};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn press(router: &mut Router, keys: &[KeyEvent]) -> Vec<Outcome> {
    keys.iter().map(|k| router.key(*k)).collect()
}

#[test]
fn ctrl_b_ctrl_b_sends_one_literal_and_not_a_binding() {
    let mut router = Router::default();
    let out = press(&mut router, &[ctrl('b'), ctrl('b')]);
    assert_eq!(
        out,
        vec![Outcome::Redrew, Outcome::ToPane("\u{2}".into())],
        "cat -v printed ^B once, so the second prefix is a byte and nothing else (#290)"
    );
    assert_eq!(router.mode(), Mode::Pane, "and the prefix is spent");

    // A third one is a fresh prefix, not a second literal.
    assert_eq!(router.key(ctrl('b')), Outcome::Redrew);
    assert_eq!(router.mode(), Mode::Prefix);
}

#[test]
fn copy_mode_is_entered_by_prefix_bracket_and_stays_until_esc() {
    let mut router = Router::default();
    press(&mut router, &[ctrl('b'), ch('[')]);
    assert_eq!(router.mode(), Mode::Copy);

    // A one-shot prefix router leaves after the first key. This one does not: every key below
    // is copy mode's, none of them reach the pane, and no prefix is involved (#290).
    assert_eq!(
        router.key(ch('j')),
        Outcome::Do(Action::Move(kampr_tui::keymap::Dir::Down))
    );
    assert_eq!(router.mode(), Mode::Copy);
    assert_eq!(router.key(ch('w')), Outcome::Do(Action::WordNext));
    assert_eq!(router.mode(), Mode::Copy);
    assert_eq!(
        router.key(ch('Z')),
        Outcome::Nothing,
        "a modal keymap swallows the keyboard; an unbound key is not the pane's"
    );
    assert_eq!(router.mode(), Mode::Copy);

    assert_eq!(router.key(key(KeyCode::Esc)), Outcome::Redrew);
    assert_eq!(router.mode(), Mode::Pane);
    assert_eq!(router.key(ch('j')), Outcome::ToPane("j".into()));
}

#[test]
fn resize_mode_is_modal_too_and_q_closes_it() {
    let mut router = Router::default();
    press(&mut router, &[ctrl('b'), ch('r')]);
    assert_eq!(router.mode(), Mode::Resize);
    assert_eq!(router.key(ch('l')), Outcome::Do(Action::Wider));
    assert_eq!(router.mode(), Mode::Resize);
    router.key(ch('q'));
    assert_eq!(router.mode(), Mode::Pane);
}

#[test]
fn a_workspace_picker_opens_navigate_mode_which_takes_no_prefix() {
    let mut router = Router::default();
    press(&mut router, &[ctrl('b'), ch('w')]);
    assert_eq!(router.mode(), Mode::Navigate);
    assert_eq!(
        router.key(ch('j')),
        Outcome::Do(Action::Move(kampr_tui::keymap::Dir::Down))
    );
    assert_eq!(router.key(ch('3')), Outcome::Do(Action::SwitchWorkspace(3)));
    assert_eq!(
        router.key(key(KeyCode::Enter)),
        Outcome::Do(Action::OpenWorkspace)
    );
    assert_eq!(router.mode(), Mode::Navigate);
}

#[test]
fn the_bind_table_is_herdrs_own() {
    use Action::*;
    let table: &[(KeyEvent, Action)] = &[
        (ch('?'), Keybinds),
        (ch('s'), Settings),
        (ch('q'), Detach),
        (ch('R'), ReloadConfig),
        (ch('o'), OpenNotificationTarget),
        (ch('N'), NewWorkspace),
        (ch('G'), NewWorktree),
        (ch('W'), RenameWorkspace),
        (ch('D'), CloseWorkspace),
        (ch('c'), NewTab),
        (ch('T'), RenameTab),
        (ch('p'), PreviousTab),
        (ch('n'), NextTab),
        (ch('4'), SwitchTab(4)),
        (ch('X'), CloseTab),
        (ch('v'), SplitVertical),
        (ch('-'), SplitHorizontal),
        (ch('x'), ClosePane),
        (ch('P'), RenamePane),
        (ch('e'), EditScrollback),
        (ch('z'), ZoomPane),
        (ch('b'), ToggleSidebar),
        (ch('h'), FocusPane(kampr_tui::keymap::Dir::Left)),
        (ch('j'), FocusPane(kampr_tui::keymap::Dir::Down)),
        (ch('k'), FocusPane(kampr_tui::keymap::Dir::Up)),
        (ch('l'), FocusPane(kampr_tui::keymap::Dir::Right)),
        (key(KeyCode::Tab), CyclePaneNext),
        (key(KeyCode::BackTab), CyclePanePrevious),
    ];
    for (pressed, want) in table {
        let mut router = Router::default();
        router.key(ctrl('b'));
        assert_eq!(
            router.key(*pressed),
            Outcome::Do(*want),
            "prefix + {pressed:?} is {want:?} in herdr (#289)"
        );
    }
}

/// The handful Kampr claims for itself, and the one thing that must stay true of each: it takes a
/// key #289's table leaves alone. `shift+h` is the herd view and plain `h` is still herdr's own
/// focus-left, which is the pair a careless `shifted` would have collapsed.
#[test]
fn the_binds_kampr_adds_take_keys_herdr_leaves_alone() {
    use Action::*;
    let mut router = Router::default();
    router.key(ctrl('b'));
    assert_eq!(router.key(ch('H')), Outcome::Do(HerdView));
    router.key(ctrl('b'));
    assert_eq!(
        router.key(ch('h')),
        Outcome::Do(FocusPane(kampr_tui::keymap::Dir::Left))
    );
}

#[test]
fn a_key_that_is_not_a_binding_after_the_prefix_goes_to_the_pane() {
    let mut router = Router::default();
    router.key(ctrl('b'));
    assert_eq!(router.key(ch('~')), Outcome::ToPane("~".into()));
    assert_eq!(router.mode(), Mode::Pane);
}

#[test]
fn the_keys_herdrs_grammar_rejects_are_escape_sequences() {
    // Home, End, PageUp, PageDown, Insert and Delete are not in herdr's key grammar (#8), so
    // they travel as bytes through `text` (#9) rather than through `keys`.
    let cases = [
        (KeyCode::Home, "\u{1b}[H"),
        (KeyCode::End, "\u{1b}[F"),
        (KeyCode::PageUp, "\u{1b}[5~"),
        (KeyCode::PageDown, "\u{1b}[6~"),
        (KeyCode::Insert, "\u{1b}[2~"),
        (KeyCode::Delete, "\u{1b}[3~"),
    ];
    let mut router = Router::default();
    for (code, want) in cases {
        assert_eq!(router.key(key(code)), Outcome::ToPane(want.into()));
    }
}

#[test]
fn a_paste_supplies_its_own_bracketing() {
    // `pane.send_text` writes raw bytes with no framing (#9), so an unbracketed multi-line paste
    // executes line by line in a shell.
    let framed = kampr_tui::input::bracketed("one\ntwo");
    assert_eq!(framed, "\u{1b}[200~one\ntwo\u{1b}[201~");
}

#[test]
fn the_escape_hatch_moves_the_prefix_so_the_pane_gets_ctrl_b_untouched() {
    // herdr uses the *local* keybindings for `--remote` and ships a flag to take the server's
    // instead. herdr's own config is not on the wire and #296 measured that a client cannot read
    // back the view it set, so the reachable half of that escape hatch is to stop claiming the
    // prefix — after which ctrl+b belongs to whatever is running in the pane.
    let mut router = Router::with_prefix(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert_eq!(router.key(ctrl('b')), Outcome::ToPane("\u{2}".into()));
    assert_eq!(router.mode(), Mode::Pane);
    assert_eq!(router.key(ctrl('a')), Outcome::Redrew);
    assert_eq!(router.mode(), Mode::Prefix);
    assert_eq!(router.key(ch('b')), Outcome::Do(Action::ToggleSidebar));
}
