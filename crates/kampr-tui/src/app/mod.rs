mod act;
mod chrome;

pub use chrome::{Chips, Layout, Placed};

use crate::convo::Convo;
use crate::image::Images;
use crate::input::Router;
use crate::manage::Manage;
use crate::mouse::Mouse;
use crate::render::fit::{self, Chrome, Need, Rung};
use crate::sidebar;
use crate::theme::Theme;
use crossterm::event::KeyEvent;
use kampr_client::{Client, Event, Managed};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// How long a note sits in the status line before the line goes back to saying what is true.
const NOTE_LIFETIME: std::time::Duration = std::time::Duration::from_secs(5);

/// Rows one notch of the wheel moves. Three is what a terminal's own scrollback does, and the
/// wheel is aiming at the same surface here.
const WHEEL: u16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub theme: Theme,
    /// herdr chose the **local** keymap for `--remote` and gave the reason: local muscle memory
    /// beats remote config, so this is `ctrl+b` and copies herdr's whole table (#289). The
    /// escape hatch is to move it: a prefix kampr does not claim reaches the pane's own program
    /// untouched, which is as close as a cell-grid client gets to running the node's keymap —
    /// herdr's config is not on the wire and #296 measured that a client cannot read it back.
    pub prefix: KeyEvent,
    /// Whether rung 2 of the fit ladder may write `CSI 8;rows;cols t` at all.
    pub resize: bool,
    /// Whether a pane this client is looking at is held at this terminal's own size for as long as
    /// it is open (ADR 0013). On, because this client is a desk; the operator turns it off per
    /// pane from the size menu, or for the process with `KAMPR_TUI_MATCH=0`.
    pub match_view: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            theme: crate::theme::PHOSPHOR,
            prefix: crate::keymap::HERDR_PREFIX,
            resize: true,
            match_view: true,
        }
    }
}

impl Options {
    /// `KAMPR_TUI_SKIN=1` paints the pane's own 16 slots with kampr's Phosphor terminal skin
    /// instead of passing them through as ordinary SGR; `KAMPR_TUI_RESIZE=0` turns rung 2 of the
    /// fit ladder off; `KAMPR_TUI_PREFIX=ctrl+a` moves the prefix off `ctrl+b`.
    pub fn from_env() -> Self {
        let on = |name: &str, default: bool| {
            std::env::var(name)
                .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(default)
        };
        Self {
            theme: crate::theme::PHOSPHOR.skinned(on("KAMPR_TUI_SKIN", false)),
            prefix: std::env::var("KAMPR_TUI_PREFIX")
                .ok()
                .and_then(|name| prefix_named(&name))
                .unwrap_or(crate::keymap::HERDR_PREFIX),
            resize: on("KAMPR_TUI_RESIZE", true),
            match_view: on("KAMPR_TUI_MATCH", true),
        }
    }
}

fn prefix_named(name: &str) -> Option<KeyEvent> {
    let (modifier, rest) = name.split_once('+')?;
    let mut chars = rest.chars();
    let ch = chars.next().filter(|_| chars.next().is_none())?;
    let modifiers = match modifier {
        "ctrl" => crossterm::event::KeyModifiers::CONTROL,
        "alt" => crossterm::event::KeyModifiers::ALT,
        _ => return None,
    };
    Some(KeyEvent::new(crossterm::event::KeyCode::Char(ch), modifiers))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Terminal,
    Conversation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Panes,
    Herd,
    /// Fleet runs, grouped by the fan-out that produced them. A separate screen rather than a
    /// section of the herd, because a fleet run is not on anybody's desk and the question it
    /// answers — how did they all go, and which one needs me — is not the herd's question.
    Fleet,
}

