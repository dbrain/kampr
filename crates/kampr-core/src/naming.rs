//! What a pane is called, from the fields the herd already carries.
//!
//! Here rather than in a client because all three of them have to agree: the same template over
//! the same fields renders byte-identically in Rust and in Kotlin, and
//! `crates/kampr-core/tests/fixtures/naming-cases.json` is what holds the two to it.
//!
//! Two shapes and no more. `{a|b|'x'}` takes the first of its choices that resolves to something,
//! and `[…]` is dropped whole when nothing inside it did — which is the entire reason the syntax
//! exists, because `{cmd}` is blank on every pane of a machine that sources ble.sh (probe #297)
//! and `kampr ()` is worse than `kampr`.

use crate::provider::AgentStatus;
use crate::wire::PaneEntry;
use std::fmt;

pub const DEFAULT_TEMPLATE: &str = "{label|workspace|cwd|pane}[ ({argv|cmd})] · {agent|'bash'}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Label,
    Workspace,
    Tab,
    Cwd,
    Pane,
    Agent,
    Status,
    Cmd,
    Argv,
}

impl Token {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "label" => Self::Label,
            "workspace" => Self::Workspace,
            "tab" => Self::Tab,
            "cwd" => Self::Cwd,
            "pane" => Self::Pane,
            "agent" => Self::Agent,
            "status" => Self::Status,
            "cmd" => Self::Cmd,
            "argv" => Self::Argv,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    UnknownToken(String),
    EmptyChoice,
    UnclosedSlot,
    UnclosedLiteral,
    UnclosedGroup,
    UnopenedGroup,
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownToken(name) => write!(
                f,
                "no such template token `{name}`; the tokens are label, workspace, tab, cwd, pane, \
                 agent, status, cmd, argv"
            ),
            Self::EmptyChoice => write!(f, "a `{{}}` needs at least one token or 'literal' in it"),
            Self::UnclosedSlot => write!(f, "a `{{` was never closed"),
            Self::UnclosedLiteral => write!(f, "a `'` was never closed"),
            Self::UnclosedGroup => write!(f, "a `[` was never closed"),
            Self::UnopenedGroup => write!(f, "a `]` closes a group that was never opened"),
        }
    }
}

impl std::error::Error for TemplateError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Choice {
    Token(Token),
    Literal(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Slot(Vec<Choice>),
    Group(Vec<Part>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
}

impl Default for Template {
    fn default() -> Self {
        Self::parse(DEFAULT_TEMPLATE).expect("the default template parses")
    }
}

impl Template {
    pub fn parse(source: &str) -> Result<Self, TemplateError> {
        let mut chars = source.chars().peekable();
        let parts = parse_parts(&mut chars, false)?;
        Ok(Self { parts })
    }

    /// Never empty: a template that resolves to nothing gives the pane's own id back, because a
    /// nameless row in a sidebar is not something an operator can act on.
    pub fn render(&self, fields: &Fields<'_>) -> String {
        let mut out = String::new();
        render_parts(&self.parts, fields, &mut out);
        match out.trim() {
            "" => fields.pane.to_string(),
            name => name.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Fields<'a> {
    pub pane: &'a str,
    pub workspace: Option<&'a str>,
    pub tab: Option<&'a str>,
    /// The whole path. `{cwd}` renders its last segment — a title has no room for the rest, and
    /// the segment is the thing six panes in one directory share.
    pub cwd: Option<&'a str>,
    pub label: Option<&'a str>,
    pub agent: Option<&'a str>,
    pub status: AgentStatus,
    pub cmd: Option<&'a str>,
    pub argv: Option<&'a str>,
}

impl<'a> Fields<'a> {
    pub fn from_entry(entry: &'a PaneEntry) -> Self {
        Self {
            pane: short_id(&entry.id),
            workspace: entry.workspace.as_deref(),
            tab: entry.tab.as_deref(),
            cwd: entry.cwd.as_deref(),
            label: entry.label.as_deref(),
            agent: entry.agent.as_deref(),
            status: entry.agent_status,
            cmd: entry.cmd.as_deref(),
            argv: entry.argv.as_deref(),
        }
    }

    pub fn from_info(info: &'a crate::provider::PaneInfo) -> Self {
        Self {
            pane: &info.pane_id,
            workspace: info.workspace.as_deref(),
            tab: info.tab.as_deref(),
            cwd: info.cwd.as_deref(),
            label: info.label.as_deref(),
            agent: info.agent.as_deref(),
            status: info.agent_status,
            cmd: info.cmd.as_deref(),
            argv: info.argv.as_deref(),
        }
    }
}

fn short_id(id: &str) -> &str {
    id.split_once('/').map_or(id, |(_, rest)| rest)
}

fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.is_empty() {
        true => path,
        false => trimmed.rsplit('/').next().unwrap_or(trimmed),
    }
}

fn status_word(status: AgentStatus) -> Option<&'static str> {
    Some(match status {
        AgentStatus::Idle => "idle",
        AgentStatus::Working => "working",
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        // Not a state to print. It is the absence of one, and a row that says so says nothing.
        AgentStatus::Unknown => return None,
    })
}

fn value<'a>(token: Token, fields: &Fields<'a>) -> Option<&'a str> {
    let raw = match token {
        Token::Label => fields.label,
        Token::Workspace => fields.workspace,
        Token::Tab => fields.tab,
        Token::Cwd => fields.cwd.map(basename),
        Token::Pane => Some(fields.pane),
        Token::Agent => fields.agent,
        Token::Status => status_word(fields.status),
        Token::Cmd => fields.cmd,
        Token::Argv => fields.argv,
    };
    raw.map(str::trim).filter(|v| !v.is_empty())
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

fn parse_parts(chars: &mut Chars<'_>, in_group: bool) -> Result<Vec<Part>, TemplateError> {
    let mut parts: Vec<Part> = Vec::new();
    let mut text = String::new();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '[' if chars.peek() == Some(&'[') => {
                chars.next();
                text.push('[');
            }
            ']' if chars.peek() == Some(&']') => {
                chars.next();
                text.push(']');
            }
            '{' => {
                flush(&mut text, &mut parts);
                parts.push(Part::Slot(parse_slot(chars)?));
            }
            '[' => {
                flush(&mut text, &mut parts);
                parts.push(Part::Group(parse_parts(chars, true)?));
            }
            ']' if in_group => {
                flush(&mut text, &mut parts);
                return Ok(parts);
            }
            ']' => return Err(TemplateError::UnopenedGroup),
            other => text.push(other),
        }
    }
    match in_group {
        true => Err(TemplateError::UnclosedGroup),
        false => {
            flush(&mut text, &mut parts);
            Ok(parts)
        }
    }
}

