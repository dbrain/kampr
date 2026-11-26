use kampr_core::conversation::facts::{Billing, Fix, Trouble};

use crate::config::{Billing as Policy, Overage};

/// What `initialize` reports about how a session would be paid for, before a word is sent.
#[derive(Debug, Clone, Default)]
pub struct Account {
    pub subscription_type: Option<String>,
    pub api_key_source: Option<String>,
    pub token_source: Option<String>,
    pub api_provider: Option<String>,
    pub email: Option<String>,
}

impl Account {
    pub fn from(v: &serde_json::Value) -> Self {
        let s = |v: &serde_json::Value, key: &str| {
            v.get(key).and_then(serde_json::Value::as_str).map(str::to_string)
        };
        Self {
            subscription_type: s(v, "subscriptionType").or_else(|| s(v, "subscription_type")),
            api_key_source: s(v, "apiKeySource").or_else(|| s(v, "api_key_source")),
            token_source: s(v, "tokenSource").or_else(|| s(v, "token_source")),
            api_provider: s(v, "apiProvider").or_else(|| s(v, "api_provider")),
            email: s(v, "email"),
        }
    }
}

/// What the account is, in the words guard 3 names it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Plan,
    Key(String),
    NoLogin,
    ThirdParty(String),
}

pub fn classify(account: &Account) -> Kind {
    let key = account.api_key_source.as_deref();
    if key.is_some_and(|s| !s.is_empty() && s != "none") {
        return Kind::Key(key.unwrap_or_default().to_string());
    }
    if account.subscription_type.is_some() {
        return Kind::Plan;
    }
    if account.token_source.as_deref() == Some("none") {
        return Kind::NoLogin;
    }
    if account.api_provider.as_deref().is_some_and(|p| !p.is_empty() && p != "firstParty") {
        return Kind::ThirdParty(account.api_provider.clone().unwrap_or_default());
    }
    Kind::NoLogin
}

/// Guard 3: may this account start a session under `policy`?
pub fn account_allowed(policy: Policy, account: &Account) -> Result<(), BillingTrouble> {
    if policy == Policy::Any {
        return Ok(());
    }
    match classify(account) {
        Kind::Plan => Ok(()),
        kind if policy == Policy::Api && matches!(kind, Kind::Key(_) | Kind::ThirdParty(_)) => Ok(()),
        kind => Err(trouble_for(kind)),
    }
}

pub fn trouble_for(kind: Kind) -> BillingTrouble {
    match kind {
        Kind::Key(name) => BillingTrouble::new(format!(
            "a key in the environment ({name}) would pay this session; the policy is plan-only"
        )),
        Kind::NoLogin => BillingTrouble::new("no login: the session has no plan and no key to pay with"),
        Kind::ThirdParty(provider) => BillingTrouble::new(format!(
            "a third-party provider ({provider}) would pay this session; the policy is plan-only"
        )),
        Kind::Plan => BillingTrouble::new("unreachable: a plan account was refused"),
    }
}

/// A guard-3 refusal, named for what was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingTrouble {
    pub text: String,
}

impl BillingTrouble {
    fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn trouble(&self) -> Trouble {
        Trouble {
            kind: "billing".into(),
            text: self.text.clone(),
            status: "stopped".into(),
            retry: None,
            fix: Some(Fix::Billing),
        }
    }
}

/// Guard 4: a turn's `system/init` carries `apiKeySource`; under plan, anything but `"none"`
/// interrupts the turn and ends the session.
pub fn turn_key_source_ok(policy: Policy, api_key_source: Option<&str>) -> bool {
    if policy != Policy::Plan {
        return true;
    }
    !api_key_source.is_some_and(|s| !s.is_empty() && s != "none")
}

/// Guard 6: what a turn that has run onto usage credits does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverageDecision {
    Continue,
    Refuse,
    Ask,
}

pub fn overage_decision(policy: Overage, is_using_overage: bool) -> OverageDecision {
    if !is_using_overage {
        return OverageDecision::Continue;
    }
    match policy {
        Overage::Refuse => OverageDecision::Refuse,
        Overage::Ask => OverageDecision::Ask,
        Overage::Allow => OverageDecision::Continue,
    }
}

/// Guard 7: a turn the model started for itself, not one the operator asked for.
pub fn is_self_started(origin: Option<&str>) -> bool {
    origin.is_some_and(|o| {
        !o.is_empty()
            && o != "user"
            && o != "prompt"
            && (o.contains("task") || o == "schedule" || o == "cron" || o == "loop")
    })
}

