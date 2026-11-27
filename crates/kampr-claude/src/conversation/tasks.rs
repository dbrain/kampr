use kampr_core::conversation::Update;
use kampr_core::conversation::facts::{Task, TaskEnd, TaskState};
use serde_json::json;

use super::machine::Machine;
use super::state::Request;
use crate::wire::{TaskInfo, TaskNotification, TaskProgress};

impl Machine {
    pub(super) fn tasks_seen(&mut self, tasks: Vec<TaskInfo>) {
        for task in tasks {
            let Some(id) = task.task_id else {
                continue;
            };
            let existing = self.st.tasks.get(&id).cloned().unwrap_or_else(|| Task {
                id: id.clone(),
                kind: task.task_type.clone().unwrap_or_else(|| "task".into()),
                call: task.tool_use_id.clone(),
                parent: task.parent_task_id.clone(),
                title: None,
                since: None,
                backgrounded: false,
                state: TaskState::Running,
            });
            let state = match task.status.as_deref() {
                Some("completed" | "failed" | "stopped") => TaskState::Ended,
                _ => TaskState::Running,
            };
            self.st.tasks.insert(
                id,
                Task {
                    kind: task.task_type.unwrap_or(existing.kind.clone()),
                    backgrounded: task.is_backgrounded || existing.backgrounded,
                    state,
                    title: task.prompt.or(existing.title.clone()),
                    ..existing
                },
            );
        }
        self.say_tasks();
    }

    pub(super) fn task_progress(&mut self, progress: TaskProgress) {
        if let Some(id) = &progress.task_id
            && let Some(task) = self.st.tasks.get_mut(id)
        {
            task.title = progress.description.or(task.title.take());
        }
        self.say_tasks();
    }

    /// A task finished. The pane watching it is told the tail it never polled and that it ended.
    pub(super) fn task_ended(&mut self, notification: TaskNotification) {
        let Some(id) = notification.task_id else {
            return;
        };
        if let Some(task) = self.st.tasks.get_mut(&id) {
            task.state = TaskState::Ended;
        }
        if let Some(total) = self.st.followed.remove(&id) {
            self.say(Update::TaskOutput {
                id,
                at: total,
                text: String::new(),
                ended: Some(TaskEnd {
                    ok: notification.status.as_deref() != Some("failed"),
                    summary: None,
                }),
            });
        }
        self.say_tasks();
    }

    fn say_tasks(&mut self) {
        let tasks = self.st.tasks.values().cloned().collect();
        self.say(Update::Tasks(tasks));
    }

    /// The pane opened a task's output. The binary only answers with the last 8 KiB of it, so
    /// following is polling that tail (#647).
    pub(super) fn follow(&mut self, id: String) {
        self.st.followed.entry(id.clone()).or_insert(0);
        self.poll_task(id);
    }

    /// Once a second, for each task the pane watches.
    pub(super) fn tick(&mut self) {
        let ids: Vec<String> = self.st.followed.keys().cloned().collect();
        for id in ids {
            self.poll_task(id);
        }
    }

    fn poll_task(&mut self, id: String) {
        self.request(
            Request::TaskOutput(id.clone()),
            "get_task_output",
            json!({ "task_id": id }),
        );
    }
}
