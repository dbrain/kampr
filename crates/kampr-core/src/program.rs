//! The record set `OSC 7501` fills (docs/24): one record per id, the session's status a fold over
//! them. Pure: the emulator hands over parsed events and reads the fold.

mod report;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use report::parse;

/// What a terminal MUST hold is 64; Rex allows up to this many, evicting the record least
/// recently written when a new id arrives at the cap.
pub const MAX_RECORDS: usize = 256;
/// Records on the wire; the node holds all of them.
pub const WIRE_ROWS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgramState {
    Idle,
    Working,
    Blocked,
    Done,
    Error,
}

impl ProgramState {
    pub fn word(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Error => "error",
        }
    }

    /// Where a state sorts when a row list is cut short: what needs somebody first.
    fn urgency(self) -> u8 {
        match self {
            Self::Blocked => 0,
            Self::Working => 1,
            Self::Error => 2,
            Self::Done => 3,
            Self::Idle => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockKind {
    Permission,
    Question,
    Auth,
}

impl BlockKind {
    pub fn word(self) -> &'static str {
        match self {
            Self::Permission => "permission",
            Self::Question => "question",
            Self::Auth => "auth",
        }
    }
}

/// `id` is the slash path, empty for the root record. `app` is only what this record said: the
/// one shown comes from [`ProgramRecords::app_of`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramRecord {
    pub id: String,
    pub state: ProgramState,
    pub kind: Option<BlockKind>,
    pub progress: Option<u8>,
    pub app: Option<String>,
    pub title: Option<String>,
    pub msg: Option<String>,
}

/// What a parsed report, and the terminal events that end a program's records, do to the set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Set(ProgramRecord),
    Clear(Option<String>),
    /// `OSC 133 A`: the shell is back at a prompt, so what it ran has ended.
    Prompt,
    /// `RIS`. `DECSTR` and an alternate-screen switch are not events: they remove nothing.
    Reset,
}

/// The session's own status, folded from the records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramStatus {
    pub state: ProgramState,
    pub kind: Option<BlockKind>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Slot {
    record: ProgramRecord,
    seq: u64,
}

/// Equal when the same records were written in the same order; the counters that order them are
/// not part of what a set says.
#[derive(Debug, Clone, Default)]
pub struct ProgramRecords {
    slots: BTreeMap<String, Slot>,
    seq: u64,
}

impl PartialEq for ProgramRecords {
    fn eq(&self, other: &Self) -> bool {
        self.saved() == other.saved()
    }
}

impl Eq for ProgramRecords {}

impl ProgramRecords {
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn get(&self, id: &str) -> Option<&ProgramRecord> {
        self.slots.get(id).map(|s| &s.record)
    }

    /// Whether the set changed. A report always counts: a repeated `done` is a new one to read.
    pub fn apply(&mut self, event: Event) -> bool {
        match event {
            Event::Set(record) => {
                if !self.slots.contains_key(&record.id) && self.slots.len() >= MAX_RECORDS {
                    self.evict();
                }
                self.seq += 1;
                self.slots.insert(
                    record.id.clone(),
                    Slot {
                        record,
                        seq: self.seq,
                    },
                );
                true
            }
            Event::Clear(None) | Event::Reset => self.remove_where(|_| true),
            Event::Clear(Some(id)) => {
                let below = format!("{id}/");
                self.remove_where(|at| at == id || at.starts_with(&below))
            }
            Event::Prompt => self.process_ended(),
        }
    }

    /// `working` and `blocked` end with the process that reported them; `done` and `error` are
    /// what the person comes back to, and stay.
    pub fn process_ended(&mut self) -> bool {
        let before = self.slots.len();
        self.slots
            .retain(|_, s| !matches!(s.record.state, ProgramState::Working | ProgramState::Blocked));
        self.slots.len() != before
    }

    /// Blocked beats working beats error; `done` and `idle` are the harness's own and come from
    /// the root record alone, since a worker finishing is not the session finishing. Of several
    /// records in the winning state the root speaks, else the newest.
    pub fn fold(&self) -> Option<ProgramStatus> {
        let record = self.speaker()?;
        Some(ProgramStatus {
            state: record.state,
            kind: record.kind,
            message: record.msg.clone(),
        })
    }

    /// How far along the record the fold speaks for says it is, while that record is working.
    pub fn progress(&self) -> Option<u8> {
        self.speaker()
            .filter(|r| r.state == ProgramState::Working)
            .and_then(|r| r.progress)
    }

    fn speaker(&self) -> Option<&ProgramRecord> {
        let root = self.slots.get("");
        let speaker = |state| {
            root.filter(|s| s.record.state == state).or_else(|| {
                self.slots
                    .values()
                    .filter(|s| s.record.state == state)
                    .max_by_key(|s| s.seq)
            })
        };
        [ProgramState::Blocked, ProgramState::Working, ProgramState::Error]
            .into_iter()
            .find_map(speaker)
            .or(root)
            .map(|s| &s.record)
    }

    /// The nearest ancestor's `app`, the record's own first.
    pub fn app_of(&self, id: &str) -> Option<&str> {
        let mut at = Some(id);
        while let Some(id) = at {
            if let Some(app) = self.slots.get(id).and_then(|s| s.record.app.as_deref()) {
                return Some(app);
            }
            at = match id.rsplit_once('/') {
                Some((parent, _)) => Some(parent),
                None if id.is_empty() => None,
                None => Some(""),
            };
        }
        None
    }

    /// The records a client is shown: most urgent first, newest first within a state, capped at
    /// [`WIRE_ROWS`]. Empty when the root is all there is.
    pub fn rows(&self) -> Vec<&ProgramRecord> {
        if self.slots.keys().all(String::is_empty) {
            return Vec::new();
        }
        let mut rows: Vec<&Slot> = self.slots.values().collect();
        rows.sort_by_key(|s| (s.record.state.urgency(), std::cmp::Reverse(s.seq)));
        rows.into_iter().take(WIRE_ROWS).map(|s| &s.record).collect()
    }

    /// Every record, the one written longest ago first, for a checkpoint.
    pub fn saved(&self) -> Vec<ProgramRecord> {
        let mut slots: Vec<&Slot> = self.slots.values().collect();
        slots.sort_by_key(|s| s.seq);
        slots.into_iter().map(|s| s.record.clone()).collect()
    }

    /// The inverse of [`Self::saved`]; a duplicate id or more records than the cap is a
    /// checkpoint nothing here wrote, and gives `None`.
    pub fn restore(saved: Vec<ProgramRecord>) -> Option<Self> {
        if saved.len() > MAX_RECORDS {
            return None;
        }
        let mut records = Self::default();
        for record in saved {
            if records.slots.contains_key(&record.id) {
                return None;
            }
            records.apply(Event::Set(record));
        }
        Some(records)
    }

    fn evict(&mut self) {
        if let Some(oldest) = self
            .slots
            .iter()
            .min_by_key(|(_, s)| s.seq)
            .map(|(id, _)| id.clone())
        {
            self.slots.remove(&oldest);
        }
    }

    fn remove_where(&mut self, mut gone: impl FnMut(&str) -> bool) -> bool {
        let before = self.slots.len();
        self.slots.retain(|id, _| !gone(id));
        self.slots.len() != before
    }
}
