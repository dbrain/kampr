use std::collections::HashMap;

use serde_json::Value;

use crate::facet::Running;
use crate::process::Started;

/// The work a session has started and has not been told is over.
///
/// **The operator's question, and it is not the one `agent_status` answers.** A pane says
/// `working` for as long as anything is outstanding, and a shell somebody left running an hour ago
/// makes it say `working` with nothing happening — so `working` on its own means "a launch has not
/// reported back", which is not the same sentence at all. This is the list that makes the
/// difference readable: what was launched, what kind of thing it is, and when.
///
/// Every rule here is measured against Claude Code 2.1.252 on this machine's own transcripts —
/// probe [#418], with the counts and the live positive control. The shape that matters:
///
/// | Measured | What it means here |
/// |---|---|
/// | A background `Bash` writes its `tool_result` **at launch**, carrying `backgroundTaskId` and an empty `stdout` | An outstanding `tool_use` is *not* how a running shell is found — it already has a result |
/// | An asynchronous `Agent` writes `status: "async_launched"` at launch (176 of 177 in the archive) | Same: the result is an acknowledgement, not an ending |
/// | A completion arrives as a `queue-operation` `enqueue` whose content is a `<task-notification>` naming `<tool-use-id>` and `<status>` | **This** is the ending, for both kinds. Statuses seen: `completed`, `killed`, `failed` |
/// | A synchronous `Agent` writes no result for 65–146 s and then writes a real one | Closed by its own `tool_result`, because no notification ever comes |
///
/// So a launch is open from its `tool_use` until either a notification names it or a `tool_result`
/// that is **not** one of the two acknowledgements above settles it. Run over two finished sessions
/// — 24 and 13 launches — that closes every one and leaves nothing standing, and over a live one it
/// leaves exactly the background command that was still running.
///
/// The harness's own note on a notification says the same task id may notify more than once,
/// because an agent can be resumed — so a later launch reopens a call rather than being ignored.
#[derive(Default)]
pub struct Watch {
    open: HashMap<String, Running>,
    order: Vec<String>,
    /// The id each open launch was handed at its acknowledgement, and the call it belongs to.
    ///
    /// **A completion names one of two things and both shapes are in one transcript** (#501): some
    /// carry `<tool-use-id>`, and some carry only `<task-id>` — which is the `agentId` an
    /// asynchronous agent's acknowledgement returned, or the `backgroundTaskId` a background
    /// shell's did. Nothing else in the file joins the two, so it is written down here at launch
    /// or the ending cannot be read at all: an agent the operator watched finish was still being
    /// counted four and a half hours later.
    tasks: HashMap<String, String>,
}

impl Watch {
    /// What is open, less whatever the process running this session cannot have launched.
    ///
    /// **A launch has no expiry of its own, and the transcript cannot give it one.** Both endings
    /// above are records somebody writes; when the harness process dies, neither is ever written —
    /// a background shell dies with its parent and nobody files a completion — so the launch stands
    /// open for the rest of the file. Measured on this machine's own archive: a `Client gate` shell
    /// that the transcript went on being written to for **2,025 minutes** after, and a
    /// `Claude code review` agent still open **4,219 minutes** on, 8 such launches across 6
    /// transcripts. The operator saw five dead shells aged 44 h, 44 h, 26 h, 7 h and 6 h beside two
    /// real agents, while the harness in the same pane listed no background shells at all.
    ///
    /// Nothing in the file separates a restart from a pause — one `sessionId` and one `version`
    /// across the whole 70 hours, 59 gaps over ten minutes, and no record at the seam. The process
    /// is what separates them: a launch cannot have happened before the process that made it, so a
    /// launch older than [`Started`] is over, exactly and with no heuristic. A launch *younger* is
    /// left alone however old it is in absolute terms — a 44-hour bench under a harness that has
    /// itself been up 44 hours is really running, and that is the case this must not break.
    pub fn running(&self, started: Started) -> Vec<Running> {
        self.order
            .iter()
            .filter_map(|call| self.open.get(call))
            .filter(|open| !started.predates(open.since.as_deref()))
            .cloned()
            .collect()
    }

