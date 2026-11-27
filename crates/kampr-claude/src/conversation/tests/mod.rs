//! The conversation machine, driven line by line: a recorder for the child, a recorder for the
//! pane, and the lines the binary really sends (their shapes are in the probe log and the
//! binary's own schema, named where they are used).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use kampr_core::conversation::facts::Fix;
use kampr_core::conversation::{Command, Update};
use serde_json::Value;

use super::child::Child;
use super::machine::{Machine, Settings};
use super::reader::Item;
use crate::config::{Billing, Config, Overage};
use crate::kept::Kept;
use crate::stored::Store;

mod asks;
mod commands;
mod fixtures;
mod guards;
mod replay;
mod transcript;
mod turn;

use fixtures::*;

struct Recorder {
    written: Arc<Mutex<Vec<String>>>,
    closed: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    refusing: Arc<AtomicBool>,
}

impl Child for Recorder {
    fn write(&mut self, line: &str) -> std::io::Result<()> {
        if self.exited.load(Ordering::SeqCst) || self.refusing.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("the child did not take the line"));
        }
        self.written.lock().unwrap().push(line.to_string());
        Ok(())
    }

    fn close(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }
}

pub(super) struct Rig {
    pub machine: Machine,
    written: Arc<Mutex<Vec<String>>>,
    updates: Arc<Mutex<Vec<Update>>>,
    closed: Arc<AtomicBool>,
    pub exited: Arc<AtomicBool>,
    /// Writes fail while the child is still there.
    pub refusing: Arc<AtomicBool>,
    pub dir: tempfile::TempDir,
}

pub(super) struct Opts {
    pub policy: Billing,
    pub overage: Overage,
    pub cap: u32,
    pub opening: Option<String>,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            policy: Billing::Plan,
            overage: Overage::Refuse,
            cap: 20,
            opening: None,
        }
    }
}

impl Rig {
    /// A conversation that has been opened and has not read any log yet.
    pub fn opened(opts: Opts) -> Self {
        Self::on(tempfile::tempdir().expect("a config dir"), opts)
    }

    /// The same session opened again by a node that restarted: the child, the pane and the
    /// machine are new, and only what is on disk is left.
    pub fn restarted(self, opts: Opts) -> Self {
        Self::on(self.dir, opts)
    }

    fn on(dir: tempfile::TempDir, opts: Opts) -> Self {
        let config = Config {
            config_dir: dir.path().display().to_string(),
            ..Config::default()
        };
        let written = Arc::new(Mutex::new(Vec::new()));
        let updates = Arc::new(Mutex::new(Vec::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let refusing = Arc::new(AtomicBool::new(false));
        let child = Recorder {
            written: written.clone(),
            closed: closed.clone(),
            exited: exited.clone(),
            refusing: refusing.clone(),
        };
        let keep = updates.clone();
        let emit = Box::new(move |update: Update| {
            keep.lock().unwrap().push(update);
            true
        });
        let session = dir.path().join("session");
        std::fs::create_dir_all(&session).expect("a session dir");
        let kept = Kept::at(session);
        if let Some(opening) = &opts.opening {
            kept.set_opening(opening).expect("the opening prompt kept");
        }
        let settings = Settings {
            policy: opts.policy,
            overage: opts.overage,
            unattended_cap: opts.cap,
            nonce: "t".into(),
            store: Store::new(&config),
            kept,
        };
        let mut machine = Machine::new(settings, "s1".into(), Box::new(child), emit);
        machine.opened();
        Rig {
            machine,
            written,
            updates,
            closed,
            exited,
            refusing,
            dir,
        }
    }

    /// A conversation on a fresh session whose log is empty, so the `initialize` is sent.
    pub fn started(opts: Opts) -> Self {
        let mut rig = Self::opened(opts);
        rig.item(Item::Replayed { alive: true });
        rig
    }

    /// The same, with the account answered: a plan login under the plan policy.
    pub fn allowed(opts: Opts) -> Self {
        let mut rig = Self::started(opts);
        rig.line(&initialize_response("t.1", PLAN_ACCOUNT));
        rig
    }

    pub fn item(&mut self, item: Item) {
        self.machine.item(item);
    }

    pub fn line(&mut self, line: &str) {
        self.machine.item(Item::Out(line.to_string()));
    }

    pub fn command(&mut self, command: Command) {
        self.machine.command(command);
    }

    pub fn written(&self) -> Vec<Value> {
        self.written
            .lock()
            .unwrap()
            .iter()
            .map(|l| serde_json::from_str(l).expect("a JSON line to the child"))
            .collect()
    }

    /// The control requests of one subtype written so far.
    pub fn requests(&self, subtype: &str) -> Vec<Value> {
        self.written()
            .into_iter()
            .filter(|l| l["type"] == "control_request" && l["request"]["subtype"] == subtype)
            .collect()
    }

    /// The `user` lines written so far: the words that were sent.
    pub fn words(&self) -> Vec<Value> {
        self.written()
            .into_iter()
            .filter(|l| l["type"] == "user")
            .collect()
    }

    /// The control responses written so far: what was answered.
    pub fn answers(&self) -> Vec<Value> {
        self.written()
            .into_iter()
            .filter(|l| l["type"] == "control_response")
            .collect()
    }

    /// The id of the request of this subtype written last.
    pub fn request_id(&self, subtype: &str) -> String {
        self.requests(subtype)
            .last()
            .unwrap_or_else(|| panic!("no {subtype} request was written: {:?}", self.written()))["request_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    pub fn updates(&self) -> Vec<Update> {
        self.updates.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.updates.lock().unwrap().clear();
    }

    /// Whether the child was hung up on.
    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn billing_trouble(&self) -> Option<String> {
        self.updates().iter().find_map(|u| match u {
            Update::Trouble(Some(t)) if t.fix.is_some() => Some(t.text.clone()),
            _ => None,
        })
    }

    pub fn trouble_fix(&self) -> Option<Fix> {
        self.updates().iter().rev().find_map(|u| match u {
            Update::Trouble(Some(t)) => t.fix.clone(),
            _ => None,
        })
    }

    pub fn last_asks(&self) -> Vec<kampr_core::conversation::Ask> {
        self.updates()
            .iter()
            .rev()
            .find_map(|u| match u {
                Update::Asks(a) => Some(a.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn notices(&self) -> Vec<String> {
        self.updates()
            .iter()
            .filter_map(|u| match u {
                Update::Notice(n) => Some(n.text.clone()),
                _ => None,
            })
            .collect()
    }
}

pub(super) fn input(text: &str) -> Command {
    Command::Input {
        text: text.into(),
        msg_id: "m1".into(),
        images: Vec::new(),
        files: Vec::new(),
        when: kampr_core::conversation::When::Idle,
    }
}
