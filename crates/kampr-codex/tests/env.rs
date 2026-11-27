//! The production launch path, `CodexSource::from_env`, against the probe's mock: the node's own
//! environment and the operator's login profile carry every variable codex reads for auth or an
//! endpoint (probe row 697), and the child's actual environment, which the launcher writes down
//! before it becomes the mock, carries the profile's `PATH` and the rest of it, and none of those.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use kampr_codex::CodexSource;
use kampr_core::conversation::{NewSession, Source, Update};
use tokio::sync::mpsc;

const SCRUBBED: &[&str] = &[
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "CODEX_ACCESS_TOKEN",
    "OPENAI_BASE_URL",
    "OPENAI_ORGANIZATION",
    "AWS_BEARER_TOKEN_BEDROCK",
    "GOOGLE_CLOUD_PROJECT",
    "CODEX_OSS_BASE_URL",
    "CODEX_APP_SERVER_CHATGPT_BASE_URL",
    "OPENAI_IDENTITY_TOKEN_FILE",
    "OPENAI_FEDERATION_RULE_ID",
    "CODEX_OSS_PORT",
];

fn launcher(dir: &Path) -> String {
    let mock = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../research/probe/codex-app-server/mock-stdio.py")
        .canonicalize()
        .unwrap();
    let path = dir.join("codex");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\npython3 -c 'import os, sys; open(sys.argv[1], \"wb\").write(b\"\\0\".join(k + b\"=\" + v for k, v in os.environb.items()))' '{}'\nexec python3 '{}' \"$@\"\n",
            dir.join("environ").display(),
            mock.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

/// A login shell from an environment as bare as a service manager's, whose profile is the one in
/// `home`: a `PATH` entry, a variable of its own, `CODEX_HOME`, and a key.
fn login_shell(dir: &Path, home: &Path, codex_home: &Path) -> String {
    let bin = dir.join("profile-bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        home.join(".profile"),
        format!(
            "echo welcome back\nexport PATH='{}':\"$PATH\"\nexport FOO_FROM_PROFILE=1\n\
             export CODEX_HOME='{}'\nexport OPENAI_API_KEY=leak\nexport ANTHROPIC_API_KEY=leak\n",
            bin.display(),
            codex_home.display()
        ),
    )
    .unwrap();
    let shell = dir.join("login-sh");
    std::fs::write(
        &shell,
        format!(
            "#!/bin/sh\nHOME='{}' PATH=/usr/bin:/bin exec /bin/sh \"$@\"\n",
            home.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
    shell.display().to_string()
}

fn environ(dir: &Path) -> Vec<(String, String)> {
    std::fs::read(dir.join("environ"))
        .unwrap()
        .split(|b| *b == 0)
        .filter_map(|kv| {
            let kv = String::from_utf8_lossy(kv);
            kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

#[test]
fn the_codex_child_starts_with_the_login_environment_less_the_provider_keys() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("codex-home");
    let user = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&user).unwrap();
    let shell = login_shell(dir.path(), &user, &home);
    // Set before any runtime thread exists; this binary holds this one test for that reason.
    unsafe {
        for key in SCRUBBED {
            std::env::set_var(key, "kampr-test-must-not-cross");
        }
        std::env::set_var("KAMPR_CODEX", launcher(dir.path()));
        std::env::remove_var("CODEX_HOME");
    }

    let env = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let source = CodexSource::from_env(&shell);
            let id = source
                .create(NewSession {
                    cwd: None,
                    prompt: None,
                })
                .await
                .unwrap();
            let conversation = source.open(&id).await.unwrap();
            let (_tx, rx) = mpsc::channel(1);
            let (seen_tx, mut seen) = mpsc::unbounded_channel();
            tokio::spawn(conversation.run(
                rx,
                Box::new(move |u: Update| seen_tx.send(matches!(u, Update::Session(_))).is_ok()),
            ));
            let deadline = Instant::now() + Duration::from_secs(20);
            while !tokio::time::timeout_at(deadline.into(), seen.recv())
                .await
                .unwrap()
                .unwrap()
            {}
            let env = environ(dir.path());
            source.herd().end(&id).await;
            env
        });

    let value = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    for key in SCRUBBED {
        assert_eq!(value(key), None, "{key} reached the codex child: {env:?}");
    }
    let bin = dir.path().join("profile-bin").display().to_string();
    assert!(
        value("PATH").is_some_and(|p| p.split(':').any(|d| d == bin)),
        "{:?}",
        value("PATH")
    );
    assert_eq!(value("FOO_FROM_PROFILE"), Some("1"));
    assert_eq!(
        value("OPENAI_API_KEY"),
        None,
        "the profile's key reached the codex child"
    );
    assert_eq!(
        value("CODEX_HOME"),
        Some(home.to_str().unwrap()),
        "the operator's own codex home, not one of Kampr's"
    );
}
