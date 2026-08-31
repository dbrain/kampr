//! `kampr` as a terminal client of its own herd: herdr's shape and herdr's habits, over the
//! mesh, across every node at once.
//!
//! The event loop and the renderer are one unit — the loop calls the renderer every tick — and
//! everything else is a module it dispatches into: [`manage`], [`mouse`], [`image`] and
//! [`convo`], all of them reached through [`app::App`].

pub mod app;

pub mod convo;
pub mod image;
pub mod input;
pub mod keymap;
pub mod manage;
pub mod mouse;
pub mod render;
pub mod sidebar;
pub mod theme;

use app::{App, Options};
use crossterm::event::{EnableBracketedPaste, Event as Term, EventStream, KeyEventKind};
use futures_util::StreamExt;
use kampr_client::{Client, Session};
use render::fit::{Chrome, Need, Tty};
use std::sync::Arc;
use std::time::Duration;

const TICK: Duration = Duration::from_millis(250);

pub async fn run(session: Session) -> anyhow::Result<()> {
    run_with(session, Options::from_env()).await
}

/// The modes this client turned on, and therefore the only ones it may turn off.
///
/// **The loop has six ways out and the reset used to sit on one of them** — the clean quit. Every
/// other exit, including the ordinary one where the node restarts and the event channel closes,
/// left the terminal reporting the mouse: `[<0;12;5M` at every click, for the life of that shell.
/// `ratatui::restore` cannot help, because it can only undo what `ratatui::init` did and these
/// modes are ours.
///
/// So it is a guard rather than a call. `Drop` runs on the clean path, on `?`, and while a panic
/// unwinds; the hook in [`arm_hook`] covers a panic that aborts instead. It resets only what it
/// armed, because a terminal that was never put in mouse mode must not be taken out of one — an
/// operator who runs kampr inside something else keeps that thing's modes.
pub struct Restore {
    paste: bool,
    mouse: bool,
    /// Where the reset goes. Stdout in the binary; a test hands it a buffer, because the whole
    /// claim here is about what `Drop` does and a test that called `emit` itself would pass with
    /// `Drop` empty — which is the defect (#191).
    out: Box<dyn std::io::Write + Send>,
}

impl Default for Restore {
    fn default() -> Self {
        Self::to(Box::new(std::io::stdout()))
    }
}

impl Restore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn to(out: Box<dyn std::io::Write + Send>) -> Self {
        Self {
            paste: false,
            mouse: false,
            out,
        }
    }

    pub fn arm_paste(&mut self) {
        self.paste = true;
    }

    pub fn arm_mouse(&mut self) {
        self.mouse = true;
    }

    /// Written as bytes rather than through `execute!` so a test can read them back, and because
    /// `Drop` has nowhere to put an error.
    fn emit(&mut self) {
        if self.mouse {
            // The same five crossterm sets (#300), reset in the reverse order it sets them.
            let _ = self
                .out
                .write_all(b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l");
            self.mouse = false;
        }
        if self.paste {
            let _ = self.out.write_all(b"\x1b[?2004l");
            self.paste = false;
        }
        let _ = self.out.flush();
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        self.emit();
    }
}

/// A panic mid-frame leaves the terminal in raw mode on the alt screen with the mouse reporting,
/// and the backtrace it is trying to print scrolls past unreadably. Chained rather than replaced,
/// so the operator still gets the panic message they need to report.
fn arm_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            drop(Restore {
                paste: true,
                mouse: true,
                out: Box::new(std::io::stdout()),
            });
            ratatui::restore();
            previous(info);
        }));
    });
}

pub async fn run_with(session: Session, options: Options) -> anyhow::Result<()> {
    arm_hook();
    let mut restore = Restore::new();
    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stdout(), EnableBracketedPaste)?;
    restore.arm_paste();
    // One prober, one batch, before the event stream owns the keyboard: a second reader on the
    // same tty would race it for the answer, and the cell size is an answer both the fit ladder
    // and the image renderer wanted (#291, #299).
    let mut tty = Tty::probe();
    let images = image::Images::with(&session, tty.named(), image::caps_in(tty.answers()), tty.cell());
    let client = Arc::new(Client::start(session));
    let result = drive(&mut terminal, client, options, images, &mut tty, &mut restore).await;
    drop(restore);
    ratatui::restore();
    result
}