pub struct App {
    pub(super) client: Arc<Client>,
    pub options: Options,
    pub router: Router,
    pub manage: Manage,
    pub mouse: Mouse,
    pub images: Images,
    pub convo: Convo,
    pub composer: crate::convo::Composer,
    pub layout: Layout,
    focus: Option<String>,
    last: Option<String>,
    pans: HashMap<String, fit::Pan>,
    scrolls: HashMap<String, u16>,
    /// The ring, decoded once per `scrollback` message rather than once per frame. Text shaping
    /// is the whole cost of a frame and an allocation per frame in the draw path is a
    /// regression however much cleaner it reads.
    rings: HashMap<String, Vec<Vec<kampr_term::Cell>>>,
    views: HashMap<String, View>,
    sidebar_open: bool,
    /// The operator's own scroll of the sidebar, which the navigator's cursor clamps rather than
    /// owns — a list taller than its box is unreachable otherwise, and it routinely is.
    sidebar_top: usize,
    pick: usize,
    screen: Screen,
    zoomed: bool,
    /// Kampr's own mosaic, when the operator has assembled one: independent `observe` streams
    /// that may come from different sessions on different hosts. Empty means the focused pane's
    /// tab, which is the herdr-shaped default.
    pinned: Vec<String>,
    split: u16,
    rung: Option<Rung>,
    fitted: Option<(Need, (u16, u16))>,
    note: String,
    noted: std::time::Instant,
    /// A **detected** URL, waiting for the operator to say so. Pane output is
    /// attacker-influenceable, so nothing here is navigated on a click.
    offered: Option<String>,
    watching: HashSet<String>,
    /// The pane this client is holding at its own window's size, and the size it asked for.
    matching: Option<(String, u16, u16)>,
    /// What the window most recently measured, and when it settled there. A drag is hundreds of
    /// sizes and each claim is a `herdr terminal session control` child, so the size has to hold
    /// still before it is asked for.
    settling: Option<((String, u16, u16), std::time::Instant)>,
    /// Panes the operator has turned matching off for, session-local. A pane is added by the
    /// release the size menu sends, which is a real op rather than a switch with no wire behind it.
    unmatched: HashSet<String>,
    /// A drawn image is not in the buffer — its cells are `Skip` — so ratatui's diff has nothing
    /// to repaint them from and the pixels outlive the view. Tearing one down asks for a wipe.
    wipe: bool,
    /// Every `manage` ack, carried back from the task that awaited it. A successful op produces
    /// no frame the surface can otherwise see, so its notice would age out instead of resolving.
    acked: UnboundedSender<Managed>,
    acks: UnboundedReceiver<Managed>,
    keybinds: bool,
    help_top: usize,
    quit: bool,
}

impl App {
    pub fn new(client: Arc<Client>, options: Options, images: Images) -> Self {
        let (acked, acks) = unbounded_channel();
        Self {
            client,
            options,
            router: Router::with_prefix(options.prefix),
            manage: Manage::new(),
            mouse: Mouse::new(),
            images,
            convo: Convo::new(),
            composer: crate::convo::Composer::default(),
            layout: Layout::default(),
            focus: None,
            last: None,
            pans: HashMap::new(),
            scrolls: HashMap::new(),
            rings: HashMap::new(),
            views: HashMap::new(),
            sidebar_open: true,
            sidebar_top: 0,
            pick: 1,
            screen: Screen::Panes,
            zoomed: false,
            pinned: Vec::new(),
            split: 50,
            rung: None,
            fitted: None,
            note: String::new(),
            noted: std::time::Instant::now(),
            offered: None,
            watching: HashSet::new(),
            matching: None,
            settling: None,
            unmatched: HashSet::new(),
            wipe: false,
            acked,
            acks,
            keybinds: false,
            help_top: 0,
            quit: false,
        }
    }

    pub fn quitting(&self) -> bool {
        self.quit
    }

    /// Everything the node said, routed to the surface that owns it.
    ///
    /// **`caps` is asked for on every greeting**, because the agent kinds and the named sessions
    /// arrive only in its answer and a menu that never got one hides those rows for the wrong
    /// reason. **`Disconnected` reaches the conversation**, because `pending` is published on a
    /// blocked-state edge: the node's first attempt at a pane that is still blocked carries
    /// nothing, so a question kept across a reconnect is answered into a pane with nothing
    /// matching to answer it.
    pub fn absorb(&mut self, event: &Event) {
        match event {
            Event::Prefs { greeting: true } => {
                self.adopt_prefs();
                self.refocus();
                self.sync_watches();
            }
            Event::Herd => {
                self.refocus();
                self.sync_watches();
            }
            Event::Connected(_) => {
                let node = self.client.state().node_name().to_string();
                self.note(format!("connected to {node}"));
                self.client.request_caps();
            }
            Event::Disconnected { reason } => {
                self.note(reason.clone());
                self.convo.absorb(event);
            }
            // Not a second greeting: only the write affordances move, and they are gated on the
            // live role rather than on the one the greeting carried. A modal collecting an op a
            // demoted device can no longer send is closed rather than left to be refused.
            Event::Role(role) => {
                if !role.writes() {
                    self.manage.close();
                }
                self.note(format!("this device is now {}", role.as_str()));
            }
            Event::Error(failure) => {
                self.note(failure.message.clone());
                self.manage.observe(event);
            }
            Event::Managed(_) | Event::Caps(_) => self.manage.observe(event),
            Event::Scrollback { pane, .. } => self.absorb_ring(pane),
            Event::Convo(_)
            | Event::ConvoTurn { .. }
            | Event::ConvoFacets { .. }
            | Event::ConvoComposer { .. }
            | Event::Pending(_) => self.convo.absorb(event),
            _ => {}
        }
    }

