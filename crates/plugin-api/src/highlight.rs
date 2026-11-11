//! Which characters of a result's title or subtitle to highlight for a query: those of the
//! match that also ranks the result ([`crate::matching::find`]), unless it is only scattered
//! letters, which would light up random characters of a path.
use crate::matching::{HIGHLIGHTED, find};
use std::ops::Range;

/// The matched characters of `text` as ranges of char indices, sorted and disjoint; empty
/// when the query is empty or matched nothing worth showing.
pub fn highlights(text: &str, query: &str) -> Vec<Range<usize>> {
    let Some(found) = find(text, query).filter(|found| found.tier <= HIGHLIGHTED) else {
        return Vec::new();
    };
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for index in found.positions {
        match ranges.last_mut() {
            Some(last) if last.end == index => last.end = index + 1,
            _ => ranges.push(index..index + 1),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_runs_of_matched_characters() {
        assert_eq!(highlights("CargoWise PRS", "cwp"), vec![0..1, 5..6, 10..11]);
        assert_eq!(highlights("CargoWise PRS", "cargop"), vec![0..5, 10..11]);
        assert_eq!(highlights("C:\\work\\my-app", "my-app"), vec![8..14]);
        assert_eq!(highlights("记事本", "jsb"), vec![0..3]);
    }

    #[test]
    fn scattered_or_missing_matches_are_not_highlighted() {
        assert!(highlights("CargoWise PRS", "cgs").is_empty());
        assert!(highlights("Paste Clipboard as Plain Text", "纯文本").is_empty());
        assert!(highlights("anything", "").is_empty());
    }
}
