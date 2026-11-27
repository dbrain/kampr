use std::collections::HashMap;

use kampr_core::conversation::facts::{Cost, Limits, RunState, Task, Trouble};
use kampr_core::conversation::{Action, ContextUse, Control};

use super::asks::Standing;
use super::controls::Catalog;
use crate::billing::Account;
use crate::wire::Init;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Preflight {
    Pending,
    Allowed,
    Refused,
}

/// A request of ours that has not been answered, by the id the answer will carry.
pub(super) enum Request {
    Initialize,
    Usage,
    Context,
    Suggest(String),
    TaskOutput(String),
    Setting {
        control: String,
        value: serde_json::Value,
    },
}

/// Why words are being kept rather than sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hold {
    /// Guard 6 with `overage = "ask"`: a person has to say the credits may be spent.
    Overage,
    /// Guard 10: a window of the plan is spent and overage is not allowed.
    Limit,
}

pub(super) struct State {
    pub preflight: Preflight,
    pub init: Option<Init>,
    pub catalog: Catalog,
    pub account: Option<Account>,
    pub overage: bool,
    /// A person said the credits may be spent, so the hold is not raised again.
    pub overage_allowed: bool,
    pub unattended: u32,
    pub tasks: HashMap<String, Task>,
    /// The tasks the pane watches, each with the last total its output reached.
    pub followed: HashMap<String, u64>,
    pub standing: Vec<Standing>,
    pub inflight: HashMap<String, Request>,
    pub requests: u64,
    pub hold: Option<Hold>,
    /// The lines kept while words are held, in the order they were asked for.
    pub held: Vec<String>,
    /// The word of the turn in flight, so an overage hold can send it again once allowed.
    pub in_flight: Option<String>,
    pub initialize_sent: bool,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub fast: Option<bool>,
    pub session_id: String,
    pub run: RunState,
    pub trouble: Option<Trouble>,
    pub limits: Option<Limits>,
    pub context: Option<ContextUse>,
    pub cost: Option<Cost>,
    pub controls: Vec<Control>,
    pub actions: Vec<Action>,
}

impl State {
    pub fn new(session_id: String) -> Self {
        Self {
            preflight: Preflight::Pending,
            init: None,
            catalog: Catalog::default(),
            account: None,
            overage: false,
            overage_allowed: false,
            unattended: 0,
            tasks: HashMap::new(),
            followed: HashMap::new(),
            standing: Vec::new(),
            inflight: HashMap::new(),
            requests: 0,
            hold: None,
            held: Vec::new(),
            in_flight: None,
            initialize_sent: false,
            mode: None,
            model: None,
            fast: None,
            session_id,
            run: RunState::Idle,
            trouble: None,
            limits: None,
            context: None,
            cost: None,
            controls: Vec::new(),
            actions: Vec::new(),
        }
    }
}
