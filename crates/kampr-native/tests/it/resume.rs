//! `OSC 7577` through real keepers (docs/25): the token a session is given, the reports it accepts,
//! the records that outlive a boot and what a restart does with them.

use kampr_core::provider::Provider;
use kampr_keep::Launch;
use kampr_native::{NativeConfig, NativeProvider, NewSession, PREFIX, Resume, ResumeMode};
use serde_json::Value;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SOON: Duration = Duration::from_secs(20);

/// What a program does with the token it finds: sign a report for `$ARGV` (base64 elements joined
/// by commas) and write it. `REPORT_TIME`, `FORGE` and `EXTRA` bend it for the cases that must fail.
const REPORTER: &str = r#"
while [ -n "$GO" ] && [ ! -e "$GO" ]; do sleep 0.1; done
[ -n "$TOKEN_OUT" ] && printenv OSC7577_TOKEN > "$TOKEN_OUT"
emit() {
  m=$(printf '%s:%s' "$1" "$2" | sha256sum | cut -d' ' -f1)
  printf '\033]7577;%s:mac=%s\033\\' "$1" "$m"
}
t=${REPORT_TIME:-$(date +%s)}
emit "pid=${PID:-$$}:time=$t:argv=$ARGV$EXTRA" "${FORGE:-$OSC7577_TOKEN}"
[ -n "$FORGE_CLEAR" ] && { sleep 1; emit "pid=$$:time=$(date +%s):clear=1" "$FORGE_CLEAR"; }
exec sleep 600
"#;

fn b64(text: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(text)
}

fn argv_of(args: &[&str]) -> String {
    args.iter().map(|a| b64(a)).collect::<Vec<_>>().join(",")
}

struct Root {
    sessions: tempfile::TempDir,
    state: tempfile::TempDir,
}

impl Root {
    fn new() -> Self {
        let make = |prefix| {
            tempfile::Builder::new()
                .prefix(prefix)
                .tempdir_in("/tmp")
                .expect("a dir")
        };
        Self {
            sessions: make("kr"),
            state: make("ks"),
        }
    }

    fn store(&self) -> PathBuf {
        self.state.path().join("resume")
    }

    fn provider(&self, mode: ResumeMode) -> NativeProvider {
        let mut config = NativeConfig::new(
            Some(self.sessions.path().to_path_buf()),
            Some(kampr_testkit::keeper()),
        )
        .expect("a config");
        config.launch = Some(Launch::Detached);
        config.resume = Some(Resume {
            mode,
            dir: self.store(),
        });
        NativeProvider::new(config).expect("a provider")
    }

    fn record(&self, pane: &str) -> Option<Value> {
        let id = pane.strip_prefix(PREFIX)?;
        serde_json::from_slice(&std::fs::read(self.store().join(format!("{id}.json"))).ok()?).ok()
    }

    fn owners(&self, pane: &str) -> Vec<Value> {
        self.record(pane)
            .and_then(|r| r["owners"].as_array().cloned())
            .unwrap_or_default()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        kampr_testkit::stop_keepers(self.sessions.path());
    }
}

async fn session(provider: &NativeProvider, env: &[(&str, &str)]) -> String {
    provider
        .create(NewSession {
            argv: vec!["sh".into(), "-c".into(), REPORTER.into()],
            cols: 80,
            rows: 24,
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            ..NewSession::default()
        })
        .await
        .expect("a session")
}

async fn until<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + SOON;
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(Instant::now() < end, "{what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn stays_empty(root: &Root, pane: &str, what: &str) {
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(root.owners(pane).is_empty(), "{what}: {:?}", root.record(pane));
}

fn sleeping() -> String {
    argv_of(&["sleep", "600"])
}

fn work(root: &Root, name: &str) -> String {
    root.state.path().join(name).display().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_report_signed_with_the_sessions_own_token_is_kept() {
    let root = Root::new();
    let provider = root.provider(ResumeMode::Ask);
    let argv = sleeping();

    let good = session(&provider, &[("ARGV", &argv)]).await;
    let owners = until("a signed report", || {
        Some(root.owners(&good)).filter(|o| !o.is_empty())
    })
    .await;
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0]["command"]["argv"], serde_json::json!(["sleep", "600"]));

    let forged = session(
        &provider,
        &[("ARGV", &argv), ("FORGE", "a-token-this-session-was-never-given")],
    )
    .await;
    stays_empty(&root, &forged, "a report signed with another token").await;

    let token = std::fs::read_to_string(
        root.sessions
            .path()
            .join(good.strip_prefix(PREFIX).unwrap())
            .join("resume.token"),
    )
    .expect("a token file");
    let borrowed = session(&provider, &[("ARGV", &argv), ("FORGE", token.trim())]).await;
    stays_empty(
        &root,
        &borrowed,
        "a report signed with another live session's real token",
    )
    .await;

    let outsider = session(&provider, &[("ARGV", &argv), ("PID", "1")]).await;
    stays_empty(
        &root,
        &outsider,
        "a signed report naming a process outside the session",
    )
    .await;
}

