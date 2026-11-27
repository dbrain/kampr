use crate::screen::Screen;
use kampr_core::program::{BlockKind, ProgramRecord, ProgramRecords, ProgramState};
use serde::{Deserialize, Serialize};

/// What follows [`super::Saved`] in a checkpoint: the program records and how many reports made
/// them. Version 1 carried the one last report as a root record, in the other order.
#[derive(Serialize, Deserialize)]
pub(super) struct Tail {
    reports: u64,
    records: Vec<ProgramRecord>,
}

#[derive(Deserialize)]
struct RootOnly {
    program: Option<(u8, Option<String>, Option<String>)>,
    reports: u64,
}

const ONE_STATES: [ProgramState; 5] = [
    ProgramState::Idle,
    ProgramState::Working,
    ProgramState::Blocked,
    ProgramState::Done,
    ProgramState::Error,
];

impl Tail {
    pub(super) fn of(screen: &Screen) -> Self {
        Self {
            reports: screen.program_reports,
            records: screen.program.saved(),
        }
    }

    pub(super) fn read(version: u32, bytes: &[u8]) -> Option<Self> {
        if version == 1 {
            let old: RootOnly = postcard::from_bytes(bytes).ok()?;
            let records = match old.program {
                None => Vec::new(),
                Some((state, kind, msg)) => vec![ProgramRecord {
                    id: String::new(),
                    state: *ONE_STATES.get(usize::from(state))?,
                    kind: kind.and_then(|k| match k.as_str() {
                        "permission" => Some(BlockKind::Permission),
                        "question" => Some(BlockKind::Question),
                        "auth" => Some(BlockKind::Auth),
                        _ => None,
                    }),
                    progress: None,
                    app: None,
                    title: None,
                    msg,
                }],
            };
            return Some(Self {
                reports: old.reports,
                records,
            });
        }
        postcard::from_bytes(bytes).ok()
    }

    pub(super) fn put(self, screen: &mut Screen) -> Option<()> {
        screen.program = ProgramRecords::restore(self.records)?;
        screen.program_reports = self.reports;
        Some(())
    }
}
