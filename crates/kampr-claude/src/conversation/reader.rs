use std::path::PathBuf;
use std::time::Duration;

use kampr_keep::{PipeDir, PipeRead, SeqReader, SessionDir};
use tokio::sync::mpsc;

/// What the keeper's log says, in the order it says it. The log is the source of truth: a node
/// that restarts reads it again from the start, so every conversation opens on a replay.
pub(super) enum Item {
    /// A line the child wrote.
    Out(String),
    /// A line this side wrote before the replay caught up: an answer already given, a request
    /// already sent.
    Past(String),
    /// The replay is over: from here on lines are live. `alive` is whether the child still runs.
    Replayed { alive: bool },
    /// Nothing more will come.
    Exited,
}

pub(super) fn spawn(path: PathBuf, tx: mpsc::Sender<Item>) {
    tokio::task::spawn_blocking(move || read(path, tx));
}

fn read(path: PathBuf, tx: mpsc::Sender<Item>) {
    let session = SessionDir::at(&path);
    let Ok(mut reader) = SeqReader::at(&path, 0) else {
        let _ = tx.blocking_send(Item::Exited);
        return;
    };
    let mut replayed = false;
    loop {
        // Checked before the read that may say CaughtUp: whatever the child wrote before it died
        // is in the log by the time exit.json is, so a caught-up reader after it has seen all.
        let exited = session.exit().is_some();
        let item = match reader.read() {
            Ok(PipeRead::Entry(entry)) => match (entry.dir, replayed) {
                (PipeDir::Out, _) => Item::Out(entry.line),
                (PipeDir::In, false) => Item::Past(entry.line),
                (PipeDir::In, true) => continue,
            },
            Ok(PipeRead::Truncated { .. }) => continue,
            Ok(PipeRead::CaughtUp) => {
                if !replayed {
                    replayed = true;
                    if tx.blocking_send(Item::Replayed { alive: !exited }).is_err() {
                        return;
                    }
                }
                if exited {
                    let _ = tx.blocking_send(Item::Exited);
                    return;
                }
                // A failed send is the only other exit, so a conversation that has gone is seen
                // here: without this the thread outlives it and pins the runtime's shutdown.
                if tx.is_closed() {
                    return;
                }
                let _ = reader.wait(Duration::from_millis(100));
                continue;
            }
            Err(_) => {
                let _ = tx.blocking_send(Item::Exited);
                return;
            }
        };
        if tx.blocking_send(item).is_err() {
            return;
        }
    }
}
