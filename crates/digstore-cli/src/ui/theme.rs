//! Visual vocabulary for the CLI: styles, status markers, and verb formatting.
//! All formatting takes an explicit `color` flag so output is deterministic and
//! testable; when `color` is false the returned string contains no ANSI.

use anstyle::{AnsiColor, Style};

/// A status marker for a working-tree entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Staged,    // '+'
    Modified,  // '~'
    Untracked, // '?'
    Removed,   // '-'
}

impl Marker {
    pub fn symbol(self) -> char {
        match self {
            Marker::Staged => '+',
            Marker::Modified => '~',
            Marker::Untracked => '?',
            Marker::Removed => '-',
        }
    }
    fn style(self) -> Style {
        match self {
            Marker::Staged => Style::new().fg_color(Some(AnsiColor::Green.into())),
            Marker::Modified => Style::new().fg_color(Some(AnsiColor::Yellow.into())),
            Marker::Untracked => Style::new().dimmed(),
            Marker::Removed => Style::new().fg_color(Some(AnsiColor::Red.into())),
        }
    }
}

/// Wrap `text` in `style` when `color`, else return it unchanged.
pub fn paint(color: bool, style: Style, text: &str) -> String {
    if color {
        format!("{}{}{}", style.render(), text, style.render_reset())
    } else {
        text.to_string()
    }
}

/// A right-aligned, colored "verb" line, cargo-style: `   Staging  3 resources`.
/// The verb column is right-aligned to width 10.
pub fn verb_line(color: bool, verb: &str, msg: &str) -> String {
    let styled = paint(
        color,
        Style::new().fg_color(Some(AnsiColor::Green.into())).bold(),
        verb,
    );
    // pad based on the *unstyled* verb width so alignment is correct with color on.
    let pad = 10usize.saturating_sub(verb.chars().count());
    format!("{}{}  {}", " ".repeat(pad), styled, msg)
}

/// `+ key` / `~ key` / `? key`, the marker colored.
pub fn marker_line(color: bool, marker: Marker, text: &str) -> String {
    let sym = paint(color, marker.style(), &marker.symbol().to_string());
    format!("{} {}", sym, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_strips_ansi() {
        assert_eq!(paint(false, Marker::Staged.style(), "x"), "x");
        assert!(!verb_line(false, "Staging", "3 files").contains('\u{1b}'));
        assert!(!marker_line(false, Marker::Modified, "a.txt").contains('\u{1b}'));
    }

    #[test]
    fn verb_is_right_aligned_to_width_10() {
        let line = verb_line(false, "Staging", "3 files");
        // 10 - 7 = 3 leading spaces, then "Staging", then two spaces, then msg.
        assert_eq!(line, "   Staging  3 files");
    }

    #[test]
    fn marker_line_uses_expected_symbol() {
        assert_eq!(marker_line(false, Marker::Staged, "a"), "+ a");
        assert_eq!(marker_line(false, Marker::Untracked, "b"), "? b");
    }

    #[test]
    fn color_on_emits_ansi_and_resets() {
        let s = paint(true, Marker::Staged.style(), "x");
        assert!(s.contains('\u{1b}'));
        assert!(s.contains('x'));
    }
}
