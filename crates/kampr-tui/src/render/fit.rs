use ratatui::layout::Rect;
use std::time::{Duration, Instant};

/// Ten times the only measurement there is: konsole landed a honoured `CSI 8;r;c t` in under
/// 5 ms with one SIGWINCH (#291), and ghostty and kitty never land at all.
pub const REFUSAL: Duration = Duration::from_millis(50);
const POLL: Duration = Duration::from_millis(5);

/// The terminal, asked rather than assumed. Every method is a measurement #291 proved is
/// answered by all three emulators on this desk.
pub trait Display {
    /// `TIOCGWINSZ`, in cells.
    fn cells(&mut self) -> Option<(u16, u16)>;
    /// `CSI >0q` — which emulator this is.
    fn host(&mut self) -> Option<String>;
    /// The largest grid the display can hold, from `CSI 14t` divided by `CSI 16t`. **Not** from
    /// `TIOCGWINSZ`, whose pixel fields go stale in konsole while `14t` stays honest (#291).
    fn largest(&mut self) -> Option<(u16, u16)>;
    /// `CSI 8;rows;cols t`.
    fn request(&mut self, cols: u16, rows: u16);

    fn settle(&mut self, was: (u16, u16)) -> Option<(u16, u16)> {
        let deadline = Instant::now() + REFUSAL;
        while Instant::now() < deadline {
            if let Some(now) = self.cells()
                && now != was
            {
                return Some(now);
            }
            std::thread::sleep(POLL);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The request was written and nothing moved inside [`REFUSAL`]. ghostty 1.3.1 and kitty
    /// 0.48.2 answer every request this way (#291).
    Ignored,
    /// Refused by this client before it was written: konsole honours a request it cannot show,
    /// so an unguarded rung 2 hands the operator a window they can see a slice of (#291).
    LargerThanDisplay,
    /// `CSI 14t`/`16t` did not answer, so there is nothing to clamp against.
    Unmeasured,
    /// Rung 2 is turned off.
    NotAsked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rung {
    /// 1 — the terminal is already wide enough.
    Fits,
    /// 2 — asked, and it worked.
    Grew { host: Option<String>, to: (u16, u16) },
    /// 3 — crop and pan. On this desktop this is the path, not the fallback.
    CropAndPan { host: Option<String>, refusal: Refusal },
}

impl Rung {
    pub fn number(&self) -> u8 {
        match self {
            Self::Fits => 1,
            Self::Grew { .. } => 2,
            Self::CropAndPan { .. } => 3,
        }
    }

    pub fn report(&self) -> String {
        let host = |h: &Option<String>| h.clone().unwrap_or_else(|| "this terminal".into());
        match self {
            Self::Fits => format!("rung {} · the terminal is wide enough", self.number()),
            Self::Grew { host: h, to } => {
                format!("rung {} · {} grew to {}×{}", self.number(), host(h), to.0, to.1)
            }
            Self::CropAndPan { host: h, refusal } => {
                let why = match refusal {
                    Refusal::Ignored => format!("rung 2 was refused by {}", host(h)),
                    Refusal::LargerThanDisplay => {
                        format!("rung 2 was refused by kampr — {} cannot show it", host(h))
                    }
                    Refusal::Unmeasured => {
                        format!("rung 2 was not tried — {} did not answer CSI 14t", host(h))
                    }
                    Refusal::NotAsked => "rung 2 is off".into(),
                };
                format!("rung {} · crop and pan · {why}", self.number())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Need {
    pub cols: u16,
    pub rows: u16,
}

/// The cells the TUI keeps for itself — sidebar, borders, tab strip, status line — which a
/// resize request has to ask for on top of the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Chrome {
    pub cols: u16,
    pub rows: u16,
}

/// The ladder, in order, each rung falling through to the next.
///
/// **It always answers, and it always says which rung it used** — including that rung 2 was
/// refused, which on ghostty is the answer every time (#291).
pub fn climb(display: &mut dyn Display, need: Need, chrome: Chrome, ask: bool) -> Rung {
    let Some(have) = display.cells() else {
        return Rung::CropAndPan {
            host: None,
            refusal: Refusal::Unmeasured,
        };
    };
    let want = (
        need.cols.saturating_add(chrome.cols),
        need.rows.saturating_add(chrome.rows),
    );
    if have.0 >= want.0 && have.1 >= want.1 {
        return Rung::Fits;
    }
    let host = display.host();
    if !ask {
        return Rung::CropAndPan {
            host,
            refusal: Refusal::NotAsked,
        };
    }
    let Some(largest) = display.largest() else {
        return Rung::CropAndPan {
            host,
            refusal: Refusal::Unmeasured,
        };
    };
    if want.0 > largest.0 || want.1 > largest.1 {
        return Rung::CropAndPan {
            host,
            refusal: Refusal::LargerThanDisplay,
        };
    }
    display.request(want.0, want.1);
    match display.settle(have) {
        Some(to) if to.0 >= want.0 && to.1 >= want.1 => Rung::Grew { host, to },
        _ => Rung::CropAndPan {
            host,
            refusal: Refusal::Ignored,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pan {
    pub col: u16,
    pub row: u16,
}

impl Pan {
    pub fn clamp(self, need: Need, view: (u16, u16)) -> Self {
        Self {
            col: self.col.min(need.cols.saturating_sub(view.0)),
            row: self.row.min(need.rows.saturating_sub(view.1)),
        }
    }
}

/// Where the live grid lands inside the paint rectangle, and what fills the rest of it.
///
/// **Scrollback and the live grid are one continuous surface**, not two panels: history scrolls
/// up out of the top and the live viewport sits at the bottom. **Never letterbox** — the live
/// rows are pinned to the bottom edge and blank space below the last row is a bug, not a layout.
/// Blank *above* is only ever an empty ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub history: Rect,
    pub grid: Rect,
    /// The first history row on screen, as an index into the pane's cached ring.
    pub skip_history: u16,
    /// The first live row on screen. Non-zero only when the live grid is taller than its slot.
    pub skip_grid: u16,
    pub pan: Pan,
    /// How far up the surface this window is, after clamping — what the position indicator says.
    pub scroll: u16,
}

pub fn place(area: Rect, need: Need, history: u16, scroll: u16, pan: Pan) -> Placement {
    let total = history.saturating_add(need.rows);
    let scroll = scroll.min(total.saturating_sub(area.height));
    let bottom = total - scroll;
    let top = bottom.saturating_sub(area.height);
    let live_from = top.max(history);
    let live = bottom.saturating_sub(live_from);
    let past = bottom.min(history).saturating_sub(top);
    let grid = Rect {
        x: area.x,
        y: area.bottom() - live,
        width: area.width,
        height: live,
    };
    Placement {
        history: Rect {
            x: area.x,
            y: grid.y - past,
            width: area.width,
            height: past,
        },
        grid,
        skip_history: top.min(history),
        skip_grid: live_from - history,
        pan: pan.clamp(need, (area.width, live.max(1))),
        scroll,
    }
}

/// The terminal itself, asked once with one batch of queries.
///
/// The batch runs **at start-up, before the event stream owns the keyboard**: a second reader on
/// the same tty would race it for the answer. Every question a client has for its emulator goes
/// in it — the host name and the two pixel sizes the fit ladder needs (#291), and the graphics
/// and sixel claims the image renderer needs (#299) — because two probers on one tty is two
/// readers racing, and the cell size is the answer both of them wanted. `TIOCGWINSZ` is read
/// live, because it is the only one that changes and the only one an ioctl can answer.
#[derive(Debug, Default)]
pub struct Tty {
    host: Option<String>,
    largest: Option<(u16, u16)>,
    cell: Option<(u16, u16)>,
    answers: String,
}

impl Tty {
    pub fn probe() -> Self {
        let Some(answers) = query::converse() else {
            return Self::default();
        };
        let host = query::host(&answers);
        let display = query::pixels(&answers, '4');
        let cell = crate::image::cell_in(&answers);
        let largest = match (display, cell) {
            (Some((h, w)), Some((cw, ch))) if ch > 0 && cw > 0 => Some((w / cw, h / ch)),
            _ => None,
        };
        Self {
            host,
            largest,
            cell,
            answers,
        }
    }

    pub fn named(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// `CSI 16t`, in pixels, `(width, height)`. #299 measured 8x15, 8x17 and 8x18 on the three
    /// emulators here, so a constant in its place is wrong on two of them.
    pub fn cell(&self) -> Option<(u16, u16)> {
        self.cell
    }

    /// Everything the terminal said, for a parser that is not the fit ladder's.
    pub fn answers(&self) -> &str {
        &self.answers
    }
}

impl Display for Tty {
    fn cells(&mut self) -> Option<(u16, u16)> {
        crossterm::terminal::size().ok()
    }

    fn host(&mut self) -> Option<String> {
        self.host.clone()
    }

    fn largest(&mut self) -> Option<(u16, u16)> {
        self.largest
    }

    fn request(&mut self, cols: u16, rows: u16) {
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = write!(out, "\u{1b}[8;{rows};{cols}t");
        let _ = out.flush();
    }
}

mod query {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    /// `O_NONBLOCK` on **our own** open of the terminal. File status flags belong to the open
    /// file description, so this cannot leave the shared tty non-blocking behind us — and a
    /// blocking read on a terminal that never answers is a wedged client.
    const O_NONBLOCK: i32 = 0o4000;
    const DEADLINE: Duration = Duration::from_millis(200);

    /// One write, in this order: the emulator's name, a 1x1 RGB pixel offered for direct
    /// transmission that kitty answers `OK` to and everything else ignores, the display and cell
    /// sizes, and `CSI c` last as the fence every terminal answers so the read has an end that is
    /// not the deadline.
    const QUERY: &str =
        "\u{1b}[>0q\u{1b}_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\u{1b}\\\u{1b}[14t\u{1b}[16t\u{1b}[c";

    pub fn converse() -> Option<String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut tty = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(O_NONBLOCK)
                .open("/dev/tty")
                .ok()?;
            tty.write_all(QUERY.as_bytes()).ok()?;
            tty.flush().ok()?;
            let deadline = Instant::now() + DEADLINE;
            let mut seen = String::new();
            let mut buffer = [0u8; 256];
            while Instant::now() < deadline {
                match tty.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        seen.push_str(&String::from_utf8_lossy(&buffer[..n]));
                        if seen.contains("\u{1b}[?") && seen.ends_with('c') {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
            (!seen.is_empty()).then_some(seen)
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    /// `DCS >| <name> ST` — `ghostty 1.3.1-arch2.2`, `kitty(0.48.2)`, `Konsole 26.04.3` (#291).
    pub fn host(answers: &str) -> Option<String> {
        let start = answers.find(">|")? + 2;
        let rest = &answers[start..];
        let end = rest.find('\u{1b}').unwrap_or(rest.len());
        let name = rest[..end].trim().to_string();
        (!name.is_empty()).then_some(name)
    }

    /// `CSI 4;h;w t` for the window. Found by its own prefix rather than by the first `[` in the
    /// batch, because there are five answers in it and only one of them is this.
    pub fn pixels(answers: &str, kind: char) -> Option<(u16, u16)> {
        let mark = format!("\u{1b}[{kind};");
        let rest = &answers[answers.find(&mark)? + mark.len()..];
        let mut parts = rest[..rest.find('t')?].split(';');
        let h = parts.next()?.trim().parse().ok()?;
        let w = parts.next()?.trim().parse().ok()?;
        Some((h, w))
    }
}
