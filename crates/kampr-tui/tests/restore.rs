//! What the terminal is left in when the client goes away.
//!
//! Every mode kampr turns on is one the operator's shell does not turn off, so a path out that
//! skips the reset leaves a terminal emitting `[<0;12;5M` at every click for the life of that
//! shell. The loop has six ways out and the reset used to sit on one of them.
//!
//! These drive `Drop` and never call the reset themselves. A test that called it would pass with
//! `Drop` empty, which is the defect it is supposed to catch (#191).

use kampr_tui::Restore;
use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Tap(Arc<Mutex<Vec<u8>>>);

impl Write for Tap {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Tap {
    fn seen(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).expect("utf-8")
    }
}

#[test]
fn every_mode_the_client_turned_on_is_turned_off_when_it_goes_away() {
    let tap = Tap::default();
    {
        let mut restore = Restore::to(Box::new(tap.clone()));
        restore.arm_paste();
        restore.arm_mouse();
    }
    let out = tap.seen();
    assert!(out.contains("\u{1b}[?2004l"), "bracketed paste is off: {out:?}");
    assert!(out.contains("\u{1b}[?1006l"), "sgr mouse is off: {out:?}");
    assert!(out.contains("\u{1b}[?1003l"), "any-motion is off (#300): {out:?}");
    assert!(out.contains("\u{1b}[?1000l"), "mouse reporting is off: {out:?}");
}

#[test]
fn a_mode_that_was_never_turned_on_is_not_turned_off() {
    let tap = Tap::default();
    {
        let mut restore = Restore::to(Box::new(tap.clone()));
        restore.arm_paste();
    }
    let out = tap.seen();
    assert!(out.contains("\u{1b}[?2004l"));
    assert!(
        !out.contains("\u{1b}[?1000l"),
        "a terminal that was never put in mouse mode is not taken out of one: {out:?}"
    );
}

#[test]
fn a_way_out_that_is_not_the_clean_one_resets_the_terminal_all_the_same() {
    let tap = Tap::default();
    let err: anyhow::Result<()> = (|| {
        let mut restore = Restore::to(Box::new(tap.clone()));
        restore.arm_paste();
        restore.arm_mouse();
        anyhow::bail!("the node went away mid-frame")
    })();
    assert!(err.is_err());
    let out = tap.seen();
    assert!(
        out.contains("\u{1b}[?1000l") && out.contains("\u{1b}[?2004l"),
        "an early return is one of the six ways out and resets like the rest: {out:?}"
    );
}
