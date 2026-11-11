//! Result titles and subtitles with the characters the query matched in the accent color.
//!
//! Slint's `StyledText` draws the colors but cannot elide, so the text is first shortened
//! here with the same width estimate the path shortening uses. Rows without a match keep
//! the plain `Text`, which elides precisely.
use mytools_plugin_api::highlight::highlights;
use slint::{Color, StyledText};

/// Whether `text` has anything to highlight for `query`.
pub fn any(text: &str, query: &str) -> bool {
    !highlights(text, query).is_empty()
}

/// `text` as styled text that fits `width`, the matched characters in `color`.
pub fn styled(text: &str, query: &str, color: Color, width: f32, font_size: f32) -> StyledText {
    let shown = crate::elide::fit_end(text, width, font_size);
    StyledText::from_markdown(&markup(&shown, query, color))
        .unwrap_or_else(|_| StyledText::from_plain_text(&shown))
}

/// CommonMark for `text` with matches wrapped in `<font color>`. Every ASCII punctuation
/// character is escaped, so titles such as "1. Notes", "- draft" or "a*b" stay plain text.
fn markup(text: &str, query: &str, color: Color) -> String {
    let ranges = highlights(text, query);
    let open = format!(
        "<font color=\"#{:02x}{:02x}{:02x}\">",
        color.red(),
        color.green(),
        color.blue()
    );
    let mut markup = String::with_capacity(text.len() * 2);
    let mut inside = false;
    // Leading spaces would start a code block; a result title never needs them.
    let offset = text.chars().count() - text.trim_start().chars().count();
    for (index, c) in text.trim_start().chars().enumerate() {
        let highlighted = ranges.iter().any(|range| range.contains(&(index + offset)));
        if highlighted != inside {
            markup.push_str(if highlighted { &open } else { "</font>" });
            inside = highlighted;
        }
        match c {
            c if c.is_ascii_punctuation() => {
                markup.push('\\');
                markup.push(c);
            }
            c if c.is_whitespace() => markup.push(' '),
            c => markup.push(c),
        }
    }
    if inside {
        markup.push_str("</font>");
    }
    markup
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCENT: Color = Color::from_rgb_u8(0x94, 0xde, 0xd1);

    #[test]
    fn wraps_matches_and_escapes_markdown() {
        assert_eq!(
            markup("Visual Studio Code", "code", ACCENT),
            "Visual Studio <font color=\"#94ded1\">Code</font>"
        );
        assert_eq!(markup("1. a*b <c>", "zzz", ACCENT), r"1\. a\*b \<c\>");
        assert_eq!(
            markup("C:\\work\\app", "work", ACCENT),
            "C\\:\\\\<font color=\"#94ded1\">work</font>\\\\app"
        );
    }

    #[test]
    fn every_markup_parses_and_keeps_the_text() {
        for (text, query) in [
            ("Visual Studio Code", "code"),
            (
                "# heading - [link](x) `code` _em_ ~~s~~ <u>u</u> &amp;",
                "link",
            ),
            ("  1. indented", "ind"),
            ("记事本", "jsb"),
        ] {
            let styled = StyledText::from_markdown(&markup(text, query, ACCENT));
            assert!(styled.is_ok(), "{text}");
        }
    }
}
