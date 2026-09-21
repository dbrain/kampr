// A `claude` for the boot-hold tests: it draws a boot screen with no composer until the ready
// file exists, then draws the composer the node's listening reader recognises. Every byte it is
// sent is appended to the log file from the moment it arrives — in a reader of its own, because
// the tests also cover the pane that never draws its composer, and the ground truth a test
// asserts arrival and order against must not sit behind the composer.
//
// The binary is compiled to the name `claude`, because the booting classification turns on the
// foreground process's name, and that is what herdr labels the pane with.

use std::fs;
use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let ready = std::env::var("FAKE_CLAUDE_READY").expect("FAKE_CLAUDE_READY");
    let log = std::env::var("FAKE_CLAUDE_LOG").expect("FAKE_CLAUDE_LOG");

    std::thread::spawn(move || {
        let mut out = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .expect("the input log");
        let mut stdin = std::io::stdin().lock();
        let mut byte = [0u8; 1];
        while let Ok(1) = stdin.read(&mut byte) {
            out.write_all(&byte).expect("the input log");
        }
    });

    // The boot screen: nothing a listening reader would accept, the caret parked below the text.
    print!("\x1b[2J\x1b[Hbooting\u{2026}\n");
    io_flush();
    while !fs::metadata(&ready).is_ok() {
        std::thread::sleep(Duration::from_millis(50));
    }
    // The order test's trigger: quit without drawing the composer, leaving the shell behind.
    if std::env::var("FAKE_CLAUDE_QUIT_ON_READY").is_ok() {
        std::process::exit(0);
    }

    // The composer: the marked row, the caret on it, nothing else on the screen.
    print!("\x1b[2J\x1b[H\u{276f} ");
    io_flush();

    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn io_flush() {
    let _ = std::io::stdout().flush();
}