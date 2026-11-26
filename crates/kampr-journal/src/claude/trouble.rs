use super::record::{Content, ContentBlock, Record};
use crate::model::{Retry, Trouble};
use serde_json::Value;

/// The six words a terminal Claude's failures come in, from the kinds the SDK names and the
/// statuses the retries carry (#651, #652).
pub fn classify(kind: Option<&str>, status: Option<u16>, text: &str) -> &'static str {
    match kind {
        Some("authentication_failed") => {
            if text.contains("Not logged in") {
                "not-logged-in"
            } else {
                "login-expired"
            }
        }
        Some("overloaded") => "overloaded",
        Some("server_error") => "server-error",
        Some("rate_limit") => "rate-limit",
        _ => match status {
            Some(401) => "login-expired",
            Some(429) => "rate-limit",
            Some(529) => "overloaded",
            Some(status) if (500..600).contains(&status) => "server-error",
            _ => "api-error",
        },
    }
}

/// A `system` record of subtype `api_error`: one per retry, with the attempt and the delay (#652).
pub fn from_retry(record: &Record) -> Option<Trouble> {
    if record.kind != "system" || record.subtype.as_deref() != Some("api_error") {
        return None;
    }
    let error = record.error.as_ref()?;
    let status = error.get("status").and_then(Value::as_u64).map(|s| s as u16);
    let text = error
        .get("formatted")
        .or_else(|| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(Trouble {
        kind: classify(None, status, text).to_string(),
        text: text.to_string(),
        status,
        retry: record.retry_attempt.map(|attempt| Retry {
            attempt,
            of: record.max_retries.unwrap_or(0),
            in_ms: record.retry_in_ms.unwrap_or(0),
        }),
    })
}

/// The `assistant` record that carries the sentence the TUI prints (#652).
pub fn from_api_message(record: &Record) -> Option<Trouble> {
    if record.is_api_error_message != Some(true) {
        return None;
    }
    let kind = record.error.as_ref().and_then(Value::as_str);
    let text = record
        .message
        .as_ref()
        .and_then(|m| m.content.as_ref())
        .and_then(|content| match content {
            Content::Text(text) => Some(text.as_str()),
            Content::Blocks(blocks) => blocks.iter().find_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            }),
        })
        .unwrap_or_default();
    Some(Trouble {
        kind: classify(kind, record.api_error_status, text).to_string(),
        text: text.to_string(),
        status: record.api_error_status,
        retry: None,
    })
}

/// The `result` line of a refused turn: `is_error` with `terminal_reason: "api_error"`, where
/// `subtype` says `"success"` (#651).
pub fn from_result(record: &Record) -> Option<Trouble> {
    if record.kind != "result"
        || record.is_error != Some(true)
        || record.terminal_reason.as_deref() != Some("api_error")
    {
        return None;
    }
    let text = record.result.as_deref().unwrap_or_default();
    Some(Trouble {
        kind: classify(None, None, text).to_string(),
        text: text.to_string(),
        status: None,
        retry: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(line: serde_json::Value) -> Record {
        serde_json::from_value(line).unwrap()
    }

    #[test]
    fn the_kinds_follow_the_word_the_sdk_says() {
        assert_eq!(
            classify(
                Some("authentication_failed"),
                None,
                "Not logged in · Please run /login"
            ),
            "not-logged-in"
        );
        assert_eq!(
            classify(
                Some("authentication_failed"),
                Some(401),
                "Failed to authenticate."
            ),
            "login-expired"
        );
        assert_eq!(
            classify(Some("overloaded"), Some(529), "API Error: 529"),
            "overloaded"
        );
        assert_eq!(
            classify(Some("server_error"), Some(500), "API Error: 500"),
            "server-error"
        );
        assert_eq!(
            classify(Some("rate_limit"), Some(429), "API Error: 429"),
            "rate-limit"
        );
        assert_eq!(
            classify(Some("billing_error"), None, "payment is past due"),
            "api-error"
        );
    }

    #[test]
    fn the_kinds_follow_the_status_where_no_word_is() {
        assert_eq!(classify(None, Some(401), ""), "login-expired");
        assert_eq!(classify(None, Some(429), ""), "rate-limit");
        assert_eq!(classify(None, Some(529), ""), "overloaded");
        assert_eq!(classify(None, Some(500), ""), "server-error");
        assert_eq!(classify(None, Some(400), ""), "api-error");
        assert_eq!(classify(None, None, ""), "api-error");
    }

    #[test]
    fn a_retry_record_yields_its_attempt_and_delay() {
        let record = record(json!({
            "type": "system",
            "subtype": "api_error",
            "error": { "status": 529, "formatted": "API Error: 529 Overloaded" },
            "retryAttempt": 3,
            "maxRetries": 10,
            "retryInMs": 8192,
        }));
        assert_eq!(
            from_retry(&record),
            Some(Trouble {
                kind: "overloaded".into(),
                text: "API Error: 529 Overloaded".into(),
                status: Some(529),
                retry: Some(Retry {
                    attempt: 3,
                    of: 10,
                    in_ms: 8192
                }),
            })
        );
    }

    #[test]
    fn a_system_record_that_is_not_a_retry_yields_nothing() {
        let record = record(json!({ "type": "system", "subtype": "turn_duration" }));
        assert_eq!(from_retry(&record), None);
    }
}
