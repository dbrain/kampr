use std::collections::HashMap;
use std::path::Path;

use crate::facet::{Compaction, FacetFold, Facets, Mode, Running, Timing, Titles};
use crate::marker::SessionMarker;
use crate::process::Started;
use crate::scan::{Appended, Cursor};
use crate::summary::one_line;

use super::record::{self, Content, Record};

pub fn collect(transcript: &Path, marker: Option<&SessionMarker>) -> Facets {
    Fold::default().facets(transcript, marker)
}

/// The same collection, kept between reads: the accumulated state and the byte it has reached, so
/// a second look costs the records the transcript has grown by.
///
/// **The title is the one facet that is not appended**, and it is read here anyway. omp keeps the
/// current title in a fixed-width 256-byte slot on line 1 and rewrites it in place, which a fold
/// reading only new bytes would see exactly once — so a rename appends a `title_change` audit
/// entry beside the rewrite, and that is what carries a later one.
#[derive(Default)]
pub struct Fold {
    cursor: Cursor,
    accumulated: Facets,
    titles: Titles,
    mode: Mode,
    watch: Watch,
    turns: Turns,
}

impl FacetFold for Fold {
    fn facets(&mut self, transcript: &Path, marker: Option<&SessionMarker>) -> Facets {
        self.advance(transcript);
        Facets {
            title: self.titles.resolve(),
            running: self.watch.running(marker.map_or(Started::Unknown, |m| m.started)),
            mode: (self.mode != Mode::default()).then(|| self.mode.clone()),
            ..self.accumulated.clone()
        }
    }

    fn titles(&mut self, transcript: &Path, _marker: Option<&SessionMarker>) -> Titles {
        self.advance(transcript);
        self.titles.clone()
    }
}

impl Fold {
    fn advance(&mut self, transcript: &Path) {
        let mut appended = Appended::open(transcript, self.cursor);
        if appended.restarted() {
            *self = Self::default();
        }
        for line in appended.by_ref() {
            self.push(&line);
        }
        self.cursor = appended.cursor();
    }

    fn push(&mut self, line: &str) {
        let Ok(record) = serde_json::from_str::<Record>(line) else {
            return;
        };
        match record {
            Record::Title(slot) | Record::TitleChange(slot) => self.retitle(slot.title, slot.source),
            Record::Session(header) => self.retitle(header.title, header.title_source),
            Record::ModeChange(change) => self.mode.mode = change.mode.or(self.mode.mode.take()),
            Record::Compaction(compacted) => self.accumulated.compactions.push(Compaction {
                at: compacted.timestamp,
                pre_tokens: compacted.tokens_before,
                ..Compaction::default()
            }),
            Record::Message(entry) => {
                if let Some(timing) = self.turns.record(&entry) {
                    self.accumulated.timings.push(timing);
                }
                self.watch.record(entry.message, entry.timestamp);
            }
            Record::CustomMessage(notice) if notice.custom_type.as_deref() == Some(ASYNC_RESULT) => {
                for job in record::finished(&notice.details) {
                    self.watch.finished(&job);
                }
            }
            _ => {}
        }
    }

    /// omp's two title sources are its own: `user` is a person typing `/rename`, `auto` the tiny
    /// model naming the session off the first prompt.
    fn retitle(&mut self, title: Option<String>, source: Option<String>) {
        let Some(text) = title.filter(|t| !t.trim().is_empty()) else {
            return;
        };
        match source.as_deref() {
            Some("user") => self.titles.manual = Some(text),
            _ => self.titles.generated = Some(text),
        }
    }
}

/// How long a turn took, and how many messages the harness wrote inside it.
///
/// **Not omp's `duration`, which is a different quantity.** omp writes `duration` and `ttft` on
/// every assistant message, and they are the model call's own time — finer than a turn's and
/// excluding every second a tool ran for. Claude's `turn_duration` is the whole turn over
/// `messageCount` messages, and that is what this field means on the wire, so this measures the
/// same span: from the instant the operator's prompt was written to the instant the message that
/// ended the turn completed, both stamps the harness recorded itself and both on one clock
/// ([#488](#)).
///
/// `stop` is what ends a turn. `toolUse` is the harness going round again, and `error` is a model
/// call the harness then **retried** — closing on either would report a fraction of the turn as
/// the whole of it. A turn that ends any other way, an abort included, reports nothing.
#[derive(Default)]
struct Turns {
    began: Option<f64>,
    messages: u32,
}

