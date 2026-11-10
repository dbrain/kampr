use super::card;
use crate::pending::Pending;
use serde_json::Value;

pub const AT_THE_TERMINAL: &str = "an MCP server's form is answered at the terminal";

#[derive(Debug)]
pub struct Elicit {
    server: String,
    message: String,
    fields: Option<String>,
}

impl Elicit {
    pub fn of(event: &Value) -> Self {
        Self {
            server: event["mcp_server_name"]
                .as_str()
                .unwrap_or("MCP server")
                .to_string(),
            message: event["message"].as_str().unwrap_or_default().to_string(),
            fields: fields(&event["requested_schema"]),
        }
    }

    pub fn view(&self) -> Pending {
        card(
            self.message.clone(),
            Some(format!("{} · MCP", self.server)),
            Vec::new(),
            self.fields.clone(),
        )
    }
}

fn fields(schema: &Value) -> Option<String> {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let lines: Vec<String> = schema["properties"]
        .as_object()?
        .iter()
        .map(|(name, field)| {
            let mut parts = vec![match field["title"].as_str().filter(|t| *t != name) {
                Some(title) => format!("{title} ({name})"),
                None => name.clone(),
            }];
            parts.extend(field["type"].as_str().map(str::to_string));
            if required.contains(&name.as_str()) {
                parts.push("required".into());
            }
            parts.join(" · ")
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}
