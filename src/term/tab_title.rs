//! The terminal tab title, owned for the session: the title stack is
//! pushed on entry, the tab set to the dashboard's role text, and the
//! stack popped on exit — the `ThemeNotifyGuard` shape for one piece
//! of terminal state. Best-effort by design: a terminal without the
//! title stack ignores the pop and keeps the last title, and no
//! attempt is made to query the old title back (replies vary and are
//! widely disabled).

use std::io::Write;

/// The glyph that marks a rat-owned tab.
pub const MARKER: &str = "▞";

const PUSH_STACK: &[u8] = b"\x1b[22;2t";
const POP_STACK: &[u8] = b"\x1b[23;2t";

/// Strip anything that could terminate or escape the OSC payload —
/// control bytes, DEL, and the C1 range — so a role text containing
/// BEL or ESC cannot break out of the sequence.
fn sanitize(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// The tab's text: the marker, then the role text — or the fallback
/// identity (the file stem) when the role is silent or sanitizes away.
pub fn tab_title_text(role: Option<&str>, stem: &str) -> String {
    let body = role
        .map(sanitize)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| sanitize(stem).trim().to_string());
    format!("{MARKER} {body}")
}

pub struct TabTitleGuard<W: Write> {
    out: W,
    last: String,
}

impl<W: Write> TabTitleGuard<W> {
    /// Push the terminal's title stack, then set the first title.
    /// One guard per session; dropping it pops the stack.
    pub fn set_over_stack(mut out: W, text: &str) -> std::io::Result<Self> {
        out.write_all(PUSH_STACK)?;
        let mut guard = TabTitleGuard {
            out,
            last: String::new(),
        };
        guard.set(text)?;
        Ok(guard)
    }

    /// Idempotent per text: re-setting the same title writes nothing,
    /// so a per-slice caller keeps the wire quiet.
    pub fn set(&mut self, text: &str) -> std::io::Result<()> {
        if self.last == text {
            return Ok(());
        }
        self.out.write_all(b"\x1b]2;")?;
        self.out.write_all(text.as_bytes())?;
        self.out.write_all(b"\x07")?;
        self.out.flush()?;
        self.last = text.to_string();
        Ok(())
    }
}

impl<W: Write> Drop for TabTitleGuard<W> {
    fn drop(&mut self) {
        let _ = self.out.write_all(POP_STACK);
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_is_marker_plus_role_or_the_stem() {
        assert_eq!(tab_title_text(Some("3 failing"), "deploy"), "▞ 3 failing");
        assert_eq!(tab_title_text(None, "deploy"), "▞ deploy");
        // Control bytes cannot terminate or escape the sequence.
        assert_eq!(
            tab_title_text(Some("a\u{7}b\u{1b}]0;evil\u{7}"), "deploy"),
            "▞ ab]0;evil"
        );
        // A role that sanitizes to nothing is silence, not a title.
        assert_eq!(tab_title_text(Some("\u{7}  \u{1b}"), "deploy"), "▞ deploy");
    }

    #[test]
    fn the_guard_pushes_sets_and_pops_and_never_repeats_itself() {
        let mut bytes: Vec<u8> = Vec::new();
        {
            let mut guard =
                TabTitleGuard::set_over_stack(&mut bytes, "▞ one").expect("write to a vec");
            guard.set("▞ one").expect("idempotent");
            guard.set("▞ two").expect("second title");
        }
        let text = String::from_utf8(bytes).expect("utf8");
        assert_eq!(
            text, "\u{1b}[22;2t\u{1b}]2;▞ one\u{7}\u{1b}]2;▞ two\u{7}\u{1b}[23;2t",
            "push, first set, ONE re-set, pop — nothing else"
        );
    }
}
