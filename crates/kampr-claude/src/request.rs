use serde_json::{Value, json};

pub struct Image {
    pub mime: String,
    pub data: String,
}

/// A `user` message: the prompt, with optional images. This is a word sent, and it is what the
/// billing guards stand in front of. `uuid` is the word's id — the handle a
/// `cancel_async_message` names — and `priority` is the binary's queue priority: `now` preempts
/// the turn in flight, `next` waits behind it.
pub fn user_message(text: &str, images: &[Image], uuid: Option<&str>, priority: Option<&str>) -> String {
    let content = if images.is_empty() {
        Value::String(text.to_string())
    } else {
        let mut blocks = vec![json!({ "type": "text", "text": text })];
        for image in images {
            blocks.push(json!({
                "type": "image",
                "source": { "type": "base64", "media_type": image.mime, "data": image.data },
            }));
        }
        Value::Array(blocks)
    };
    let mut line = json!({
        "type": "user",
        "message": { "role": "user", "content": content },
        "parent_tool_use_id": null,
        "session_id": "",
    });
    if let Some(uuid) = uuid {
        line["uuid"] = Value::String(uuid.to_string());
    }
    if let Some(priority) = priority {
        line["priority"] = Value::String(priority.to_string());
    }
    line.to_string()
}

/// A control request, our side: `initialize`, `get_usage`, `set_model`, ... The reply is matched
/// to the caller's `request_id`.
pub fn control_request(request_id: &str, subtype: &str, body: Value) -> String {
    let mut request = body;
    if let Some(obj) = request.as_object_mut() {
        obj.insert("subtype".to_string(), Value::String(subtype.to_string()));
    }
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": request,
    })
    .to_string()
}

fn answer(request_id: &str, body: Value) -> String {
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request_id, "response": body },
    })
    .to_string()
}

/// The `can_use_tool` allow. The binary's schema takes `updatedInput` as an object or not at all,
/// so a request that carried no input is answered with an empty one (#645).
pub fn permission_allow(request_id: &str, updated_input: Value, updated_permissions: Vec<Value>) -> String {
    let input = if updated_input.is_object() {
        updated_input
    } else {
        json!({})
    };
    let mut body = json!({ "behavior": "allow", "updatedInput": input });
    if !updated_permissions.is_empty() {
        body["updatedPermissions"] = Value::Array(updated_permissions);
    }
    answer(request_id, body)
}

/// The `can_use_tool` deny: the schema requires the message, and the model reads it (#624).
pub fn permission_deny(request_id: &str, message: &str) -> String {
    answer(request_id, json!({ "behavior": "deny", "message": message }))
}

/// The answer to an MCP `elicitation`: `accept`, `decline` or `cancel`, with the form's values
/// for an accept (#684).
pub fn elicitation_answer(request_id: &str, action: &str, content: Value) -> String {
    let mut body = json!({ "action": action });
    if action == "accept" {
        body["content"] = content;
    }
    answer(request_id, body)
}

/// A request of the binary's that this source has no answer for. Left unanswered it would hold
/// the turn open.
pub fn control_error(request_id: &str, error: &str) -> String {
    json!({
        "type": "control_response",
        "response": { "subtype": "error", "request_id": request_id, "error": error },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strict;

    fn parsed(line: &str) -> Value {
        serde_json::from_str(line).expect("a JSON line")
    }

    #[test]
    fn a_user_message_carries_its_uuid_priority_and_images() {
        let plain = parsed(&user_message("hi", &[], Some("m1"), Some("next")));
        assert_eq!(plain["message"]["content"], "hi");
        assert_eq!(
            (plain["uuid"].as_str(), plain["priority"].as_str()),
            (Some("m1"), Some("next"))
        );

        let image = Image {
            mime: "image/png".into(),
            data: "AAAA".into(),
        };
        let with = parsed(&user_message("see", &[image], None, None));
        assert_eq!(with["message"]["content"][1]["source"]["media_type"], "image/png");
        assert!(with.get("priority").is_none());
    }

    #[test]
    fn a_control_request_carries_its_subtype_and_body() {
        let v = parsed(&control_request("r1", "set_model", json!({ "model": "sonnet" })));
        assert_eq!(v["request_id"], "r1");
        assert_eq!(v["request"]["subtype"], "set_model");
        assert_eq!(v["request"]["model"], "sonnet");
    }

    // The binary's schema: allow takes `updatedInput` as a record or not at all, so a null is
    // rejected and every Allow has to echo an object (#645).
    #[test]
    fn an_allow_always_carries_an_object_input() {
        for input in [Value::Null, json!("text"), json!({"command": "ls"})] {
            let v = parsed(&permission_allow("r1", input, Vec::new()));
            strict::permission_answer(&v).expect("a body the binary accepts");
            assert!(v["response"]["response"]["updatedInput"].is_object());
        }
        let echoed = parsed(&permission_allow("r1", json!({"command": "ls"}), Vec::new()));
        assert_eq!(echoed["response"]["response"]["updatedInput"]["command"], "ls");
    }

    #[test]
    fn an_always_allow_applies_the_suggestions_as_updated_permissions() {
        let suggestion = json!({"type": "setMode", "mode": "acceptEdits", "destination": "session"});
        let v = parsed(&permission_allow("r1", json!({}), vec![suggestion.clone()]));
        strict::permission_answer(&v).unwrap();
        assert_eq!(v["response"]["response"]["updatedPermissions"][0], suggestion);
    }

    #[test]
    fn a_deny_always_carries_a_message() {
        let v = parsed(&permission_deny("r1", "not now"));
        strict::permission_answer(&v).unwrap();
        assert_eq!(v["response"]["response"]["message"], "not now");
    }

    #[test]
    fn an_elicitation_answer_is_an_action_with_content_only_on_accept() {
        let accepted = parsed(&elicitation_answer(
            "r1",
            "accept",
            json!({"answer": "forty-two"}),
        ));
        assert_eq!(accepted["response"]["response"]["action"], "accept");
        assert_eq!(accepted["response"]["response"]["content"]["answer"], "forty-two");
        let cancelled = parsed(&elicitation_answer("r1", "cancel", json!({"answer": "x"})));
        assert!(cancelled["response"]["response"].get("content").is_none());
    }
}
