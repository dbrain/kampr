use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A keymap that persists. **Copy mode and resize mode are modal** — a second keymap that wins
/// until it is closed, taking no prefix — and so is the navigate mode a workspace picker opens
/// (#289, #290). A router that assumes "prefix, then one key, then back to the pane" is wrong
/// for all three, which is why this is a stack rather than a flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Pane,
    Prefix,
    Copy,
    Resize,
    Navigate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Down,
    Up,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Keybinds,
    Settings,
    Detach,
    ReloadConfig,
    OpenNotificationTarget,
    NewWorkspace,
    NewWorktree,
    RenameWorkspace,
    CloseWorkspace,
    NewTab,
    RenameTab,
    PreviousTab,
    NextTab,
    SwitchTab(u8),
    CloseTab,
    SplitVertical,
    SplitHorizontal,
    ClosePane,
    RenamePane,
    EditScrollback,
    ZoomPane,
    ToggleSidebar,
    /// The triage screen a desk cannot draw: every node, every workspace, every pane, blocked
    /// first. #289's table binds no `prefix+shift+h`, so this is one of the handful Kampr claims
    /// for itself rather than a departure from herdr's keymap.
    HerdView,
    FocusPane(Dir),
    CyclePaneNext,
    CyclePanePrevious,
    LastPane,
    ToggleView,
    ToggleMouse,
    Pan(Dir),
    PanEdge(Dir),
    /// Up out of the live viewport and into the ring. Scrollback and the live grid are one
    /// surface, so this is the same window moving rather than a second panel.
    Scroll(Dir),
    PanReset,
    Move(Dir),
    WordNext,
    WordBack,
    WordEnd,
    ParagraphNext,
    ParagraphBack,
    SearchForward,
    SearchBack,
    RepeatSearch,
    RepeatSearchBack,
    Select,
    Copy,
    OpenWorkspace,
    SwitchWorkspace(u8),
    /// Kampr's own mosaic: put the pane the navigator is on beside the one already on screen.
    /// It may come from a different session on a different host, which is the whole reason this
    /// client exists — and it needs no protocol support beyond watching several panes at once.
    PinPane,
    ClearMosaic,
    Wider,
    Narrower,
    Taller,
    Shorter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bind {
    Do(Action),
    /// Push a keymap that persists until it is closed.
    Enter(Mode),
    /// Close the keymap on top of the stack.
    Leave,
}

fn ch(key: KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) => Some(c),
        _ => None,
    }
}

fn shifted(key: KeyEvent, want: char) -> bool {
    ch(key).is_some_and(|c| {
        c == want.to_ascii_uppercase() || (c == want && key.modifiers.contains(KeyModifiers::SHIFT))
    })
}

fn plain(key: KeyEvent, want: char) -> bool {
    ch(key) == Some(want) && !key.modifiers.contains(KeyModifiers::CONTROL)
}

/// herdr's own prefix, and this client's default. It is a value rather than a constant because
/// the escape hatch moves it: a prefix kampr does not claim is a prefix the pane's own program
/// gets, which is as close as a cell-grid client can come to "take the node's keymap instead".
pub const HERDR_PREFIX: KeyEvent = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);

pub fn same(key: KeyEvent, prefix: KeyEvent) -> bool {
    key.code == prefix.code && key.modifiers == prefix.modifiers
}