    /// A `queue-operation` whose content is one of the harness's own completion notifications.
    pub fn notified(&mut self, content: Option<&Value>) {
        let Some(text) = content.and_then(Value::as_str) else {
            return;
        };
        if let Some(call) = tagged(text, "tool-use-id") {
            self.close(&call);
            return;
        }
        // The other shape, and the only handle it gives is the one the launch was answered with.
        if let Some(call) = tagged(text, "task-id").and_then(|task| self.tasks.get(&task).cloned()) {
            self.close(&call);
        }
    }

    /// One `assistant` or `user` record: its launches, and its endings.
    pub fn record(&mut self, message: Option<&Value>, result: Option<&Value>, at: Option<&str>) {
        let Some(blocks) = message.and_then(|m| m.get("content")).and_then(Value::as_array) else {
            return;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => self.launched(block, at),
                Some("tool_result") => {
                    let Some(call) = block.get("tool_use_id").and_then(Value::as_str) else {
                        continue;
                    };
                    match acknowledgement(result) {
                        true => {
                            if let Some(task) = handed_out(result) {
                                self.tasks.insert(task, call.to_string());
                            }
                        }
                        false => self.close(call),
                    }
                }
                _ => {}
            }
        }
    }

    fn launched(&mut self, block: &Value, at: Option<&str>) {
        let Some(call) = block.get("id").and_then(Value::as_str) else {
            return;
        };
        let input = block.get("input");
        let text = |key: &str| {
            input
                .and_then(|i| i.get(key))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        // `subagent_type` is what makes a call a launch of an agent, and `run_in_background` what
        // makes one a launch of a shell. Both are on the call's own input, so this is known at the
        // moment the call is written rather than when something answers it.
        let (kind, title) = if input.is_some_and(|i| i.get("subagent_type").is_some()) {
            ("agent", text("description"))
        } else if input.is_some_and(|i| i.get("run_in_background") == Some(&Value::Bool(true))) {
            ("shell", text("description").or_else(|| text("command")))
        } else {
            return;
        };
        let entry = Running {
            call: call.to_string(),
            kind: kind.to_string(),
            name: block.get("name").and_then(Value::as_str).map(str::to_string),
            title,
            since: at.map(str::to_string),
        };
        if self.open.insert(call.to_string(), entry).is_none() {
            self.order.push(call.to_string());
        }
    }

    fn close(&mut self, call: &str) {
        if self.open.remove(call).is_some() {
            self.order.retain(|held| held != call);
            // The map is the open launches' ids and nothing else, so a session that launches all
            // day carries no more of it than it is waiting on.
            self.tasks.retain(|_, held| held != call);
        }
    }
}

/// The id a launch's acknowledgement handed back, which is what its completion will name when it
/// names no call: `agentId` for an asynchronous agent, `backgroundTaskId` for a background shell.
fn handed_out(result: Option<&Value>) -> Option<String> {
    let result = result?;
    ["backgroundTaskId", "agentId"]
        .iter()
        .find_map(|key| result.get(key).and_then(Value::as_str))
        .map(str::to_string)
}

/// A result that is the harness saying "started", not "finished".
fn acknowledgement(result: Option<&Value>) -> bool {
    let Some(result) = result else {
        return false;
    };
    result.get("backgroundTaskId").is_some()
        || result.get("status") == Some(&Value::String("async_launched".into()))
}

