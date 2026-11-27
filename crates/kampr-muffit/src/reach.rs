use serde_json::Value;

/// What a muffit daemon does when this node calls it. A refusal is told from an absence because
/// only an absence is a supported state: a daemon that answers and will not take the token is a
/// node pointed at the wrong daemon, and lists nothing for ever (docs/18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    Answers,
    Refused(Refusal),
    Away(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub why: String,
}

impl Reach {
    pub fn from_status(status: u16, body: &Value) -> Self {
        match status {
            200..=299 => Self::Answers,
            401 | 403 => Self::Refused(Refusal {
                status,
                why: crate::client::error_of(body),
            }),
            _ => Self::Away(format!("muffit answered {status}")),
        }
    }

    /// A failed websocket upgrade carries the daemon's own response.
    pub fn from_error(error: &anyhow::Error) -> Self {
        use tokio_tungstenite::tungstenite::Error;
        match error.downcast_ref::<Error>() {
            Some(Error::Http(response)) => {
                let body: Value = response
                    .body()
                    .as_deref()
                    .and_then(|bytes| serde_json::from_slice(bytes).ok())
                    .unwrap_or(Value::Null);
                Self::from_status(response.status().as_u16(), &body)
            }
            _ => Self::Away(format!("{error:#}")),
        }
    }
}

/// Tells of a refusal once per change of state, so a daemon refusing every retry is one line in
/// the log and not one per poll.
#[derive(Default)]
pub struct Refusals {
    last: Option<u16>,
}

impl Refusals {
    pub fn note<'a>(&mut self, reach: &'a Reach) -> Option<&'a Refusal> {
        let status = match reach {
            Reach::Refused(refusal) => Some(refusal.status),
            _ => None,
        };
        let changed = status != self.last;
        self.last = status;
        match reach {
            Reach::Refused(refusal) if changed => Some(refusal),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(status: u16) -> Reach {
        Reach::Refused(Refusal {
            status,
            why: "wrong bearer token".into(),
        })
    }

    #[test]
    fn a_refusal_is_told_when_it_starts_or_changes_and_not_while_it_goes_on() {
        let away = Reach::Away("connection refused".into());
        let steps = [
            (refused(401), true),
            (refused(401), false),
            (refused(401), false),
            (refused(403), true),
            (Reach::Answers, false),
            (refused(403), true),
            (away.clone(), false),
            (refused(403), true),
        ];
        let mut refusals = Refusals::default();
        for (reach, told) in steps {
            assert_eq!(refusals.note(&reach).is_some(), told, "{reach:?}");
        }
    }

    #[test]
    fn only_a_401_or_a_403_is_a_refusal() {
        let body = serde_json::json!({ "error": "wrong bearer token" });
        for (status, want) in [
            (200, Reach::Answers),
            (401, refused(401)),
            (403, refused(403)),
            (404, Reach::Away("muffit answered 404".into())),
            (502, Reach::Away("muffit answered 502".into())),
        ] {
            assert_eq!(Reach::from_status(status, &body), want, "{status}");
        }
    }
}