    /// Whether the terminal has to be wiped before the next frame.
    pub fn wiping(&mut self) -> bool {
        std::mem::take(&mut self.wipe)
    }

    /// The pixels of a drawn image are not in the buffer, so a view that stops drawing one has to
    /// take them down itself.
    fn teardown(&mut self) {
        if self.images.drawn() == 0 {
            return;
        }
        self.images.clear();
        self.wipe = true;
    }

    /// The size this client is holding `pane` at, if it is holding it at all.
    pub fn matched(&self, pane: &str) -> Option<(u16, u16)> {
        self.matching
            .as_ref()
            .filter(|(held, _, _)| held == pane)
            .map(|(_, cols, rows)| (*cols, *rows))
    }

    pub fn rung(&self) -> Option<&Rung> {
        self.rung.as_ref()
    }

    pub fn note(&mut self, text: impl Into<String>) {
        self.note = text.into();
        self.noted = std::time::Instant::now();
    }

    /// The terminal changed shape, so whatever the ladder decided was decided about a window
    /// that is no longer there.
    pub fn rethink_fit(&mut self) {
        self.fitted = None;
    }

    pub fn clicked(&mut self, click: crate::mouse::Click) {
        use crate::mouse::Click;
        match click {
            Click::None => {}
            Click::Focus(pane) => {
                self.focus(pane);
                self.open(Screen::Panes);
            }
            Click::Tab(tab) => self.open_tab(&tab),
            Click::OpenHerd => self.open(Screen::Herd),
            Click::Answer { pane, key } => {
                self.client.answer(&pane, &key);
            }
            Click::Save { pane, id } => self.save(&pane, &id),
            Click::Passthrough { pane, text } => {
                self.client.input(&pane, &text);
            }
            Click::Wheel { pane, up } => self.wheel(pane, up),
            Click::Menu(menu) => self.context_menu(menu),
        }
    }

    /// The right button opened a menu about the thing under the pointer. **Silent when there is
    /// nothing to offer** — a read-only device, a node that does not claim `manage` — because a
    /// right-click is an exploratory gesture rather than an op the operator asked for, and a note
    /// on every stray one is noise the keyboard path is the right place for.
    fn context_menu(&mut self, menu: crate::mouse::Menu) {
        use crate::mouse::Menu;
        let state = self.client.state();
        let (target, pane) = match &menu {
            Menu::Pane(pane) => (crate::manage::Target::Pane, Some(pane.clone())),
            Menu::Space(pane) => (crate::manage::Target::Space, Some(pane.clone())),
            // The strip carries a tab id and the ops carry it too, but the entry a menu is built
            // from is a pane's: the tab's own name and its workspace are only ever read off one.
            Menu::Tab(tab) => (
                crate::manage::Target::Tab,
                state
                    .herd
                    .panes
                    .iter()
                    .find(|p| p.tab_id.as_deref() == Some(tab.as_str()))
                    .map(|p| p.id.clone()),
            ),
        };
        let Some(pane) = pane else { return };
        let caps = state.caps();
        self.manage.context(target, &state.herd, &pane, &caps, state.role);
    }

    /// The wheel over kampr's own surfaces. Whichever of the two a pane is showing moves; the
    /// other one does not, which is the same rule the keyboard already follows.
    fn wheel(&mut self, pane: Option<String>, up: bool) {
        let Some(pane) = pane else {
            self.sidebar_top = match up {
                true => self.sidebar_top.saturating_sub(WHEEL as usize),
                false => self.sidebar_top.saturating_add(WHEEL as usize),
            };
            return;
        };
        if self.view(&pane) == View::Conversation
            && self.convo.has(&pane)
            && self.convo.wheel(&pane, up, WHEEL as usize)
        {
            self.teardown();
            return;
        }
        let at = self.scrolls.entry(pane).or_default();
        *at = match up {
            true => at.saturating_add(WHEEL),
            false => at.saturating_sub(WHEEL),
        };
    }

