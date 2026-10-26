use kampr_client::Herd;
use kampr_core::provider::AgentStatus;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// herdr's own default debounce (#615), for a desk where nothing says otherwise.
const DELAY: Duration = Duration::from_secs(1);

/// herdr's `[ui.toast] delivery`, with its meanings: `terminal` asks the outer terminal for a
/// desktop notification, `system` asks the OS directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Delivery {
    Off,
    Terminal,
    System,
}

/// kampr's own `[toast]` in `tui.toml`, for a desk with no herdr on it — native sessions need
/// none — and for one that wants kampr to differ from herdr.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Setting {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_seconds: Option<u64>,
}

/// `KAMPR_TUI_TOAST`, then kampr's own setting, then herdr's if herdr is on this desk, then a
/// terminal notification after herdr's default debounce.
pub fn settle(
    env: Option<Delivery>,
    mine: Option<Setting>,
    herdr: Option<(Delivery, Duration)>,
) -> (Delivery, Duration) {
    let mine = mine.unwrap_or_default();
    let delivery = env
        .or(mine.delivery)
        .or(herdr.map(|(delivery, _)| delivery))
        .unwrap_or(Delivery::Terminal);
    let delay = mine
        .delay_seconds
        .map(Duration::from_secs)
        .or(herdr.map(|(_, delay)| delay))
        .unwrap_or(DELAY);
    (delivery, delay)
}

pub fn env() -> Option<Delivery> {
    std::env::var("KAMPR_TUI_TOAST").ok().as_deref().and_then(parse)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub pane: String,
    pub title: String,
    pub body: String,
}

/// An agent off screen that started needing the operator or finished, said once it has stayed
/// that way for herdr's debounce — the same toast herdr raises for a background workspace, for
/// every node in the herd.
pub struct Alerts {
    pub delivery: Delivery,
    pub delay: Duration,
    seen: Option<HashMap<String, AgentStatus>>,
    waiting: HashMap<String, (AgentStatus, Instant, Toast)>,
}

impl Alerts {
    pub fn new(delivery: Delivery, delay: Duration) -> Self {
        Self {
            delivery,
            delay,
            seen: None,
            waiting: HashMap::new(),
        }
    }

    /// Before this desk's own setting is read: the environment and herdr's.
    pub fn from_env() -> Self {
        let (delivery, delay) = settle(env(), None, herdr_toast());
        Self::new(delivery, delay)
    }

    pub fn adopt(&mut self, mine: Setting) {
        (self.delivery, self.delay) = settle(env(), Some(mine), herdr_toast());
    }

    /// The first herd is the state of things, not news: what was already blocked at launch is on
    /// the sidebar, and a toast for each of them would be a burst nobody asked for.
    pub fn herd(&mut self, herd: &Herd, on_screen: &[String]) {
        let now: HashMap<String, AgentStatus> = herd
            .panes
            .iter()
            .filter(|p| p.agent.is_some())
            .map(|p| (p.id.clone(), p.agent_status))
            .collect();
        if let Some(seen) = &self.seen {
            for pane in herd.panes.iter().filter(|p| p.agent.is_some()) {
                let was = seen.get(&pane.id).copied();
                let status = pane.agent_status;
                if was == Some(status) || on_screen.contains(&pane.id) {
                    continue;
                }
                let agent = pane.agent.as_deref().unwrap_or("agent");
                let title = match status {
                    AgentStatus::Blocked => format!("{agent} needs attention"),
                    AgentStatus::Done => format!("{agent} finished"),
                    _ => {
                        self.waiting.remove(&pane.id);
                        continue;
                    }
                };
                let node = herd
                    .node(&pane.node_id)
                    .map(|n| n.name.as_str())
                    .unwrap_or_default();
                let body = [
                    crate::sidebar::row_name(pane, false),
                    pane.workspace.clone().unwrap_or_default(),
                    node.to_string(),
                ]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
                self.waiting.insert(
                    pane.id.clone(),
                    (
                        status,
                        Instant::now() + self.delay,
                        Toast {
                            pane: pane.id.clone(),
                            title,
                            body,
                        },
                    ),
                );
            }
        }
        self.waiting
            .retain(|pane, (status, _, _)| now.get(pane) == Some(status) && !on_screen.contains(pane));
        self.seen = Some(now);
    }

