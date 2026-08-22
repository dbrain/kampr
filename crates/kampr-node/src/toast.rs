use kampr_herdr::Herdr;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// A toast on the operator's *desktop*, through herdr (probe #50).
///
/// One caller: a device redeeming a pairing code, announced on the same screen the code was
/// printed on. That is the only channel that reaches somebody who is not holding the phone, and a
/// pairing nobody expected is the one worth noticing.
///
/// Two rules make it useful rather than noise. It is **always attributed** — the desk sees what
/// raised it, because an unlabelled toast on an operator's screen is a phishing surface. And it is
/// **rate limited**, because anything that can put arbitrary text on someone's desktop as fast as
/// it likes is a denial of service against the person, not the machine.
const MIN_INTERVAL: Duration = Duration::from_secs(5);

const MAX_TITLE: usize = 60;
const MAX_BODY: usize = 200;

#[derive(Debug, Default)]
pub struct Toaster {
    last: Mutex<Option<Instant>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Toast {
    /// Herdr showed it. `shown` is its own word.
    Shown,
    /// Herdr took it and had nowhere to put it — a headless session has no attached client
    /// (probe #77), which is exactly what the plugin and the systemd unit both produce. Reported
    /// rather than dressed up as success: a toast nobody saw is worth knowing about.
    NoDesk(String),
    TooSoon,
    Refused(String),
}

impl Toaster {
    /// `who` names what raised it, and it is prefixed by the node rather than taken from a client.
    pub async fn show(&self, herdr: &Herdr, who: &str, title: &str, body: Option<&str>) -> Toast {
        {
            let mut last = self.last.lock().await;
            if last.is_some_and(|at| at.elapsed() < MIN_INTERVAL) {
                return Toast::TooSoon;
            }
            *last = Some(Instant::now());
        }
        let params = json!({
            "title": format!("kampr · {}", clip(who, MAX_TITLE)),
            "body": clip(&title_body(title, body), MAX_BODY),
        });
        match herdr.call::<Value>("notification.show", params).await {
            Ok(reply) => match reply["shown"].as_bool() {
                Some(true) => Toast::Shown,
                _ => Toast::NoDesk(
                    reply["reason"]
                        .as_str()
                        .unwrap_or("herdr did not show it")
                        .to_string(),
                ),
            },
            Err(e) => Toast::Refused(e.to_string()),
        }
    }
}

fn title_body(title: &str, body: Option<&str>) -> String {
    match body.map(str::trim).filter(|b| !b.is_empty()) {
        Some(body) => format!("{} — {}", title.trim(), body),
        None => title.trim().to_string(),
    }
}

/// Newlines and control characters are stripped, not escaped: this text is rendered by a TUI, and
/// a client that can emit an escape sequence into the operator's own chrome can repaint it.
fn clip(text: &str, max: usize) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match clean.chars().count() > max {
        true => clean.chars().take(max).collect(),
        false => clean,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A toast the desk cannot attribute is a phishing surface, and pane output — like a client —
    /// is attacker-influenceable.
    #[test]
    fn the_device_name_is_prefixed_and_control_characters_never_reach_the_tui() {
        assert_eq!(clip("phone", MAX_TITLE), "phone");
        assert_eq!(clip("a\u{1b}[2Jb\nc", MAX_BODY), "a [2Jb c");
        assert_eq!(clip(&"x".repeat(300), MAX_BODY).chars().count(), MAX_BODY);
    }

    #[test]
    fn a_body_is_optional_and_an_empty_one_is_not_a_dangling_dash() {
        assert_eq!(title_body("kampr paired", None), "kampr paired");
        assert_eq!(title_body("kampr paired", Some("  ")), "kampr paired");
        assert_eq!(
            title_body("kampr paired", Some("from the phone")),
            "kampr paired — from the phone"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_second_toast_inside_the_window_is_refused_without_reaching_herdr() {
        let toaster = Toaster::default();
        let herdr = Herdr::new("/nowhere/herdr.sock");
        // The first attempt reaches a socket that is not there, which is still an attempt.
        assert!(matches!(
            toaster.show(&herdr, "phone", "one", None).await,
            Toast::Refused(_)
        ));
        assert_eq!(toaster.show(&herdr, "phone", "two", None).await, Toast::TooSoon);
        tokio::time::advance(MIN_INTERVAL + Duration::from_millis(1)).await;
        assert!(matches!(
            toaster.show(&herdr, "phone", "three", None).await,
            Toast::Refused(_)
        ));
    }
}
