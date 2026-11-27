use super::App;
use crate::convo::{Held, Sheet, SheetDo, SheetKind, SheetTook};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kampr_client::{ConvoUpdate, Event};
use kampr_core::conversation::InputAbility;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::sync::atomic::{AtomicU64, Ordering};

static MINTED: AtomicU64 = AtomicU64::new(0);

fn mint() -> String {
    let n = MINTED.fetch_add(1, Ordering::Relaxed);
    format!("tui-{}-{n}", super::unix_now())
}

impl App {
    fn held(&self, pane: &str) -> Held {
        self.declared.get(pane).cloned().unwrap_or_default()
    }

    pub(super) fn absorb_declared(&mut self, event: &Event) {
        self.declared.absorb(event);
        if let Event::ConvoUpdate {
            update: ConvoUpdate::Acted { action, ok, text },
            ..
        } = event
        {
            let said = text.clone().unwrap_or_else(|| match ok {
                true => format!("{action} · done"),
                false => format!("{action} · refused"),
            });
            if let Some(sheet) = self.sheet.as_mut() {
                sheet.acted(said.clone());
            }
            self.note(said);
        }
    }

    /// The keys the declared vocabulary answers in a conversation view, each only where the source
    /// declared what it drives (docs/27 § Keys).
    pub(super) fn declared_key(&mut self, pane: &str, key: KeyEvent) -> bool {
        let held = self.held(pane);
        let writes = self.writes();
        if key.modifiers == KeyModifiers::ALT
            && let KeyCode::Char(c) = key.code
        {
            match c {
                'k' if writes && held.stops() => {
                    self.client.convo_steer(pane, "interrupt", None);
                    self.note("stop sent");
                    return true;
                }
                'n' if writes && held.running() && held.can(InputAbility::SendNow) => {
                    let text = self.composer.text(pane).trim().to_string();
                    if !text.is_empty() {
                        self.composer.replace(pane, "");
                        self.say(pane, &text, Some("now"));
                    }
                    return true;
                }
                'u' if writes && held.can(InputAbility::Unqueue) => {
                    match self.declared.take_back(pane) {
                        Some((id, text)) => {
                            self.client.unqueue(pane, &id);
                            self.composer.take(pane, &text);
                        }
                        None => self.note("nothing of yours is queued"),
                    }
                    return true;
                }
                's' if held.has_session() => {
                    self.sheet = Some(Sheet::new(pane, SheetKind::Session));
                    return true;
                }
                't' if !held.tasks.is_empty() => {
                    self.sheet = Some(Sheet::new(pane, SheetKind::Tasks));
                    return true;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::Tab
            && key.modifiers.is_empty()
            && let Some(done) = held.complete(self.composer.text(pane))
        {
            self.composer.replace(pane, &done);
            return true;
        }
        false
    }

    /// A reply to a source that works a queue: one `input` with its own id, queued behind a running
    /// turn where the source takes that. `false` leaves the reply to the terminal's path.
    pub(super) fn say_queued(&mut self, pane: &str, text: &str) -> bool {
        let held = self.held(pane);
        if !held.queues() {
            return false;
        }
        let when = (held.running() && held.can(InputAbility::QueueNext)).then_some("next");
        self.say(pane, text, when);
        true
    }

    fn say(&mut self, pane: &str, text: &str, when: Option<&str>) {
        let id = mint();
        if !self.client.say(pane, text, &id, when) {
            self.note("not delivered — the socket is down");
            return;
        }
        self.declared.sent(pane, id, text.to_string());
        self.note(match when {
            Some("next") => "queued",
            Some(_) => "sent now",
            None => "sent",
        });
    }

    pub(super) fn suggest(&mut self, pane: &str) {
        if !self.writes() {
            return;
        }
        if let Some((kind, query)) = self.held(pane).asking(self.composer.text(pane)) {
            self.client.convo_suggest(pane, kind, &query);
        }
    }

    pub(super) fn sheet_key(&mut self, key: KeyEvent) {
        let Some(sheet) = self.sheet.as_mut() else { return };
        let pane = sheet.pane.clone();
        let held = self.declared.get(&pane).cloned().unwrap_or_default();
        match sheet.key(key, &held) {
            SheetTook::Stay => {}
            SheetTook::Close => self.sheet = None,
            SheetTook::Do(act) => self.sheet_do(&pane, act),
        }
    }

    fn sheet_do(&mut self, pane: &str, act: SheetDo) {
        if !self.writes() && !matches!(act, SheetDo::Follow(_) | SheetDo::Sub { .. }) {
            self.note("this device is read-only");
            return;
        }
        let sent = match act {
            SheetDo::Set { control, value } => self.client.convo_set(pane, &control, value),
            SheetDo::Act { action, args } => self.client.convo_act(pane, &action, args),
            SheetDo::Steer { steer, text } => self.client.convo_steer(pane, &steer, text.as_deref()),
            SheetDo::Follow(id) => self.client.convo_follow(pane, &id, 0),
            SheetDo::Sub { id, head } => {
                self.sheet = None;
                self.convo
                    .open_launched(pane, &crate::convo::Launch { id: id.clone(), head });
                self.teardown();
                self.client.convo_sub(pane, &id, None)
            }
        };
        if !sent {
            self.note("not delivered — the socket is down");
        }
    }

    pub(super) fn draw_sheet(&self, area: Rect, buf: &mut Buffer) {
        let Some(sheet) = self.sheet.as_ref() else { return };
        let held = self.held(&sheet.pane);
        sheet.render(&held, area, buf, &self.options.theme);
    }
}