async fn drive(
    terminal: &mut ratatui::DefaultTerminal,
    client: Arc<Client>,
    options: Options,
    images: image::Images,
    tty: &mut Tty,
    restore: &mut Restore,
) -> anyhow::Result<()> {
    let mut app = App::new(client.clone(), options, images);
    // Unconditional, and it has to be: the tabs, the sidebar rows, the herd view and a prompt's
    // option chips are kampr's own chrome and are clickable whatever any pane is doing. Whether
    // a *pane* sees a report is a separate gate the operator holds, because #292 measured that
    // nothing on herdr's socket can answer it.
    let mousing = app.mouse.capture();
    if mousing {
        crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
        restore.arm_mouse();
    }
    app.note(format!(
        "{} · images {:?}",
        tty.named().unwrap_or("this terminal"),
        app.images.protocol()
    ));
    let mut events = client.events();
    let mut keys = EventStream::new();
    let mut tick = tokio::time::interval(TICK);
    loop {
        // An image drawn inline is not in the buffer — its cells are `Skip` — so ratatui's diff
        // cannot repaint them and the pixels outlive the view that put them there.
        if app.wiping() {
            terminal.clear()?;
        }
        terminal.draw(|frame| app.draw(frame))?;
        fit(&mut app, &client, tty);
        if app.quitting() {
            return Ok(());
        }
        tokio::select! {
            key = keys.next() => match key {
                Some(Ok(Term::Key(key))) if key.kind != KeyEventKind::Release => app.key(key),
                Some(Ok(Term::Paste(data))) => app.paste(&data),
                Some(Ok(Term::Resize(_, _))) => app.rethink_fit(),
                Some(Ok(Term::Mouse(event))) => {
                    let role = client.state().role;
                    let click = app.mouse.hit(event, &app.layout, role);
                    app.clicked(click);
                }
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()),
                _ => {}
            },
            event = events.recv() => match event {
                Ok(event) => app.absorb(&event),
                // A consumer that lagged redraws from the state, which is authoritative.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => app.refocus(),
                Err(_) => return Ok(()),
            },
            _ = tick.tick() => {}
        }
    }
}

fn fit(app: &mut App, client: &Arc<Client>, tty: &mut Tty) {
    let Some(pane) = app.focused().map(str::to_string) else {
        return;
    };
    let need = {
        let state = client.state();
        let cols = state.herd.pane(&pane).and_then(|e| e.cols);
        let held = state.pane(&pane).map(|p| p.geometry());
        match (cols, held) {
            (Some(cols), Some((_, rows))) => Need { cols, rows },
            (None, Some((cols, rows))) if cols > 0 => Need { cols, rows },
            _ => return,
        }
    };
    let Some(body) = app.layout.pane(&pane).map(|placed| placed.rect) else {
        return;
    };
    let Ok((cols, rows)) = crossterm::terminal::size() else {
        return;
    };
    let chrome = Chrome {
        cols: cols.saturating_sub(body.width.saturating_sub(2)),
        rows: rows.saturating_sub(body.height.saturating_sub(2)),
    };
    app.fit(tty, need, chrome);
}

/// The clipboard, over OSC 52, so a copy works the same way through an ssh session as it does at
/// the desk and this crate needs no clipboard of its own.
pub fn osc52(text: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = write!(out, "\u{1b}]52;c;{}\u{7}", b64(text.as_bytes()));
    let _ = out.flush();
}

fn b64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        for i in 0..4 {
            match i <= chunk.len() {
                true => out.push(ALPHABET[(n >> (18 - i * 6)) as usize & 0x3f] as char),
                false => out.push('='),
            }
        }
    }
    out
}

/// One block of [`help`]. `manage` marks a section whose binds a read-only device, or a node that
/// does not claim the capability, cannot use — absent rather than disabled, the same rule the
/// footer follows.
pub struct Section {
    pub title: &'static str,
    pub rows: &'static [(&'static str, &'static str)],
    pub manage: bool,
}

