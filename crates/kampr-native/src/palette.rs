use alacritty_terminal::vte::ansi::NamedColor;

pub(crate) const FOREGROUND: usize = NamedColor::Foreground as usize;
pub(crate) const BACKGROUND: usize = NamedColor::Background as usize;
pub(crate) const CURSOR: usize = NamedColor::Cursor as usize;

/// The default ("Soft") skin of `client/shared/.../theme/TerminalPalette.kt`: its ground, its ink
/// (which is also the colour the client's block cursor takes on a default cell), and its 16 slots.
/// Every skin keeps a dark ground (ADR 0009), so a program that picks a theme from the answer
/// picks the right one under all of them.
const GROUND: (u8, u8, u8) = (0x0b, 0x0d, 0x12);
const INK: (u8, u8, u8) = (0xdd, 0xe3, 0xee);
const SLOTS: [(u8, u8, u8); 16] = [
    (0x1b, 0x1f, 0x27),
    (0xf2, 0x70, 0x7a),
    (0x5f, 0xd6, 0x8f),
    (0xe9, 0xc4, 0x6a),
    (0x7f, 0xb0, 0xff),
    (0xc7, 0x9b, 0xe0),
    (0x5f, 0xd2, 0xd2),
    (0xc6, 0xcc, 0xd9),
    (0x7c, 0x84, 0x96),
    (0xff, 0x8e, 0x96),
    (0x86, 0xe9, 0xab),
    (0xf7, 0xd9, 0x8c),
    (0xa6, 0xc8, 0xff),
    (0xdd, 0xb8, 0xf0),
    (0x8c, 0xe7, 0xe7),
    (0xf2, 0xf5, 0xfa),
];

pub(crate) fn default(slot: usize) -> (u8, u8, u8) {
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match slot {
        0..16 => SLOTS[slot],
        16..232 => {
            let i = slot - 16;
            (STEPS[i / 36], STEPS[(i / 6) % 6], STEPS[i % 6])
        }
        232..=255 => {
            let v = (8 + (slot - 232) * 10) as u8;
            (v, v, v)
        }
        BACKGROUND => GROUND,
        _ => INK,
    }
}
