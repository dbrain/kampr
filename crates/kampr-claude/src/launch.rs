use std::path::Path;

use kampr_core::child_env;
use kampr_keep::{Launch, Mode, Spec};

use crate::config::Billing as Policy;

/// Everything a launch needs, gathered before a word is sent.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub binary: String,
    pub cwd: Option<String>,
    pub policy: Policy,
    /// A `CLAUDE_CONFIG_DIR` to run under; `None` is the operator's own `~/.claude`.
    pub config_dir: Option<String>,
    /// The mock address, under `KAMPR_TEST=1`. Guard 11.
    pub base_url: Option<String>,
    /// The permission mode a session starts in (from `[claude] mode`).
    pub permission_mode: Option<String>,
    /// A `--max-budget-usd` brake (from `[claude] turn_budget_usd`).
    pub max_budget_usd: Option<f64>,
    /// A Claude session to resume (`-r`); the transcript it continues is its own.
    pub resume: Option<String>,
    /// The Claude session id a new conversation is given (`--session-id`), so its transcript
    /// is named by something Kampr chose. Unused when `resume` names one.
    pub conversation: Option<String>,
    /// The keeper session's own id.
    pub id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("under KAMPR_TEST=1 a mock ANTHROPIC_BASE_URL is required; refusing to start")]
    NoMockUnderTest,
    #[error("the keeper refused to start the session: {0}")]
    Spawn(String),
}

/// Guard 5: the launch line. `--allow-dangerously-skip-permissions` is always on offer, so the
/// mode control lists bypass whatever mode the session started in; and `--bare` is never passed,
/// because a bare session reads no login and reports `tokenSource: "none"`.
pub fn argv(plan: &Plan) -> Result<Vec<String>, LaunchError> {
    if std::env::var("KAMPR_TEST").ok().as_deref() == Some("1") && plan.base_url.is_none() {
        return Err(LaunchError::NoMockUnderTest);
    }
    let mut args = vec![
        plan.binary.clone(),
        "-p".into(),
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
        "--permission-prompt-tool".into(),
        "stdio".into(),
        "--allow-dangerously-skip-permissions".into(),
        "--forward-subagent-text".into(),
    ];
    if let Some(mode) = &plan.permission_mode {
        args.push("--permission-mode".into());
        args.push(mode.clone());
    }
    if let Some(budget) = plan.max_budget_usd {
        args.push("--max-budget-usd".into());
        args.push(format!("{budget}"));
    }
    match (&plan.resume, &plan.conversation) {
        (Some(id), _) => args.extend(["-r".into(), id.clone()]),
        (None, Some(id)) => args.extend(["--session-id".into(), id.clone()]),
        (None, None) => {}
    }
    Ok(args)
}

/// Guard 2: the operator's login environment less what decides who pays or where a turn goes
/// (`kampr_core::child_env::CLAUDE`), which crosses only when the operator chose api or any
/// billing; a `CLAUDE_CONFIG_DIR` only when the operator configured one. A pipe-mode keeper
/// starts the child from this alone.
pub fn env(plan: &Plan, login: impl IntoIterator<Item = (String, String)>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = match plan.policy {
        Policy::Plan => child_env::CLAUDE.strip(login),
        _ => login.into_iter().collect(),
    };
    out.retain(|(k, _)| k != "CLAUDE_CONFIG_DIR");
    if let Some(url) = &plan.base_url {
        out.retain(|(k, _)| k != "ANTHROPIC_BASE_URL");
        out.push(("ANTHROPIC_BASE_URL".into(), url.clone()));
    }
    if let Some(dir) = &plan.config_dir {
        out.push(("CLAUDE_CONFIG_DIR".into(), dir.clone()));
    }
    out
}

/// The keeper's `Spec` for a headless Claude: the launch line, the guard-2 environment, and
/// pipe mode — the child's stdin and stdout are the keeper's pipes, not a pty.
pub fn spec(plan: &Plan, login: impl IntoIterator<Item = (String, String)>) -> Result<Spec, LaunchError> {
    Ok(Spec {
        id: plan.id.clone(),
        argv: argv(plan)?,
        cwd: plan.cwd.clone(),
        env: env(plan, login),
        mode: Mode::Pipe,
        cols: 0,
        rows: 0,
        label: None,
        log_cap: None,
        created_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        cohort: None,
        workspace: None,
        container: None,
    })
}