/// A report `cat` from a file reaches the terminal like any other output, so only the token tells
/// a program reporting itself from one replaying a report; `SIGN` unset signs with the session's.
const CATTED: &str = r#"
t=$(date +%s)
m=$(printf '%s:%s' "pid=$$:time=$t:argv=$ARGV" "${SIGN-$OSC7577_TOKEN}" | sha256sum | cut -d' ' -f1)
printf '\033]7577;pid=%s:time=%s:argv=%s:mac=%s\033\\' $$ "$t" "$ARGV" "$m" > "$FILE"
cat "$FILE"
exec sleep 600
"#;

#[tokio::test(flavor = "multi_thread")]
async fn a_report_catted_from_a_file_without_the_token_does_nothing() {
    let root = Root::new();
    let provider = root.provider(ResumeMode::Ask);
    let catted = async |name: &str, sign: Option<&str>| {
        let mut env = vec![
            ("ARGV".to_string(), sleeping()),
            ("FILE".to_string(), work(&root, name)),
        ];
        env.extend(sign.map(|s| ("SIGN".to_string(), s.to_string())));
        provider
            .create(NewSession {
                argv: vec!["sh".into(), "-c".into(), CATTED.into()],
                cols: 80,
                rows: 24,
                env,
                ..NewSession::default()
            })
            .await
            .expect("a session")
    };
    let own = catted("own", None).await;
    until("the session's own report, catted", || {
        Some(root.owners(&own)).filter(|o| !o.is_empty())
    })
    .await;
    let unsigned = catted("unsigned", Some("")).await;
    stays_empty(&root, &unsigned, "a catted report without the token").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forged_clear_does_not_touch_a_record() {
    let root = Root::new();
    let provider = root.provider(ResumeMode::Ask);
    let pane = session(
        &provider,
        &[("ARGV", &sleeping()), ("FORGE_CLEAR", "another-token-0123456789")],
    )
    .await;
    until("the record", || {
        Some(root.owners(&pane)).filter(|o| !o.is_empty())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(root.owners(&pane).len(), 1, "{:?}", root.record(&pane));
}

// set_var before the runtime exists: no other thread may be reading the environment.
#[test]
fn each_session_has_its_own_private_token_and_a_docker_shell_or_an_off_node_has_none() {
    const INHERITED: &str = "a-token-the-node-was-started-with";
    // A node started from a pane of another node inherits that pane's token.
    unsafe { std::env::set_var("OSC7577_TOKEN", INHERITED) };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(async {
            let root = Root::new();
            let provider = root.provider(ResumeMode::Ask);
            let (a_out, b_out, docker_out) = (work(&root, "a"), work(&root, "b"), work(&root, "docker"));
            let a = session(&provider, &[("TOKEN_OUT", &a_out), ("ARGV", &sleeping())]).await;
            let b = session(&provider, &[("TOKEN_OUT", &b_out), ("ARGV", &sleeping())]).await;
            let docker = provider
                .create(NewSession {
                    argv: vec!["sh".into(), "-c".into(), REPORTER.into()],
                    cols: 80,
                    rows: 24,
                    env: vec![
                        ("TOKEN_OUT".into(), docker_out.clone()),
                        ("ARGV".into(), sleeping()),
                    ],
                    container: Some("db".into()),
                    ..NewSession::default()
                })
                .await
                .expect("a docker shell");
            let (ta, tb, td) = (
                until_file(&a_out).await,
                until_file(&b_out).await,
                until_file(&docker_out).await,
            );
            assert_ne!(ta, tb, "two sessions share a token");
            assert!(
                ta != INHERITED && tb != INHERITED,
                "a session was given the node's own token"
            );
            assert_eq!(td, "", "a docker shell has no token");
            for (pane, token) in [(&a, &ta), (&b, &tb)] {
                let file = root.sessions.path().join(pane.strip_prefix(PREFIX).unwrap());
                let on_disk = std::fs::read_to_string(file.join("resume.token")).expect("a token file");
                assert_eq!(&on_disk, token);
                assert_eq!(
                    std::fs::metadata(file.join("resume.token"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
                let spec = std::fs::read_to_string(file.join("spec.json")).unwrap();
                assert!(!spec.contains(token.as_str()), "the token stayed in spec.json");
            }
            let docker_dir = root.sessions.path().join(docker.strip_prefix(PREFIX).unwrap());
            assert!(!docker_dir.join("resume.token").exists());
            stays_empty(&root, &docker, "a docker shell's report").await;

            let off = Root::new();
            let quiet = off.provider(ResumeMode::Off);
            let off_out = work(&off, "off");
            let pane = session(&quiet, &[("TOKEN_OUT", &off_out), ("ARGV", &sleeping())]).await;
            assert_eq!(until_file(&off_out).await, "", "an off node gave a token");
            assert!(
                !off.sessions
                    .path()
                    .join(pane.strip_prefix(PREFIX).unwrap())
                    .join("resume.token")
                    .exists()
            );
            stays_empty(&off, &pane, "a report to an off node").await;
            assert!(!off.store().exists(), "an off node made a store");
        });
}

async fn until_file(path: &str) -> String {
    until("a file", || std::fs::read_to_string(path).ok())
        .await
        .trim()
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_record_is_private_survives_a_node_restart_and_goes_when_its_session_is_closed() {
    let root = Root::new();
    let first = root.provider(ResumeMode::Ask);
    let pane = session(&first, &[("ARGV", &sleeping())]).await;
    until("the record", || {
        Some(root.owners(&pane)).filter(|o| !o.is_empty())
    })
    .await;
    let id = pane.strip_prefix(PREFIX).unwrap();
    let file = root.store().join(format!("{id}.json"));
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(root.store()).unwrap().permissions().mode() & 0o777,
        0o700
    );

    drop(first);
    let second = root.provider(ResumeMode::Ask);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(root.owners(&pane).len(), 1, "a restarted node lost the record");
    assert!(
        second.resumable().is_empty(),
        "a live session's record is not offered"
    );

    second.close(&pane).expect("closed");
    until("the record to go", || (!file.exists()).then_some(())).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_boot_ask_lists_a_record_and_a_resume_runs_each_element_as_data() {
    let root = Root::new();
    let first = root.provider(ResumeMode::Ask);
    let argv = argv_of(&["printf", "<%s>", "$(touch SENTINEL)", "a; b", "'q'"]);
    let cwd = root.state.path().display().to_string();
    let pane = session(
        &first,
        &[("ARGV", &argv), ("EXTRA", &format!(":cwd={}", b64(&cwd)))],
    )
    .await;
    let gone = session(
        &first,
        &[
            ("ARGV", &sleeping()),
            ("EXTRA", &format!(":cwd={}", b64("/no/such/directory"))),
        ],
    )
    .await;
    until("both records", || {
        (root.owners(&pane).len() == 1 && root.owners(&gone).len() == 1).then_some(())
    })
    .await;

    drop(first);
    kampr_testkit::stop_keepers(root.sessions.path());
    for pane in [&pane, &gone] {
        std::fs::remove_dir_all(root.sessions.path().join(pane.strip_prefix(PREFIX).unwrap())).unwrap();
    }
    tokio::time::sleep(Duration::from_secs(3)).await;

    let after = root.provider(ResumeMode::Ask);
    let listed = after.resumable();
    assert_eq!(listed.len(), 1, "a missing cwd is not offered: {listed:?}");
    assert_eq!(listed[0].command.argv[0], "printf");
    assert_eq!(listed[0].command.cwd.as_deref(), Some(cwd.as_str()));

    let resumed = after.resume(&listed[0].id).await.expect("resumed");
    let shown = until("what the resumed command printed", || {
        after.visible(&resumed).filter(|s| s.contains("<a; b>"))
    })
    .await;
    for line in ["<$(touch SENTINEL)>", "<a; b>", "<'q'>"] {
        assert!(shown.contains(line), "{line} in {shown}");
    }
    assert!(
        !root.state.path().join("SENTINEL").exists(),
        "an element was run as shell text"
    );
    assert!(
        after.resume(&listed[0].id).await.is_err(),
        "a record was run twice"
    );
    assert!(after.resumable().is_empty());
    assert!(
        root.record(&pane).is_none(),
        "a spent record stayed in the store: {:?}",
        root.record(&pane)
    );

    let again = root.provider(ResumeMode::Ask);
    assert!(
        again.resumable().is_empty(),
        "a second boot offered a spent record"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_starts_what_ask_would_list() {
    let root = Root::new();
    let first = root.provider(ResumeMode::Auto);
    let argv = argv_of(&["echo", "back-again"]);
    let pane = session(&first, &[("ARGV", &argv)]).await;
    until("the record", || {
        Some(root.owners(&pane)).filter(|o| !o.is_empty())
    })
    .await;
    drop(first);
    kampr_testkit::stop_keepers(root.sessions.path());
    std::fs::remove_dir_all(root.sessions.path().join(pane.strip_prefix(PREFIX).unwrap())).unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    let after = root.provider(ResumeMode::Auto);
    after.resume_all().await;
    let panes = after.list_panes().await.expect("panes");
    assert_eq!(panes.len(), 1);
    until("the output", || {
        after
            .visible(&panes[0].pane_id)
            .filter(|s| s.contains("back-again"))
    })
    .await;
}

// A Mac's /tmp sweep deletes files nobody has touched in days, and spec.json is written once: a
// session still running must not look gone to a restarted node under `auto`, nor to attach.
#[tokio::test(flavor = "multi_thread")]
async fn a_live_session_whose_spec_was_swept_is_not_resumed_a_second_time() {
    let root = Root::new();
    let first = root.provider(ResumeMode::Auto);
    let pane = session(&first, &[("ARGV", &sleeping())]).await;
    until("the record", || {
        Some(root.owners(&pane)).filter(|o| !o.is_empty())
    })
    .await;
    drop(first);
    let id = pane.strip_prefix(PREFIX).unwrap();
    std::fs::remove_file(root.sessions.path().join(id).join("spec.json")).unwrap();

    let after = root.provider(ResumeMode::Auto);
    after.resume_all().await;
    assert!(after.resumable().is_empty(), "{:?}", after.resumable());
    let resume = Resume {
        mode: ResumeMode::Auto,
        dir: root.store(),
    };
    assert!(kampr_native::resume_offered(&resume, root.sessions.path(), id).is_empty());
    let dirs = std::fs::read_dir(root.sessions.path())
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .count();
    assert_eq!(dirs, 1, "a second copy was started");
    assert_eq!(root.owners(&pane).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_time_window_applies_to_a_report_read_live_and_not_to_one_a_later_node_replays() {
    let root = Root::new();
    let first = root.provider(ResumeMode::Ask);
    let old = "1000000000";
    let argv = sleeping();
    let held = work(&root, "held");
    let live = session(&first, &[("ARGV", &argv), ("REPORT_TIME", old), ("GO", &held)]).await;
    until("the node following the session", || first.visible(&live)).await;
    std::fs::write(&held, b"").unwrap();
    stays_empty(&root, &live, "a stale report read as it was written").await;

    let go = work(&root, "go");
    let unwatched = session(&first, &[("ARGV", &argv), ("REPORT_TIME", old), ("GO", &go)]).await;
    drop(first);
    std::fs::write(&go, b"").unwrap();
    let dir = kampr_keep::SessionDir::at(root.sessions.path().join(unwatched.strip_prefix(PREFIX).unwrap()));
    until("the report in the log", || {
        let mut log = Vec::new();
        let mut reader = kampr_keep::SizedReader::at(dir.path(), 0).ok()?;
        let mut buf = vec![0u8; 1 << 16];
        while let Ok(kampr_keep::Replay::Data(n)) = reader.read(&mut buf) {
            log.extend_from_slice(&buf[..n]);
        }
        log.windows(6).any(|w| w == b"]7577;").then_some(())
    })
    .await;
    let _later = root.provider(ResumeMode::Ask);
    let owners = until("the report read from the log", || {
        Some(root.owners(&unwatched)).filter(|o| !o.is_empty())
    })
    .await;
    assert_eq!(owners[0]["time"], 1_000_000_000);
}

#[test]
fn a_mode_is_one_of_three_words() {
    assert_eq!("ask".parse::<ResumeMode>(), Ok(ResumeMode::Ask));
    assert_eq!("auto".parse::<ResumeMode>(), Ok(ResumeMode::Auto));
    assert_eq!("off".parse::<ResumeMode>(), Ok(ResumeMode::Off));
    assert!("yes".parse::<ResumeMode>().is_err());
    let _ = Path::new("/");
}