    /// Clamped by the draw, which is the only place that knows how tall the panel came out.
    pub(super) fn scroll_help(&mut self, by: i32) {
        self.help_top = self.help_top.saturating_add_signed(by as isize);
    }

    /// The first sidebar row on screen: the operator's own scroll, clamped to the list and then to
    /// wherever the navigator's cursor is, so walking off the bottom of the box brings the box.
    pub(super) fn sidebar_view(&mut self, rows: usize, height: usize, selected: Option<usize>) -> usize {
        let last = rows.saturating_sub(height);
        let mut top = self.sidebar_top.min(last);
        if let Some(pick) = selected.filter(|pick| *pick < rows) {
            top = top.min(pick);
            top = top.max((pick + 1).saturating_sub(height));
        }
        self.sidebar_top = top;
        top
    }

    fn open(&mut self, screen: Screen) {
        if self.screen == screen {
            return;
        }
        self.teardown();
        self.screen = screen;
    }

    /// The bytes of an attachment this terminal will not draw, written where the operator can
    /// find them. Only a picture that has actually landed has anything to write.
    fn save(&mut self, pane: &str, id: &str) {
        let Some(name) = self
            .images
            .offer(pane, id)
            .filter(|offer| offer.ready)
            .map(|offer| offer.name.unwrap_or(id).to_string())
        else {
            self.note("nothing has landed for that attachment");
            return;
        };
        let to = downloads().join(sanitise(&name));
        match self.images.save(pane, id, &to) {
            Ok(()) => self.note(format!("saved {}", to.display())),
            Err(e) => self.note(format!("could not save it · {e}")),
        }
    }