/// Guard 11, the keeper side of it: a spawn under test with no mock is refused before a word is
/// sent, so the launcher and the keeper agree on the refusal.
pub fn spawn(
    root: &Path,
    plan: &Plan,
    login: impl IntoIterator<Item = (String, String)>,
    launch: Launch,
    exe: &Path,
) -> Result<kampr_keep::SessionDir, LaunchError> {
    let spec = spec(plan, login)?;
    kampr_keep::spawn(root, spec, launch, exe).map_err(|e| LaunchError::Spawn(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            binary: "claude".into(),
            cwd: Some("/w".into()),
            policy: Policy::Plan,
            config_dir: None,
            base_url: None,
            permission_mode: None,
            max_budget_usd: None,
            resume: None,
            conversation: Some("0b1c2d3e-0000-4000-8000-000000000001".into()),
            id: "s1".into(),
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn value<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    // guard 2: under plan the login environment crosses less what decides who pays or where.
    #[test]
    fn under_plan_the_login_env_crosses_less_what_pays() {
        let env = env(
            &plan(),
            vars(&[
                ("HOME", "/home/u"),
                ("PATH", "/usr/bin"),
                ("ANTHROPIC_API_KEY", "sk-ant-api03-bogus"),
                ("ANTHROPIC_AUTH_TOKEN", "t"),
                ("AWS_SECRET", "nope"),
                ("GOOGLE_APPLICATION_CREDENTIALS", "nope"),
                ("CLAUDE_CODE_USE_BEDROCK", "1"),
                ("CLAUDE_CODE_OAUTH_TOKEN", "another-account"),
                ("CLAUDE_CONFIG_DIR", "/node/claude"),
                ("XDG_CONFIG_HOME", "/home/u/.config"),
                ("SSH_AUTH_SOCK", "/tmp/sock"),
                ("OTHER", "x"),
            ]),
        );
        for kept in [
            "HOME",
            "PATH",
            "XDG_CONFIG_HOME",
            "SSH_AUTH_SOCK",
            "OTHER",
            "AWS_SECRET",
            "GOOGLE_APPLICATION_CREDENTIALS",
        ] {
            assert!(value(&env, kept).is_some(), "{kept} crosses: {env:?}");
        }
        for dropped in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CONFIG_DIR",
        ] {
            assert_eq!(value(&env, dropped), None, "{dropped} must not cross: {env:?}");
        }
    }

    // guard 2, the other way: an operator who chose api billing lets the provider keys cross.
    #[test]
    fn a_provider_key_crosses_under_api_billing() {
        let mut plan = plan();
        plan.policy = Policy::Api;
        let env = env(
            &plan,
            vars(&[("ANTHROPIC_API_KEY", "sk-ant-api03-bogus"), ("HOME", "/h")]),
        );
        assert_eq!(value(&env, "ANTHROPIC_API_KEY"), Some("sk-ant-api03-bogus"));
    }

    // guard 2: a configured CLAUDE_CONFIG_DIR is the one that crosses, never the node's own.
    #[test]
    fn a_configured_config_dir_crosses() {
        let mut plan = plan();
        plan.config_dir = Some("/tmp/claude".into());
        let env = env(&plan, vars(&[("CLAUDE_CONFIG_DIR", "/should-not-cross")]));
        assert_eq!(value(&env, "CLAUDE_CONFIG_DIR"), Some("/tmp/claude"));
    }

    // guard 5: the launch line never carries --bare, and always offers bypass.
    #[test]
    fn the_launch_line_never_bare_and_always_offers_bypass() {
        let args = argv(&plan()).unwrap();
        assert!(args.iter().any(|a| a == "--allow-dangerously-skip-permissions"));
        assert!(!args.contains(&"--bare".to_string()), "{args:?}");
        assert!(args.iter().any(|a| a == "stream-json"));
    }

    fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let at = args.iter().position(|a| a == flag)?;
        args.get(at + 1).map(String::as_str)
    }

    // guard 7: a turn budget becomes --max-budget-usd.
    #[test]
    fn a_turn_budget_and_a_mode_become_flags() {
        let mut plan = plan();
        plan.max_budget_usd = Some(2.5);
        plan.permission_mode = Some("acceptEdits".into());
        let args = argv(&plan).unwrap();
        assert_eq!(after(&args, "--max-budget-usd"), Some("2.5"));
        assert_eq!(after(&args, "--permission-mode"), Some("acceptEdits"));
    }

    // The transcript is found by the id Kampr chose, so the launch line has to carry it; a
    // resume continues the session it names and is given no second id.
    #[test]
    fn a_new_conversation_is_named_and_a_resume_is_not() {
        let args = argv(&plan()).unwrap();
        assert_eq!(
            after(&args, "--session-id"),
            Some("0b1c2d3e-0000-4000-8000-000000000001")
        );
        assert!(!args.iter().any(|a| a == "-r"), "{args:?}");

        let mut resume = plan();
        resume.resume = Some("abc".into());
        let args = argv(&resume).unwrap();
        assert_eq!(after(&args, "-r"), Some("abc"));
        assert!(!args.iter().any(|a| a == "--session-id"), "{args:?}");
    }

    // guard 11: under KAMPR_TEST=1 the launcher refuses without a mock address.
    #[test]
    fn under_test_the_launcher_refuses_without_a_mock() {
        unsafe { std::env::set_var("KAMPR_TEST", "1") };
        let err = argv(&plan()).unwrap_err();
        assert!(matches!(err, LaunchError::NoMockUnderTest));
        let mut plan = plan();
        plan.base_url = Some("http://127.0.0.1:9".into());
        assert!(argv(&plan).is_ok());
        unsafe { std::env::remove_var("KAMPR_TEST") };
    }

    // guard 11, the other half: the mock address is what the child is pointed at, under every
    // policy, and a stray base URL of the node's never is.
    #[test]
    fn the_mock_address_reaches_the_child_and_a_stray_base_url_does_not() {
        for policy in [Policy::Plan, Policy::Any] {
            let mut plan = plan();
            plan.policy = policy;
            plan.base_url = Some("http://127.0.0.1:18080".into());
            let env = env(
                &plan,
                vars(&[("ANTHROPIC_BASE_URL", "http://real/"), ("HOME", "/h")]),
            );
            assert_eq!(value(&env, "ANTHROPIC_BASE_URL"), Some("http://127.0.0.1:18080"));
        }
        let env = env(
            &plan(),
            vars(&[("ANTHROPIC_BASE_URL", "http://real/"), ("HOME", "/h")]),
        );
        assert_eq!(value(&env, "ANTHROPIC_BASE_URL"), None, "{env:?}");
    }
}
