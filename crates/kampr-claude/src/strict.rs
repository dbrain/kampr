//! The binary's own schema for what we answer it, written out so a test can fail on a line the
//! real binary would reject. Read off 2.1.326: a `can_use_tool` answer is `{behavior: "allow",
//! updatedInput?: record, updatedPermissions?: array}` or `{behavior: "deny", message: string}`.

use serde_json::Value;

pub fn permission_answer(line: &Value) -> Result<(), String> {
    let body = line
        .pointer("/response/response")
        .ok_or("a control_response carries its answer under response.response")?;
    match body.get("behavior").and_then(Value::as_str) {
        Some("allow") => {
            if body.get("updatedInput").is_some_and(|i| !i.is_object()) {
                return Err(format!(
                    "updatedInput must be an object, got {}",
                    body["updatedInput"]
                ));
            }
            if body.get("updatedPermissions").is_some_and(|p| !p.is_array()) {
                return Err("updatedPermissions must be an array".into());
            }
            Ok(())
        }
        Some("deny") if body.get("message").is_some_and(Value::is_string) => Ok(()),
        Some("deny") => Err("a deny carries a message".into()),
        other => Err(format!("no such behavior: {other:?}")),
    }
}
