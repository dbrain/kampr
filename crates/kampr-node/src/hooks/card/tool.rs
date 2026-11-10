use super::{NO_WORDS, Pressed, capped, card, decision, not_an_answer, option, worded};
use crate::pending::Pending;
use serde_json::{Value, json};

const CONTEXT_CHARS: usize = 2000;
const PLAN_CHARS: usize = 20_000;
const DECLINED: &str = "Declined from Kampr";

#[derive(Debug)]
pub struct Tool {
    tool: String,
    question: String,
    context: Option<String>,
    suggestions: Option<Value>,
}

impl Tool {
    pub fn of(tool: &str, event: &Value) -> Self {
        let input = &event["tool_input"];
        let question = input["description"]
            .as_str()
            .filter(|d| !d.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Allow {tool}?"));
        let suggestions = event
            .get("permission_suggestions")
            .filter(|s| s.as_array().is_some_and(|a| !a.is_empty()))
            .cloned();
        Self {
            tool: tool.to_string(),
            question,
            context: context(input),
            suggestions,
        }
    }

    pub fn view(&self) -> Pending {
        let mut options = vec![option("1", "Yes", None, false)];
        if let Some(suggestions) = &self.suggestions {
            options.push(option("2", &always(suggestions), None, false));
        }
        options.push(option("3", "No", None, false));
        card(
            self.question.clone(),
            Some(self.tool.clone()),
            options,
            self.context.clone(),
        )
    }

    pub fn press(&self, key: &str, text: Option<&str>) -> Result<Pressed, String> {
        let decided = match (key, &self.suggestions) {
            ("1", _) => json!({ "behavior": "allow" }),
            ("2", Some(suggestions)) => json!({ "behavior": "allow", "updatedPermissions": suggestions }),
            ("3", _) => json!({ "behavior": "deny", "message": DECLINED }),
            _ => return Err(not_an_answer(key)),
        };
        if text.is_some() {
            return Err(NO_WORDS.into());
        }
        Ok(Pressed::Decided(decision(decided)))
    }
}

#[derive(Debug)]
pub struct Plan {
    input: Value,
}

impl Plan {
    pub fn of(input: &Value) -> Self {
        Self { input: input.clone() }
    }

    pub fn view(&self) -> Pending {
        card(
            "Approve Claude's plan?".into(),
            Some("Plan".into()),
            vec![
                option("1", "Yes, auto-accept edits", None, false),
                option("2", "Yes, manually approve edits", None, false),
                worded("3", "No, tell Claude what to change"),
            ],
            self.input["plan"]
                .as_str()
                .filter(|p| !p.trim().is_empty())
                .map(|p| capped(p, PLAN_CHARS)),
        )
    }

    // A bare allow leaves the plan dialog standing; only the request's own input approves it,
    // and the mode it names is the one plan mode is left for (#635).
    pub fn press(&self, key: &str, text: Option<&str>) -> Result<Pressed, String> {
        let approved = |mode: &str| {
            json!({
                "behavior": "allow",
                "updatedInput": self.input,
                "updatedPermissions": [ { "type": "setMode", "mode": mode, "destination": "session" } ],
            })
        };
        let decided = match (key, text) {
            ("1", None) => approved("acceptEdits"),
            ("2", None) => approved("default"),
            ("1" | "2", Some(_)) => return Err(NO_WORDS.into()),
            ("3", said) => json!({ "behavior": "deny", "message": said.unwrap_or(DECLINED) }),
            _ => return Err(not_an_answer(key)),
        };
        Ok(Pressed::Decided(decision(decided)))
    }
}

fn context(input: &Value) -> Option<String> {
    let named = ["command", "file_path", "notebook_path", "url", "query"]
        .iter()
        .find_map(|field| input[*field].as_str());
    let text = match named {
        Some(text) => text.to_string(),
        None if input.as_object().is_none_or(|o| o.is_empty()) => return None,
        None => input.to_string(),
    };
    Some(capped(&text, CONTEXT_CHARS))
}

fn always(suggestions: &Value) -> String {
    let named: Vec<String> = suggestions
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| match s["type"].as_str()? {
            "addRules" | "replaceRules" => {
                let rules: Vec<String> = s["rules"]
                    .as_array()?
                    .iter()
                    .filter_map(|r| {
                        let tool = r["toolName"].as_str()?;
                        Some(match r["ruleContent"].as_str() {
                            Some(content) => format!("{tool}({content})"),
                            None => tool.to_string(),
                        })
                    })
                    .collect();
                let verb = match s["behavior"].as_str() {
                    Some("deny") => "deny",
                    _ => "allow",
                };
                Some(format!("{verb} {}", rules.join(", ")))
            }
            "setMode" => Some(format!("switch to {}", s["mode"].as_str()?)),
            "addDirectories" => {
                let dirs: Vec<&str> = s["directories"]
                    .as_array()?
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                Some(format!("add {}", dirs.join(", ")))
            }
            other => Some(other.to_string()),
        })
        .collect();
    format!("Yes, and {}", named.join("; "))
}