/// Guard 9: where this session's usage is billed, for the pane's header.
pub fn billing_of(account: &Account, overage: bool) -> Billing {
    Billing {
        provider: account
            .subscription_type
            .clone()
            .or_else(|| account.api_provider.clone()),
        key: account.api_key_source.clone().filter(|s| !s.is_empty() && s != "none"),
        rate_limited: overage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account(v: serde_json::Value) -> Account {
        Account::from(&v)
    }

    fn plan() -> Account {
        account(json!({"subscriptionType": "Claude Max", "apiProvider": "firstParty"}))
    }
    fn key() -> Account {
        account(json!({"tokenSource": "claude.ai", "apiKeySource": "ANTHROPIC_API_KEY", "apiProvider": "firstParty"}))
    }
    fn logged_out() -> Account {
        account(json!({"tokenSource": "none", "apiProvider": "firstParty"}))
    }
    fn third_party() -> Account {
        account(json!({"tokenSource": "claude.ai", "apiProvider": "bedrock"}))
    }

    // guard 3: a plan starts under plan; everything else is refused and named.
    #[test]
    fn a_plan_account_starts_under_plan() {
        assert!(account_allowed(Policy::Plan, &plan()).is_ok());
        assert!(account_allowed(Policy::Any, &plan()).is_ok());
    }

    #[test]
    fn a_key_in_the_environment_is_refused_under_plan_and_named() {
        let err = account_allowed(Policy::Plan, &key()).unwrap_err();
        assert!(err.text.contains("key in the environment"), "{}", err.text);
        assert!(err.text.contains("ANTHROPIC_API_KEY"), "{}", err.text);
        // the operator who chose api billing is allowed the key.
        assert!(account_allowed(Policy::Api, &key()).is_ok());
    }

    #[test]
    fn a_logged_out_session_is_refused_under_plan_and_named() {
        let err = account_allowed(Policy::Plan, &logged_out()).unwrap_err();
        assert!(err.text.contains("no login"), "{}", err.text);
        assert!(err.trouble().fix == Some(Fix::Billing));
        assert!(account_allowed(Policy::Any, &logged_out()).is_ok());
    }

    #[test]
    fn a_third_party_provider_is_refused_under_plan_and_named() {
        let err = account_allowed(Policy::Plan, &third_party()).unwrap_err();
        assert!(err.text.contains("third-party"), "{}", err.text);
        assert!(err.text.contains("bedrock"), "{}", err.text);
    }

    // guard 4: a per-turn key source other than "none" ends a plan session.
    #[test]
    fn a_plan_turn_with_a_key_source_is_refused() {
        assert!(turn_key_source_ok(Policy::Plan, Some("none")));
        assert!(turn_key_source_ok(Policy::Plan, None));
        assert!(!turn_key_source_ok(Policy::Plan, Some("ANTHROPIC_API_KEY")));
        assert!(turn_key_source_ok(Policy::Api, Some("ANTHROPIC_API_KEY")));
        assert!(turn_key_source_ok(Policy::Any, Some("ANTHROPIC_API_KEY")));
    }

    // guard 6: an overage is refused by default, held for ask, allowed for allow.
    #[test]
    fn an_overage_is_refused_by_default() {
        assert_eq!(overage_decision(Overage::Refuse, true), OverageDecision::Refuse);
        assert_eq!(overage_decision(Overage::Ask, true), OverageDecision::Ask);
        assert_eq!(overage_decision(Overage::Allow, true), OverageDecision::Continue);
        assert_eq!(overage_decision(Overage::Refuse, false), OverageDecision::Continue);
    }

    // guard 7: a task or schedule origin is self-started; an operator prompt is not.
    #[test]
    fn a_task_origin_is_self_started_and_a_prompt_is_not() {
        assert!(is_self_started(Some("task_notification")));
        assert!(is_self_started(Some("schedule")));
        assert!(is_self_started(Some("cron")));
        assert!(!is_self_started(None));
        assert!(!is_self_started(Some("user")));
        assert!(!is_self_started(Some("")));
    }

    // guard 9: the billing facts name the plan, a key, and an overage.
    #[test]
    fn the_billing_facts_name_what_pays_and_the_overage() {
        let plan = billing_of(&plan(), false);
        assert_eq!(plan.provider.as_deref(), Some("Claude Max"));
        assert!(plan.key.is_none());
        assert!(!plan.rate_limited);

        let key = billing_of(&key(), true);
        assert_eq!(key.key.as_deref(), Some("ANTHROPIC_API_KEY"));
        assert!(key.rate_limited);
    }
}