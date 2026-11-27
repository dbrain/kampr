//! A program to resume after a reboot (docs/25), as `native.resumable` lists it.

use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Record {
    pub id: String,
    pub label: Option<String>,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
}

impl Record {
    pub fn program(&self) -> &str {
        self.argv
            .first()
            .and_then(|p| p.rsplit('/').next())
            .filter(|p| !p.is_empty())
            .unwrap_or("command")
    }

    /// The command as a shell would read it back, each argument quoted where it needs it, so what
    /// is shown is what runs (docs/25: a record is escaped for display).
    pub fn command(&self) -> String {
        self.argv.iter().map(|a| quoted(a)).collect::<Vec<_>>().join(" ")
    }

    pub fn line(&self) -> String {
        let mut parts = vec![self.program().to_string()];
        parts.extend(self.label.clone());
        parts.extend(
            self.cwd
                .as_deref()
                .map(|c| kampr_core::naming::home_relative(c).into_owned()),
        );
        parts.push(self.command());
        parts.join(" · ")
    }
}

fn quoted(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./=:@%+,-".contains(c));
    match plain {
        true => arg.to_string(),
        false => {
            let escaped: String = arg
                .chars()
                .map(|c| match c {
                    '\'' => "'\\''".to_string(),
                    c if c.is_control() => c.escape_default().to_string(),
                    c => c.to_string(),
                })
                .collect();
            format!("'{escaped}'")
        }
    }
}

pub fn records(list: Option<&serde_json::Value>) -> Vec<Record> {
    list.cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}
