//! What a pane can talk the operator into opening.
//!
//! A pane declares its own OSC 8 URIs and `docs/04-wire-protocol.md` says plainly that pane output
//! is attacker-influenceable, so "the harness declared it" is not a reason to trust a URI. Two
//! paths used to reach the opener and only one of them checked the scheme — and the unchecked one
//! was the one the status line told the operator to use.

use kampr_tui::app::{navigable, open_url};

#[test]
fn a_scheme_that_is_not_the_web_is_never_handed_to_the_opener() {
    for url in [
        "file:///tmp/evil.desktop",
        "/tmp/evil.desktop",
        "smb://attacker/share",
        "vscode://ms-vscode.remote/x",
        "javascript:alert(1)",
        "data:text/html,<script>",
        "HTTPS://upper.example",
        "",
        "-version",
    ] {
        assert!(!navigable(url), "{url:?} is not a web URL");
        assert!(
            !open_url(url),
            "{url:?} reached the opener — the gate has to live inside open_url, because a second \
             call site will forget it and one already did"
        );
    }
}

#[test]
fn an_ordinary_web_url_is_still_recognised() {
    assert!(navigable("https://herdr.dev"));
    assert!(navigable("http://192.168.1.24:8790"));
}