fn flush(text: &mut String, parts: &mut Vec<Part>) {
    if !text.is_empty() {
        parts.push(Part::Text(std::mem::take(text)));
    }
}

fn parse_slot(chars: &mut Chars<'_>) -> Result<Vec<Choice>, TemplateError> {
    let mut choices = Vec::new();
    let mut word = String::new();
    let mut literal: Option<String> = None;
    let mut closed = false;
    for c in chars.by_ref() {
        match c {
            '\'' if literal.is_some() => {
                choices.push(Choice::Literal(literal.take().expect("a literal is open")));
                word.clear();
            }
            '\'' => literal = Some(String::new()),
            _ if literal.is_some() => literal.as_mut().expect("a literal is open").push(c),
            '|' | '}' => {
                let name = word.trim();
                if !name.is_empty() {
                    let token =
                        Token::parse(name).ok_or_else(|| TemplateError::UnknownToken(name.to_string()))?;
                    choices.push(Choice::Token(token));
                }
                word.clear();
                if c == '}' {
                    closed = true;
                    break;
                }
            }
            other => word.push(other),
        }
    }
    if literal.is_some() {
        return Err(TemplateError::UnclosedLiteral);
    }
    if !closed {
        return Err(TemplateError::UnclosedSlot);
    }
    match choices.is_empty() {
        true => Err(TemplateError::EmptyChoice),
        false => Ok(choices),
    }
}

/// Whether anything in `parts` had a slot to fill and filled it — which is what decides a group.
fn render_parts(parts: &[Part], fields: &Fields<'_>, out: &mut String) -> Filled {
    let mut filled = Filled::default();
    for part in parts {
        match part {
            Part::Text(text) => out.push_str(text),
            Part::Slot(choices) => {
                filled.seen = true;
                for choice in choices {
                    let resolved = match choice {
                        Choice::Token(token) => value(*token, fields),
                        Choice::Literal(text) => Some(text.as_str()),
                    };
                    if let Some(resolved) = resolved {
                        out.push_str(resolved);
                        filled.any = true;
                        break;
                    }
                }
            }
            Part::Group(inner) => {
                let mut buffer = String::new();
                let inner = render_parts(inner, fields, &mut buffer);
                // A group with no slots in it is prose, not a conditional, so it stays.
                if inner.any || !inner.seen {
                    out.push_str(&buffer);
                    filled.any |= inner.any;
                    filled.seen |= inner.seen;
                }
            }
        }
    }
    filled
}

#[derive(Debug, Clone, Copy, Default)]
struct Filled {
    seen: bool,
    any: bool,
}
