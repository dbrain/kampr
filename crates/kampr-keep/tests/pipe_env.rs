//! A pipe-mode child is started from the spec's environment alone: the operator's login
//! environment less what pays, as the launcher built it. The keeper's own is systemd's, or the
//! node's, and a headless Claude launched from either would be paid for by whatever key was in it
//! (docs/21 guard 2).

mod common;

use common::{Live, root, sh};
use std::time::Duration;

const SOON: Duration = Duration::from_secs(10);
use kampr_keep::{Launch, Mode, PipeDir, PipeRead, SeqReader, close};

#[test]
fn a_pipe_child_does_not_inherit_the_keepers_environment() {
    unsafe {
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-api03-from-the-keeper");
        std::env::set_var("ANTHROPIC_AUTH_TOKEN", "token-from-the-keeper");
        std::env::set_var("CLAUDE_CODE_USE_BEDROCK", "1");
    }
    let root = root();
    let mut spec = sh("env; echo END-OF-ENV; sleep 30");
    spec.mode = Mode::Pipe;
    spec.env = vec![
        ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
        ("HOME".into(), "/home/spec".into()),
        ("FOO_FROM_PROFILE".into(), "1".into()),
    ];
    let live = Live::spawn(root.path(), spec, Launch::Detached);

    let mut reader = SeqReader::at(live.session.path(), 0).expect("a reader from the start");
    let mut env = Vec::new();
    let deadline = std::time::Instant::now() + SOON;
    while std::time::Instant::now() < deadline {
        match reader.read().expect("the log") {
            PipeRead::Entry(entry) if entry.dir == PipeDir::Out => {
                env.push(entry.line);
                if entry_is_end(&env) {
                    break;
                }
            }
            PipeRead::CaughtUp => {
                let _ = reader.wait(std::time::Duration::from_millis(50));
            }
            _ => {}
        }
    }
    close(&live.session).expect("close");

    for given in ["HOME=/home/spec", "FOO_FROM_PROFILE=1"] {
        assert!(
            env.iter().any(|l| l == given),
            "the spec's {given} crosses ({} lines)",
            env.len()
        );
    }
    for leaked in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
    ] {
        assert!(
            !env.iter().any(|l| l.starts_with(&format!("{leaked}="))),
            "{leaked} reached the child"
        );
    }
}

fn entry_is_end(lines: &[String]) -> bool {
    lines.last().is_some_and(|l| l == "END-OF-ENV")
}