    /// The toasts whose debounce has run out, each said once.
    pub fn due(&mut self, on_screen: &[String]) -> Vec<Toast> {
        if self.delivery == Delivery::Off {
            self.waiting.clear();
            return Vec::new();
        }
        let now = Instant::now();
        self.waiting.retain(|pane, _| !on_screen.contains(pane));
        let due: Vec<String> = self
            .waiting
            .iter()
            .filter(|(_, (_, at, _))| *at <= now)
            .map(|(pane, _)| pane.clone())
            .collect();
        due.into_iter()
            .filter_map(|pane| self.waiting.remove(&pane).map(|(_, _, toast)| toast))
            .collect()
    }

    pub fn raise(&self, toast: &Toast) {
        match self.delivery {
            Delivery::Off => {}
            Delivery::Terminal => {
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = out.write_all(osc99(toast).as_bytes());
                let _ = out.flush();
            }
            Delivery::System => {
                let (program, args) = match cfg!(target_os = "macos") {
                    true => (
                        "osascript",
                        vec![
                            "-e".to_string(),
                            format!(
                                "display notification {:?} with title {:?}",
                                toast.body, toast.title
                            ),
                        ],
                    ),
                    false => (
                        "notify-send",
                        vec![
                            "--app-name".into(),
                            "kampr".into(),
                            toast.title.clone(),
                            toast.body.clone(),
                        ],
                    ),
                };
                let _ = std::process::Command::new(program)
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            }
        }
    }
}

/// What herdr writes for `delivery = "terminal"` (#615): kitty's desktop notification protocol,
/// the title chunk then the body, under one id.
pub fn osc99(toast: &Toast) -> String {
    let clean = |text: &str| text.replace(['\u{1b}', '\u{7}'], "");
    format!(
        "\u{1b}]99;i=1:d=0;{}\u{1b}\\\u{1b}]99;i=1:p=body;{}\u{1b}\\",
        clean(&toast.title),
        clean(&toast.body)
    )
}

fn parse(named: &str) -> Option<Delivery> {
    match named {
        "off" | "herdr" => Some(Delivery::Off),
        "terminal" => Some(Delivery::Terminal),
        "system" => Some(Delivery::System),
        _ => None,
    }
}

fn herdr_config() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HERDR_CONFIG_PATH") {
        return Some(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("herdr").join("config.toml"))
}

fn herdr_toast() -> Option<(Delivery, Duration)> {
    let text = std::fs::read_to_string(herdr_config()?).ok()?;
    toast_in(&text)
}

/// herdr's in-app toast has no counterpart in a terminal kampr is drawing, so `herdr` is off here.
pub fn toast_in(config: &str) -> Option<(Delivery, Duration)> {
    let config: toml::Table = toml::from_str(config).ok()?;
    let toast = config.get("ui")?.get("toast")?;
    let delivery = toast
        .get("delivery")
        .and_then(|d| d.as_str())
        .and_then(parse)
        .unwrap_or(Delivery::Off);
    let delay = toast
        .get("delay_seconds")
        .and_then(|d| d.as_integer())
        .map_or(DELAY, |s| Duration::from_secs(s.max(0) as u64));
    Some((delivery, delay))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_beats_this_desk_which_beats_herdr_which_beats_the_default() {
        let herdr = Some((Delivery::System, Duration::from_secs(3)));
        let mine = Some(Setting {
            delivery: Some(Delivery::Off),
            delay_seconds: None,
        });
        let cases = [
            (None, None, None, (Delivery::Terminal, DELAY)),
            (None, None, herdr, (Delivery::System, Duration::from_secs(3))),
            (None, mine, herdr, (Delivery::Off, Duration::from_secs(3))),
            (None, mine, None, (Delivery::Off, DELAY)),
            (
                Some(Delivery::Terminal),
                mine,
                herdr,
                (Delivery::Terminal, Duration::from_secs(3)),
            ),
        ];
        for (env, mine, herdr, want) in cases {
            assert_eq!(settle(env, mine, herdr), want, "{env:?} {mine:?} {herdr:?}");
        }
    }

    #[test]
    fn herdrs_own_toast_settings_are_read_and_its_in_app_toast_is_off_here() {
        let mine = "[ui.toast]\ndelivery = \"terminal\"\ndelay_seconds = 3\n";
        assert_eq!(toast_in(mine), Some((Delivery::Terminal, Duration::from_secs(3))));
        assert_eq!(
            toast_in("[ui.toast]\ndelivery = \"herdr\"\n"),
            Some((Delivery::Off, DELAY))
        );
        assert_eq!(toast_in("onboarding = false\n"), None);
    }
}
