use std::path::Path;

use kampr_keep::{Launch, Mode, Spec};

use crate::config::Billing as Policy;

/// The environment names the allow-list carries under plan (guard 2). Nothing beginning
/// `ANTHROPIC_`, `AWS_`, `GOOGLE_` or `CLAUDE_CODE_USE_` crosses under plan.
pub const ALLOWED: &[&str] = &["HOME", "PATH", "USER", "LANG", "TERM", "SSH_AUTH_SOCK"];
pub const DENIED_PREFIXES: &[&str] = &["ANTHROPIC_", "AWS_", "GOOGLE_", "CLAUDE_CODE_USE_"];

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
    /// A session id to resume (an `open` of a stored transcript).
    pub resume: Option<String>,
    pub session_id: String,
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
    if let Some(id) = &plan.resume {
        args.push("-r".into());
        args.push(id.clone());
    }
    Ok(args)
}

/// Guard 2: the scrubbed environment. An allow-list, not the node's environment minus a
/// deny-list: only the named names and `XDG_*` cross, and a `CLAUDE_CONFIG_DIR` only when the
/// operator configured one. Provider keys cross only when the operator chose api or any billing.
pub fn env(plan: &Plan, inherited: impl IntoIterator<Item = (String, String)>) -> Vec<(String, String)> {
    let inherited: Vec<(String, String)> = inherited.into_iter().collect();
    let mut out: Vec<(String, String)> = inherited
        .iter()
        .filter(|(k, _)| {
            if k == "CLAUDE_CONFIG_DIR" {
                return plan.config_dir.is_some();
            }
            if ALLOWED.contains(&k.as_str()) || k.starts_with("XDG_") {
                return true;
            }
            plan.policy != Policy::Plan && DENIED_PREFIXES.iter().any(|p| k.starts_with(p))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Some(url) = &plan.base_url {
        out.retain(|(k, _)| k != "ANTHROPIC_BASE_URL");
        out.push(("ANTHROPIC_BASE_URL".into(), url.clone()));
    }
    if let Some(dir) = &plan.config_dir {
        out.retain(|(k, _)| k != "CLAUDE_CONFIG_DIR");
        out.push(("CLAUDE_CONFIG_DIR".into(), dir.clone()));
    }
    // The keeper layers `spec.env` over its own environment, so a provider key the node carries
    // is removed by blanking it here, not by omitting it. Under `plan` nothing provider-shaped
    // may cross; `ANTHROPIC_BASE_URL` is kept only while a mock address is set (under
    // `KAMPR_TEST`), so a stray base URL does not reach a production plan child.
    if plan.policy == Policy::Plan {
        for (k, _) in inherited.into_iter() {
            if k.starts_with("ANTHROPIC_")
                && (k != "ANTHROPIC_BASE_URL" || plan.base_url.is_none())
                || k.starts_with("AWS_")
                || k.starts_with("GOOGLE_")
                || k.starts_with("CLAUDE_CODE_USE_")
            {
                out.retain(|(have, _)| have != &k);
                out.push((k, String::new()));
            }
        }
    }
    out
}

/// The keeper's `Spec` for a headless Claude: the launch line, the scrubbed environment, and
/// pipe mode — the child's stdin and stdout are the keeper's pipes, not a pty.
pub fn spec(plan: &Plan, inherited: impl IntoIterator<Item = (String, String)>) -> Result<Spec, LaunchError> {
    Ok(Spec {
        id: plan.session_id.clone(),
        argv: argv(plan)?,
        cwd: plan.cwd.clone(),
        env: env(plan, inherited),
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
    inherited: impl IntoIterator<Item = (String, String)>,
    launch: Launch,
    exe: &Path,
) -> Result<kampr_keep::SessionDir, LaunchError> {
    let spec = spec(plan, inherited)?;
    kampr_keep::spawn(root, spec, launch, exe)
        .map_err(|e| LaunchError::Spawn(e.to_string()))
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
            session_id: "s1".into(),
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    // guard 2: only the allow-list crosses under plan; a provider key does not.
    #[test]
    fn the_scrubbed_env_keeps_only_the_allow_list_under_plan() {
        let plan = plan();
        let env = env(&plan, vars(&[
            ("HOME", "/home/u"),
            ("PATH", "/usr/bin"),
            ("ANTHROPIC_API_KEY", "sk-ant-api03-bogus"),
            ("AWS_SECRET", "nope"),
            ("GOOGLE_APPLICATION_CREDENTIALS", "nope"),
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("XDG_CONFIG_HOME", "/home/u/.config"),
            ("SSH_AUTH_SOCK", "/tmp/sock"),
            ("OTHER", "x"),
        ]));
        let keys: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let value = |k: &str| keys.iter().find(|(name, _)| *name == k).map(|(_, v)| *v);
        assert!(value("HOME").is_some());
        assert!(value("PATH").is_some());
        assert!(value("XDG_CONFIG_HOME").is_some());
        assert!(value("SSH_AUTH_SOCK").is_some());
        // A stray provider key is blanked, not present with its value: the keeper layers this
        // over its own env, so empty is what stops the node's key crossing under plan.
        assert_eq!(value("ANTHROPIC_API_KEY"), Some(""), "{keys:?}");
        assert_eq!(value("AWS_SECRET"), Some(""), "{keys:?}");
        assert_eq!(value("GOOGLE_APPLICATION_CREDENTIALS"), Some(""), "{keys:?}");
        assert_eq!(value("CLAUDE_CODE_USE_BEDROCK"), Some(""), "{keys:?}");
        assert_eq!(value("OTHER"), None, "{keys:?}");
        assert_eq!(value("CLAUDE_CONFIG_DIR"), None, "no config dir was configured: {keys:?}");
    }

    // guard 2, the other way: an operator who chose api billing lets the provider keys cross.
    #[test]
    fn a_provider_key_crosses_under_api_billing() {
        let mut plan = plan();
        plan.policy = Policy::Api;
        let env = env(&plan, vars(&[("ANTHROPIC_API_KEY", "sk-ant-api03-bogus"), ("HOME", "/h")]));
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"ANTHROPIC_API_KEY"), "{keys:?}");
    }

    // guard 2: a configured CLAUDE_CONFIG_DIR is the one that crosses.
    #[test]
    fn a_configured_config_dir_crosses() {
        let mut plan = plan();
        plan.config_dir = Some("/tmp/claude".into());
        let env = env(&plan, vars(&[("CLAUDE_CONFIG_DIR", "/should-not-cross")]));
        let value = env.iter().find(|(k, _)| k == "CLAUDE_CONFIG_DIR").map(|(_, v)| v.as_str());
        assert_eq!(value, Some("/tmp/claude"));
    }

    // guard 5: the launch line never carries --bare, and always offers bypass.
    #[test]
    fn the_launch_line_never_bare_and_always_offers_bypass() {
        let args = argv(&plan()).unwrap();
        assert!(args.iter().any(|a| a == "--allow-dangerously-skip-permissions"));
        assert!(!args.contains(&"--bare".to_string()), "{args:?}");
        assert!(args.iter().any(|a| a == "stream-json"));
    }

    // guard 7: a turn budget becomes --max-budget-usd.
    #[test]
    fn a_turn_budget_becomes_max_budget_usd() {
        let mut plan = plan();
        plan.max_budget_usd = Some(2.5);
        plan.permission_mode = Some("acceptEdits".into());
        plan.resume = Some("abc".into());
        let args = argv(&plan).unwrap();
        let at = args.iter().position(|a| a == "--max-budget-usd").unwrap();
        assert_eq!(args[at + 1], "2.5");
        let at = args.iter().position(|a| a == "--permission-mode").unwrap();
        assert_eq!(args[at + 1], "acceptEdits");
        let at = args.iter().position(|a| a == "-r").unwrap();
        assert_eq!(args[at + 1], "abc");
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

    // guard 11, the other half: the mock address is what the child is pointed at.
    #[test]
    fn the_mock_address_reaches_the_child_env() {
        let mut plan = plan();
        plan.policy = Policy::Any;
        plan.base_url = Some("http://127.0.0.1:18080".into());
        let env = env(&plan, vars(&[("ANTHROPIC_BASE_URL", "http://real/"), ("HOME", "/h")]));
        let url = env.iter().find(|(k, _)| k == "ANTHROPIC_BASE_URL").map(|(_, v)| v.as_str());
        assert_eq!(url, Some("http://127.0.0.1:18080"));
    }

    // guard 2, the production half: with no mock address a stray base URL is blanked under plan,
    // so it does not reach a production child (the mock address is the only case that is kept).
    #[test]
    fn a_stray_base_url_is_blanked_under_plan_without_a_mock() {
        let plan = plan();
        let env = env(&plan, vars(&[("ANTHROPIC_BASE_URL", "http://real/"), ("HOME", "/h")]));
        let url = env.iter().find(|(k, _)| k == "ANTHROPIC_BASE_URL").map(|(_, v)| v.as_str());
        assert_eq!(url, Some(""), "a production plan child must not inherit a base URL");
    }

    // guard 2, the test half: a mock address is only ever present under KAMPR_TEST, and while it
    // is set it is what crosses under plan — a stray node base URL does not.
    #[test]
    fn the_mock_address_crosses_under_plan_while_a_mock_is_set() {
        let mut plan = plan();
        plan.base_url = Some("http://127.0.0.1:18080".into());
        let env = env(&plan, vars(&[("ANTHROPIC_BASE_URL", "http://real/"), ("HOME", "/h")]));
        let url = env.iter().find(|(k, _)| k == "ANTHROPIC_BASE_URL").map(|(_, v)| v.as_str());
        assert_eq!(url, Some("http://127.0.0.1:18080"), "the mock, not a stray URL: {env:?}");
    }
}