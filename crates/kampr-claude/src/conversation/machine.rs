use kampr_core::conversation::facts::RunState;
use kampr_core::conversation::{Emit, Notice, Update};
use serde_json::{Value, json};

use super::child::Child;
use super::reader::Item;
use super::state::{Preflight, Request, State};
use super::{controls, facts, transcript};
use crate::config::{Billing as Policy, Overage};
use crate::kept::Kept;
use crate::request;
use crate::stored::Store;

pub(super) struct Settings {
    pub policy: Policy,
    pub overage: Overage,
    pub unattended_cap: u32,
    /// Makes this run's request ids differ from every earlier run's, which are still in the log.
    pub nonce: String,
    pub store: Store,
    pub kept: Kept,
}

/// Everything a headless Claude pane does, with the child and the pane both behind a seam: a
/// line of the child's goes in, `Update`s and lines of ours come out. The run loop is a thin
/// pump around it, and the tests drive it with a recorder for the child.
pub(super) struct Machine {
    pub settings: Settings,
    pub child: Box<dyn Child>,
    emit: Emit,
    pub st: State,
    /// Lines the child wrote before this conversation opened are being read again: they rebuild
    /// what is standing, and say nothing to the pane until they are done.
    pub replaying: bool,
    detached: bool,
    ended: bool,
}

impl Machine {
    pub fn new(settings: Settings, session_id: String, child: Box<dyn Child>, emit: Emit) -> Self {
        let mut st = State::new(session_id);
        st.held = settings.kept.held();
        Self {
            settings,
            child,
            emit,
            st,
            replaying: true,
            detached: false,
            ended: false,
        }
    }

    /// The conversation should stop being run: the pane has gone, a guard ended the session, or
    /// the child did.
    pub fn done(&self) -> bool {
        self.detached || self.ended
    }

    pub fn finish(&mut self) {
        self.ended = true;
    }

    /// Speak to the pane unless the replay is still rebuilding what it would be told.
    pub fn say(&mut self, update: Update) {
        if !self.replaying {
            self.tell(update);
        }
    }

    /// Speak to the pane whatever the replay is doing.
    pub fn tell(&mut self, update: Update) {
        if !(self.emit)(update) {
            self.detached = true;
        }
    }

    pub fn notice(&mut self, text: impl Into<String>) {
        self.tell(Update::Notice(Notice { text: text.into() }));
    }

    /// A line to the child. A line that did not get there is said, unless the child is gone, in
    /// which case its exit is what the pane is told.
    pub fn write(&mut self, line: &str) -> bool {
        match self.child.write(line) {
            Ok(()) => true,
            Err(e) => {
                if !self.child.exited() {
                    self.notice(format!("the session did not take that: {e}"));
                }
                false
            }
        }
    }

    fn control(&mut self, subtype: &str, body: Value) -> Option<String> {
        self.st.requests += 1;
        let id = format!("{}.{}", self.settings.nonce, self.st.requests);
        self.write(&request::control_request(&id, subtype, body))
            .then_some(id)
    }

    /// A request whose answer is read when it comes.
    pub fn request(&mut self, kind: Request, subtype: &str, body: Value) {
        if let Some(id) = self.control(subtype, body) {
            self.st.inflight.insert(id, kind);
        }
    }

    /// A request nobody waits on the answer to.
    pub fn fire(&mut self, subtype: &str, body: Value) {
        self.control(subtype, body);
    }

    /// Stop the turn in flight. A replay is reading what already happened, so it stops nothing.
    pub fn interrupt(&mut self) {
        if !self.replaying {
            self.fire("interrupt", json!({}));
        }
    }

    /// What the pane sees first: what this conversation can do, and the history it opens on.
    pub fn opened(&mut self) {
        self.tell(Update::Abilities(controls::abilities()));
        let page = transcript::history(self).unwrap_or_else(|| kampr_journal::Page {
            turns: Vec::new(),
            cursor: None,
            more: false,
        });
        self.tell(Update::Page(page));
    }

    pub fn item(&mut self, item: Item) {
        match item {
            Item::Out(line) => self.line(&line),
            Item::Past(line) => self.past(&line),
            Item::Replayed { alive } => self.replayed(alive),
            Item::Exited => {
                self.replaying = false;
                self.tell(Update::State(RunState::Ended));
                self.finish();
            }
        }
    }

    fn replayed(&mut self, alive: bool) {
        self.replaying = false;
        self.settle();
        if !alive {
            return;
        }
        if !self.st.initialize_sent {
            self.st.initialize_sent = true;
            self.request(Request::Initialize, "initialize", json!({}));
        } else if self.st.preflight == Preflight::Allowed {
            self.refresh_reads();
        }
        if let Some(prompt) = self.settings.kept.opening()
            && self.send_word(request::user_message(&prompt, &[], None, None))
        {
            self.settings.kept.opened();
        }
    }

    /// Everything the replay rebuilt, said once.
    fn settle(&mut self) {
        let facts = facts::session_facts(&self.st);
        self.tell(Update::Session(facts));
        if !self.st.actions.is_empty() {
            self.tell(Update::Actions(self.st.actions.clone()));
        }
        if !self.st.controls.is_empty() {
            self.tell(Update::Controls(self.st.controls.clone()));
        }
        if !self.st.tasks.is_empty() {
            self.tell(Update::Tasks(self.st.tasks.values().cloned().collect()));
        }
        if let Some(limits) = self.st.limits.clone() {
            self.tell(Update::Limits(limits));
        }
        if let Some(context) = self.st.context {
            self.tell(Update::Context(context));
        }
        if let Some(cost) = self.st.cost {
            self.tell(Update::Cost(cost));
        }
        if let Some(trouble) = self.st.trouble.clone() {
            self.tell(Update::Trouble(Some(trouble)));
        }
        if !self.st.standing.is_empty() {
            self.say_asks();
        }
        self.tell(Update::State(self.st.run));
    }

    /// The reads that cost no turn and that the pane's bars follow.
    pub fn refresh_reads(&mut self) {
        if self.replaying {
            return;
        }
        self.request(Request::Usage, "get_usage", json!({}));
        self.request(Request::Context, "get_context_usage", json!({}));
    }
}
