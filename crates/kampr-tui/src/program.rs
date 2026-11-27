//! OSC 7501 as the herd carries it (docs/24): a failed program, its progress, and its records.

use kampr_core::wire::{PaneEntry, ProgramEntry};

/// The one-glyph mark a sidebar row carries: the failure, else the progress.
pub fn mark(entry: &PaneEntry) -> Option<String> {
    match (entry.program_error, entry.program_progress) {
        (true, _) => Some("✗".into()),
        (false, Some(p)) => Some(format!("{p}%")),
        _ => None,
    }
}

/// What the pane's name is followed by, wherever it is named.
pub fn note(entry: &PaneEntry) -> Option<String> {
    let mut parts = Vec::new();
    if entry.program_error {
        parts.push(match entry.program_msg.as_deref().filter(|m| !m.is_empty()) {
            Some(msg) => format!("✗ failed: {msg}"),
            None => "✗ failed".into(),
        });
    } else if let Some(p) = entry.program_progress {
        parts.push(format!("{p}%"));
    }
    if let Some(records) = entry.program.as_ref().filter(|r| !r.is_empty()) {
        parts.push(format!("{} programs", records.len()));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub struct Record {
    pub depth: usize,
    pub name: String,
    pub note: String,
}

/// Root first, then each record after its parent: an id is its parent's id, a `/`, and its own.
pub fn records(entry: &PaneEntry) -> Vec<Record> {
    let mut rows: Vec<&ProgramEntry> = entry.program.iter().flatten().collect();
    rows.sort_by(|a, b| (!a.id.is_empty(), &a.id).cmp(&(!b.id.is_empty(), &b.id)));
    rows.into_iter()
        .map(|r| Record {
            depth: match r.id.is_empty() {
                true => 0,
                false => r.id.matches('/').count() + 1,
            },
            name: name(r),
            note: [
                Some(r.state.clone()),
                r.progress.map(|p| format!("{p}%")),
                r.msg.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
        })
        .collect()
}

fn name(r: &ProgramEntry) -> String {
    let named: Vec<&str> = [r.app.as_deref(), r.title.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    match named.as_slice() {
        [] => {
            r.id.rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("program")
                .into()
        }
        [a, b] if a == b => a.to_string(),
        parts => parts.join(" · "),
    }
}
