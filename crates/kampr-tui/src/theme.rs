use kampr_term::{Cell, Color as Ink};
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub bg: Color,
    pub bar: Color,
    pub surface: Color,
    pub raise: Color,
    pub line: Color,
    pub text: Color,
    pub dim: Color,
    pub mute: Color,
    pub accent: Color,
    pub accent_hi: Color,
    pub on_accent: Color,
    pub accent_soft: Color,
    pub blocked: Color,
    pub blocked_bg: Color,
    pub working: Color,
    pub idle: Color,
    pub done: Color,
    /// Whether the pane's own 16 slots are remapped to the Phosphor terminal skin. **Off by
    /// default**: in a real terminal the emulator already owns the ground and those slots, and
    /// the operator's goal is a herdr clone, so slots 0-15 and `Default` pass through as
    /// ordinary SGR. ADR 0009's actual concern — that absolute values stay absolute — is kept
    /// either way, because 16-255 and truecolour are never remapped.
    pub skinned: bool,
}

const fn rgb(v: u32) -> Color {
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const PHOSPHOR: Theme = Theme {
    bg: rgb(0x0A0B0A),
    bar: rgb(0x0C0E0C),
    surface: rgb(0x101210),
    raise: rgb(0x171A17),
    line: rgb(0x1D211D),
    text: rgb(0xCDD6CD),
    dim: rgb(0x8F998F),
    mute: rgb(0x7D8A7D),
    accent: rgb(0xF5C542),
    accent_hi: rgb(0xFFD968),
    on_accent: rgb(0x0A0B0A),
    accent_soft: rgb(0x16130A),
    blocked: rgb(0xFF5F56),
    blocked_bg: rgb(0x150D0C),
    working: rgb(0xF5C542),
    idle: rgb(0x7D8B7D),
    done: rgb(0x57C46B),
    skinned: false,
};

const SKIN_GROUND: Color = rgb(0x050705);
const SKIN_INK: Color = rgb(0xC9D5C7);
const SKIN_SLOTS: [Color; 16] = [
    rgb(0x131A13),
    rgb(0xFF5F4A),
    rgb(0x57C46B),
    rgb(0xF5C542),
    rgb(0x58A6B8),
    rgb(0xD98BA0),
    rgb(0x7FE0C8),
    rgb(0xC4CFC2),
    rgb(0x6F7D6D),
    rgb(0xFF8A72),
    rgb(0x7FDC8E),
    rgb(0xFFD968),
    rgb(0x7FC8D8),
    rgb(0xEEADBE),
    rgb(0xA6F0DE),
    rgb(0xE9F0E7),
];

/// The classic SGR names, so a slot the operator has themed in their own emulator arrives as
/// `30`-`37`/`90`-`97` rather than as an absolute value this client picked.
const ANSI: [Color; 16] = [
    Color::Black,
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::DarkGray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
    Color::White,
];

impl Theme {
    pub fn skinned(self, skinned: bool) -> Self {
        Self { skinned, ..self }
    }

    fn ink(&self, ink: Ink, ground: bool) -> Color {
        match (ink, self.skinned) {
            (Ink::Default, false) => Color::Reset,
            (Ink::Default, true) if ground => SKIN_GROUND,
            (Ink::Default, true) => SKIN_INK,
            (Ink::Indexed(i), true) if i < 16 => SKIN_SLOTS[i as usize],
            (Ink::Indexed(i), false) if i < 16 => ANSI[i as usize],
            (Ink::Indexed(i), _) => Color::Indexed(i),
            (Ink::Rgb(r, g, b), _) => Color::Rgb(r, g, b),
        }
    }

    /// Reverse, dim and hidden ride as modifiers rather than being composited here: the
    /// operator's emulator does that itself, and doing it twice is what stops this being a clone.
    pub fn cell(&self, cell: &Cell) -> Style {
        let mut modifier = Modifier::empty();
        let a = cell.attrs;
        modifier.set(Modifier::BOLD, a.bold);
        modifier.set(Modifier::DIM, a.dim);
        modifier.set(Modifier::ITALIC, a.italic);
        modifier.set(Modifier::UNDERLINED, a.underline);
        modifier.set(Modifier::SLOW_BLINK, a.blink);
        modifier.set(Modifier::REVERSED, a.reverse);
        modifier.set(Modifier::CROSSED_OUT, a.strike);
        modifier.set(Modifier::HIDDEN, a.hidden);
        Style::default()
            .fg(self.ink(cell.fg, false))
            .bg(self.ink(cell.bg, true))
            .add_modifier(modifier)
    }

    pub fn status(&self, status: kampr_core::provider::AgentStatus) -> Color {
        use kampr_core::provider::AgentStatus::*;
        match status {
            Blocked => self.blocked,
            Working => self.working,
            Done => self.done,
            Idle => self.idle,
            Unknown => self.mute,
        }
    }
}
