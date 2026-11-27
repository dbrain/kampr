use kampr_core::conversation::{Ask, AskAbility, AskField, AskOption, Reply, Update};
use serde_json::{Value, json};

use super::guards;
use super::machine::Machine;
use crate::wire::ControlRequest;
use crate::{plan, request};

/// One thing the binary or a guard is waiting on a person for.
pub(super) enum Standing {
    /// A tool's permission.
    Tool(ControlRequest),
    /// `AskUserQuestion`: every question is its own ask, and the binary is answered once all are.
    Question {
        request: ControlRequest,
        questions: Vec<Question>,
        chosen: Vec<Option<String>>,
        /// What is ticked on each question that takes several answers, by option.
        ticked: Vec<Vec<bool>>,
    },
    /// `ExitPlanMode`: the plan is in `input.plan` (#674).
    Plan(ControlRequest),
    /// An MCP server's form (#684).
    Form(ControlRequest),
    /// Guard 6 and guard 10 under `overage = "ask"`.
    Overage,
}

pub(super) struct Question {
    text: String,
    header: Option<String>,
    options: Vec<(String, Option<String>)>,
    multi: bool,
}

const OVERAGE: &str = "overage";
const NO_WORDS: &str = "this answer takes no words";
// AskUserQuestion takes any words as an answer, and the terminal offers them as this (#639).
const TYPE_SOMETHING: &str = "Type something";

fn option(key: String, label: String, detail: Option<String>) -> AskOption {
    AskOption {
        key,
        label,
        detail,
        chosen: false,
        text: false,
    }
}

