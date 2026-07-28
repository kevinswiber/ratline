use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// The key vocabulary the UI state reducers speak. Reducers never see
/// crossterm types, keeping them pure and easy to test.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Space,
    CtrlC,
    CtrlA,
    CtrlE,
    CtrlU,
    CtrlW,
    Char(char),
}

/// Map a crossterm event; None for anything the UIs ignore, including key
/// release events (delivered on Windows and kitty-protocol terminals).
pub fn from_crossterm(ev: KeyEvent) -> Option<Key> {
    if ev.kind == KeyEventKind::Release {
        return None;
    }
    if ev.modifiers.contains(KeyModifiers::CONTROL) {
        return match ev.code {
            KeyCode::Char('c') => Some(Key::CtrlC),
            KeyCode::Char('a') => Some(Key::CtrlA),
            KeyCode::Char('e') => Some(Key::CtrlE),
            KeyCode::Char('u') => Some(Key::CtrlU),
            KeyCode::Char('w') => Some(Key::CtrlW),
            _ => None,
        };
    }
    match ev.code {
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::PageUp => Some(Key::PageUp),
        KeyCode::PageDown => Some(Key::PageDown),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::BackTab => Some(Key::BackTab),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Delete => Some(Key::Delete),
        KeyCode::Char(' ') => Some(Key::Space),
        KeyCode::Char(c) => Some(Key::Char(c)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn control_chars_map_to_named_keys() {
        assert_eq!(
            from_crossterm(press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Key::CtrlC)
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            Some(Key::CtrlA)
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            Some(Key::CtrlU)
        );
    }

    #[test]
    fn plain_chars_pass_through() {
        assert_eq!(
            from_crossterm(press(KeyCode::Char('c'), KeyModifiers::NONE)),
            Some(Key::Char('c'))
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Char('C'), KeyModifiers::SHIFT)),
            Some(Key::Char('C'))
        );
    }

    #[test]
    fn named_keys_map() {
        assert_eq!(
            from_crossterm(press(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Key::Esc)
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Key::Enter)
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Up, KeyModifiers::NONE)),
            Some(Key::Up)
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(Key::Space)
        );
    }

    #[test]
    fn every_watch_binding_survives_the_crossterm_map() {
        // The Windows input path: watch's navigation and view keys must
        // arrive as the same Key variants the unix scanner produces.
        for (code, key) in [
            (KeyCode::Up, Key::Up),
            (KeyCode::Down, Key::Down),
            (KeyCode::Left, Key::Left),
            (KeyCode::Right, Key::Right),
            (KeyCode::Home, Key::Home),
            (KeyCode::End, Key::End),
            (KeyCode::PageUp, Key::PageUp),
            (KeyCode::PageDown, Key::PageDown),
            (KeyCode::Esc, Key::Esc),
        ] {
            assert_eq!(from_crossterm(press(code, KeyModifiers::NONE)), Some(key));
        }
        // The snapshot key, the resume alias, and the gutter toggle
        // arrive shifted; the freeze key plain.
        assert_eq!(
            from_crossterm(press(KeyCode::Char('S'), KeyModifiers::SHIFT)),
            Some(Key::Char('S'))
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Char('D'), KeyModifiers::SHIFT)),
            Some(Key::Char('D'))
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Char('F'), KeyModifiers::SHIFT)),
            Some(Key::Char('F'))
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Char('p'), KeyModifiers::NONE)),
            Some(Key::Char('p'))
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Char('c'), KeyModifiers::NONE)),
            Some(Key::Char('c'))
        );
        assert_eq!(
            from_crossterm(press(KeyCode::Char('t'), KeyModifiers::NONE)),
            Some(Key::Char('t'))
        );
        // The scrub keys: shifted and unshifted spellings both survive.
        for (ch, mods) in [
            ('<', KeyModifiers::SHIFT),
            ('>', KeyModifiers::SHIFT),
            (',', KeyModifiers::NONE),
            ('.', KeyModifiers::NONE),
        ] {
            assert_eq!(
                from_crossterm(press(KeyCode::Char(ch), mods)),
                Some(Key::Char(ch))
            );
        }
    }

    #[test]
    fn release_events_are_ignored() {
        let mut ev = press(KeyCode::Char('c'), KeyModifiers::NONE);
        ev.kind = KeyEventKind::Release;
        assert_eq!(from_crossterm(ev), None);
    }
}
