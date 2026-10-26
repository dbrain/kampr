use crossterm::event::{KeyCode, KeyEvent};
use kampr_client::{Minted, Session};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// A code for another device, minted by the herd this client is signed in to — the app's "Pair a
/// device", from a keyboard. Asked for over an authenticated request, so it works as handed back.
pub struct Invite {
    origin: String,
    state: State,
}

enum State {
    Minting(oneshot::Receiver<Result<Minted, String>>),
    Ready { code: String, until: Instant },
    Failed(String),
}

pub enum Done {
    Closed,
    Copy(String),
}

impl Invite {
    pub fn mint(session: &Session) -> Self {
        let (done, minting) = oneshot::channel();
        let origin = session.origin.trim_end_matches('/').to_string();
        let session = session.clone();
        tokio::spawn(async move {
            let _ = done.send(kampr_client::mint(&session).await.map_err(|e| e.to_string()));
        });
        Self {
            origin,
            state: State::Minting(minting),
        }
    }

    /// `y` copies the code; anything else puts it away.
    pub fn key(&mut self, key: KeyEvent) -> Done {
        match (&self.state, key.code) {
            (State::Ready { code, .. }, KeyCode::Char('y')) => Done::Copy(code.clone()),
            _ => Done::Closed,
        }
    }

    fn settle(&mut self) {
        if let State::Minting(minting) = &mut self.state {
            self.state = match minting.try_recv() {
                Ok(Ok(minted)) => State::Ready {
                    code: minted.code,
                    until: Instant::now() + Duration::from_secs(minted.expires_in),
                },
                Ok(Err(why)) => State::Failed(why),
                Err(oneshot::error::TryRecvError::Closed) => State::Failed("the request went nowhere".into()),
                Err(oneshot::error::TryRecvError::Empty) => return,
            };
        }
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, t: &crate::theme::Theme) {
        self.settle();
        let text = Style::default().fg(t.text).bg(t.surface);
        let mute = Style::default().fg(t.mute).bg(t.surface);
        let lines = match &self.state {
            State::Minting(_) => vec![Line::from(Span::styled(" asking for a code…", mute))],
            State::Failed(why) => vec![Line::from(Span::styled(format!(" {why}"), text))],
            State::Ready { code, until } => {
                let left = until.saturating_duration_since(Instant::now()).as_secs();
                vec![
                    Line::from(vec![
                        Span::styled(" code  ", mute),
                        Span::styled(
                            code.clone(),
                            Style::default()
                                .fg(t.accent_hi)
                                .bg(t.surface)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("   good for {}:{:02}", left / 60, left % 60), mute),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(" on the other machine:", mute)),
                    Line::from(Span::styled(
                        format!("   kampr connect {} --code {code}", self.origin),
                        text,
                    )),
                    Line::from(Span::styled(
                        " or in the app or a browser, type it where it asks for a code",
                        mute,
                    )),
                ]
            }
        };
        let width = area.width.saturating_sub(4).min(76);
        let height = (lines.len() as u16 + 2).min(area.height);
        if width < 24 || height < 3 {
            return;
        }
        let popup = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        };
        Clear.render(popup, buf);
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent))
                    .title(Span::styled(
                        " pair a device ",
                        Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                    ))
                    .title_bottom(Span::styled(" y copy · esc ", Style::default().fg(t.mute))),
            )
            .style(Style::default().bg(t.surface))
            .render(popup, buf);
    }
}
