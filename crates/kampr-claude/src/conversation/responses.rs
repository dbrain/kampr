use kampr_core::conversation::Update;
use serde_json::Value;

use super::controls::Catalog;
use super::facts::{context_use, file_suggestions};
use super::guards::account_trouble;
use super::machine::Machine;
use super::state::{Preflight, Request};
use crate::billing::{Account, account_allowed};
use crate::wire::ControlResponse;

impl Machine {
    /// The binary's answer to a request of ours. One whose id this run never sent is another
    /// run's, still in the log, and is not read.
    pub(super) fn control_response(&mut self, response: ControlResponse) {
        let Some(kind) = self.st.inflight.remove(&response.request_id) else {
            return;
        };
        match kind {
            Request::Initialize => self.initialized(&response),
            Request::Usage if response.ok => self.usage_read(&response.body),
            Request::Context if response.ok => {
                if let Some(context) = context_use(&response.body) {
                    self.st.context = Some(context);
                    self.say(Update::Context(context));
                }
            }
            Request::Suggest(query) if response.ok => self.say(Update::Suggested {
                kind: kampr_core::conversation::SuggestKind::Files,
                query,
                items: file_suggestions(&response.body),
            }),
            Request::TaskOutput(id) if response.ok => self.task_output(id, &response.body),
            Request::Setting { control, value } => self.setting_done(&control, &value, &response),
            Request::Usage | Request::Context | Request::Suggest(_) | Request::TaskOutput(_) => {}
        }
    }

    /// Guard 3: `initialize` names how the session would be paid for, before a word is sent. An
    /// answer that is not a plan under the plan policy ends the session; an answer that is not an
    /// answer does not allow it.
    fn initialized(&mut self, response: &ControlResponse) {
        if !response.ok {
            self.st.preflight = Preflight::Refused;
            let why = response.error.as_deref().unwrap_or("it gave no answer");
            return self.end_session(account_trouble(format!(
                "the account could not be checked: {why}"
            )));
        }
        let body = &response.body;
        let account = Account::from(body.get("account").unwrap_or(&Value::Null));
        self.st.catalog = Catalog::from(body);
        // An SDK session is told fast mode needs an opt-in this source cannot make (#696), so the
        // control is not offered rather than offered and refused.
        self.st.fast = body
            .get("fast_mode_state")
            .and_then(Value::as_str)
            .filter(|_| body.get("fast_mode_disabled_reason").is_none())
            .map(|state| state == "on");
        if let Some(mode) = body.get("current_permission_mode").and_then(Value::as_str) {
            self.st.mode = Some(mode.to_string());
        }
        let verdict = account_allowed(self.settings.policy, &account);
        self.st.account = Some(account);
        if let Err(trouble) = verdict {
            self.st.preflight = Preflight::Refused;
            self.publish_session();
            return self.end_session(account_trouble(trouble.text));
        }
        self.st.preflight = Preflight::Allowed;
        self.publish_controls();
        self.publish_session();
        self.refresh_reads();
        self.release();
    }

    fn task_output(&mut self, id: String, body: &Value) {
        let Some(last) = self.st.followed.get_mut(&id) else {
            return;
        };
        let total = body.get("total_bytes").and_then(Value::as_u64).unwrap_or(0);
        if total == *last {
            return;
        }
        *last = total;
        let text = body
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let at = total.saturating_sub(text.len() as u64);
        self.say(Update::TaskOutput {
            id,
            at,
            text,
            ended: None,
        });
    }

    fn setting_done(&mut self, control: &str, value: &Value, response: &ControlResponse) {
        if !response.ok {
            let why = response.error.as_deref().unwrap_or("the session refused it");
            return self.notice(format!("{control} was not changed: {why}"));
        }
        match control {
            "model" => self.st.model = value.as_str().map(str::to_string),
            "mode" | "bypass" => self.st.mode = value.as_str().map(str::to_string),
            "fast" => self.st.fast = value.as_bool(),
            _ => {}
        }
        self.publish_controls();
        self.publish_session();
    }
}