fn tagged(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str, name: &str, input: Value) -> Value {
        json!({ "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }] })
    }

    fn result(id: &str) -> Value {
        json!({ "content": [{ "type": "tool_result", "tool_use_id": id }] })
    }

    fn notification(call: &str, status: &str) -> Value {
        json!(format!(
            "<task-notification>\n<task-id>b1</task-id>\n<tool-use-id>{call}</tool-use-id>\n\
             <status>{status}</status>\n</task-notification>"
        ))
    }

    /// The other shape, verbatim from the transcript in probe #501: the same event, naming the
    /// task it was given at launch and **no call at all**.
    fn notification_by_task(task: &str, status: &str) -> Value {
        json!(format!(
            "<task-notification>\n<task-id>{task}</task-id>\n<output-file>/tmp/out</output-file>\n\
             <status>{status}</status>\n<summary>done</summary>\n</task-notification>"
        ))
    }

    /// The operator, on 0.1.65, looking at a conversation on their phone: *"a 4 hour long agent
    /// that's apparently still running but Claude terminal itself doesn't have an agent running
    /// and that agent finished ages ago"*.
    ///
    /// **A completion names one of two things, and only one of them was read.** Measured across a
    /// live 9 MB transcript (#501): of its 373 notifications, some carry `<tool-use-id>` and some
    /// carry only `<task-id>` — the id the *acknowledgement* handed out, `agentId` for an
    /// asynchronous agent and `backgroundTaskId` for a background shell. The launch the operator
    /// was looking at was ended by the second shape 18 minutes in, and the strip counted it for
    /// four and a half hours because nothing had written down which call that task belonged to.
    #[test]
    fn a_completion_that_names_only_the_task_it_was_given_at_launch_still_ends_it() {
        let mut watch = Watch::default();
        watch.record(
            Some(&call(
                "toolu_01XJ",
                "Agent",
                json!({ "subagent_type": "general-purpose", "description": "Survey coding models" }),
            )),
            None,
            Some("2026-09-05T07:47:49.634Z"),
        );
        watch.record(
            Some(&result("toolu_01XJ")),
            Some(&json!({ "isAsync": true, "status": "async_launched", "agentId": "ac2f8ed8659ad1ee7" })),
            None,
        );
        assert_eq!(
            watch.running(Started::Unknown).len(),
            1,
            "the launch was never open"
        );

        watch.notified(Some(&notification_by_task("ac2f8ed8659ad1ee7", "completed")));
        assert!(
            watch.running(Started::Unknown).is_empty(),
            "the agent finished and the strip went on counting it",
        );
    }

    /// And a task id nobody launched settles nothing — the map is what was handed out at launch,
    /// not a wildcard.
    #[test]
    fn a_completion_for_a_task_this_session_never_launched_ends_nothing() {
        let mut watch = Watch::default();
        watch.record(
            Some(&call(
                "t1",
                "Agent",
                json!({ "subagent_type": "Explore", "description": "look" }),
            )),
            None,
            Some("t"),
        );
        watch.record(
            Some(&result("t1")),
            Some(&json!({ "status": "async_launched", "agentId": "mine" })),
            None,
        );
        watch.notified(Some(&notification_by_task("somebody-elses", "completed")));
        assert_eq!(watch.running(Started::Unknown).len(), 1);
    }

    #[test]
    fn a_background_shell_is_running_even_though_its_result_already_arrived() {
        let mut watch = Watch::default();
        watch.record(
            Some(&call(
                "t1",
                "Bash",
                json!({ "run_in_background": true, "description": "the build" }),
            )),
            None,
            Some("2026-09-01T23:46:16.370Z"),
        );
        // Measured: the result lands within milliseconds, carrying only the task id.
        watch.record(
            Some(&result("t1")),
            Some(&json!({ "stdout": "", "backgroundTaskId": "buanpuhrj" })),
            None,
        );
        let open = watch.running(Started::Unknown);
        assert_eq!(open.len(), 1, "the launch acknowledgement ended the run");
        assert_eq!(open[0].kind, "shell");
        assert_eq!(open[0].title.as_deref(), Some("the build"));
        assert_eq!(open[0].since.as_deref(), Some("2026-09-01T23:46:16.370Z"));
    }

    #[test]
    fn a_completion_notification_is_what_ends_a_background_shell() {
        let mut watch = Watch::default();
        watch.record(
            Some(&call(
                "t1",
                "Bash",
                json!({ "run_in_background": true, "command": "make" }),
            )),
            None,
            Some("t"),
        );
        watch.record(
            Some(&result("t1")),
            Some(&json!({ "backgroundTaskId": "b1" })),
            None,
        );
        watch.notified(Some(&notification("t1", "completed")));
        assert!(watch.running(Started::Unknown).is_empty());
    }

    /// `killed` and `failed` are both endings — the operator wants them off the list of what is
    /// still going, and the difference belongs to the turn that reports it.
    #[test]
    fn a_run_that_was_killed_or_failed_is_no_longer_running() {
        for status in ["killed", "failed"] {
            let mut watch = Watch::default();
            watch.record(
                Some(&call(
                    "t1",
                    "Agent",
                    json!({ "subagent_type": "Explore", "description": "look" }),
                )),
                None,
                Some("t"),
            );
            watch.notified(Some(&notification("t1", status)));
            assert!(
                watch.running(Started::Unknown).is_empty(),
                "{status} left it running"
            );
        }
    }

    /// The path with no notification at all: a synchronous launch writes no result for a minute or
    /// two and then writes a real one, and that real one is the ending.
    #[test]
    fn a_synchronous_agent_is_ended_by_its_own_result_because_nothing_else_ever_names_it() {
        let mut watch = Watch::default();
        watch.record(
            Some(&call(
                "t1",
                "Agent",
                json!({ "subagent_type": "Plan", "description": "design it" }),
            )),
            None,
            Some("t"),
        );
        assert_eq!(watch.running(Started::Unknown).len(), 1);
        watch.record(
            Some(&result("t1")),
            Some(&json!({ "content": "here is the plan" })),
            None,
        );
        assert!(watch.running(Started::Unknown).is_empty());
    }

    /// An asynchronous agent's result is an acknowledgement in the same way a shell's is, and the
    /// field that says so is different — which is why both are checked.
    #[test]
    fn an_async_launch_acknowledgement_is_not_an_ending() {
        let mut watch = Watch::default();
        watch.record(
            Some(&call(
                "t1",
                "Agent",
                json!({ "subagent_type": "Explore", "description": "sweep" }),
            )),
            None,
            Some("t"),
        );
        watch.record(
            Some(&result("t1")),
            Some(&json!({ "status": "async_launched", "agentId": "a1" })),
            None,
        );
        assert_eq!(watch.running(Started::Unknown).len(), 1);
    }

    /// An ordinary call is not a launch, and the list must not fill up with every `Read` and `Bash`
    /// a session makes.
    #[test]
    fn a_call_that_launched_nothing_is_not_on_the_list() {
        let mut watch = Watch::default();
        watch.record(
            Some(&call("t1", "Bash", json!({ "command": "ls" }))),
            None,
            Some("t"),
        );
        watch.record(
            Some(&call("t2", "Read", json!({ "file_path": "/x" }))),
            None,
            Some("t"),
        );
        watch.record(
            Some(&call(
                "t3",
                "Bash",
                json!({ "run_in_background": false, "command": "ls" }),
            )),
            None,
            Some("t"),
        );
        assert!(watch.running(Started::Unknown).is_empty());
    }

    /// Order is the order they were launched in, so the list does not reshuffle under a reader
    /// every time one of them ends.
    #[test]
    fn what_is_still_running_is_listed_in_the_order_it_was_started() {
        let mut watch = Watch::default();
        for (id, title) in [("t1", "first"), ("t2", "second"), ("t3", "third")] {
            watch.record(
                Some(&call(
                    id,
                    "Agent",
                    json!({ "subagent_type": "Explore", "description": title }),
                )),
                None,
                Some("t"),
            );
        }
        watch.notified(Some(&notification("t2", "completed")));
        let titles: Vec<_> = watch
            .running(Started::Unknown)
            .iter()
            .filter_map(|r| r.title.clone())
            .collect();
        assert_eq!(titles, vec!["first", "third"]);
    }

    /// The harness's own note on a notification: the same task may notify more than once, because
    /// an agent that stopped can be sent another message. A second launch of the same call id is a
    /// resume, and it is running again.
    #[test]
    fn a_resumed_agent_is_running_again_rather_than_stuck_finished() {
        let mut watch = Watch::default();
        let launch = call(
            "t1",
            "Agent",
            json!({ "subagent_type": "Explore", "description": "sweep" }),
        );
        watch.record(Some(&launch), None, Some("first"));
        watch.notified(Some(&notification("t1", "completed")));
        assert!(watch.running(Started::Unknown).is_empty());
        watch.record(Some(&launch), None, Some("second"));
        assert_eq!(watch.running(Started::Unknown).len(), 1);
        assert_eq!(
            watch.running(Started::Unknown)[0].since.as_deref(),
            Some("second")
        );
    }

    #[test]
    fn a_notification_naming_nothing_this_session_launched_changes_nothing() {
        let mut watch = Watch::default();
        watch.record(
            Some(&call(
                "t1",
                "Agent",
                json!({ "subagent_type": "Explore", "description": "sweep" }),
            )),
            None,
            Some("t"),
        );
        watch.notified(Some(&notification("somebody-else", "completed")));
        watch.notified(Some(&json!("a prompt a person typed")));
        watch.notified(None);
        assert_eq!(watch.running(Started::Unknown).len(), 1);
    }
}