impl Turns {
    fn record(&mut self, entry: &record::Entry) -> Option<Timing> {
        match &entry.message {
            record::Message::User { .. } => {
                self.began = record::wrote_at(&entry.message);
                self.messages = 0;
                None
            }
            record::Message::Assistant {
                completed_at,
                stop_reason,
                ..
            } => {
                self.messages += 1;
                if stop_reason.as_deref() != Some("stop") {
                    return None;
                }
                let began = self.began.take()?;
                let ended = (*completed_at)?;
                Some(Timing {
                    turn: entry.id.clone()?,
                    duration_ms: (ended - began).max(0.0).round() as u64,
                    messages: Some(std::mem::take(&mut self.messages)),
                })
            }
            _ => None,
        }
    }
}

/// The notice a detached spawn's yield comes back on.
const ASYNC_RESULT: &str = "async-result";

/// The spawns a session has started and has not been told are over.
///
/// **A spawn is detached by default, and its call is answered immediately.** The `task` result
/// reads `Spawned agent \`x\` (job \`x\`). Its result auto-delivers on yield…` and lands
/// milliseconds after the call, so an outstanding tool call is not how a running agent is found —
/// it already has a result. The ending is the `async-result` notice naming the job, measured on
/// omp 18.1.10 with `details.jobs[].jobId` ([#484](#)). A spawn the parent blocked on has no notice at all
/// and is closed by its own result, which is why the acknowledgement is what separates them.
#[derive(Default)]
struct Watch {
    open: HashMap<String, Running>,
    order: Vec<String>,
    /// The job name a call spawned, because the notice names the job and the launch is keyed by
    /// the call that asked for it.
    jobs: HashMap<String, String>,
}

impl Watch {
    fn running(&self, started: Started) -> Vec<Running> {
        self.order
            .iter()
            .filter_map(|call| self.open.get(call))
            .filter(|open| !started.predates(open.since.as_deref()))
            .cloned()
            .collect()
    }

    fn record(&mut self, message: record::Message, at: Option<String>) {
        match message {
            record::Message::Assistant {
                content: Content::Blocks(blocks),
                ..
            } => {
                for block in blocks {
                    if let record::Block::ToolCall { id, name, arguments } = block
                        && name == super::TASK
                    {
                        self.launched(id, &arguments, at.clone());
                    }
                }
            }
            record::Message::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                let text = record::result_text(&content);
                for name in record::spawned(&text) {
                    self.jobs.insert(name, tool_call_id.clone());
                }
                if !record::acknowledgement(&text) {
                    self.close(&tool_call_id);
                }
            }
            _ => {}
        }
    }

    fn launched(&mut self, call: String, arguments: &serde_json::Value, at: Option<String>) {
        for spawn in record::spawns(arguments) {
            if let Some(name) = &spawn.name {
                self.jobs.insert(name.clone(), call.clone());
            }
            let entry = Running {
                call: call.clone(),
                kind: "agent".into(),
                name: Some(super::TASK.into()),
                title: spawn
                    .task
                    .clone()
                    .or_else(|| spawn.name.clone())
                    .map(|t| one_line(&t)),
                since: at.clone(),
            };
            if self.open.insert(call.clone(), entry).is_none() {
                self.order.push(call.clone());
            }
        }
    }

    fn finished(&mut self, job: &str) {
        if let Some(call) = self.jobs.get(job).cloned() {
            self.close(&call);
        }
    }

    fn close(&mut self, call: &str) {
        if self.open.remove(call).is_some() {
            self.order.retain(|open| open != call);
        }
    }
}
