//! What a bind does. The keymap is herdr's, the router is [`crate::input`], and this is the far
//! end of both: one arm per [`Action`], plus the surfaces a key reaches before the pane does.

use super::{App, Screen, View, navigable, open_url};
use crate::input::Outcome;
use crate::keymap::{Action, Dir, Mode};
use crate::manage::Progress;
use crate::mouse::Link;
use crate::render::fit;
use crate::sidebar::{self, Row};
use crossterm::event::KeyEvent;
use kampr_client::{Event, ManageError, Managed};
use kampr_term::Cell;

impl App {
    pub fn key(&mut self, key: KeyEvent) {
        self.settle_manage();
        // **The panel keeps the keyboard while it is open.** Any key used to dismiss it, so it
        // could not be paged through even once it had more than a screen in it.
        if self.keybinds {
            self.help_key(key);
            return;
        }
        // The modal has the keyboard while it is open, and it has it **first**: a digit typed
        // into a manage prompt, on a pane in conversation view with an outstanding question,
        // would otherwise be eaten as the answer to that question.
        if self.manage.active() {
            match self.manage.key(key) {
                Progress::Idle => {}
                Progress::Consumed | Progress::Cancelled => return,
                Progress::Send(op) => {
                    self.dispatch(op);
                    return;
                }
            }
        }
        if self.router.mode() == Mode::Pane
            && !crate::keymap::same(key, self.router.prefix())
            && self.conversation_key(key)
        {
            return;
        }
        let before = self.router.mode();
        match self.router.key(key) {
            Outcome::Nothing => {}
            Outcome::Redrew => {}
            Outcome::Do(action) => self.act(action),
            Outcome::ToPane(text) => self.send(&text),
        }
        if before != Mode::Navigate && self.router.mode() == Mode::Navigate {
            self.show_what_is_being_navigated();
        }
    }

    /// **The navigator has to have something to navigate.** It used to force the herd screen for
    /// as long as it was open, and that screen is always drawn; walking the sidebar instead means
    /// it can be opened against one that is hidden — `prefix b` — or one the terminal is too
    /// narrow to draw at all, and then the cursor moves and the arrows are swallowed with nothing
    /// on screen to show for it.
    ///
    /// The last frame's rects are what say whether the sidebar was drawable, because the width
    /// test belongs to the draw and the keyboard does not know the terminal's size. **A frame that
    /// has not happened yet is not a frame that drew no sidebar** — `status` is zero-sized only
    /// before the first draw, and reading the two the same way sent every fresh client to the herd.
    fn show_what_is_being_navigated(&mut self) {
        self.sidebar_open = true;
        let drawn = self.layout.status.height > 0;
        if drawn && self.layout.sidebar.width == 0 {
            self.open(Screen::Herd);
        }
    }