/// What `prefix+?` draws.
///
/// **Every row here is a bind this build actually routes**, apart from the last section, which is
/// labelled as commands rather than keys. It is grouped rather than flat because the flat version
/// was thirty rows of `prefix w / g · herd navigator (modal)` — true, and no use at all to somebody
/// who does not yet know that is how you walk the sidebar.
pub fn help(manage: bool) -> impl Iterator<Item = &'static Section> {
    SECTIONS.iter().filter(move |section| manage || !section.manage)
}

static SECTIONS: &[Section] = &[
    Section {
        title: "getting around",
        manage: false,
        rows: &[
            (
                "prefix w / g",
                "walk the sidebar — up/down, enter opens, esc leaves",
            ),
            (
                "prefix shift+h",
                "the herd: every node, every pane, blocked first",
            ),
            ("prefix shift+f", "the fleet board: every run, needs-you first"),
            ("prefix shift+e", "run one command on every online node"),
            ("prefix b", "show or hide the sidebar"),
            ("prefix tab", "cycle the panes on screen"),
            ("prefix space", "back to the last pane"),
            ("prefix h/j/k/l", "focus a pane"),
            ("prefix p / n", "previous / next tab"),
            ("prefix 1..9", "switch to a tab by number"),
            ("prefix q", "detach"),
        ],
    },
    Section {
        title: "reading a pane",
        manage: false,
        rows: &[
            ("prefix up/down", "scroll back through the ring"),
            ("prefix pgup/pgdn", "scroll a screen at a time"),
            ("prefix left/right", "pan a grid wider than the window"),
            ("prefix home/end", "pan to the row's edges"),
            ("prefix 0", "reset the pan and the scroll"),
            ("prefix z", "zoom this pane to fill the mosaic"),
            ("prefix shift+v", "terminal ⇄ conversation"),
            ("wheel", "scrolls whatever is under the pointer"),
        ],
    },
    Section {
        title: "the conversation — prefix shift+v opens one",
        manage: false,
        rows: &[
            ("type", "writes in the reply box; nothing reaches the agent yet"),
            ("enter", "sends what is in the box"),
            ("alt+enter", "a second line"),
            ("esc", "clears the box"),
            ("up/down pgup/pgdn", "move the transcript, not the pane's ring"),
            (
                "1..9 on an empty box",
                "answers the question a blocked agent is asking",
            ),
        ],
    },
    Section {
        title: "two panes at once — any node, any host",
        manage: false,
        rows: &[
            ("in the sidebar: space", "put that pane beside this one"),
            ("prefix shift+m", "drop the mosaic back to this tab"),
            ("prefix r", "resize mode — kampr's own split, never the pane"),
        ],
    },
    Section {
        title: "copying, and links",
        manage: false,
        rows: &[
            ("prefix [", "copy mode"),
            ("drag", "select; the copy is the logical text, not the grid"),
            ("prefix o", "open the link a click offered"),
            ("prefix m", "let this pane have the mouse (off by default)"),
        ],
    },
    Section {
        title: "changing the herd",
        manage: true,
        rows: &[
            ("prefix c", "new tab"),
            ("prefix , / shift+t", "rename tab"),
            ("prefix v / -", "split vertical / horizontal"),
            ("prefix x", "close pane"),
            ("prefix shift+p", "rename pane"),
            ("prefix shift+x", "close tab"),
            ("prefix shift+n", "new workspace"),
            ("prefix shift+g", "new worktree"),
            ("prefix shift+w", "rename workspace"),
            ("prefix shift+d", "close workspace"),
        ],
    },
    Section {
        title: "the modal keymaps — no prefix while they are open",
        manage: false,
        rows: &[
            ("copy", "h/j/k/l w/b/e { } move · / ? search · v select · y copy"),
            ("resize", "h/l width · j/k height · esc done"),
            (
                "sidebar",
                "up/down row · enter open · space beside · 1-9 workspace",
            ),
        ],
    },
    Section {
        title: "commands, not keys",
        manage: false,
        rows: &[
            ("kampr setup", "pair a device with this machine's herd"),
            (
                "kampr connect URL --code C",
                "open another machine's herd from here",
            ),
            (
                "kampr doctor",
                "check everything that has to be true, and say what is not",
            ),
        ],
    },
];

/// The two facts every row above assumes, said once.
pub const HELP_HEAD: &str = "the prefix is ctrl+b · ctrl+b ctrl+b sends a literal one";
