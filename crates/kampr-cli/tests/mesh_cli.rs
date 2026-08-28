//! `kampr mesh` against the real binary, in a throwaway config and state directory.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Cli {
    config: PathBuf,
    state: PathBuf,
    socket: PathBuf,
    _dir: tempfile::TempDir,
}

impl Cli {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        Self {
            config: dir.path().join("config"),
            state: dir.path().join("state"),
            socket: dir.path().join("herdr.sock"),
            _dir: dir,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_kampr"))
            .args(args)
            .arg("--config-dir")
            .arg(&self.config)
            .arg("--state-dir")
            .arg(&self.state)
            .env("HERDR_SOCKET_PATH", &self.socket)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("kampr")
            .wait_with_output()
            .expect("kampr output")
    }

    fn init(&self) {
        assert!(self.run(&["init"]).status.success());
    }

    fn accept(&self, yes: bool) {
        let path = self.config.join("config.toml");
        let text = std::fs::read_to_string(&path).expect("a config");
        let text = match text.contains("[mesh]") {
            true => text
                .replace("accept = true", &format!("accept = {yes}"))
                .replace("accept = false", &format!("accept = {yes}")),
            false => format!("{text}\n[mesh]\naccept = {yes}\n"),
        };
        std::fs::write(&path, text).expect("a config");
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A join code is only spendable against a node that answers `/mesh`. Minting one on a node that
/// does not is handing the operator a code and a URL that will refuse them, with nothing on the
/// screen saying why — so the refusal has to happen here, and has to name the switch.
#[test]
fn an_invite_from_a_node_that_is_not_a_hub_says_what_to_turn_on() {
    let cli = Cli::new();
    cli.init();
    cli.accept(false);

    let out = cli.run(&["mesh", "invite"]);
    assert!(
        !out.status.success(),
        "a code nothing can spend is not a success:\n{}",
        stdout(&out)
    );
    let said = format!("{}{}", stdout(&out), stderr(&out));
    assert!(said.contains("accept"), "{said}");
    assert!(said.contains("config.toml"), "{said}");
}

#[test]
fn an_invite_from_a_hub_prints_the_join_line() {
    let cli = Cli::new();
    cli.init();
    cli.accept(true);

    let out = cli.run(&["mesh", "invite"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("kampr mesh join --hub"), "{text}");
    assert!(text.contains("--fingerprint"), "{text}");
}

const ONE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TWO: &str = "2222222222222222222222222222222222222222222222222222222222222222";

impl Cli {
    /// A row in this node's own enrolment table, written the way a handshake writes one.
    fn enrol(&self, pubkey: &str, node_id: &str, name: &str) {
        let db = kampr_node::Config::state_db(&self.state);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(async {
                kampr_auth::Store::open(&db)
                    .await
                    .expect("the state database")
                    .mesh()
                    .enrol(
                        pubkey,
                        node_id,
                        name,
                        kampr_auth::MeshRole::Peer,
                        None,
                        kampr_auth::now(),
                    )
                    .await
                    .expect("the row");
            });
    }
}

/// A peer picks the name it enrols under, so a machine can take its neighbour's and two rows then
/// answer to `laptop`. Cutting off whichever the table reached first cuts off the one the operator
/// did not mean, and tells them it worked.
#[test]
fn a_name_two_nodes_answer_to_is_a_choice_the_operator_is_shown() {
    let cli = Cli::new();
    cli.init();
    cli.enrol(ONE, "01JOWNER", "laptop");
    cli.enrol(TWO, "01JIMPOSTOR", "laptop");

    let out = cli.run(&["mesh", "revoke", "laptop"]);
    assert!(!out.status.success(), "one of two was cut off:\n{}", stdout(&out));
    let said = format!("{}{}", stdout(&out), stderr(&out));
    assert!(said.contains("01JOWNER"), "{said}");
    assert!(said.contains("01JIMPOSTOR"), "{said}");

    // The credential and the bound node id are never ambiguous.
    let out = cli.run(&["mesh", "revoke", "01JOWNER"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let out = cli.run(&["mesh", "revoke", &kampr_auth::identity::fingerprint_of(TWO)]);
    assert!(out.status.success(), "{}", stderr(&out));
}
