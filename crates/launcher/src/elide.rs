//! Shortens long paths in result subtitles by removing folders from the middle, so the drive
//! and the most specific folders stay visible. Slint can only elide at the end, which hides
//! exactly the part that tells similar results apart.
//!
//! Slint offers no text measurement to Rust, so widths are estimated from per-character
//! advances of the UI font (Segoe UI, Microsoft YaHei for CJK). Estimates err slightly wide;
//! if one is still short, Slint's own end elision trims the last few characters.

const ELLIPSIS: &str = "…";
const SAFETY: f32 = 1.06;

/// Returns `text` unchanged when it fits `width` (logical px) at `font_size`, or when it
/// contains no path; otherwise removes the fewest middle folders, keeping the end first.
pub fn fit_path(text: &str, width: f32, font_size: f32) -> String {
    if width <= 0.0 || font_size <= 0.0 || measure(text, font_size) <= width {
        return text.to_owned();
    }
    let Some(start) = path_start(text) else {
        return text.to_owned();
    };
    let (prefix, path) = text.split_at(start);
    let separator = if path.contains('\\') { '\\' } else { '/' };
    let segments: Vec<&str> = path.split(separator).collect();
    // UNC (\\server\share) and Unix (/home) paths start with empty segments.
    let mut head = segments.iter().take_while(|s| s.is_empty()).count() + 1;
    let mut tail = segments.len().saturating_sub(1);
    if head >= tail {
        return text.to_owned();
    }
    let compose = |head: usize, tail: usize| {
        let mut shortened = String::from(prefix);
        for segment in &segments[..head] {
            shortened.push_str(segment);
            shortened.push(separator);
        }
        shortened.push_str(ELLIPSIS);
        for segment in &segments[tail..] {
            shortened.push(separator);
            shortened.push_str(segment);
        }
        shortened
    };
    let fits = |candidate: &str| measure(candidate, font_size) <= width;
    // Keep as many trailing folders as fit, then give leftover room back to the head.
    while tail > head + 1 && fits(&compose(head, tail - 1)) {
        tail -= 1;
    }
    while head + 1 < tail && fits(&compose(head + 1, tail)) {
        head += 1;
    }
    compose(head, tail)
}

/// Byte offset of the first `X:\`, `\\` or `/x` path that starts the text or follows a space.
fn path_start(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    (0..bytes.len()).find(|&i| {
        if i > 0 && bytes[i - 1] != b' ' {
            return false;
        }
        let rest = &bytes[i..];
        let drive = rest.len() >= 3
            && rest[0].is_ascii_alphabetic()
            && rest[1] == b':'
            && matches!(rest[2], b'\\' | b'/');
        let unc = rest.starts_with(br"\\") && rest.len() > 2 && rest[2] != b'\\';
        let unix = rest.len() >= 2 && rest[0] == b'/' && rest[1].is_ascii_alphanumeric();
        drive || unc || unix
    })
}

fn measure(text: &str, font_size: f32) -> f32 {
    text.chars().map(advance).sum::<f32>() * font_size * SAFETY
}

/// Approximate advance width in em.
fn advance(c: char) -> f32 {
    match c {
        'i' | 'j' | 'l' | 'I' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' | '·' => 0.26,
        ' ' | 'f' | 'r' | 't' | '(' | ')' | '[' | ']' | '{' | '}' | '-' => 0.36,
        '\\' | '/' => 0.4,
        'm' | 'w' | 'M' | 'W' => 0.86,
        'A'..='Z' | '_' | '%' | '&' | '@' | '#' => 0.64,
        '0'..='9' | '…' => 0.58,
        c if c.is_ascii() => 0.55,
        c if is_wide(c) => 1.0,
        _ => 0.62,
    }
}

fn is_wide(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{115F}'
        | '\u{2E80}'..='\u{A4CF}'
        | '\u{AC00}'..='\u{D7A3}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FE30}'..='\u{FE4F}'
        | '\u{FF00}'..='\u{FF60}'
        | '\u{FFE0}'..='\u{FFE6}'
        | '\u{1F300}'..='\u{1FAFF}'
        | '\u{20000}'..='\u{3FFFD}')
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: &str = r"Everything · Folder · C:\git\GitHub\WiseTechGlobal4\Glow.wt-WI01134353\DotNet\Business\OceanCarrier\Business.OceanCarrier.Test\Shipment";

    #[test]
    fn fitting_or_pathless_text_is_unchanged() {
        assert_eq!(
            fit_path(r"Everything · C:\work", 400.0, 12.0),
            r"Everything · C:\work"
        );
        let words = "a very long description without any path in it at all, repeated often";
        assert_eq!(fit_path(words, 50.0, 12.0), words);
        assert_eq!(fit_path(LONG, 0.0, 12.0), LONG);
    }

    #[test]
    fn removes_middle_folders_and_keeps_the_end() {
        let fitted = fit_path(LONG, 420.0, 12.0);
        assert!(fitted.starts_with(r"Everything · Folder · C:\"), "{fitted}");
        assert!(
            fitted.ends_with(r"\Business.OceanCarrier.Test\Shipment"),
            "{fitted}"
        );
        assert!(fitted.contains('…'));
        assert!(measure(&fitted, 12.0) <= 420.0);
        // More room keeps more folders.
        assert!(fit_path(LONG, 600.0, 12.0).len() > fitted.len());
    }

    #[test]
    fn too_narrow_still_keeps_root_and_last_segment() {
        assert_eq!(
            fit_path(LONG, 30.0, 12.0),
            r"Everything · Folder · C:\…\Shipment"
        );
    }

    #[test]
    fn unc_and_unix_roots_are_kept() {
        let unc = r"\\server\share\a\b\c\d\e\f\g\file.txt";
        let fitted = fit_path(unc, 100.0, 12.0);
        assert!(fitted.starts_with(r"\\server\"), "{fitted}");
        assert!(fitted.ends_with(r"\file.txt"));
        let unix = fit_path("Files · /Users/me/a/b/c/d/e/f/g/h/notes.md", 120.0, 12.0);
        assert!(unix.starts_with("Files · /Users/…/"), "{unix}");
    }

    #[test]
    fn path_must_start_at_a_word_boundary() {
        assert_eq!(path_start(r"x C:\a"), Some(2));
        assert_eq!(path_start(r"abc:\a"), None);
        assert_eq!(path_start("a/b"), None);
    }
}
