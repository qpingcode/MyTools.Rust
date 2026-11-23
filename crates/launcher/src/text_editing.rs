//! Text edits that Slint's language cannot express, for the `TextEditing` global.

/// `text` with the bytes from `start` to `end` replaced by `insert`. Offsets are clamped to
/// the text and moved back to character boundaries, so a stale cursor cannot split a
/// character.
pub fn replace(text: &str, start: i32, end: i32, insert: &str) -> String {
    let boundary = |offset: i32| {
        let mut offset = usize::try_from(offset).unwrap_or(0).min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    };
    let (start, end) = (boundary(start.min(end)), boundary(start.max(end)));
    [&text[..start], insert, &text[end..]].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_the_selection_on_character_boundaries() {
        assert_eq!(replace("ab", 1, 1, "\n"), "a\nb");
        assert_eq!(replace("hello", 4, 1, "\n"), "h\no");
        assert_eq!(replace("中文", 3, 3, "\n"), "中\n文");
        // An offset inside 文 moves back to its start; one past the end is clamped.
        assert_eq!(replace("中文", 4, 99, "!"), "中!");
        assert_eq!(replace("", -1, 0, "\n"), "\n");
    }
}
