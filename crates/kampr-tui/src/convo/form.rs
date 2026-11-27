//! A form's fields, filled in the box one at a time and typed as each field says.

use kampr_client::PendingField;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    fields: Vec<PendingField>,
    at: usize,
    values: Map<String, Value>,
}

pub enum Filled {
    Next,
    Done(Value),
}

impl Form {
    pub fn new(fields: Vec<PendingField>) -> Self {
        Self {
            fields,
            at: 0,
            values: Map::new(),
        }
    }

    /// `Colour · 1 of 3 · red|teal`: which field the box is collecting, and what it takes.
    pub fn hint(&self) -> String {
        let field = &self.fields[self.at];
        let mut parts = vec![
            field.label().to_string(),
            format!("{} of {}", self.at + 1, self.fields.len()),
        ];
        if !field.choices.is_empty() {
            parts.push(field.choices.join("|"));
        } else if field.kind == "boolean" {
            parts.push("y|n".into());
        }
        parts.extend(field.detail.clone());
        if !field.required {
            parts.push("optional".into());
        }
        parts.join(" · ")
    }

    /// Takes the box's words for the field it is on, or says why they do not fit it.
    pub fn take(&mut self, text: &str) -> Result<Filled, String> {
        let field = &self.fields[self.at];
        if let Some(value) = value_of(field, text.trim())? {
            self.values.insert(field.name.clone(), value);
        }
        self.at += 1;
        match self.at == self.fields.len() {
            true => Ok(Filled::Done(Value::Object(std::mem::take(&mut self.values)))),
            false => Ok(Filled::Next),
        }
    }
}

fn value_of(field: &PendingField, text: &str) -> Result<Option<Value>, String> {
    let name = field.label();
    if text.is_empty() {
        return match field.required {
            true => Err(format!("{name} is required")),
            false => Ok(None),
        };
    }
    if !field.choices.is_empty() && !field.choices.iter().any(|c| c == text) {
        return Err(format!("{name} is one of {}", field.choices.join(", ")));
    }
    let value = match field.kind.as_str() {
        "integer" => json!(
            text.parse::<i64>()
                .map_err(|_| format!("{name} is a whole number"))?
        ),
        "number" => json!(text.parse::<f64>().map_err(|_| format!("{name} is a number"))?),
        "boolean" => match text.to_ascii_lowercase().as_str() {
            "y" | "yes" | "true" => json!(true),
            "n" | "no" | "false" => json!(false),
            _ => return Err(format!("{name} is yes or no")),
        },
        _ => json!(text),
    };
    Ok(Some(value))
}
