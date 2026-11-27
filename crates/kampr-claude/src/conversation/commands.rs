use kampr_core::conversation::abilities::{Fact, Steer};
use kampr_core::conversation::{Command, FileRef, Image, SuggestKind, Suggestion, Update, When};
use serde_json::{Value, json};

use super::controls::{self, BYPASS};
use super::machine::Machine;
use super::state::{Hold, Preflight, Request};
use super::transcript;
use crate::request;

impl Machine {
    /// A command from the pane, mapped to a line to the child's stdin.
    pub(super) fn command(&mut self, command: Command) {
        match command {
            Command::Input {
                text,
                msg_id,
                images,
                files,
                when,
            } => self.input(&text, &msg_id, &images, &files, when),
            Command::Unqueue { msg_id } => {
                self.st
                    .held
                    .retain(|line| uuid_of(line).as_deref() != Some(msg_id.as_str()));
                self.fire("cancel_async_message", json!({ "message_uuid": msg_id }));
            }
            Command::Answer { ask, reply } => self.answer(&ask, reply),
            Command::Press { ask, key, text } => self.press(&ask, &key, text.as_deref()),
            Command::Submit { ask } => self.submit(&ask),
            Command::Steer(Steer::Interrupt, _) => self.interrupt(),
            Command::Set { control, value } => self.set(control, value),
            Command::Act { action, args } => self.act(&action, &args),
            Command::Task { id, .. } => self.follow(id),
            Command::Unfollow { id } => {
                self.st.followed.remove(&id);
            }
            Command::Suggest { kind, query } => self.suggest(kind, query),
            Command::Refresh(Fact::Context) => self.fetch_context(),
            Command::Refresh(Fact::Limits) => self.request(Request::Usage, "get_usage", json!({})),
            Command::Older { before } => transcript::older(self, before.as_deref()),
            Command::Find { query, cap } => transcript::find(self, query, cap),
            Command::Steer(..) | Command::Refresh(_) | Command::Sub { .. } => {}
        }
    }

    fn input(&mut self, text: &str, msg_id: &str, images: &[Image], files: &[FileRef], when: When) {
        // What the composer held is @-refs the agent reads; the pane's queue word rides on the
        // line the binary queues.
        let text = if files.is_empty() {
            text.to_string()
        } else {
            let refs = files
                .iter()
                .map(|f| format!("@{}", f.path.display()))
                .collect::<Vec<_>>()
                .join(" ");
            format!("{text} {refs}").trim().to_string()
        };
        let images: Vec<request::Image> = images
            .iter()
            .map(|i| request::Image {
                mime: i.mime.clone(),
                data: i.data.clone(),
            })
            .collect();
        let priority = match when {
            When::Now => Some("now"),
            When::Next => Some("next"),
            When::Idle => None,
        };
        self.st.unattended = 0;
        self.send_word(request::user_message(&text, &images, Some(msg_id), priority));
    }

    /// A slash command goes in as the text of a prompt: the binary reads `/compact`, `/clear` and
    /// `/model` off a user line and does not take them as a control request (#677).
    fn act(&mut self, action: &str, args: &Value) {
        let args = args
            .as_str()
            .or_else(|| args.get("args").and_then(Value::as_str))
            .unwrap_or_default();
        let text = format!("/{} {args}", action.trim_start_matches('/'));
        self.st.unattended = 0;
        self.send_text(text.trim(), None);
        self.say(Update::Acted {
            action: action.to_string(),
            ok: true,
            text: None,
        });
    }

    pub(super) fn send_text(&mut self, text: &str, msg_id: Option<&str>) {
        self.send_word(request::user_message(text, &[], msg_id, None));
    }

    /// A word is a turn, and a turn is what the guards stand in front of. Until the account is
    /// checked, or while a hold stands, it is kept; once either is settled the pane's order is
    /// kept. An overage the operator is being asked about holds prompts only: a slash command
    /// is theirs to run while they decide (/compact is how a spent window is made to last).
    /// Whether the word was sent or kept.
    pub(super) fn send_word(&mut self, line: String) -> bool {
        match (self.st.preflight, self.st.hold) {
            (Preflight::Refused, _) => {
                self.notice("not sent: this session was refused and has ended");
                false
            }
            (Preflight::Allowed, Some(Hold::Overage)) if is_slash_command(&line) => self.dispatch_word(line),
            (Preflight::Pending, _) | (_, Some(_)) => {
                self.st.held.push(line);
                self.settings.kept.hold(&self.st.held);
                true
            }
            (Preflight::Allowed, None) => self.dispatch_word(line),
        }
    }

    fn dispatch_word(&mut self, line: String) -> bool {
        let sent = self.write(&line);
        if sent {
            self.st.in_flight = Some(line);
        }
        sent
    }

    /// Send what was kept, now that nothing holds it.
    pub(super) fn release(&mut self) {
        if self.st.preflight != Preflight::Allowed || self.st.hold.is_some() || self.replaying {
            return;
        }
        for line in std::mem::take(&mut self.st.held) {
            self.dispatch_word(line);
        }
        self.settings.kept.hold(&[]);
    }

    fn set(&mut self, control: String, value: Value) {
        let (subtype, body, applied) = match control.as_str() {
            "model" => match value.as_str().filter(|m| self.st.catalog.has_model(m)) {
                Some(model) => ("set_model", json!({ "model": model }), value.clone()),
                None => return self.notice("that model is not one the session lists"),
            },
            "mode" => match value.as_str().filter(|m| controls::is_mode(m)) {
                Some(mode) => ("set_permission_mode", json!({ "mode": mode }), value.clone()),
                None => return self.notice("that is not a permission mode this control sets"),
            },
            "bypass" => {
                let mode = if value.as_bool() == Some(true) {
                    BYPASS
                } else {
                    "default"
                };
                ("set_permission_mode", json!({ "mode": mode }), json!(mode))
            }
            "fast" => {
                let on = value.as_bool() == Some(true);
                (
                    "apply_flag_settings",
                    json!({ "settings": { "fastMode": on } }),
                    json!(on),
                )
            }
            _ => return self.notice("the session lists no such control"),
        };
        self.request(
            Request::Setting {
                control,
                value: applied,
            },
            subtype,
            body,
        );
    }

    fn suggest(&mut self, kind: SuggestKind, query: String) {
        match kind {
            // The catalog the session listed at open: completion the pane can do itself.
            SuggestKind::Commands => {
                let needle = query.to_ascii_lowercase();
                let items = self
                    .st
                    .catalog
                    .commands
                    .iter()
                    .filter(|(name, _)| needle.is_empty() || name.to_ascii_lowercase().contains(&needle))
                    .map(|(name, description)| Suggestion {
                        id: format!("/{name}"),
                        label: format!("/{name}"),
                        detail: description.clone(),
                    })
                    .collect();
                self.say(Update::Suggested {
                    kind: SuggestKind::Commands,
                    query,
                    items,
                });
            }
            SuggestKind::Files => {
                self.request(
                    Request::Suggest(query.clone()),
                    "file_suggestions",
                    json!({ "query": query }),
                );
            }
        }
    }
}

fn is_slash_command(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|l| {
            l.pointer("/message/content")
                .and_then(Value::as_str)
                .map(|t| t.trim_start().starts_with('/'))
        })
        .unwrap_or(false)
}

fn uuid_of(line: &str) -> Option<String> {
    serde_json::from_str::<Value>(line)
        .ok()?
        .get("uuid")?
        .as_str()
        .map(str::to_string)
}
