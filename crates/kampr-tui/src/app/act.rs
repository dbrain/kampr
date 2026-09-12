//! What a bind does. The keymap is herdr's, the router is [`crate::input`], and this is the far
//! end of both: one arm per [`Action`], plus the surfaces a key reaches before the pane does.

use super::{Acked, App, Claim, Screen, View, navigable, open_url};
use crate::input::Outcome;
use crate::keymap::{Action, Dir, Mode};
use crate::manage::{MATCH_MIN_COLS, MATCH_MIN_ROWS, Progress};
use crate::mouse::Link;
use crate::render::fit;
use crate::sidebar::{self, Row};
use crossterm::event::KeyEvent;
use kampr_client::{Event, ManageError, Managed};
use kampr_term::Cell;
use serde_json::json;
use std::time::Duration;

/// How long the window has to hold still before its size is asked for.
const MATCH_SETTLE: Duration = Duration::from_millis(250);

/// How many times a refused claim is asked again before the window stops asking about that pane at
/// that size. A refusal is something else holding the pane (#21) or a link that dropped under the
/// op, which is seconds; asking for ever is a hot loop, and asking once gives up on a controller
/// that was a moment from letting go.
const MATCH_TRIES: u8 = 3;

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
        // The find prompt takes the keyboard the same way the modal above it does, and for the
        // same reason: every character of a query is a character some other surface would claim.
        match self.find.key(key) {
            crate::find::Took::Ignored => {}
            crate::find::Took::Consumed => return,
            crate::find::Took::Search { query, backward } => {
                self.search_scrollback(&query, backward);
                return;
            }
        }
        // The transcript's search prompt, for the same reason the one above it takes the keyboard:
        // every character of a query is a character the reply box would otherwise claim.
        match self.search.key(key) {
            crate::convo::Searched::Ignored => {}
            crate::convo::Searched::Consumed => return,
            crate::convo::Searched::Search(query) => {
                self.search_transcript(&query);
                return;
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
        // Leaving the results puts the line down with them: a standing count over a transcript
        // nothing is stepping any more is a surface that looks live and answers no key.
        if before == Mode::Results && self.router.mode() != Mode::Results {
            self.search.close();
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

    /// The conversation surface gets the keyboard before the pane does, and this is the order it
    /// hands it on in: a pending prompt, then the transcript's own scrolling, then the reply box.
    ///
    /// **Nothing here falls through to the PTY.** It used to: a key the conversation did not claim
    /// went straight to the pane, blind, with no box on screen to say where it had gone — which is
    /// what made "there is no clear way to enter a message" the right complaint about a surface
    /// that was in fact typing every character straight into the agent.
    fn conversation_key(&mut self, key: KeyEvent) -> bool {
        let Some(pane) = self.focus.clone() else {
            return false;
        };
        if self.view(&pane) != View::Conversation {
            return false;
        }
        // **A prompt's keys are only the prompt's while the box is empty.** Once there is a draft,
        // `1` is a character somebody is writing and not an answer to a question behind it.
        if self.composer.empty(&pane)
            && let crossterm::event::KeyCode::Char(c) = key.code
            && let Some(offered) = self.convo.answer(&pane, c)
        {
            // Only the key that was offered. The node decides whether a submit key follows, per
            // harness, and an Enter is never synthesised (#43).
            self.client.answer(&pane, &offered);
            return true;
        }
        // **Back out of a launched conversation before the box gets the key.** A draft the
        // composer would have cleared is kept: the reader is leaving a surface, not abandoning
        // what they were writing to the pane underneath it.
        if key.code == crossterm::event::KeyCode::Esc && self.convo.leave_launched(&pane) {
            return true;
        }
        // The transcript's own scrolling, which is a different surface from the pane's ring and
        // from the box below it. Left and right are the box's, so they are not offered here.
        if matches!(
            key.code,
            crossterm::event::KeyCode::Up
                | crossterm::event::KeyCode::Down
                | crossterm::event::KeyCode::PageUp
                | crossterm::event::KeyCode::PageDown
        ) {
            if key.code == crossterm::event::KeyCode::PageUp
                && let Some(more) = self.convo.load_more(&pane)
            {
                match more {
                    crate::convo::More::Pane(before) => {
                        self.client.convo_load(&pane, Some(&before));
                    }
                    crate::convo::More::Launched { id, before } => {
                        self.client.convo_sub(&pane, &id, Some(&before));
                    }
                }
                return true;
            }
            if self.convo.key(&pane, key) {
                self.teardown();
                return true;
            }
        }
        match self.composer.key(&pane, key) {
            crate::convo::Typed::Ignored => false,
            crate::convo::Typed::Changed => true,
            crate::convo::Typed::Send(text) => {
                self.reply(&pane, &text);
                true
            }
        }
    }

    /// The text, then the carriage return, as **two** messages — so a harness that debounces sees
    /// the words settle before the newline that submits them. `pane.send_text` writes raw bytes
    /// with no framing (#9), which is why a multi-line reply is bracketed on its way out.
    fn reply(&mut self, pane: &str, text: &str) {
        if !self.writes() {
            self.note("this device is read-only");
            return;
        }
        let body = match text.contains('\n') {
            true => crate::input::bracketed(text),
            false => text.to_string(),
        };
        if !self.client.input(pane, &body) || !self.client.input(pane, "\r") {
            self.note("not delivered — the socket is down");
            return;
        }
        self.note("sent");
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
        // **The off switch, and it is a real op rather than a switch with no wire behind it.** The
        // size menu's release is what an operator presses to stop a pane being matched, so this is
        // where the answer is remembered — otherwise the next `fit` would claim it straight back.
        if op["op"] == "pane.size"
            && let Some(at) = op["at"].as_str()
        {
            match op["mode"].as_str() {
                Some("release") => {
                    self.unmatched.insert(at.to_string());
                    if self.matching.as_ref().is_some_and(|(p, _, _)| p == at) {
                        self.matching = None;
                    }
                    if self.asking.as_ref().is_some_and(|(p, _, _)| p == at) {
                        self.asking = None;
                    }
                    self.declined = None;
                    self.settling = None;
                }
                Some("match") => {
                    self.unmatched.remove(at);
                }
                _ => {}
            }
        }
        self.send_manage(op);
    }

    /// The standing intent to hold the focused pane at this window's own size, and the one place
    /// in this client that asks for a resize nobody typed.
    ///
    /// It is ADR 0012's op under a setting, not a second path: the same `pane.size`, the same
    /// 80x24 floor, and a hold the node ties to this websocket so that a client which dies takes
    /// the hold with it. See [ADR 0013](../../../../docs/adr/0013-a-standing-intent-to-match-the-view.md).
    ///
    /// **A fleet pane is skipped and needs to be.** It is a pty this node forked for a job of its
    /// own, sized when the run started, with no operator desk to trample — rule 3's other half.
    pub fn match_view(&mut self, pane: Option<&str>, cols: u16, rows: u16) {
        self.matching_step(pane, cols, rows);
        // What the size menu offers, and whether it offers "start" or "stop".
        self.manage.observing(crate::manage::Matched {
            cols,
            rows,
            on: self.matching.is_some(),
        });
    }

    fn matching_step(&mut self, pane: Option<&str>, cols: u16, rows: u16) {
        let want = pane.filter(|pane| {
            self.options.match_view
                && !self.unmatched.contains(*pane)
                && cols >= MATCH_MIN_COLS
                && rows >= MATCH_MIN_ROWS
                && self
                    .client
                    .state()
                    .herd
                    .pane(pane)
                    .is_some_and(|entry| entry.fleet.is_none())
        });
        let target = want.map(|pane| (pane.to_string(), cols, rows));
        if self
            .declined
            .as_ref()
            .is_some_and(|(claim, _)| Some(claim) != target.as_ref())
        {
            self.declined = None;
        }
        // What this window is holding or has asked to hold, which are the same answer to "is this
        // target already spoken for" and never both at once.
        let standing = self.matching.as_ref().or(self.asking.as_ref()).cloned();
        if standing != target
            && let Some((held, _, _)) = self.matching.take().or_else(|| self.asking.take())
        {
            self.send_manage(json!({ "op": "pane.size", "at": held, "mode": "release" }));
        }
        let Some(target) = target else {
            self.settling = None;
            return;
        };
        // An ask already out, or one the node has refused often enough to stop asking about, is
        // not asked again by the next redraw.
        if standing.as_ref() == Some(&target)
            || self
                .declined
                .as_ref()
                .is_some_and(|(claim, tries)| *claim == target && *tries >= MATCH_TRIES)
        {
            return;
        }
        // The window has to hold still first. A drag arrives as a run of `Resize` events and every
        // one of them would otherwise claim the PTY again.
        let now = std::time::Instant::now();
        match &self.settling {
            Some((seen, since)) if *seen == target => {
                if now.duration_since(*since) < MATCH_SETTLE {
                    return;
                }
            }
            _ => {
                self.settling = Some((target, now));
                return;
            }
        }
        let claim = self.settling.take().expect("a settled size").0;
        let (pane, cols, rows) = claim.clone();
        self.asking = Some(claim.clone());
        self.send_manage_claiming(
            json!({ "op": "pane.size", "at": pane, "cols": cols, "rows": rows, "mode": "match" }),
            Some(claim),
        );
    }

    fn send_manage(&mut self, op: serde_json::Value) {
        self.send_manage_claiming(op, None);
    }

    fn send_manage_claiming(&mut self, op: serde_json::Value, claim: Option<Claim>) {
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
            let _ = acked.send(Acked { ack, claim });
        });
    }

    /// #241 sanctions this: a session ack is a promise the host already agrees, and the kinds and
    /// sessions a menu draws come from `caps` alone — so an op that changed them asks again.
    pub(super) fn settle_manage(&mut self) {
        while let Ok(Acked { ack, claim }) = self.acks.try_recv() {
            let refresh = ack.ok && ack.op.starts_with("session.");
            // **And it stops there.** The manage panel reports ops the operator typed; the
            // standing hold is not one, and putting its ack on the panel's strip covered the
            // sentence that says the pane is being held with a notice about an op nobody asked
            // for, for eight seconds, on every window resize.
            if let Some(claim) = claim {
                self.claim_answered(claim, ack.ok);
                continue;
            }
            self.manage.observe(&Event::Managed(ack));
            if refresh {
                self.client.request_caps();
            }
        }
    }

    /// A pane is held when the node says it is. A refusal is a contended controller rather than a
    /// wrong size — herdr refuses the second one outright (#21) — so it is asked again a few times
    /// and then left to the next thing that moves the window or the pane.
    fn claim_answered(&mut self, claim: Claim, ok: bool) {
        if self.asking.as_ref() != Some(&claim) {
            return;
        }
        self.asking = None;
        if ok {
            self.declined = None;
            self.matching = Some(claim);
            return;
        }
        let tries = match &self.declined {
            Some((declined, tries)) if *declined == claim => tries + 1,
            _ => 1,
        };
        self.declined = Some((claim, tries));
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
            OpenLaunched => self.open_launched(),
            TakeDeskLine => self.take_desk_line(),
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
            SearchForward => self.open_find(false),
            SearchBack => self.open_find(true),
            RepeatSearch => self.step_search(true),
            RepeatSearchBack => self.step_search(false),
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
        // Armed, the borrowed row carries `mouse → pane` for as long as it is armed, and a note
        // saying the same thing for five seconds would only outrank it. Disarming has no standing
        // indicator, so it is the half that still needs saying.
        if !on {
            self.note("the mouse stays with kampr");
        }
    }

    /// **The selection, not the grid.** A whole-screen copy is not what `prefix [ y` means at a
    /// desk, and it is not what the operator dragged over.
    /// **One key, two surfaces.** On the grid it is the selection a drag left behind; on a
    /// transcript there is no selection to have — the mouse is captured and copy mode walks the
    /// pane's ring, not the conversation — so it is the code block the reader is looking at.
    fn copy(&mut self) {
        let Some(pane) = self.focus.clone() else { return };
        if self.view(&pane) == View::Conversation {
            match self.convo.code(&pane) {
                Some(text) => {
                    crate::osc52(&text);
                    self.note(format!("copied {} lines of code", text.lines().count()));
                }
                None => self.note("no code block above the fold to copy"),
            }
            return;
        }
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

    /// Takes what is half-typed at the pane's own keyboard, because `input` **appends** to it: a
    /// sentence begun at the desk and a reply sent from here submit as one run-on line, and the
    /// strip that says so is the only warning there has ever been.
    ///
    /// The words arrive in the box **before** the pane is emptied, so a dropped socket costs a
    /// clear that did not happen rather than a sentence that is nowhere.
    fn take_desk_line(&mut self) {
        let Some(pane) = self.focus.clone() else { return };
        let Some((line, clear)) = self
            .convo
            .desk_line(&pane)
            .map(|(l, c)| (l.to_string(), c.map(str::to_string)))
        else {
            self.note("nothing is half-typed at that pane's own keyboard");
            return;
        };
        let Some(clear) = clear else {
            self.note("nobody has measured how to empty this harness's composer — left alone");
            return;
        };
        if !self.writes() {
            self.note("this device is read-only");
            return;
        }
        self.composer.take(&pane, &line);
        match self.client.input(&pane, &clear) {
            true => self.note("taken — the pane's own line is yours now"),
            false => self.note("not delivered — the socket is down"),
        }
    }

    /// The conversation the pane's agent launched, opened for reading.
    ///
    /// **Newest first, then back through the older ones**, because a turn that launched three
    /// subagents at once gives a reader no other way to name which one they want, and the newest
    /// is the one the question is nearly always about. The node follows one at a time, so opening
    /// another is what replaces it.
    fn open_launched(&mut self) {
        let Some(pane) = self.focus.clone() else { return };
        if self.view(&pane) != View::Conversation {
            self.note("prefix shift+v for the conversation first");
            return;
        }
        let launches = self.convo.launches(&pane);
        if launches.is_empty() {
            self.note("this agent has not launched a conversation");
            return;
        }
        let at = match self.convo.reading(&pane).and_then(|open| {
            launches
                .iter()
                .position(|launch| launch.id == open)
                .map(|at| at.checked_sub(1).unwrap_or(launches.len() - 1))
        }) {
            Some(at) => at,
            None => launches.len() - 1,
        };
        let launch = launches[at].clone();
        self.convo.open_launched(&pane, &launch);
        self.client.convo_sub(&pane, &launch.id, None);
        match launches.len() {
            1 => self.note(format!("{} · esc to come back", launch.head)),
            total => self.note(format!(
                "{} · {} of {total} launched · esc to come back",
                launch.head,
                total - at
            )),
        }
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
                // A navigable one is carried by `offered` on the borrowed row until it is
                // replaced; only the refusal needs a note of its own.
                if !navigable(&url) {
                    self.note(format!("{url} — not a web link; prefix o will not open it"));
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

    /// Hidden rather than disabled when the node has no verb for it, which is the rule every other
    /// affordance here follows. A prompt that took a query and then waited for ever would be worse
    /// than saying so, and a client newer than the node it dialled is ordinary.
    /// **Two histories, one key.** A pane's scrollback is what was drawn on it; its transcript is
    /// what the agent recorded, which outlives any number of `clear`s — so the surface on screen
    /// decides which of them `/` is about. Copy mode walks the grid whatever view is under it, so
    /// a search opened from inside it is the grid's.
    fn open_find(&mut self, backward: bool) {
        let conversation = self
            .focus
            .clone()
            .is_some_and(|pane| self.view(&pane) == View::Conversation);
        if conversation && self.router.mode() != Mode::Copy {
            self.search.open();
            return;
        }
        if !self.client.state().caps().find {
            self.note("this node has no search — it is older than this client");
            return;
        }
        self.find.open(backward);
    }

    /// The whole transcript where the node has the verb, and the turns held here where it has
    /// not. **The count says which**, because one that covers the page the reader opened on must
    /// never read as though it covered the conversation.
    fn search_transcript(&mut self, query: &str) {
        let Some(pane) = self.focus.clone() else {
            self.note("no pane to search");
            return;
        };
        self.router.enter(Mode::Results);
        if self.client.state().caps().convo_find && self.client.convo_find(&pane, query) {
            self.search.asking(&pane, query);
            return;
        }
        let matches = self.convo.search_held(&pane, query);
        let total = matches.len() as u32;
        self.search.found(&pane, query, matches, total, false);
        self.aim_at_match(&pane);
    }

    /// Puts the transcript on the turn the search is standing on, walking back through pages
    /// where the hit is older than anything held.
    pub(super) fn aim_at_match(&mut self, pane: &str) {
        let Some(turn) = self.search.at(pane).map(|hit| hit.turn.clone()) else {
            return;
        };
        match self.convo.aim(pane, &turn) {
            crate::convo::Aimed::Shown => {}
            crate::convo::Aimed::Paging(before) => {
                self.client.convo_load(pane, Some(&before));
            }
            crate::convo::Aimed::Gone => self.note("that turn is older than this transcript goes"),
        }
    }

    /// Sends the query to the node, which is the only thing that can answer it: this client holds
    /// a window on the pane's history and the search is of the whole of it (#511).
    fn search_scrollback(&mut self, query: &str, backward: bool) {
        let Some(pane) = self.focus.clone() else {
            self.note("no pane to search");
            return;
        };
        if !self.client.find(&pane, query, backward, None) {
            self.note("not delivered — the socket is down");
        }
    }

    fn step_search(&mut self, forward: bool) {
        let Some(pane) = self.focus.clone() else { return };
        if self.router.mode() == Mode::Results {
            match self.search.step(&pane, forward).is_some() {
                true => self.aim_at_match(&pane),
                false => self.note("nothing to step through"),
            }
            return;
        }
        match self.find.step(&pane, forward) {
            Some(from_bottom) => self.show_match(&pane, from_bottom),
            None => self.note("nothing to step through — search first"),
        }
    }

    /// Puts the matched row on screen with a little history above it, because a match pinned to
    /// the top edge is a match with no context, and the context is what somebody searching wants.
    pub(super) fn show_match(&mut self, pane: &str, from_bottom: u32) {
        const CONTEXT: u32 = 3;
        let at = u16::try_from(from_bottom.saturating_sub(CONTEXT)).unwrap_or(u16::MAX);
        self.scrolls.insert(pane.to_string(), at);
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