    fn help_key(&mut self, key: KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Up => self.scroll_help(-1),
            KeyCode::Down => self.scroll_help(1),
            KeyCode::PageUp => self.scroll_help(-10),
            KeyCode::PageDown => self.scroll_help(10),
            KeyCode::Home => self.scroll_help(i32::MIN / 2),
            KeyCode::End => self.scroll_help(i32::MAX / 2),
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') => {
                self.keybinds = false;
                self.help_top = 0;
            }
            _ => {}
        }
    }

    /// The conversation surface gets the keyboard before the pane does, and the pending strip
    /// gets it before the conversation: answering a blocked agent on another host from the
    /// sidebar is the one thing this client can do that a herdr at the desk cannot.
    fn conversation_key(&mut self, key: KeyEvent) -> bool {
        let Some(pane) = self.focus.clone() else {
            return false;
        };
        if self.view(&pane) != View::Conversation {
            return false;
        }
        if let crossterm::event::KeyCode::Char(c) = key.code
            && let Some(offered) = self.convo.answer(&pane, c)
        {
            // Only the key that was offered. The node decides whether a submit key follows, per
            // harness, and an Enter is never synthesised (#43).
            self.client.answer(&pane, &offered);
            return true;
        }
        // **Consumed is not the same as unhandled.** An arrow key the conversation took would
        // otherwise fall through to the pane and land in the agent's PTY.
        if self.convo.key(&pane, key) {
            self.teardown();
            return true;
        }
        if key.code == crossterm::event::KeyCode::PageUp
            && let Some(before) = self.convo.load_more(&pane)
        {
            self.client.convo_load(&pane, Some(&before));
            return true;
        }
        false
    }

    pub fn paste(&mut self, data: &str) {
        self.send(&crate::input::bracketed(data));
    }

    fn send(&mut self, text: &str) {
        let Some(pane) = self.focus.clone() else { return };
        if !self.writes() {
            self.note("this device is read-only");
            return;
        }
        if !self.client.input(&pane, text) {
            self.note("not delivered — the socket is down");
        }
    }

    /// The ack is carried back rather than dropped. A **successful** op produces no frame the
    /// manage surface can otherwise see, so its in-flight notice would age out instead of
    /// resolving — and `layout.export`'s tree would never arrive.
    /// Sends one op — or, for a fleet run, the one op per host it expands into.
    ///
    /// **The expansion happens here rather than in the panel** because it needs the herd as it is
    /// at the moment of sending, and because one instruction reaching several machines is worth
    /// having in exactly one place.
    fn dispatch(&mut self, op: serde_json::Value) {
        if op["op"] == "fleet.run" && op["node"].is_null() {
            let command = op["command"].as_str().unwrap_or_default().to_string();
            let ops = {
                let state = self.client.state();
                kampr_client::fleet::fan_out(&command, &state.herd)
            };
            match ops {
                Ok(ops) => {
                    self.open(Screen::Fleet);
                    for one in ops {
                        self.send_manage(one);
                    }
                }
                Err(e) => self.manage.refused("fleet.run", &e.to_string()),
            }
            return;
        }
        self.send_manage(op);
    }

    fn send_manage(&mut self, op: serde_json::Value) {
        let client = self.client.clone();
        let acked = self.acked.clone();
        let name = op["op"].as_str().unwrap_or_default().to_string();
        tokio::spawn(async move {
            let ack = match client.manage(op).await {
                Ok(ack) => ack,
                Err(ManageError::Refused { op, code, message }) => Managed {
                    op,
                    ok: false,
                    code: Some(code),
                    message: Some(message),
                    ..Managed::default()
                },
                Err(e) => Managed {
                    op: name,
                    ok: false,
                    message: Some(e.to_string()),
                    ..Managed::default()
                },
            };
            let _ = acked.send(ack);
        });
    }

    /// #241 sanctions this: a session ack is a promise the host already agrees, and the kinds and
    /// sessions a menu draws come from `caps` alone — so an op that changed them asks again.
    pub(super) fn settle_manage(&mut self) {
        while let Ok(ack) = self.acks.try_recv() {
            let refresh = ack.ok && ack.op.starts_with("session.");
            self.manage.observe(&Event::Managed(ack));
            if refresh {
                self.client.request_caps();
            }
        }
    }

    fn act(&mut self, action: Action) {
        use Action::*;
        match action {
            Detach => self.quit = true,
            Keybinds => self.keybinds = true,
            ToggleSidebar => self.sidebar_open = !self.sidebar_open,
            HerdView => self.open(match self.screen {
                Screen::Herd => Screen::Panes,
                Screen::Panes | Screen::Fleet => Screen::Herd,
            }),
            FleetView => self.open(match self.screen {
                Screen::Fleet => Screen::Panes,
                Screen::Panes | Screen::Herd => Screen::Fleet,
            }),
            // Zoom narrows the mosaic to one pane and widens it back to the tab's, so it moves
            // the subscription the same way a focus does.
            ZoomPane => {
                self.zoomed = !self.zoomed;
                self.sync_watches();
            }
            LastPane => {
                if let Some(last) = self.last.clone() {
                    self.focus(last);
                }
            }
            CyclePaneNext => self.cycle(1),
            CyclePanePrevious => self.cycle(-1),
            FocusPane(Dir::Left) | FocusPane(Dir::Up) => self.cycle(-1),
            FocusPane(_) => self.cycle(1),
            NextTab => self.tab(1),
            PreviousTab => self.tab(-1),
            SwitchTab(n) => self.switch_tab(n),
            Scroll(dir) => self.scroll(dir),
            // Vertical movement scrolls the surface rather than panning a second axis: history
            // and the live grid are one window, not two panels.
            Pan(Dir::Up) => self.scroll(Dir::Up),
            Pan(Dir::Down) => self.scroll(Dir::Down),
            Pan(dir) => self.pan(dir, 4),
            PanEdge(dir) => self.pan(dir, u16::MAX),
            PanReset => {
                if let Some(pane) = self.focus.clone() {
                    self.pans.insert(pane.clone(), fit::Pan::default());
                    self.scrolls.insert(pane, 0);
                }
            }
            // The navigator's `Move` walks the sidebar; copy mode's walks the surface. Without
            // a selection cursor — W5's — moving the view is the honest half of herdr's h/j/k/l.
            Move(dir) => match self.router.mode() {
                Mode::Navigate => self.move_pick(dir),
                _ => match dir {
                    Dir::Up | Dir::Down => self.scroll(dir),
                    Dir::Left | Dir::Right => self.pan(dir, 1),
                },
            },
            OpenWorkspace => self.open_pick(),
            PinPane => self.pin(),
            ClearMosaic => {
                self.pinned.clear();
                self.sync_watches();
                self.note("the mosaic is this tab's panes again");
            }
            SwitchWorkspace(n) => {
                self.pick = n as usize;
                self.open_pick();
            }
            Wider => self.resplit(5),
            Narrower => self.resplit(-5),
            Taller | Shorter => self.note("the mosaic is one row — there is no height to give"),
            ToggleView => self.toggle_view(),
            ToggleMouse => self.toggle_mouse(),
            Copy => self.copy(),
            Select => self.note("drag over the grid — there is no keyboard cursor yet"),
            // The one gesture that navigates a URL a pane's output offered, and it is the
            // operator's rather than the pane's.
            OpenNotificationTarget => match self.offered.take() {
                Some(url) if open_url(&url) => self.note(format!("opened {url}")),
                Some(url) => self.note(format!("nothing here could open {url}")),
                None => self.note("no link has been offered"),
            },
            ReloadConfig | Settings | EditScrollback => self.note("not in this build"),
            other => self.begin_manage(other),
        }
    }

    fn begin_manage(&mut self, action: Action) {
        let pane = self.focus.clone();
        let state = self.client.state();
        let caps = state.caps();
        let role = state.role;
        let opened = self
            .manage
            .begin(action, &state.herd, pane.as_deref(), &caps, role);
        drop(state);
        match opened {
            Some(prompt) => {
                if let Some(op) = prompt.op {
                    self.dispatch(op);
                }
            }
            None if !role.writes() => self.note("this device is read-only"),
            None if !caps.manage => self.note("this node does not claim manage"),
            None => self.note("not in this build"),
        }
    }

    fn toggle_view(&mut self) {
        let Some(pane) = self.focus.clone() else { return };
        let next = match self.view(&pane) {
            View::Terminal => View::Conversation,
            View::Conversation => View::Terminal,
        };
        self.views.insert(pane.clone(), next);
        self.teardown();
        let name = match next {
            View::Terminal => "terminal",
            View::Conversation => "conversation",
        };
        // A merge, so storing the view does not forget anything else this pane's blob holds.
        self.client
            .write_prefs(&pane, serde_json::json!({ "view": name }));
    }

    fn toggle_mouse(&mut self) {
        let Some(pane) = self.focus.clone() else { return };
        let on = !self.mouse.passes_through(&pane);
        self.mouse.set_passthrough(&pane, on);
        self.client.write_prefs(&pane, serde_json::json!({ "mouse": on }));
        self.note(match on {
            true => "this pane is passing the mouse through",
            false => "the mouse stays with kampr",
        });
    }

    /// **The selection, not the grid.** A whole-screen copy is not what `prefix [ y` means at a
    /// desk, and it is not what the operator dragged over.
    fn copy(&mut self) {
        let Some(pane) = self.focus.clone() else { return };
        let text = {
            let state = self.client.state();
            let Some(held) = state.pane(&pane) else { return };
            let ring = self.rings.get(&pane).map(Vec::as_slice).unwrap_or_default();
            let surface: Vec<&[Cell]> = ring
                .iter()
                .map(Vec::as_slice)
                .chain(held.rows().iter().map(Vec::as_slice))
                .collect();
            let (cols, _) = held.geometry();
            self.mouse.selected_text(&surface, cols)
        };
        let Some(text) = text else {
            self.note("nothing is selected — drag over the grid first");
            return;
        };
        crate::osc52(&text);
        self.note(format!("copied {} characters", text.chars().count()));
    }

    /// What the pointer left behind on the pane that was just drawn: the text of a finished drag,
    /// and the link under the cell that was clicked.
    ///
    /// A **declared** OSC 8 URI opens; a **detected** bare URL is offered and never followed,
    /// because pane output is attacker-influenceable. A declared one is narrowed to the two
    /// schemes a terminal client has any business handing to a desktop opener, for the same
    /// reason: a harness declares the URI, but so can anything else writing to that PTY.
    pub(super) fn pointed(&mut self, copied: Option<String>, link: Option<Link>) {
        if let Some(text) = copied {
            crate::osc52(&text);
            self.note(format!("copied {} characters", text.chars().count()));
        }
        // **Declared is offered, not followed.** A harness declares an OSC 8 URI and so does
        // anything else writing to that PTY, so a pane can wrap its whole visible region in one
        // link and turn every click — including the one that only meant to focus it — into a
        // navigation it chose. `prefix o` is the single gesture that navigates, and it is the
        // operator's.
        match link {
            Some(Link::Declared(url)) | Some(Link::Detected(url)) => {
                match navigable(&url) {
                    true => self.note(format!("{url} — prefix o opens it")),
                    false => self.note(format!("{url} — not a web link; prefix o will not open it")),
                }
                self.offered = Some(url);
            }
            None => {}
        }
    }

    /// **Kampr's own split, never the pane's** (ADR 0002). herdr's resize mode moves a PTY;
    /// this moves the boundary between two `observe` streams the client is arranging.
    fn resplit(&mut self, by: i32) {
        self.split = (self.split as i32 + by).clamp(20, 80) as u16;
        match self.mosaic().len() {
            2 => self.note(format!("split {}/{}", self.split, 100 - self.split)),
            _ => self.note("there is one pane on screen — nothing to split"),
        }
    }

    fn cycle(&mut self, by: i32) {
        let mosaic = self.mosaic();
        if mosaic.len() < 2 {
            return;
        }
        let at = mosaic
            .iter()
            .position(|p| Some(p.as_str()) == self.focus.as_deref())
            .unwrap_or(0);
        let next = (at as i32 + by).rem_euclid(mosaic.len() as i32) as usize;
        self.focus(mosaic[next].clone());
    }

    pub(super) fn tabs(&self) -> Vec<(String, String)> {
        let Some(focus) = self.focus.clone() else {
            return Vec::new();
        };
        let state = self.client.state();
        let Some(entry) = state.herd.pane(&focus) else {
            return Vec::new();
        };
        let mut seen: Vec<(String, String)> = Vec::new();
        for pane in state
            .herd
            .panes
            .iter()
            .filter(|p| p.workspace_id == entry.workspace_id)
        {
            let Some(id) = pane.tab_id.clone() else { continue };
            if seen.iter().any(|(t, _)| *t == id) {
                continue;
            }
            let name = pane
                .tab
                .clone()
                .or_else(|| pane.agent.clone())
                .unwrap_or_else(|| id.rsplit(':').next().unwrap_or("tab").to_string());
            seen.push((id, name));
        }
        seen
    }

    fn tab(&mut self, by: i32) {
        let tabs = self.tabs();
        if tabs.is_empty() {
            return;
        }
        let here = self
            .focus
            .as_ref()
            .and_then(|f| self.client.state().herd.pane(f).and_then(|p| p.tab_id.clone()));
        let at = here
            .and_then(|id| tabs.iter().position(|(t, _)| *t == id))
            .unwrap_or(0);
        let next = (at as i32 + by).rem_euclid(tabs.len() as i32) as usize;
        self.open_tab(&tabs[next].0);
    }

    fn switch_tab(&mut self, n: u8) {
        let tabs = self.tabs();
        if let Some((id, _)) = tabs.get(n as usize - 1) {
            let id = id.clone();
            self.open_tab(&id);
        }
    }

    pub(super) fn open_tab(&mut self, tab: &str) {
        let pick = self
            .client
            .state()
            .herd
            .panes
            .iter()
            .find(|p| p.tab_id.as_deref() == Some(tab))
            .map(|p| p.id.clone());
        if let Some(pick) = pick {
            self.focus(pick);
        }
    }

    fn pan(&mut self, dir: Dir, by: u16) {
        let Some(pane) = self.focus.clone() else { return };
        let pan = self.pans.entry(pane).or_default();
        match dir {
            Dir::Left => pan.col = pan.col.saturating_sub(by),
            Dir::Right => pan.col = pan.col.saturating_add(by),
            Dir::Up => pan.row = pan.row.saturating_sub(by),
            Dir::Down => pan.row = pan.row.saturating_add(by),
        }
    }

    /// A `scrollback` message landed: decode this pane's ring once, here, so the draw path only
    /// ever reads it.
    pub fn absorb_ring(&mut self, pane: &str) {
        let state = self.client.state();
        let Some(held) = state.pane(pane) else { return };
        let rows: Vec<Vec<kampr_term::Cell>> =
            held.history().doc().rows.into_iter().map(|r| r.cells).collect();
        drop(state);
        self.rings.insert(pane.to_string(), rows);
    }

    fn scroll(&mut self, dir: Dir) {
        let Some(pane) = self.focus.clone() else { return };
        let by = self
            .layout
            .pane(&pane)
            .map(|placed| placed.rect.height.saturating_sub(2).max(1))
            .unwrap_or(10);
        let at = self.scrolls.entry(pane).or_default();
        *at = match dir {
            Dir::Up | Dir::Left => at.saturating_add(by),
            Dir::Down | Dir::Right => at.saturating_sub(by),
        };
    }

    pub(super) fn rows(&self) -> Vec<Row> {
        let state = self.client.state();
        let mut rows = sidebar::spaces(&state.herd);
        rows.push(Row::Blank);
        rows.extend(sidebar::agents(&state.herd, |pane| {
            self.convo.pending(pane).and_then(|p| p.question.clone())
        }));
        rows
    }

    fn move_pick(&mut self, dir: Dir) {
        let rows = self.rows();
        let step = |from: usize, by: i32| -> usize {
            let mut at = from as i32;
            for _ in 0..rows.len() {
                at = (at + by).rem_euclid(rows.len() as i32);
                if rows[at as usize].pane().is_some() {
                    break;
                }
            }
            at as usize
        };
        self.pick = match dir {
            Dir::Up | Dir::Left => step(self.pick, -1),
            Dir::Down | Dir::Right => step(self.pick, 1),
        };
    }

    /// Panes from two hosts side by side is what a herdr at one desk structurally cannot do: it
    /// attaches to exactly one server (ADR 0002).
    fn pin(&mut self) {
        let rows = self.rows();
        let Some(pane) = rows.get(self.pick).and_then(|r| r.pane()).map(str::to_string) else {
            return;
        };
        if self.pinned.is_empty() {
            self.pinned = self.mosaic();
        }
        if !self.pinned.contains(&pane) {
            self.pinned.push(pane.clone());
        }
        self.zoomed = false;
        self.focus(pane);
        self.note(format!("{} panes side by side", self.pinned.len()));
    }

    fn open_pick(&mut self) {
        let rows = self.rows();
        if let Some(pane) = rows.get(self.pick).and_then(|r| r.pane()).map(str::to_string) {
            self.focus(pane);
            self.router.leave();
            self.screen = Screen::Panes;
        }
    }
}
