use kampr_core::desk::Desk;
use kampr_keep::{Host, SessionDir, Spec};
use kampr_native::{NativeConfig, NativeProvider};
use std::path::PathBuf;

const ID: &str = "01kdesk";

fn provider(root: &std::path::Path) -> NativeProvider {
    let mut config = NativeConfig::new(Some(root.to_path_buf()), Some(PathBuf::from("/opt/kampr/keep")))
        .expect("a config");
    config.attach = PathBuf::from("/opt/kampr/bin/kampr");
    NativeProvider::new(config).expect("a provider")
}

fn session(root: &std::path::Path) -> SessionDir {
    let dir = root.join(ID);
    std::fs::create_dir_all(&dir).expect("a session dir");
    let mut spec = Spec::new(vec!["bash".into()]);
    spec.id = ID.into();
    spec.label = Some("review".into());
    std::fs::write(dir.join("spec.json"), serde_json::to_vec(&spec).unwrap()).unwrap();
    SessionDir::at(dir)
}

#[tokio::test]
async fn a_host_is_found_from_the_file_its_attach_wrote_and_only_with_a_herdr_pane() {
    let root = tempfile::tempdir().expect("a root");
    let session = session(root.path());
    let provider = provider(root.path());
    assert!(provider.hosting().is_empty());

    let plain = kampr_keep::register_host(
        &session,
        Host {
            pid: std::process::id(),
            herdr_pane: None,
            herdr_socket: None,
            cols: 80,
            rows: 24,
            answers_from: None,
        },
    )
    .unwrap();
    assert!(
        provider.hosting().is_empty(),
        "a terminal that is not herdr hosts no desk pane"
    );
    drop(plain);

    let _desk = kampr_keep::register_host(
        &session,
        Host {
            pid: std::process::id(),
            herdr_pane: Some("w1:p3".into()),
            herdr_socket: Some("/run/herdr.sock".into()),
            cols: 80,
            rows: 24,
            answers_from: None,
        },
    )
    .unwrap();
    let hosting = provider.hosting();
    assert_eq!(hosting.len(), 1);
    assert_eq!(hosting[0].herdr_pane, "w1:p3");
    assert_eq!(hosting[0].socket, PathBuf::from("/run/herdr.sock"));
    assert_eq!(hosting[0].native_pane, format!("n:{ID}"));
    let report = provider.report(&hosting[0].native_pane).expect("a report");
    assert_eq!(
        (report.agent.as_str(), report.title.as_str()),
        ("kampr", "review")
    );
}

/// herdr refuses a `resume_argv` whose first element is a path, and a restored pane does not get
/// the `env` it was split with, so the root has to ride in the argv (#583).
#[tokio::test]
async fn what_herdr_is_told_to_run_starts_with_a_plain_command_and_carries_the_root() {
    let root = tempfile::tempdir().expect("a root");
    let provider = provider(root.path());
    let argv = provider.resume_argv(ID);
    assert!(!argv[0].contains('/'), "{argv:?}");
    assert_eq!(
        argv[1..],
        [
            format!("KAMPR_KEEP_ROOT={}", root.path().display()),
            "/opt/kampr/bin/kampr".into(),
            "attach".into(),
            ID.into(),
        ]
    );
    let line = provider.attach_line(ID);
    assert!(line.starts_with(" unset HISTFILE; exec env "), "{line:?}");
    assert!(
        line.ends_with(&format!(" /opt/kampr/bin/kampr attach {ID}\r")),
        "{line:?}"
    );
}
