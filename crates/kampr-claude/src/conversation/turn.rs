use kampr_core::conversation::facts::{Cost, RunState, Trouble};
use kampr_core::conversation::{Live, LiveTool, Update};

use super::guards;
use super::machine::Machine;
use crate::billing::{is_self_started, turn_key_source_ok};
use crate::wire::{self, Assistant, BlockStart, ContentBlock, Init, StreamEvent};

impl Machine {
    /// A turn's `system/init`: guard 4 reads how this turn is paid for, and the session facts
    /// the pane shows are refreshed from it.
    pub(super) fn turn_init(&mut self, init: Init) {
        if !turn_key_source_ok(self.settings.policy, init.api_key_source.as_deref()) {
            let source = init.api_key_source.as_deref().unwrap_or("a key");
            return self.end_session(guards::key_source_trouble(source));
        }
        if let Some(id) = &init.session_id {
            self.st.session_id.clone_from(id);
        }
        if init.model.is_some() {
            self.st.model.clone_from(&init.model);
        }
        if init.permission_mode.is_some() {
            self.st.mode.clone_from(&init.permission_mode);
        }
        self.st.init = Some(init);
        self.publish_controls();
        self.publish_session();
    }

    pub(super) fn status(&mut self, status: wire::Status) {
        let mode_only = status.permission_mode_only();
        if let Some(mode) = status.permission_mode {
            self.st.mode = Some(mode);
            self.publish_controls();
            self.publish_session();
        }
        let run = match status.status.as_deref() {
            Some("requesting" | "compacting") => RunState::Running,
            Some(_) => RunState::Idle,
            None if mode_only => return,
            None => RunState::Idle,
        };
        self.st.run = run;
        self.say(Update::State(run));
    }

    pub(super) fn thinking(&mut self, tokens: u64) {
        self.say(Update::Live(Live {
            thinking: Some(tokens),
            ..Live::default()
        }));
    }

    pub(super) fn api_retry(&mut self, retry: wire::ApiRetry) {
        let trouble = Trouble {
            kind: "api".into(),
            text: retry.error.unwrap_or_else(|| "the api is retrying".into()),
            status: "retrying".into(),
            retry: Some(kampr_core::conversation::facts::Retry {
                attempt: retry.attempt.unwrap_or(1) as u32,
                of: retry.max_retries.unwrap_or(3) as u32,
                in_ms: retry.retry_delay_ms.unwrap_or(0),
            }),
            fix: None,
        };
        self.set_trouble(Some(trouble));
    }

    pub(super) fn set_trouble(&mut self, trouble: Option<Trouble>) {
        self.st.trouble.clone_from(&trouble);
        self.say(Update::Trouble(trouble));
    }

    pub(super) fn assistant(&mut self, a: Assistant) {
        let mut live = Live::default();
        for block in &a.content {
            match block {
                ContentBlock::Text { text } => live.text.push_str(text),
                ContentBlock::ToolUse { id, name, input } => live.tools.push(LiveTool {
                    id: id.clone(),
                    tool: name.clone(),
                    args: input.to_string(),
                    state: kampr_journal::ToolState::Running,
                }),
                ContentBlock::Thinking { .. } | ContentBlock::Other => {}
            }
        }
        // A subagent's message is filed under the subagent that wrote it (#650).
        match a.parent_tool_use_id {
            Some(id) => self.say(Update::SubLive { id, live }),
            None => {
                self.say(Update::Live(live));
                if a.is_api_error_message || a.error.is_some() {
                    let text = a.error.unwrap_or_else(|| "an api error".into());
                    self.set_trouble(Some(Trouble {
                        kind: "api".into(),
                        text,
                        status: "error".into(),
                        retry: None,
                        fix: None,
                    }));
                }
            }
        }
    }

    pub(super) fn stream_event(&mut self, event: StreamEvent) {
        if let Some(live) = live_of(event) {
            self.say(Update::Live(live));
        }
    }

    /// A subagent's live edge, filed under its name so nothing files a subagent's words as the
    /// parent's.
    pub(super) fn subagent_live(&mut self, live: wire::SubagentLive) {
        // Unnamed: no subagent pane to file it under, and no guess to make.
        let Some(id) = live.parent_tool_use_id.or(live.task_id) else {
            return;
        };
        if let Some(edge) = live_of(live.event) {
            self.say(Update::SubLive { id, live: edge });
        }
    }

    pub(super) fn result(&mut self, result: wire::ResultLine) {
        self.st.in_flight = None;
        // Guard 7: a turn nobody asked for is counted, and the cap ends the session.
        if is_self_started(result.origin.as_ref()) {
            self.st.unattended += 1;
        } else {
            self.st.unattended = 0;
        }
        let cost = Cost {
            usd: result.total_cost_usd,
            input_tokens: result.usage.input_tokens,
            output_tokens: result.usage.output_tokens,
        };
        self.st.cost = Some(cost);
        self.say(Update::Cost(cost));
        if self.st.unattended > 0 && self.st.unattended >= self.settings.unattended_cap {
            let (count, cap) = (self.st.unattended, self.settings.unattended_cap);
            return self.end_session(guards::unattended_trouble(count, cap));
        }
        if result.is_error {
            let text = result
                .error
                .or(result.terminal_reason)
                .unwrap_or_else(|| "the turn failed".into());
            self.set_trouble(Some(Trouble {
                kind: "api".into(),
                text,
                status: "error".into(),
                retry: None,
                fix: None,
            }));
        } else if self.st.trouble.as_ref().is_some_and(|t| t.kind == "api") {
            self.set_trouble(None);
        }
        // The turn changed what the window holds: the pane's bar follows it.
        self.fetch_context();
        self.st.run = RunState::Idle;
        self.say(Update::State(RunState::Idle));
    }
}

/// The live edge a stream event is: the pane's words while a turn writes them. `None` is an
/// event the live edge does not draw.
fn live_of(event: StreamEvent) -> Option<Live> {
    let tool = |id: String, tool: String, args: String| LiveTool {
        id,
        tool,
        args,
        state: kampr_journal::ToolState::Running,
    };
    match event {
        StreamEvent::TextDelta { text } => Some(Live {
            text,
            ..Live::default()
        }),
        StreamEvent::ThinkingDelta { .. } => Some(Live::default()),
        StreamEvent::InputJsonDelta { partial_json } => Some(Live {
            tools: vec![tool(String::new(), String::new(), partial_json)],
            ..Live::default()
        }),
        StreamEvent::BlockStart {
            block: BlockStart::ToolUse { id, name },
            ..
        } => Some(Live {
            tools: vec![tool(id, name, String::new())],
            ..Live::default()
        }),
        StreamEvent::MessageDelta { .. } | StreamEvent::BlockStart { .. } | StreamEvent::Other => None,
    }
}