    pub fn focused(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    pub fn pinned(&self) -> &[String] {
        &self.pinned
    }

    fn writes(&self) -> bool {
        self.client.state().role.writes()
    }

    /// The per-pane view choice lives in `prefs`, so it follows the operator between machines.
    /// A pane with none opens on the **terminal**, whatever it is running.
    pub fn adopt_prefs(&mut self) {
        let stored: Vec<(String, String)> = {
            let state = self.client.state();
            state
                .prefs
                .iter()
                .filter_map(|(pane, blob)| Some((pane.clone(), blob.get("view")?.as_str()?.to_string())))
                .collect()
        };
        for (pane, view) in stored {
            let view = match view.as_str() {
                "conversation" => View::Conversation,
                _ => View::Terminal,
            };
            self.views.insert(pane, view);
        }
        let armed: Vec<String> = {
            let state = self.client.state();
            state
                .prefs
                .iter()
                .filter(|(_, blob)| blob.get("mouse").and_then(|m| m.as_bool()) == Some(true))
                .map(|(pane, _)| pane.clone())
                .collect()
        };
        for pane in armed {
            self.mouse.set_passthrough(&pane, true);
        }
    }

    /// **The terminal is what a terminal client opens on.** This used to open an agent pane on its
    /// conversation, which is what a phone wants and not what somebody who typed `kampr` into a
    /// shell does: a client that answers a terminal with a transcript is a worse herdr. The
    /// conversation is one keystroke away — `prefix shift+v` — and the choice sticks per pane.
    pub fn view(&self, pane: &str) -> View {
        self.views.get(pane).copied().unwrap_or(View::Terminal)
    }

    /// The panes on screen: every pane of the focused pane's tab, which is kampr's own mosaic —
    /// a client-side arrangement of independent `observe` streams, not `manage`'s `pane.split`.
    pub fn mosaic(&self) -> Vec<String> {
        let Some(focus) = self.focus.clone() else {
            return Vec::new();
        };
        if self.zoomed {
            return vec![focus];
        }
        if !self.pinned.is_empty() {
            return self.pinned.clone();
        }
        let state = self.client.state();
        let Some(entry) = state.herd.pane(&focus) else {
            return vec![focus];
        };
        let tab = entry.tab_id.clone();
        match tab {
            Some(tab) => state
                .herd
                .panes
                .iter()
                .filter(|p| p.tab_id.as_deref() == Some(tab.as_str()))
                .map(|p| p.id.clone())
                .collect(),
            None => vec![focus],
        }
    }

    pub fn refocus(&mut self) {
        let alive = self
            .focus
            .as_ref()
            .is_some_and(|id| self.client.state().herd.pane(id).is_some());
        if alive {
            return;
        }
        let pick = {
            let state = self.client.state();
            sidebar::triage(&state.herd)
                .first()
                .map(|p| p.id.clone())
                .or_else(|| state.herd.panes.first().map(|p| p.id.clone()))
        };
        if let Some(pick) = pick {
            self.focus(pick);
        }
    }

    /// **A focus is a subscription change**, because [`Self::mosaic`] is the focused pane's tab.
    /// Nothing else was saying so: only the greeting and a `herd` frame restated the watches, and
    /// an agent pane hid it by churning its status until one arrived. A shell at its prompt makes
    /// no herd traffic at all and sat on "waiting for the first frame" for ever.
    ///
    /// Unconditional, including on the pane that is already focused, so a caller that moved the
    /// mosaic some other way — pinning a second pane beside this one — is covered by focusing into
    /// it rather than by remembering to say so twice.
    fn focus(&mut self, pane: String) {
        if self.focus.as_deref() != Some(pane.as_str()) {
            self.teardown();
            self.last = self.focus.take();
            self.focus = Some(pane);
            self.zoomed = false;
        }
        self.sync_watches();
    }

    /// A watch is stated once and re-issued by the client on every reconnection, so this only
    /// has to say what has changed.
    pub fn sync_watches(&mut self) {
        let want: HashSet<String> = self.mosaic().into_iter().collect();
        let caps = self.client.state().caps();
        for pane in want.difference(&self.watching) {
            self.client.watch(pane, caps.scrollback, caps.conversation);
        }
        for pane in self.watching.difference(&want) {
            self.client.unwatch(pane);
        }
        self.watching = want;
    }

    /// The fit ladder, climbed once per pane geometry and terminal size rather than per frame:
    /// rung 2 writes to the terminal, and writing it every tick would be a resize storm.
    pub fn fit(&mut self, display: &mut dyn fit::Display, need: Need, chrome: Chrome) {
        let Some(size) = display.cells() else { return };
        if self.fitted == Some((need, size)) {
            return;
        }
        self.fitted = Some((need, size));
        // **Not while this window is holding the pane at its own size.** The ladder's second rung
        // asks the terminal to grow to the pane; a matched pane is already the size of the
        // terminal, so the two would take turns for ever (ADR 0013).
        let ask = self.options.resize && self.matching.is_none();
        let rung = fit::climb(display, need, chrome, ask);
        // **Said when it happens, not for ever after.** The ladder climbs once per pane geometry
        // and terminal size, and its report is a long sentence about why a pane is being cropped —
        // as a standing tenant of the borrowed row that put it on screen permanently for any pane
        // wider than the window. The pan window says the same thing in eight characters and is the
        // one that stays.
        //
        // Rung 1 is not news. It is the ordinary case, it says nothing the operator cannot see, and
        // announcing it would mean every launch on a wide terminal opens with a sentence about
        // ladders — noise on the one row this client just spent four rows buying back.
        if rung != fit::Rung::Fits {
            self.note(rung.report());
        }
        self.rung = Some(rung);
    }
}

/// Only what a terminal client has any business handing to a desktop opener. A harness declares
/// an OSC 8 URI, but so can anything else writing to that PTY, and `xdg-open` will happily give
/// an unknown scheme to whatever claims it.
pub fn navigable(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// **The gate lives here rather than at the call sites, because a call site forgot it.**
///
/// There were two ways in — a click, which checked the scheme, and `prefix o`, which did not — and
/// the unchecked one was the one the status line advertised. A pane declares its own OSC 8 URIs
/// and pane output is attacker-influenceable, so `file:///tmp/x.desktop` reached `xdg-open` on the
/// operator's own desktop for one keystroke the interface had just asked for. That is the
/// two-paths-one-gated shape #233 is named for, and the answer is one path.
pub fn open_url(url: &str) -> bool {
    if !navigable(url) {
        return false;
    }
    let opener = match cfg!(target_os = "macos") {
        true => "open",
        false => "xdg-open",
    };
    std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

fn downloads() -> std::path::PathBuf {
    std::env::var_os("XDG_DOWNLOAD_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join("Downloads")))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// The name rides on a transcript a harness wrote, so it names a file in one directory and never
/// a path.
fn sanitise(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(
            |c| match c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                true => c,
                false => '_',
            },
        )
        .collect();
    match cleaned.trim_matches('.').is_empty() {
        true => "attachment".to_string(),
        false => cleaned,
    }
}
