use crate::palette;
use alacritty_terminal::Term;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::{Params, Perform};
use base64::Engine as _;
use kampr_core::provider::{ProgramState, ProgramStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ask {
    Primary,
    Secondary,
    Version,
    Status,
    Cursor,
    ExtendedCursor,
    PrivateMode(u16),
    AnsiMode(u16),
    TextArea,
    Keyboard,
    Program,
    Colour {
        prefix: String,
        index: usize,
        bell: bool,
    },
}

/// What `alacritty_terminal` cannot tell afterwards: which questions were asked, and whether its
/// history was emptied rather than pulled back onto a taller screen. A piece also ends after each
/// sequence that can push a screenful into history at once (`screen.rs`, `PIECE`).
#[derive(Default)]
pub(crate) struct Found {
    pub asks: Vec<Ask>,
    pub cleared: bool,
    /// The last `OSC 7501` report in the piece; `Some(None)` is `state=clear`, the harness gone.
    pub program: Option<Option<ProgramStatus>>,
}

/// A second `vte` parser over the same bytes, stopping right after each sequence the screen must
/// act on so that the emulator is fed exactly that far before it is read. `alacritty_terminal`
/// answers some of these itself, differently from herdr (#88) or not at all (XTVERSION, DECXCPR).
#[derive(Default)]
pub(crate) struct Scan {
    pub found: Found,
    stop: bool,
}

impl Scan {
    pub fn take(&mut self) -> Found {
        self.stop = false;
        std::mem::take(&mut self.found)
    }

    fn ask(&mut self, ask: Ask) {
        self.found.asks.push(ask);
        self.stop = true;
    }
}

fn first(params: &Params) -> u16 {
    params.iter().next().and_then(|p| p.first().copied()).unwrap_or(0)
}

impl Perform for Scan {
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        let p = first(params);
        match (intermediates, action) {
            ([], 'c') if p == 0 => self.ask(Ask::Primary),
            ([b'>'], 'c') if p == 0 => self.ask(Ask::Secondary),
            ([b'>'], 'q') if p == 0 => self.ask(Ask::Version),
            ([], 'n') if p == 5 => self.ask(Ask::Status),
            ([], 'n') if p == 6 => self.ask(Ask::Cursor),
            ([b'?'], 'n') if p == 6 => self.ask(Ask::ExtendedCursor),
            ([b'?', b'$'], 'p') => self.ask(Ask::PrivateMode(p)),
            ([b'$'], 'p') => self.ask(Ask::AnsiMode(p)),
            ([], 't') if p == 18 => self.ask(Ask::TextArea),
            ([b'?'], 'u') => self.ask(Ask::Keyboard),
            ([], 'J') if p == 3 => {
                self.found.cleared = true;
                self.stop = true;
            }
            ([], 'J') if p == 2 => self.stop = true,
            ([], 'S' | 'M') => self.stop = true,
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if intermediates.is_empty() && byte == b'c' {
            self.found.cleared = true;
            self.stop = true;
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell: bool) {
        match params {
            [b"4", rest @ ..] => {
                for pair in rest.chunks(2) {
                    if let [index, b"?"] = pair
                        && let Some(index) = number(index).filter(|i| *i < 256)
                    {
                        self.ask(Ask::Colour {
                            prefix: format!("4;{index}"),
                            index,
                            bell,
                        });
                    }
                }
            }
            [b"7501", b"?"] => self.ask(Ask::Program),
            [b"7501", report] => {
                if let Some(report) = program(report) {
                    self.found.program = Some(report);
                }
            }
            [code, rest @ ..] if matches!(*code, b"10" | b"11" | b"12") => {
                let Some(mut code) = number(code) else { return };
                for param in rest {
                    if code > 12 {
                        return;
                    }
                    if *param == b"?" {
                        self.ask(Ask::Colour {
                            prefix: code.to_string(),
                            index: code,
                            bell,
                        });
                    }
                    code += 1;
                }
            }
            _ => {}
        }
    }

    fn terminated(&self) -> bool {
        self.stop
    }
}

/// `state=…:app=claude-code[:id=…][:kind=…][:progress=…][:title=…][:msg=<base64>]` (#600). A
/// report with an `id` is one of the session's subagents, not the session.
fn program(report: &[u8]) -> Option<Option<ProgramStatus>> {
    let report = std::str::from_utf8(report).ok()?;
    let mut state = None;
    let mut kind = None;
    let mut message = None;
    for field in report.split(':') {
        let (key, value) = field.split_once('=')?;
        match key {
            "id" => return None,
            "state" => state = Some(value),
            "kind" => kind = Some(value.to_string()),
            "msg" => {
                message = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .ok()
                    .map(|m| String::from_utf8_lossy(&m).into_owned())
            }
            _ => {}
        }
    }
    let state = match state? {
        "idle" => ProgramState::Idle,
        "working" => ProgramState::Working,
        "blocked" => ProgramState::Blocked,
        "done" => ProgramState::Done,
        "error" => ProgramState::Error,
        "clear" => return Some(None),
        _ => return None,
    };
    Some(Some(ProgramStatus { state, kind, message }))
}

fn number(bytes: &[u8]) -> Option<usize> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// DA1 and DA2 as herdr answers them (#88). A native session answers as a good terminal does, and
/// one hosted on a herdr desk is answered by herdr, which keeps kitty flags too (#597): the node
/// answers only what no host took (`live.rs`). Answering the kitty query changes nothing
/// Claude Code draws (#595), and it asks no colour question (#594); the colours are the clients'.
/// `OSC 7501;?` is answered as the probe that found it did, and from then on Claude reports its
/// state on that channel and draws no cell differently, though its title loses the state glyph
/// (#600, #601). herdr does not answer it, so over a desk `kampr attach` does (`attach.rs`).
pub(crate) fn answer<T: EventListener>(term: &Term<T>, ask: &Ask) -> String {
    let grid = term.grid();
    let cursor = || {
        let point = grid.cursor.point;
        (point.line.0 + 1, point.column.0.min(grid.columns() - 1) + 1)
    };
    match ask {
        Ask::Primary => "\x1b[?62;22c".into(),
        Ask::Secondary => "\x1b[>0;0;0c".into(),
        Ask::Version => format!("\x1bP>|kampr {}\x1b\\", env!("CARGO_PKG_VERSION")),
        Ask::Status => "\x1b[0n".into(),
        Ask::Cursor => {
            let (row, col) = cursor();
            format!("\x1b[{row};{col}R")
        }
        Ask::ExtendedCursor => {
            let (row, col) = cursor();
            format!("\x1b[?{row};{col};1R")
        }
        Ask::PrivateMode(mode) => format!("\x1b[?{mode};{}$y", private_mode(*term.mode(), *mode)),
        Ask::AnsiMode(mode) => format!("\x1b[{mode};0$y"),
        Ask::TextArea => format!("\x1b[8;{};{}t", grid.screen_lines(), grid.columns()),
        Ask::Keyboard => format!("\x1b[?{}u", keyboard_flags(*term.mode())),
        Ask::Program => PROGRAM_ANSWER.into(),
        Ask::Colour { prefix, index, bell } => {
            let slot = match *index {
                10 => palette::FOREGROUND,
                11 => palette::BACKGROUND,
                12 => palette::CURSOR,
                i => i,
            };
            let rgb = term.colors()[slot].map_or_else(|| palette::default(slot), |c| (c.r, c.g, c.b));
            let (r, g, b) = rgb;
            let end = if *bell { "\x07" } else { "\x1b\\" };
            format!("\x1b]{prefix};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}{end}")
        }
    }
}

pub(crate) const PROGRAM_ANSWER: &str = "\x1b]7501;?\x1b\\";

pub(crate) fn keyboard_flags(mode: TermMode) -> u8 {
    ((mode & TermMode::KITTY_KEYBOARD_PROTOCOL).bits()
        >> TermMode::DISAMBIGUATE_ESC_CODES.bits().trailing_zeros()) as u8
}

/// DECRPM's `Ps`: 1 set, 2 reset, 0 not recognised.
fn private_mode(mode: TermMode, asked: u16) -> u8 {
    let flag = |bit: TermMode| if mode.contains(bit) { 1 } else { 2 };
    match asked {
        1 => flag(TermMode::APP_CURSOR),
        6 => flag(TermMode::ORIGIN),
        7 => flag(TermMode::LINE_WRAP),
        25 => flag(TermMode::SHOW_CURSOR),
        47 | 1047 | 1049 => flag(TermMode::ALT_SCREEN),
        1000 => flag(TermMode::MOUSE_REPORT_CLICK),
        1002 => flag(TermMode::MOUSE_DRAG),
        1003 => flag(TermMode::MOUSE_MOTION),
        1004 => flag(TermMode::FOCUS_IN_OUT),
        1005 => flag(TermMode::UTF8_MOUSE),
        1006 => flag(TermMode::SGR_MOUSE),
        1007 => flag(TermMode::ALTERNATE_SCROLL),
        2004 => flag(TermMode::BRACKETED_PASTE),
        _ => 0,
    }
}
