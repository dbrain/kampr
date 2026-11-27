use serde_json::Value;

use super::machine::Machine;
use super::state::Request;
use crate::wire::Line;

impl Machine {
    /// One line of the child's stdout.
    pub(super) fn line(&mut self, raw: &str) {
        let Some(line) = Line::parse(raw) else {
            return;
        };
        match line {
            Line::Init(init) => self.turn_init(init),
            Line::Status(status) => self.status(status),
            Line::ThinkingTokens { tokens } => self.thinking(tokens),
            Line::TaskStarted(task) | Line::TaskUpdated(task) => self.tasks_seen(vec![task]),
            Line::BackgroundTasks(tasks) => self.tasks_seen(tasks),
            Line::TaskProgress(progress) => self.task_progress(progress),
            Line::TaskNotification(notification) => self.task_ended(notification),
            Line::ApiRetry(retry) => self.api_retry(retry),
            Line::StreamEvent(event) => self.stream_event(event),
            Line::SubagentLive(live) => self.subagent_live(live),
            Line::Assistant(assistant) => self.assistant(assistant),
            Line::Result(result) => self.result(result),
            Line::RateLimit(limit) => self.rate_limit(limit),
            Line::ControlRequest(request) => self.control_request(request),
            Line::ControlResponse(response) => self.control_response(response),
            Line::User(_) | Line::Other => {}
        }
    }

    /// One line this side wrote before the replay caught up. What it names is what is no longer
    /// standing, or already asked.
    pub(super) fn past(&mut self, raw: &str) {
        let Ok(line) = serde_json::from_str::<Value>(raw) else {
            return;
        };
        match line.get("type").and_then(Value::as_str) {
            Some("control_response") => {
                if let Some(id) = line.pointer("/response/request_id").and_then(Value::as_str) {
                    self.st.standing.retain(|s| !s.owns_request(id));
                }
            }
            Some("control_request") => {
                let id = line.get("request_id").and_then(Value::as_str).unwrap_or_default();
                let kind = match line.pointer("/request/subtype").and_then(Value::as_str) {
                    Some("initialize") => {
                        self.st.initialize_sent = true;
                        Request::Initialize
                    }
                    Some("get_usage") => Request::Usage,
                    Some("get_context_usage") => Request::Context,
                    _ => return,
                };
                self.st.inflight.insert(id.to_string(), kind);
            }
            Some("user") => self.st.unattended = 0,
            _ => {}
        }
    }
}