/// #289's table, verbatim, plus the handful of binds herdr ships **unbound** that Kampr needs
/// for panning, the view swap and the mouse toggle. `prefix+space` is `last_pane`, which is the
/// operator's own config rather than the shipped default (#289).
pub fn prefix(key: KeyEvent) -> Option<Bind> {
    use Action::*;
    if key.code == KeyCode::Esc {
        return Some(Bind::Leave);
    }
    if key.code == KeyCode::Tab {
        return Some(Bind::Do(CyclePaneNext));
    }
    if key.code == KeyCode::BackTab {
        return Some(Bind::Do(CyclePanePrevious));
    }
    match key.code {
        KeyCode::Left => return Some(Bind::Do(Pan(Dir::Left))),
        KeyCode::Right => return Some(Bind::Do(Pan(Dir::Right))),
        KeyCode::Up => return Some(Bind::Do(Pan(Dir::Up))),
        KeyCode::Down => return Some(Bind::Do(Pan(Dir::Down))),
        KeyCode::Home => return Some(Bind::Do(PanEdge(Dir::Left))),
        KeyCode::End => return Some(Bind::Do(PanEdge(Dir::Right))),
        KeyCode::PageUp => return Some(Bind::Do(Scroll(Dir::Up))),
        KeyCode::PageDown => return Some(Bind::Do(Scroll(Dir::Down))),
        _ => {}
    }
    if let Some(c) = ch(key)
        && let Some(n) = c.to_digit(10)
        && (1..=9).contains(&n)
        && !key.modifiers.contains(KeyModifiers::SHIFT)
    {
        return Some(Bind::Do(SwitchTab(n as u8)));
    }
    let bind = if shifted(key, 'r') {
        Bind::Do(ReloadConfig)
    } else if shifted(key, 'n') {
        Bind::Do(NewWorkspace)
    } else if shifted(key, 'g') {
        Bind::Do(NewWorktree)
    } else if shifted(key, 'w') {
        Bind::Do(RenameWorkspace)
    } else if shifted(key, 'd') {
        Bind::Do(CloseWorkspace)
    } else if shifted(key, 't') {
        Bind::Do(RenameTab)
    } else if shifted(key, 'x') {
        Bind::Do(CloseTab)
    } else if shifted(key, 'p') {
        Bind::Do(RenamePane)
    } else if shifted(key, 'h') {
        Bind::Do(HerdView)
    } else if shifted(key, 'v') {
        Bind::Do(ToggleView)
    } else if plain(key, '?') {
        Bind::Do(Keybinds)
    } else if plain(key, 's') {
        Bind::Do(Settings)
    } else if plain(key, 'q') {
        Bind::Do(Detach)
    } else if plain(key, 'o') {
        Bind::Do(OpenNotificationTarget)
    } else if plain(key, 'w') || plain(key, 'g') {
        Bind::Enter(Mode::Navigate)
    } else if plain(key, 'c') {
        Bind::Do(NewTab)
    } else if plain(key, ',') {
        Bind::Do(RenameTab)
    } else if plain(key, 'p') {
        Bind::Do(PreviousTab)
    } else if plain(key, 'n') {
        Bind::Do(NextTab)
    } else if plain(key, 'v') {
        Bind::Do(SplitVertical)
    } else if plain(key, '-') {
        Bind::Do(SplitHorizontal)
    } else if plain(key, 'x') {
        Bind::Do(ClosePane)
    } else if plain(key, 'e') {
        Bind::Do(EditScrollback)
    } else if plain(key, '[') {
        Bind::Enter(Mode::Copy)
    } else if plain(key, 'z') {
        Bind::Do(ZoomPane)
    } else if plain(key, 'r') {
        Bind::Enter(Mode::Resize)
    } else if plain(key, 'b') {
        Bind::Do(ToggleSidebar)
    } else if plain(key, 'm') {
        Bind::Do(ToggleMouse)
    } else if shifted(key, 'm') {
        Bind::Do(ClearMosaic)
    } else if plain(key, ' ') {
        Bind::Do(LastPane)
    } else if plain(key, '0') {
        Bind::Do(PanReset)
    } else if plain(key, 'h') {
        Bind::Do(FocusPane(Dir::Left))
    } else if plain(key, 'j') {
        Bind::Do(FocusPane(Dir::Down))
    } else if plain(key, 'k') {
        Bind::Do(FocusPane(Dir::Up))
    } else if plain(key, 'l') {
        Bind::Do(FocusPane(Dir::Right))
    } else {
        return None;
    };
    Some(bind)
}

/// #290's footer is the whole grammar: `h/j/k/l w/b/e { } move - / ? search - n/N repeat -
/// v/space select - y/enter copy - q/esc exit`.
pub fn copy(key: KeyEvent) -> Option<Bind> {
    use Action::*;
    let bind = match key.code {
        KeyCode::Esc => Bind::Leave,
        KeyCode::Enter => Bind::Do(Copy),
        KeyCode::Left => Bind::Do(Move(Dir::Left)),
        KeyCode::Down => Bind::Do(Move(Dir::Down)),
        KeyCode::Up => Bind::Do(Move(Dir::Up)),
        KeyCode::Right => Bind::Do(Move(Dir::Right)),
        KeyCode::PageUp => Bind::Do(Scroll(Dir::Up)),
        KeyCode::PageDown => Bind::Do(Scroll(Dir::Down)),
        KeyCode::Char(c) => match c {
            'q' => Bind::Leave,
            'h' => Bind::Do(Move(Dir::Left)),
            'j' => Bind::Do(Move(Dir::Down)),
            'k' => Bind::Do(Move(Dir::Up)),
            'l' => Bind::Do(Move(Dir::Right)),
            'w' => Bind::Do(WordNext),
            'b' => Bind::Do(WordBack),
            'e' => Bind::Do(WordEnd),
            '{' => Bind::Do(ParagraphBack),
            '}' => Bind::Do(ParagraphNext),
            '/' => Bind::Do(SearchForward),
            '?' => Bind::Do(SearchBack),
            'n' => Bind::Do(RepeatSearch),
            'N' => Bind::Do(RepeatSearchBack),
            'v' | ' ' => Bind::Do(Select),
            'y' => Bind::Do(Copy),
            _ => return None,
        },
        _ => return None,
    };
    Some(bind)
}

