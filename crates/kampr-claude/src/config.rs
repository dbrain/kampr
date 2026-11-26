use serde::{Deserialize, Serialize};

/// The `[claude]` section of a node's config: the billing policy and its safeties.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Guard 1: `plan` starts a session that would be paid any other way refuses.
    pub billing: Billing,
    /// Guard 6: what an overage (`isUsingOverage`) does to a turn.
    pub overage: Overage,
    /// Guard 7: the cap on self-started turns since the operator's last prompt.
    #[serde(default = "unattended_default")]
    pub unattended_turns: u32,
    /// Guard 7: a `--max-budget-usd` brake, set on a plan it counts list price.
    pub turn_budget_usd: Option<f64>,
    /// The permission mode a session starts in; empty is the session's own.
    pub mode: String,
    /// A `CLAUDE_CONFIG_DIR` to run under; empty is the operator's own `~/.claude`.
    pub config_dir: String,
    /// The `claude` binary; empty is `claude` on `PATH`.
    pub binary: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            billing: Billing::default(),
            overage: Overage::default(),
            unattended_turns: unattended_default(),
            turn_budget_usd: None,
            mode: String::new(),
            config_dir: String::new(),
            binary: String::new(),
        }
    }
}

fn unattended_default() -> u32 {
    20
}

impl Config {
    pub fn config_dir(&self) -> Option<&str> {
        (!self.config_dir.is_empty()).then_some(self.config_dir.as_str())
    }

    pub fn binary(&self) -> &str {
        if self.binary.is_empty() {
            "claude"
        } else {
            self.binary.as_str()
        }
    }
}

/// Guard 1: how a session may be paid for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Billing {
    #[default]
    Plan,
    Api,
    Any,
}

/// Guard 6: the answer to a turn that has run onto usage credits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Overage {
    #[default]
    Refuse,
    Ask,
    Allow,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_section_is_a_plan_session_at_the_default_cap() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.billing, Billing::Plan);
        assert_eq!(config.overage, Overage::Refuse);
        assert_eq!(config.unattended_turns, 20);
        assert!(config.turn_budget_usd.is_none());
    }

    #[test]
    fn a_configured_section_reads_its_words() {
        let config: Config = toml::from_str(
            r#"
billing = "api"
overage = "ask"
unattended_turns = 4
turn_budget_usd = 2.5
mode = "acceptEdits"
config_dir = "/tmp/claude"
binary = "/usr/local/bin/claude"
"#,
        )
        .unwrap();
        assert_eq!(config.billing, Billing::Api);
        assert_eq!(config.overage, Overage::Ask);
        assert_eq!(config.unattended_turns, 4);
        assert_eq!(config.turn_budget_usd, Some(2.5));
        assert_eq!(config.mode, "acceptEdits");
        assert_eq!(config.config_dir(), Some("/tmp/claude"));
        assert_eq!(config.binary(), "/usr/local/bin/claude");
    }
}