fn questions_of(input: &Value) -> Vec<Question> {
    input
        .get("questions")
        .and_then(Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .filter_map(|q| {
                    Some(Question {
                        text: q.get("question")?.as_str()?.to_string(),
                        header: q.get("header").and_then(Value::as_str).map(str::to_string),
                        options: q
                            .get("options")
                            .and_then(Value::as_array)
                            .map(|options| {
                                options
                                    .iter()
                                    .filter_map(|o| {
                                        Some((
                                            o.get("label")?.as_str()?.to_string(),
                                            o.get("description").and_then(Value::as_str).map(str::to_string),
                                        ))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        multi: q.get("multiSelect").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Standing {
    pub(super) fn of(request: ControlRequest) -> Option<Self> {
        match (request.subtype.as_str(), request.tool_name()) {
            ("can_use_tool", Some("AskUserQuestion")) => {
                let questions = questions_of(&request.input());
                if questions.is_empty() {
                    return Some(Self::Tool(request));
                }
                let chosen = vec![None; questions.len()];
                let ticked = questions.iter().map(|q| vec![false; q.options.len()]).collect();
                Some(Self::Question {
                    request,
                    questions,
                    chosen,
                    ticked,
                })
            }
            ("can_use_tool", Some("ExitPlanMode")) => Some(Self::Plan(request)),
            ("can_use_tool", _) => Some(Self::Tool(request)),
            ("elicitation", _) => Some(Self::Form(request)),
            _ => None,
        }
    }

    /// What answering it no longer would be, said in a hand-over's confirmation.
    pub fn stops(&self) -> Option<String> {
        match self {
            Self::Tool(r) => Some(format!(
                "the permission it is asking for ({})",
                r.tool_name().unwrap_or("a tool")
            )),
            Self::Question { .. } => Some("the question it is asking".into()),
            Self::Plan(_) => Some("the plan waiting for approval".into()),
            Self::Form(_) => Some("the form waiting to be filled".into()),
            Self::Overage => None,
        }
    }

    pub fn owns_request(&self, id: &str) -> bool {
        match self {
            Self::Tool(r) | Self::Plan(r) | Self::Form(r) => r.request_id == id,
            Self::Question { request, .. } => request.request_id == id,
            Self::Overage => false,
        }
    }

    fn owns(&self, ask: &str) -> bool {
        match self {
            Self::Overage => ask == OVERAGE,
            Self::Question {
                request, questions, ..
            } => (0..questions.len()).any(|i| ask == format!("{}#{i}", request.request_id)),
            _ => self.owns_request(ask),
        }
    }

    pub fn asks(&self) -> Vec<Ask> {
        let by = |r: &ControlRequest| r.agent_id().map(str::to_string);
        match self {
            Self::Overage => vec![guards::overage_ask()],
            Self::Tool(r) => {
                let mut options = vec![option("1".into(), "Allow".into(), None)];
                if !r.permission_suggestions().is_empty() {
                    options.push(option("2".into(), "Always allow".into(), None));
                }
                options.push(option("3".into(), "Deny".into(), None));
                vec![Ask {
                    id: Some(r.request_id.clone()),
                    kind: AskAbility::Permission,
                    by: by(r),
                    header: r.tool_name().map(str::to_string),
                    question: r.tool_name().unwrap_or("allow this tool?").to_string(),
                    multi: false,
                    context: Some(r.input().to_string()),
                    options,
                    fields: Vec::new(),
                }]
            }
            Self::Question {
                request,
                questions,
                chosen,
                ticked,
            } => questions
                .iter()
                .enumerate()
                .filter(|(i, _)| chosen[*i].is_none())
                .map(|(i, q)| {
                    let mut options: Vec<AskOption> = q
                        .options
                        .iter()
                        .enumerate()
                        .map(|(n, (label, detail))| AskOption {
                            chosen: ticked[i][n],
                            ..option((n + 1).to_string(), label.clone(), detail.clone())
                        })
                        .collect();
                    options.push(AskOption {
                        text: true,
                        ..option((options.len() + 1).to_string(), TYPE_SOMETHING.into(), None)
                    });
                    Ask {
                        id: Some(format!("{}#{i}", request.request_id)),
                        kind: AskAbility::Question,
                        by: by(request),
                        header: q.header.clone(),
                        question: q.text.clone(),
                        multi: q.multi,
                        context: None,
                        options,
                        fields: Vec::new(),
                    }
                })
                .collect(),
            Self::Plan(r) => vec![Ask {
                id: Some(r.request_id.clone()),
                kind: AskAbility::Plan,
                by: by(r),
                header: Some("plan".into()),
                question: "approve this plan?".into(),
                multi: false,
                context: r.input().get("plan").and_then(Value::as_str).map(str::to_string),
                options: plan::ANSWERS
                    .iter()
                    .map(|(key, label, mode)| AskOption {
                        text: mode.is_none(),
                        ..option(key.to_string(), label.to_string(), None)
                    })
                    .collect(),
                fields: Vec::new(),
            }],
            Self::Form(r) => vec![Ask {
                id: Some(r.request_id.clone()),
                kind: AskAbility::Form,
                by: r
                    .request
                    .get("mcp_server_name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                header: r.request.get("title").and_then(Value::as_str).map(str::to_string),
                question: r
                    .request
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("an MCP server asks for input")
                    .to_string(),
                multi: false,
                context: None,
                options: vec![
                    option("1".into(), "Submit".into(), None),
                    option("3".into(), "Decline".into(), None),
                ],
                fields: fields_of(&r.request),
            }],
        }
    }

    /// Which of a question's parts an ask names; with none named, the first still unanswered.
    fn part(&self, ask: &str) -> Option<usize> {
        let Self::Question { request, chosen, .. } = self else {
            return None;
        };
        (0..chosen.len())
            .find(|i| ask == format!("{}#{i}", request.request_id))
            .or_else(|| chosen.iter().position(Option::is_none))
    }

    /// The reply a pressed key and its words mean here, or none where the press only ticks.
    fn pressed(&mut self, ask: &str, key: &str, words: Option<&str>) -> Result<Option<Reply>, String> {
        let part = self.part(ask);
        let unworded = |reply: Reply| match words {
            Some(_) => Err(NO_WORDS.to_string()),
            None => Ok(Some(reply)),
        };
        let not_one = || format!("{key} is not one of this question's answers");
        match (self, key) {
            (Self::Overage | Self::Tool(_), "1") => unworded(Reply::Allow {
                input: None,
                remember: false,
            }),
            (Self::Tool(_), "2") => unworded(Reply::Allow {
                input: None,
                remember: true,
            }),
            (Self::Overage | Self::Tool(_) | Self::Form(_), "3") => unworded(Reply::Deny { message: None }),
            (Self::Plan(_), key) if plan::mode(key).is_some() => unworded(Reply::Plan {
                approve: true,
                mode: plan::mode(key).map(str::to_string),
                feedback: None,
            }),
            (Self::Plan(_), "3") => Ok(Some(Reply::Plan {
                approve: false,
                mode: None,
                feedback: words.map(str::to_string),
            })),
            (Self::Form(r), "1") => match fields_of(&r.request).iter().any(|f| f.required) {
                true => Err("this form is answered with its fields filled".into()),
                false => unworded(Reply::Form { values: json!({}) }),
            },
            (
                Self::Question {
                    questions, ticked, ..
                },
                key,
            ) => {
                let i = part.ok_or_else(not_one)?;
                let q = &questions[i];
                let n = key
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=q.options.len() + 1).contains(n));
                let n = n.ok_or_else(not_one)?;
                if n > q.options.len() {
                    let words = words.ok_or("that answer needs words")?;
                    let mut answers = ticked_labels(q, &ticked[i]);
                    answers.push(words.to_string());
                    return Ok(Some(Reply::Choose { answers }));
                }
                if words.is_some() {
                    return Err(NO_WORDS.into());
                }
                if q.multi {
                    ticked[i][n - 1] = !ticked[i][n - 1];
                    return Ok(None);
                }
                Ok(Some(Reply::Choose {
                    answers: vec![q.options[n - 1].0.clone()],
                }))
            }
            _ => Err(not_one()),
        }
    }

    fn submitted(&self, ask: &str) -> Result<Reply, String> {
        let part = self.part(ask);
        let Self::Question {
            questions, ticked, ..
        } = self
        else {
            return Err("this question takes one answer, and a press gives it".into());
        };
        let i = part.ok_or("nothing is waiting on that answer")?;
        if !questions[i].multi {
            return Err("this question takes one answer, and a press gives it".into());
        }
        let answers = ticked_labels(&questions[i], &ticked[i]);
        if answers.is_empty() {
            return Err("nothing is ticked".into());
        }
        Ok(Reply::Choose { answers })
    }
}

fn ticked_labels(question: &Question, ticked: &[bool]) -> Vec<String> {
    question
        .options
        .iter()
        .zip(ticked)
        .filter(|(_, on)| **on)
        .map(|((label, _), _)| label.clone())
        .collect()
}

/// A form's inputs in the order the server asked for them (#684's `property_order`), then any it
/// left out.
fn fields_of(request: &Value) -> Vec<AskField> {
    let schema = &request["requested_schema"];
    let Some(properties) = schema["properties"].as_object() else {
        return Vec::new();
    };
    let required: Vec<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut names: Vec<&str> = request["property_order"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|name| properties.contains_key(*name))
        .collect();
    for name in properties.keys() {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    names
        .into_iter()
        .map(|name| {
            let field = &properties[name];
            AskField {
                name: name.to_string(),
                title: field["title"].as_str().filter(|t| *t != name).map(str::to_string),
                detail: field["description"].as_str().map(str::to_string),
                kind: field["type"].as_str().unwrap_or("string").to_string(),
                required: required.contains(&name),
                choices: field["enum"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            }
        })
        .collect()
}

impl Machine {
    /// A request of the binary's mid-turn.
    pub(super) fn control_request(&mut self, request: ControlRequest) {
        let id = request.request_id.clone();
        match Standing::of(request) {
            Some(standing) => {
                self.st.standing.push(standing);
                self.say_asks();
            }
            None => {
                self.write(&request::control_error(
                    &id,
                    "kampr answers permissions, questions, plans and forms",
                ));
            }
        }
    }

    pub(super) fn say_asks(&mut self) {
        let asks = self.st.standing.iter().flat_map(Standing::asks).collect();
        self.say(Update::Asks(asks));
    }

    /// The pane's answer to the ask it names. An old client names none, and the oldest standing
    /// is the one it means.
    pub(super) fn answer(&mut self, ask: &str, reply: Reply) {
        let Some(at) = self.standing_at(ask) else {
            return;
        };
        match self.reply(at, ask, reply) {
            Ok(done) => {
                if done {
                    self.st.standing.remove(at);
                }
                self.say_asks();
            }
            Err(why) => self.notice(why),
        }
    }

    fn standing_at(&mut self, ask: &str) -> Option<usize> {
        let at = if ask.is_empty() {
            (!self.st.standing.is_empty()).then_some(0)
        } else {
            self.st.standing.iter().position(|s| s.owns(ask))
        };
        if at.is_none() {
            self.notice("nothing is waiting on that answer");
        }
        at
    }

    pub(super) fn press(&mut self, ask: &str, key: &str, text: Option<&str>) {
        let Some(at) = self.standing_at(ask) else {
            return;
        };
        let words = text.map(str::trim).filter(|t| !t.is_empty());
        match self.st.standing[at].pressed(ask, key, words) {
            Ok(Some(reply)) => self.answer(ask, reply),
            Ok(None) => self.say_asks(),
            Err(why) => self.notice(why),
        }
    }

    pub(super) fn submit(&mut self, ask: &str) {
        let Some(at) = self.standing_at(ask) else {
            return;
        };
        match self.st.standing[at].submitted(ask) {
            Ok(reply) => self.answer(ask, reply),
            Err(why) => self.notice(why),
        }
    }

    /// Returns whether the ask is finished, or why this reply does not answer it.
    fn reply(&mut self, at: usize, ask: &str, reply: Reply) -> Result<bool, String> {
        let line = match (&mut self.st.standing[at], reply) {
            (Standing::Overage, Reply::Allow { .. }) => {
                self.st.hold = None;
                self.st.overage_allowed = true;
                self.release();
                return Ok(true);
            }
            (Standing::Overage, _) => {
                self.st.standing.remove(at);
                self.end_session(guards::account_trouble(
                    "usage credits were not allowed; the session was stopped".into(),
                ));
                return Ok(false);
            }
            (Standing::Tool(r), Reply::Allow { input, remember }) => {
                let perms = if remember {
                    r.permission_suggestions()
                } else {
                    Vec::new()
                };
                request::permission_allow(&r.request_id, input.unwrap_or_else(|| r.input()), perms)
            }
            (Standing::Tool(r), Reply::Deny { message }) => {
                request::permission_deny(&r.request_id, message.as_deref().unwrap_or("Denied from Kampr"))
            }
            (Standing::Tool(r) | Standing::Plan(r), Reply::Dismiss) => {
                request::permission_deny(&r.request_id, "Dismissed from Kampr")
            }
            (
                Standing::Plan(r),
                Reply::Plan {
                    approve: true, mode, ..
                },
            ) => request::permission_allow(&r.request_id, r.input(), mode_change(mode)),
            (Standing::Plan(r), Reply::Allow { .. }) => {
                request::permission_allow(&r.request_id, r.input(), Vec::new())
            }
            (Standing::Plan(r), Reply::Plan { feedback, .. }) => {
                request::permission_deny(&r.request_id, feedback.as_deref().unwrap_or("Revise the plan"))
            }
            (Standing::Plan(r), Reply::Deny { message }) => {
                request::permission_deny(&r.request_id, message.as_deref().unwrap_or("Revise the plan"))
            }
            (Standing::Form(r), Reply::Form { values }) => {
                request::elicitation_answer(&r.request_id, "accept", values)
            }
            (Standing::Form(r), Reply::Allow { .. }) => {
                request::elicitation_answer(&r.request_id, "accept", json!({}))
            }
            (Standing::Form(r), Reply::Deny { .. }) => {
                request::elicitation_answer(&r.request_id, "decline", Value::Null)
            }
            (Standing::Form(r), Reply::Dismiss) => {
                request::elicitation_answer(&r.request_id, "cancel", Value::Null)
            }
            (standing @ Standing::Question { .. }, reply) => match reply {
                Reply::Choose { answers } => {
                    let i = standing.part(ask).unwrap_or(0);
                    let Standing::Question {
                        request,
                        questions,
                        chosen,
                        ..
                    } = standing
                    else {
                        unreachable!("matched above")
                    };
                    chosen[i] = Some(answers.join(", "));
                    if chosen.iter().any(Option::is_none) {
                        return Ok(false);
                    }
                    let answers: serde_json::Map<String, Value> = questions
                        .iter()
                        .zip(chosen.iter())
                        .map(|(q, a)| (q.text.clone(), Value::String(a.clone().unwrap_or_default())))
                        .collect();
                    let mut input = request.input();
                    input["answers"] = Value::Object(answers);
                    request::permission_allow(&request.request_id, input, Vec::new())
                }
                Reply::Deny { .. } | Reply::Dismiss => {
                    let Standing::Question { request, .. } = standing else {
                        unreachable!("matched above")
                    };
                    request::permission_deny(&request.request_id, "The user declined to answer the questions")
                }
                _ => return Err("this question is answered with a choice".into()),
            },
            _ => return Err("that answer does not fit what is waiting".into()),
        };
        self.write(&line);
        Ok(true)
    }
}

fn mode_change(mode: Option<String>) -> Vec<Value> {
    mode.map(|m| json!({ "type": "setMode", "mode": m, "destination": "session" }))
        .into_iter()
        .collect()
}
