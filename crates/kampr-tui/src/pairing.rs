use crossterm::event::{KeyCode, KeyEvent};
use kampr_client::{ClientConfig, HubOffer, Session, Via};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// The code prompt for pairing with this machine's hub, and the pair request it sends.
///
/// The code comes from the hub's console (`kampr pair` waits for Enter there) or from any device
/// already signed in to it — the app's "Pair a device", or `prefix A` here — so the only thing this
/// side has to collect is the code itself.
pub struct Pairing {
    hub: HubOffer,
    code: Option<String>,
    sending: bool,
    done: UnboundedSender<Result<Session, String>>,
    results: UnboundedReceiver<Result<Session, String>>,
}

pub enum Paired {
    Waiting,
    Failed(String),
    Ready(Session),
}

impl Pairing {
    pub fn new(hub: HubOffer) -> Self {
        let (done, results) = unbounded_channel();
        Self {
            hub,
            code: None,
            sending: false,
            done,
            results,
        }
    }

    pub fn hub(&self) -> &HubOffer {
        &self.hub
    }

    pub fn open(&mut self) {
        if !self.sending {
            self.code = Some(String::new());
        }
    }

    /// Every key while the prompt is up is the prompt's.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        let Some(code) = self.code.as_mut() else {
            return false;
        };
        if self.sending {
            if key.code == KeyCode::Esc {
                self.code = None;
            }
            return true;
        }
        match key.code {
            KeyCode::Esc => self.code = None,
            KeyCode::Backspace => {
                code.pop();
            }
            KeyCode::Char(c) if !c.is_whitespace() => code.push(c),
            KeyCode::Enter if !code.is_empty() => {
                let code = code.clone();
                self.sending = true;
                self.send(code);
            }
            _ => {}
        }
        true
    }

    fn send(&self, code: String) {
        let hub = self.hub.clone();
        let done = self.done.clone();
        tokio::spawn(async move {
            let _ = done.send(pair(&hub, &code).await);
        });
    }

    pub fn poll(&mut self) -> Paired {
        match self.results.try_recv() {
            Ok(Ok(session)) => {
                self.sending = false;
                self.code = None;
                Paired::Ready(session)
            }
            Ok(Err(why)) => {
                self.sending = false;
                if let Some(code) = self.code.as_mut() {
                    code.clear();
                }
                Paired::Failed(why)
            }
            Err(_) => Paired::Waiting,
        }
    }

    pub fn render(&self, buf: &mut Buffer, area: Rect, t: &crate::theme::Theme) {
        let Some(code) = &self.code else { return };
        if area.height == 0 || area.width == 0 {
            return;
        }
        let row = Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        };
        let hub = &self.hub.name;
        let tail = match self.sending {
            true => "  pairing…".to_string(),
            false => "▏  ↵ pair · esc".to_string(),
        };
        Clear.render(row, buf);
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" pair with {hub} "),
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" code from `kampr pair` on {hub}, or any signed-in kampr: "),
                Style::default().fg(t.dim).bg(t.accent_soft),
            ),
            Span::styled(
                code.clone(),
                Style::default()
                    .fg(t.accent_hi)
                    .bg(t.accent_soft)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(tail, Style::default().fg(t.mute).bg(t.accent_soft)),
        ]))
        .style(Style::default().bg(t.accent_soft))
        .render(row, buf);
    }
}

async fn pair(hub: &HubOffer, code: &str) -> Result<Session, String> {
    let paired = kampr_client::pair(&hub.config_dir, &hub.origin, code, Some(&hub.name), None)
        .await
        .map_err(|e| e.to_string())?;
    let config = ClientConfig::load(&hub.config_dir).map_err(|e| e.to_string())?;
    let profile = config
        .profiles
        .get(&paired.name)
        .ok_or_else(|| format!("paired, but no profile named {} was saved", paired.name))?;
    Ok(Session {
        origin: profile.origin.clone(),
        token: profile.token.clone(),
        via: Via::Profile { name: paired.name },
    })
}
