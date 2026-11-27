//! docs/26: a native pane tells its clients what its program asked of the mouse and the cursor keys.

use super::*;

fn modes(mouse: &str, sgr: bool, alt_scroll: bool, app_cursor: bool, alt_screen: bool) -> Value {
    json!({
        "mouse": mouse, "mouse_sgr": sgr, "alt_scroll": alt_scroll,
        "app_cursor": app_cursor, "alt_screen": alt_screen
    })
}

/// **What the program set is what the herd says, and what it reset is gone.** `dd` is the job while
/// it waits, so the pane has one to name.
#[tokio::test]
async fn a_pane_reports_the_modes_its_program_set_and_clears_them_when_it_resets() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    for (what, set, reset, want) in [
        (
            "claude",
            "\\033[?1049h\\033[?1000h\\033[?1006h",
            "\\033[?1000l\\033[?1006l\\033[?1049l",
            modes("press", true, true, false, true),
        ),
        (
            "less",
            "\\033[?1049h\\033[?1h\\033[?1007h",
            "\\033[?1l\\033[?1049l",
            modes("none", false, true, true, true),
        ),
    ] {
        let script = format!(
            "stty -echo raw; printf '{set}READY\\r\\n'; dd bs=1 count=1 2>/dev/null; \
             printf '{reset}CLEARED\\r\\n'; exec cat"
        );
        let id = client.create(&["sh", "-c", &script], json!({})).await;
        let pane = client.pane(&id, |p| p["modes"] == want).await;
        assert_eq!(pane["modes"], want, "{what}: {pane}");

        client.type_text(&id, "x").await;
        let pane = client
            .pane(&id, |p| p["modes"] == modes("none", false, true, false, false))
            .await;
        assert_eq!(pane["cmd"], "cat", "{what}: {pane}");
    }
}

/// **Modes are the program's only while one is running.** A program that dies with mouse tracking
/// on leaves it on, and the prompt it left behind has asked for no report.
#[tokio::test]
async fn the_modes_a_program_left_on_are_not_reported_for_the_prompt_it_left_behind() {
    let h = Harness::start().await;
    let mut client = h.client().await;
    let id = client
        .create(
            &["sh", "-c", "printf '\\033[?1000h\\033[?1006h'; exec sh -i"],
            json!({}),
        )
        .await;
    client.watch(&id, false).await;
    client.shows(&id, "$").await;
    let pane = client.pane(&id, |p| p["cmd"].is_null()).await;
    assert!(pane.get("modes").is_none(), "a shell's prompt: {pane}");

    client.type_text(&id, "cat\r").await;
    client
        .pane(&id, |p| {
            p["cmd"] == "cat" && p["modes"] == modes("press", true, true, false, false)
        })
        .await;
    client.type_text(&id, "\x04").await;
    let pane = client
        .pane(&id, |p| p["cmd"].is_null() && p.get("modes").is_none())
        .await;
    assert!(pane.get("modes").is_none(), "{pane}");
}
