//! `kampr new`, `ls` and `attach` as a person types them: the real binary on a terminal this test
//! owns, keepers detached under a throwaway root.

use kampr_keep::SessionDir;
use rustix::termios::{self, LocalModes, Winsize};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const KAMPR: &str = env!("CARGO_BIN_EXE_kampr");
const SOON: Duration = Duration::from_secs(10);

const ECHO: &str = r#"echo prior-output; while read l; do case $l in size) stty size;; quit) exit 3;; *) echo "got $l";; esac; done"#;

struct Root(tempfile::TempDir, tempfile::TempDir);

impl Root {
    fn new() -> Self {
        // ctl.sock must fit in sun_path, so not under a long TMPDIR.
        Self(
            tempfile::Builder::new()
                .prefix("kc")
                .tempdir_in("/tmp")
                .expect("root"),
            tempfile::tempdir().expect("a config dir"),
        )
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(KAMPR);
        command
            .args(args)
            .env("KAMPR_CONFIG_DIR", self.1.path())
            .env("KAMPR_KEEP_ROOT", self.0.path())
            .env("KAMPR_KEEP_LAUNCH", "detached")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_SOCKET_PATH");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).stdin(Stdio::null()).output().expect("kampr")
    }

    fn new_detached(&self, script: &str) -> SessionDir {
        let out = self.run(&["new", "-d", "--label", "probe", "--", "sh", "-c", script]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        SessionDir::at(self.0.path().join(id))
    }

    fn sessions(&self) -> Vec<SessionDir> {
        kampr_keep::list(self.0.path()).expect("list")
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        for session in kampr_keep::list(self.0.path()).unwrap_or_default() {
            if let Some(keeper) = session.keeper() {
                unsafe {
                    libc::kill(-(keeper.child as i32), libc::SIGKILL);
                    libc::kill(keeper.keeper as i32, libc::SIGKILL);
                }
            }
        }
    }
}

struct Term {
    master: File,
    slave: OwnedFd,
    seen: Arc<Mutex<Vec<u8>>>,
    child: Child,
}

fn winsize(cols: u16, rows: u16) -> Winsize {
    Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

impl Term {
    fn open(root: &Root, args: &[&str], cols: u16, rows: u16) -> Self {
        Self::open_as(root, args, cols, rows, false)
    }

    /// A terminal that answers DA1 the moment it reads one, as herdr does, and nothing else: herdr
    /// does not answer `OSC 7501;?` (#611).
    fn open_as(root: &Root, args: &[&str], cols: u16, rows: u16, answers_da1: bool) -> Self {
        use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).expect("openpt");
        rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC).expect("cloexec");
        grantpt(&master).expect("grantpt");
        unlockpt(&master).expect("unlockpt");
        let name = ptsname(&master, Vec::new()).expect("ptsname");
        let slave = rustix::fs::open(
            name.as_c_str(),
            rustix::fs::OFlags::RDWR | rustix::fs::OFlags::NOCTTY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .expect("slave");
        termios::tcsetwinsize(&master, winsize(cols, rows)).expect("size");
        let mut command = root.command(args);
        command
            .stdin(Stdio::from(slave.try_clone().expect("dup")))
            .stdout(Stdio::from(slave.try_clone().expect("dup")))
            .stderr(Stdio::from(slave.try_clone().expect("dup")));
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().expect("kampr");
        let master = File::from(master);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (mut reader, sink) = (master.try_clone().expect("dup"), seen.clone());
        let mut answerer = master.try_clone().expect("dup");
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                if answers_da1 && buf[..n].windows(3).any(|w| w == b"\x1b[c") {
                    let _ = answerer.write_all(b"\x1b[?62;22c");
                }
                sink.lock().expect("seen").extend_from_slice(&buf[..n]);
            }
        });
        Self {
            master,
            slave,
            seen,
            child,
        }
    }

    fn screen(&self) -> String {
        let ws = termios::tcgetwinsize(&self.master).expect("size");
        let mut screen = kampr_native::Screen::new(ws.ws_col, ws.ws_row, 0);
        screen.feed(&self.seen.lock().expect("seen"));
        let first = screen.first_row();
        screen
            .read_rows(first, first + u32::from(ws.ws_row) - 1)
            .expect("the screen's rows")
            .join("\n")
    }

    fn raw(&self) -> String {
        String::from_utf8_lossy(&self.seen.lock().expect("seen")).into_owned()
    }

    fn shows(&self, needle: &str) {
        let shown = until(|| self.screen().contains(needle));
        assert!(shown, "{needle:?} never appeared:\n{}", self.screen());
    }

    fn type_line(&self, line: &str) {
        (&self.master)
            .write_all(format!("{line}\r").as_bytes())
            .expect("type");
    }

    fn resize(&self, cols: u16, rows: u16) {
        termios::tcsetwinsize(&self.master, winsize(cols, rows)).expect("resize");
    }

    fn exit_code(&mut self) -> i32 {
        let deadline = Instant::now() + SOON;
        loop {
            if let Some(status) = self.child.try_wait().expect("wait") {
                return status.code().expect("exit code");
            }
            assert!(
                Instant::now() < deadline,
                "attach never exited:\n{}",
                self.screen()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn until(mut ok: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + SOON;
    while Instant::now() < deadline {
        if ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    ok()
}

fn holder(session: &SessionDir) -> String {
    session.size().map(|s| s.holder).unwrap_or_default()
}

#[test]
fn new_attaches_claims_its_size_follows_a_resize_and_leaves_with_the_status() {
    let root = Root::new();
    let mut term = Term::open(&root, &["new", "--", "sh", "-c", ECHO], 100, 30);
    let cooked = termios::tcgetattr(&term.slave).expect("termios");
    term.shows("prior-output");
    let [session] = root.sessions().try_into().expect("one session");
    assert!(until(|| holder(&session) == format!("attach:{}", term.child.id())));
    let host = session.hosts();
    assert_eq!(host.len(), 1, "{host:?}");
    assert_eq!(
        (host[0].pid, host[0].cols, host[0].rows),
        (term.child.id(), 100, 30)
    );

    term.type_line("hello");
    term.shows("got hello");
    term.type_line("size");
    term.shows("30 100");
    term.resize(90, 20);
    assert!(until(|| session
        .hosts()
        .first()
        .is_some_and(|h| (h.cols, h.rows) == (90, 20))));
    term.type_line("size");
    term.shows("20 90");

    let raw = termios::tcgetattr(&term.slave).expect("termios");
    assert!(!raw.local_modes.contains(LocalModes::ICANON | LocalModes::ECHO));
    term.type_line("quit");
    let code = term.exit_code();
    assert_eq!(code, 3, "{}", term.raw());
    assert!(
        term.raw().contains(&format!("kampr: {} exited 3", session.id())),
        "{}",
        term.raw()
    );
    let after = termios::tcgetattr(&term.slave).expect("termios");
    assert_eq!(after.local_modes, cooked.local_modes);
    assert_eq!(after.input_modes, cooked.input_modes);
    assert_eq!(after.output_modes, cooked.output_modes);
    assert!(session.hosts().is_empty());
}

#[test]
fn a_second_attach_takes_the_size_and_the_first_mirrors_it_under_a_banner() {
    let root = Root::new();
    let session = root.new_detached(ECHO);
    let first = Term::open(&root, &["attach", session.id()], 80, 24);
    first.type_line("typeahead");
    first.shows("got typeahead");
    first.shows("prior-output");
    let mut second = Term::open(&root, &["attach", &session.id()[..20]], 100, 30);
    second.shows("prior-output");
    let banner = format!("100x30 is held by attach:{}", second.child.id());
    first.shows(&banner);

    second.type_line("mirrored");
    second.shows("got mirrored");
    first.shows("got mirrored");
    second.type_line("size");
    second.shows("30 100");

    first.resize(70, 20);
    assert!(until(
        || holder(&session) == format!("attach:{}", first.child.id())
    ));
    first.type_line("size");
    first.shows("20 70");
    second.shows(&format!("70x20 is held by attach:{}", first.child.id()));
    assert_eq!(session.hosts().len(), 2);

    let cooked_before = termios::tcgetattr(&first.slave).expect("termios");
    unsafe { libc::kill(second.child.id() as i32, libc::SIGTERM) };
    assert_eq!(second.exit_code(), 0);
    let restored = termios::tcgetattr(&second.slave).expect("termios");
    assert!(
        restored
            .local_modes
            .contains(LocalModes::ICANON | LocalModes::ECHO)
    );
    assert!(!cooked_before.local_modes.contains(LocalModes::ICANON));
    assert_eq!(session.hosts().len(), 1);
    assert!(session.alive());
}

#[test]
fn ls_lists_and_attach_resolves_prefixes_and_reports_an_ended_session() {
    let root = Root::new();
    let live = root.new_detached("exec sleep 60");
    let done = root.new_detached("exit 5");
    assert!(until(|| done.exit().is_some()));

    let ls = String::from_utf8_lossy(&root.run(&["ls"]).stdout).into_owned();
    let line = |id: &str| ls.lines().find(|l| l.starts_with(id)).map(str::to_string);
    let live_line = line(live.id()).unwrap_or_else(|| panic!("{ls}"));
    let done_line = line(done.id()).unwrap_or_else(|| panic!("{ls}"));
    assert!(
        live_line.contains(" live ") && live_line.contains("80x24") && live_line.ends_with("probe"),
        "{ls}"
    );
    assert!(done_line.contains(" exited 5 "), "{ls}");

    let common = live
        .id()
        .chars()
        .zip(done.id().chars())
        .take_while(|(a, b)| a == b)
        .count();
    let ambiguous = root.run(&["attach", &live.id()[..common]]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("matches 2 sessions"));
    let missing = root.run(&["attach", "zzz"]);
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no session matches"));

    let ended = root.run(&["attach", &done.id()[..common + 1]]);
    assert_eq!(ended.status.code(), Some(5));
    assert_eq!(
        String::from_utf8_lossy(&ended.stdout).trim(),
        format!("kampr: {} exited 5", done.id())
    );
}

/// The node keeps its sessions where `native.root` says, so the CLI must look there too, or
/// `kampr ls` on that machine lists nothing the phone can see.
#[test]
fn new_ls_and_attach_use_the_root_the_node_is_configured_with() {
    let env = Root::new();
    let configured = Root::new();
    let mut config = kampr_node::Config::bootstrap("cli");
    config.native.root = configured.0.path().display().to_string();
    config.save(env.1.path()).expect("a config");

    let out = env.run(&["new", "-d", "--", "sh", "-c", "exit 4"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        std::fs::read_dir(env.0.path()).unwrap().next().is_none(),
        "made under KAMPR_KEEP_ROOT, not the configured root"
    );
    let made = SessionDir::at(configured.0.path().join(&id));
    assert!(made.path().exists());
    assert!(until(|| made.exit().is_some()));

    let ls = String::from_utf8_lossy(&env.run(&["ls"]).stdout).into_owned();
    assert!(ls.lines().any(|l| l.starts_with(made.id())), "{ls}");
    let ended = env.run(&["attach", made.id()]);
    assert_eq!(
        ended.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&ended.stderr)
    );
}

/// **A session that boots under `kampr attach` still gets Claude's status channel.** Its terminal
/// answers DA1 and not `OSC 7501;?`, so the attach answers 7501 itself, ahead of the bytes that
/// reach its terminal and so ahead of DA1's answer (#600). With two attaches showing it, one
/// answers: the program reads one 7501 answer and then DA1's.
#[test]
fn an_attach_answers_osc_7501_once_and_before_its_terminal_answers_da1() {
    let root = Root::new();
    let go = root.0.path().join("go");
    let script = format!(
        "stty -echo -icanon min 0 time 30; while [ ! -e {go} ]; do sleep 0.05; done; \
         printf '\\033]7501;?\\007\\033[c'; r=$(dd bs=1 count=19 2>/dev/null); \
         printf 'GOT[%s]\\n' \"$(printf %s \"$r\" | tr '\\033' E)\"; stty min 1 time 0; exec cat",
        go = go.display()
    );
    let session = root.new_detached(&script);
    let desk = Term::open_as(&root, &["attach", session.id()], 80, 24, true);
    let other = Term::open(&root, &["attach", session.id()], 80, 24);
    assert!(until(|| session.hosts().len() == 2));
    std::fs::write(&go, b"").unwrap();
    desk.shows("GOT[");
    other.shows("GOT[");
    assert!(
        desk.screen().contains("GOT[E]7501;?E\\E[?62;22c]"),
        "{}",
        desk.screen()
    );
}

/// **A query is answered once, by whoever sees it.** Passed through, the terminal sees it and
/// answers. Cropped by a phone holding the size, the attach paints from its own emulator, so the
/// question never reaches the terminal, and the node leaves a query past a live host's offset to
/// that host: the attach answers it. Two desks passing it through would both answer, so only the
/// answering attach's terminal is shown the question. The program reads for long enough to see a
/// second answer.
#[test]
fn a_query_is_answered_once_whether_the_attach_passes_it_through_is_cropped_or_has_company() {
    for (case, desks) in [("passed", 1), ("cropped", 1), ("two desks", 2)] {
        let root = Root::new();
        let go = root.0.path().join("go");
        let script = format!(
            "stty -echo -icanon min 0 time 20; while [ ! -e {go} ]; do sleep 0.05; done; \
             printf '\\033[c'; r=$(dd bs=1 count=18 2>/dev/null); \
             printf 'GOT[%s]\\n' \"$(printf %s \"$r\" | tr '\\033' E)\"; stty min 1 time 0; exec cat",
            go = go.display()
        );
        let session = root.new_detached(&script);
        let desks: Vec<Term> = (0..desks)
            .map(|_| {
                let desk = Term::open_as(&root, &["attach", session.id()], 80, 24, true);
                let me = format!("attach:{}", desk.child.id());
                assert!(until(|| holder(&session) == me));
                desk
            })
            .collect();
        assert!(until(|| session.hosts().len() == desks.len()));
        if case == "cropped" {
            kampr_keep::claim_size(&session, 100, 30, "phone:test").expect("a phone's claim");
            desks[0].shows("100x30 is held by phone:test");
        }
        std::fs::write(&go, b"").unwrap();
        assert!(
            until(|| replayed(&session).contains("GOT[")),
            "{case}: {}",
            replayed(&session)
        );
        let shown = replayed(&session);
        assert!(shown.contains("GOT[E[?62;22c]"), "{case}: {shown}");
    }
}

/// **A fresh attach paints the screen the session has, not the log redrawn at today's size.**
/// After each resize the program writes output whose shape depends on the size at that moment;
/// the attach's first paint must equal a replay that applies each resize at its byte.
#[test]
fn a_fresh_attach_paints_a_session_resized_mid_output_exactly() {
    let root = Root::new();
    let session = root.new_detached(
        r#"stty -echo; while read l; do stty size; printf '\033[999C|\r\n'; seq -s, 1 40; printf 'at \033[999B\033[999C#\r\n'; echo "done $l"; done"#,
    );
    let master = kampr_keep::borrow(&session).expect("the master");
    for (cols, rows) in [(50, 20), (120, 25), (40, 30), (90, 24)] {
        kampr_keep::claim_size(&session, cols, rows, "test").expect("a resize");
        let done = format!("done {cols}x{rows}");
        rustix::io::write(&master, format!("{cols}x{rows}\n").as_bytes()).expect("typed");
        assert!(
            until(|| replayed(&session).contains(&done)),
            "{done} never logged"
        );
    }
    let want = replayed(&session);
    let term = Term::open(&root, &["attach", session.id()], 90, 24);
    term.shows("done 90x24");
    assert_eq!(term.screen(), want);
}

fn replayed(session: &SessionDir) -> String {
    use kampr_keep::Replay;
    let mut reader = kampr_keep::SizedReader::at(session.path(), 0).expect("a reader");
    let mut screen = kampr_native::Screen::new(reader.size().cols, reader.size().rows, 1000);
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match reader.read(&mut buf).expect("a read") {
            Replay::Data(n) => {
                screen.feed(&buf[..n]);
            }
            Replay::Resize(to) => screen.resize(to.cols, to.rows),
            Replay::Truncated { .. } => panic!("a short session was trimmed"),
            Replay::CaughtUp => break,
        }
    }
    let top = screen.first_row() + screen.history_rows();
    let (_, rows) = screen.size();
    screen
        .read_rows(top, top + u32::from(rows) - 1)
        .expect("the screen's rows")
        .join("\n")
}
