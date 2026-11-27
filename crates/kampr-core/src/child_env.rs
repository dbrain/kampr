//! The environment a headless harness child starts with: the operator's login environment, read
//! by [`crate::login`] as their own terminal has it, less what decides who pays for a turn or
//! which endpoint answers it. A node run as a service has the service manager's `PATH`
//! (`/usr/local/bin:/usr/bin` on .208), so the node's own would leave the child's tools without
//! `~/.local/bin`, cargo, nvm, proxies or certificates.

use tokio::sync::OnceCell;
use tracing::warn;

/// What a harness reads from its environment to decide who pays or where a turn goes.
pub struct Payers {
    names: &'static [&'static str],
    prefixes: &'static [&'static str],
}

/// docs/21 guard 2: a key or another account's token beside the login, a third-party provider,
/// another endpoint.
pub const CLAUDE: Payers = Payers {
    names: &["CLAUDE_CODE_OAUTH_TOKEN"],
    prefixes: &["ANTHROPIC_", "CLAUDE_CODE_USE_"],
};

/// Probe row 697, and the OpenAI organisation and endpoint names beside them.
pub const CODEX: Payers = Payers {
    names: &[
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "CODEX_ACCESS_TOKEN",
        "OPENAI_BASE_URL",
        "OPENAI_ORGANIZATION",
        "OPENAI_PROJECT",
        "AWS_BEARER_TOKEN_BEDROCK",
        "GOOGLE_CLOUD_PROJECT",
        "OPENAI_IDENTITY_TOKEN_FILE",
        "OPENAI_FEDERATION_RULE_ID",
        "CODEX_OSS_BASE_URL",
        "CODEX_OSS_PORT",
        "CODEX_APP_SERVER_CHATGPT_BASE_URL",
    ],
    prefixes: &[],
};

impl Payers {
    pub fn pays(&self, name: &str) -> bool {
        self.names.contains(&name) || self.prefixes.iter().any(|p| name.starts_with(p))
    }

    pub fn strip(&self, env: impl IntoIterator<Item = (String, String)>) -> Vec<(String, String)> {
        env.into_iter().filter(|(k, _)| !self.pays(k)).collect()
    }
}

/// The operator's login environment, read once and kept for the life of the source that owns it.
pub struct LoginEnv {
    shell: String,
    captured: OnceCell<Vec<(String, String)>>,
}

impl LoginEnv {
    pub fn new(shell: impl Into<String>) -> Self {
        Self {
            shell: shell.into(),
            captured: OnceCell::new(),
        }
    }

    /// A shell that says nothing is asked again next time; meanwhile the child gets the node's
    /// own environment rather than none.
    pub async fn get(&self) -> Vec<(String, String)> {
        if let Some(env) = self.captured.get() {
            return env.clone();
        }
        let shell = self.shell.clone();
        let captured = tokio::task::spawn_blocking(move || crate::login::capture(&shell))
            .await
            .ok()
            .flatten();
        match captured {
            Some(env) => {
                let _ = self.captured.set(env.clone());
                env
            }
            None => {
                warn!(shell = %self.shell, "could not read the login environment; the child gets the node's");
                std::env::vars().collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_harness_strips_its_own_payers_and_nothing_else() {
        for (payers, gone, kept) in [
            (
                &CLAUDE,
                ["ANTHROPIC_API_KEY", "CLAUDE_CODE_USE_FOUNDRY"],
                ["OPENAI_API_KEY", "AWS_PROFILE"],
            ),
            (
                &CODEX,
                ["OPENAI_API_KEY", "CODEX_OSS_PORT"],
                ["ANTHROPIC_API_KEY", "AWS_PROFILE"],
            ),
        ] {
            let env = payers.strip(gone.iter().chain(&kept).map(|k| (k.to_string(), "v".to_string())));
            let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
            assert_eq!(names, kept);
        }
    }

    #[tokio::test]
    async fn a_shell_that_says_nothing_leaves_the_child_the_nodes_environment() {
        let env = LoginEnv::new("/bin/false").get().await;
        assert_eq!(env, std::env::vars().collect::<Vec<_>>());
    }
}
