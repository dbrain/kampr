use kampr_core::conversation::facts::{Fix, Limits, RunState, Trouble};
use kampr_core::conversation::{Ask, AskAbility, AskOption, Update};
use serde_json::json;

use super::asks::Standing;
use super::facts;
use super::machine::Machine;
use super::state::Hold;
use crate::billing::{OverageDecision, overage_decision, window_exhausted};
use crate::config::Overage;
use crate::wire::RateLimit;

/// The kind of the trouble a spent window raises, so a fresh read can take it back down.
const LIMIT: &str = "limits";

fn billing(text: String, status: &str) -> Trouble {
    Trouble {
        kind: "billing".into(),
        text,
        status: status.into(),
        retry: None,
        fix: Some(Fix::Billing),
    }
}

pub(super) fn key_source_trouble(source: &str) -> Trouble {
    billing(
        format!("this turn is paid by a key in the environment ({source}); the policy is plan-only"),
        "stopped",
    )
}

pub(super) fn unattended_trouble(count: u32, cap: u32) -> Trouble {
    billing(
        format!("{count} self-started turns since your last prompt; the cap is {cap}"),
        "paused",
    )
}

fn overage_trouble(what: &str) -> Trouble {
    billing(
        format!("this turn is running on usage credits; overage is {what}"),
        "stopped",
    )
}

pub(super) fn account_trouble(text: String) -> Trouble {
    billing(text, "stopped")
}

impl Machine {
    /// A guard has found what it stops on: say why, interrupt the turn in flight and hang up on
    /// the child. The hang-up is what makes it a stop: a trouble the pane is told of while the
    /// turn runs on is a warning, and a warning does not hold back money.
    pub(super) fn end_session(&mut self, trouble: Trouble) {
        self.st.trouble = Some(trouble.clone());
        self.tell(Update::Trouble(Some(trouble)));
        self.fire("interrupt", json!({}));
        self.child.close();
        self.st.run = RunState::Ended;
        self.tell(Update::State(RunState::Ended));
        self.finish();
    }

    /// Guard 6, and the limits on screen: a plan running on its usage credits.
    pub(super) fn rate_limit(&mut self, limit: RateLimit) {
        self.st.overage = self.st.overage || limit.is_using_overage;
        let percent = |w: &Option<crate::wire::Window>| w.as_ref().map(|w| w.utilization * 100.0);
        let resets = |w: &Option<crate::wire::Window>| w.as_ref().and_then(|w| w.resets_at).map(facts::iso);
        let limits = Limits {
            plan: self.plan_name(),
            usage: Some(
                json!({ "five_hour": percent(&limit.five_hour), "seven_day": percent(&limit.seven_day) }),
            ),
            resets_at: resets(&limit.five_hour).or_else(|| resets(&limit.seven_day)),
        };
        self.st.limits = Some(limits.clone());
        self.say(Update::Limits(limits));
        self.publish_session();
        let policy = if self.st.overage_allowed {
            Overage::Allow
        } else {
            self.settings.overage
        };
        match overage_decision(policy, limit.is_using_overage) {
            OverageDecision::Continue => {}
            OverageDecision::Refuse => self.end_session(overage_trouble("refused")),
            OverageDecision::Ask => self.hold_for_overage(),
        }
    }

    /// `overage = "ask"`: stop the turn that is spending credits, keep its prompt and whatever
    /// follows, and wait for a person to say the credits may be spent.
    fn hold_for_overage(&mut self) {
        if self.st.hold == Some(Hold::Overage) {
            return;
        }
        if self.st.run == RunState::Running || self.st.in_flight.is_some() {
            self.interrupt();
        }
        if let Some(word) = self.st.in_flight.take() {
            self.st.held.insert(0, word);
            self.settings.kept.hold(&self.st.held);
        }
        self.st.hold = Some(Hold::Overage);
        self.st.standing.push(Standing::Overage);
        self.say_asks();
    }

    /// Guard 10: what `get_usage` says of the plan's windows, read before a word is sent. A
    /// spent window is a turn that would run on credits when the account can pay with them, so
    /// it is held the way guard 6 holds one; where the account cannot, the pane is told and the
    /// API's own refusal is what a prompt would meet (#685).
    pub(super) fn usage_read(&mut self, usage: &serde_json::Value) {
        let window = |key: &str| usage.pointer(&format!("/rate_limits/{key}"));
        let percent = |key: &str| {
            window(key)
                .and_then(|w| w.get("utilization"))
                .and_then(serde_json::Value::as_f64)
        };
        let resets = |key: &str| {
            window(key)
                .and_then(|w| w.get("resets_at"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        let (five, seven) = (percent("five_hour"), percent("seven_day"));
        let limits = Limits {
            plan: self.plan_name(),
            usage: Some(json!({ "five_hour": five, "seven_day": seven })),
            resets_at: resets("five_hour").or_else(|| resets("seven_day")),
        };
        self.st.limits = Some(limits.clone());
        self.say(Update::Limits(limits));

        if !(window_exhausted(five) || window_exhausted(seven)) {
            if self.st.hold == Some(Hold::Limit) {
                self.st.hold = None;
            }
            if self.st.trouble.as_ref().is_some_and(|t| t.kind == LIMIT) {
                self.set_trouble(None);
            }
            return self.release();
        }
        let credits = usage
            .pointer("/rate_limits/extra_usage/is_enabled")
            .and_then(serde_json::Value::as_bool)
            != Some(false);
        let until = if window_exhausted(five) {
            resets("five_hour")
        } else {
            resets("seven_day")
        };
        let fix = until.map_or(Fix::Billing, |until| Fix::Wait { until });
        match self.settings.overage {
            Overage::Allow => {}
            Overage::Ask if credits => self.hold_for_overage(),
            Overage::Refuse if credits => {
                self.st.hold = Some(Hold::Limit);
                self.set_trouble(Some(Trouble {
                    kind: LIMIT.into(),
                    text: "a limit of the plan is spent; a prompt would run on usage credits, and overage is refused".into(),
                    status: "held".into(),
                    retry: None,
                    fix: Some(fix),
                }));
            }
            _ => self.set_trouble(Some(Trouble {
                kind: LIMIT.into(),
                text: "a limit of the plan is spent".into(),
                status: "limit".into(),
                retry: None,
                fix: Some(fix),
            })),
        }
    }

    fn plan_name(&self) -> Option<String> {
        self.st.account.as_ref().and_then(|a| a.subscription_type.clone())
    }
}

pub(super) fn overage_ask() -> Ask {
    let option = |key: &str, label: &str| AskOption {
        key: key.into(),
        label: label.into(),
        detail: None,
        chosen: false,
        text: false,
    };
    Ask {
        id: Some("overage".into()),
        kind: AskAbility::Always,
        by: Some("billing".into()),
        header: Some("usage credits".into()),
        question: "this turn is running on usage credits; allow it to continue?".into(),
        multi: false,
        context: None,
        // Keys 1 and 3 are the ones the node reads as allow and deny.
        options: vec![option("1", "Allow"), option("3", "Stop")],
        fields: Vec::new(),
    }
}