/// #290's footer: `h/l width - j/k height - esc done`. **What it resizes is kampr's own mosaic
/// split, never the pane** — ADR 0002 — so a herdr user's fingers land somewhere honest instead
/// of on a code path that does not exist.
pub fn resize(key: KeyEvent) -> Option<Bind> {
    use Action::*;
    let bind = match key.code {
        KeyCode::Esc => Bind::Leave,
        KeyCode::Left => Bind::Do(Narrower),
        KeyCode::Right => Bind::Do(Wider),
        KeyCode::Down => Bind::Do(Shorter),
        KeyCode::Up => Bind::Do(Taller),
        KeyCode::Char('h') => Bind::Do(Narrower),
        KeyCode::Char('l') => Bind::Do(Wider),
        KeyCode::Char('j') => Bind::Do(Shorter),
        KeyCode::Char('k') => Bind::Do(Taller),
        KeyCode::Char('q') => Bind::Leave,
        _ => return None,
    };
    Some(bind)
}

/// #289's navigate mode: the second keymap that wins while `prefix+w`/`prefix+g` is open, and
/// takes no prefix.
pub fn navigate(key: KeyEvent) -> Option<Bind> {
    use Action::*;
    if let Some(c) = ch(key)
        && let Some(n) = c.to_digit(10)
        && (1..=9).contains(&n)
    {
        return Some(Bind::Do(SwitchWorkspace(n as u8)));
    }
    let bind = match key.code {
        KeyCode::Esc => Bind::Leave,
        KeyCode::Enter => Bind::Do(OpenWorkspace),
        KeyCode::Tab => Bind::Do(CyclePaneNext),
        KeyCode::BackTab => Bind::Do(CyclePanePrevious),
        KeyCode::Up => Bind::Do(Move(Dir::Up)),
        KeyCode::Down => Bind::Do(Move(Dir::Down)),
        KeyCode::Left => Bind::Do(Move(Dir::Left)),
        KeyCode::Right => Bind::Do(Move(Dir::Right)),
        KeyCode::Char('q') => Bind::Leave,
        KeyCode::Char(' ') => Bind::Do(PinPane),
        KeyCode::Char('h') => Bind::Do(Move(Dir::Left)),
        KeyCode::Char('j') => Bind::Do(Move(Dir::Down)),
        KeyCode::Char('k') => Bind::Do(Move(Dir::Up)),
        KeyCode::Char('l') => Bind::Do(Move(Dir::Right)),
        _ => return None,
    };
    Some(bind)
}

pub fn lookup(mode: Mode, key: KeyEvent) -> Option<Bind> {
    match mode {
        Mode::Pane => None,
        Mode::Prefix => prefix(key),
        Mode::Copy => copy(key),
        Mode::Resize => resize(key),
        Mode::Navigate => navigate(key),
    }
}

/// The footers herdr draws, word for word (#290), so the strip under a Kampr pane says what the
/// strip under a herdr pane says.
pub fn footer(mode: Mode) -> Option<&'static str> {
    match mode {
        Mode::Pane => None,
        Mode::Prefix => Some("PREFIX  esc cancel · ctrl+b send prefix · w sidebar · shift+h herd · ? help"),
        Mode::Copy => Some(
            "COPY  h/j/k/l w/b/e { } move · / ? search · n/N repeat · v/space select · y/enter copy · q/esc exit",
        ),
        Mode::Resize => Some("RESIZE  h/l width · j/k height · esc done · kampr's own split, never the pane"),
        Mode::Navigate => Some(
            "NAVIGATE the sidebar  esc back · up/down row · enter open · space beside · tab cycle · 1-9 workspace",
        ),
    }
}
