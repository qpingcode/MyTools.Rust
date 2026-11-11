//! Which characters of a result's title or subtitle a query matched, so the launcher can
//! highlight them. Follows [`crate::matching::rank`], but only for matches a reader can
//! follow: the query (or each of its words) as written, the same ignoring punctuation and
//! spaces, or pinyin that covers whole characters. Scattered subsequence matches such as
//! "vsc" in a path would light up random letters, so they are not highlighted.
use crate::matching::normalize;
use pinyin::ToPinyin;
use std::ops::Range;
use unicode_normalization::UnicodeNormalization;

/// The matched characters of `text` as ranges of char indices, sorted and disjoint; empty
/// when the query is empty or matched nothing a reader could see.
pub fn highlights(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = normalize(query);
    if query.is_empty() {
        return Vec::new();
    }
    let folded = fold(text);
    let indices = as_written(&folded, &query)
        .or_else(|| without_punctuation(&folded, &query))
        .or_else(|| by_pinyin(text, &query))
        .unwrap_or_default();
    ranges(indices)
}

/// Each normalized, lower-case character with the index of the text character it came from.
fn fold(text: &str) -> Vec<(char, usize)> {
    let mut folded = Vec::new();
    for (index, c) in text.chars().enumerate() {
        for normalized in c.to_string().nfkc().flat_map(char::to_lowercase) {
            let normalized = if normalized.is_whitespace() {
                ' '
            } else {
                normalized
            };
            folded.push((normalized, index));
        }
    }
    folded
}

/// The whole query, else each of its words, where it appears; at a word start if it can.
fn as_written(folded: &[(char, usize)], query: &str) -> Option<Vec<usize>> {
    let hay: Vec<char> = folded.iter().map(|(c, _)| *c).collect();
    let covered = |needle: &str| {
        let needle: Vec<char> = needle.chars().collect();
        find(&hay, &needle).map(|start| {
            folded[start..start + needle.len()]
                .iter()
                .map(|(_, index)| *index)
                .collect::<Vec<_>>()
        })
    };
    covered(query).or_else(|| {
        let words: Option<Vec<Vec<usize>>> = query.split_whitespace().map(covered).collect();
        words.map(|words| words.concat())
    })
}

/// The query's letters and digits in a row once punctuation and spaces are dropped, e.g.
/// "vscode" in "VS Code".
fn without_punctuation(folded: &[(char, usize)], query: &str) -> Option<Vec<usize>> {
    let compact: Vec<(char, usize)> = folded
        .iter()
        .copied()
        .filter(|(c, _)| c.is_alphanumeric())
        .collect();
    let needle: Vec<char> = query.chars().filter(|c| c.is_alphanumeric()).collect();
    let hay: Vec<char> = compact.iter().map(|(c, _)| *c).collect();
    let start = find(&hay, &needle)?;
    Some(
        compact[start..start + needle.len()]
            .iter()
            .map(|(_, index)| *index)
            .collect(),
    )
}

/// Full pinyin starting at a character ("jishi" or "jish" in 记事本), or the initials the
/// title starts with ("jsb").
fn by_pinyin(text: &str, query: &str) -> Option<Vec<usize>> {
    let needle: String = query.chars().filter(|c| c.is_alphanumeric()).collect();
    if needle.is_empty() || !text.chars().any(|c| c.to_pinyin().is_some()) {
        return None;
    }
    // Each character as its pinyin or itself, and the initials `rank` reads.
    let mut units: Vec<(String, usize)> = Vec::new();
    let mut initials: Vec<(char, usize)> = Vec::new();
    let mut boundary = true;
    for (index, c) in text.chars().enumerate() {
        let lower: String = c.to_lowercase().collect();
        if let Some(pinyin) = c.to_pinyin() {
            initials.extend(
                pinyin
                    .plain()
                    .chars()
                    .next()
                    .map(|initial| (initial, index)),
            );
            units.push((pinyin.plain().to_owned(), index));
            boundary = true;
        } else {
            if c.is_alphanumeric() && boundary {
                initials.extend(lower.chars().next().map(|initial| (initial, index)));
            }
            boundary = !c.is_alphanumeric();
            units.push((lower, index));
        }
    }
    for start in 0..units.len() {
        let rest: String = units[start..]
            .iter()
            .map(|(unit, _)| unit.as_str())
            .collect();
        if rest.starts_with(&needle) {
            let mut covered = Vec::new();
            let mut length = 0;
            for (unit, index) in &units[start..] {
                if length >= needle.len() {
                    break;
                }
                covered.push(*index);
                length += unit.len();
            }
            return Some(covered);
        }
    }
    let typed: String = initials.iter().map(|(c, _)| *c).collect();
    typed.starts_with(&needle).then(|| {
        initials[..needle.chars().count()]
            .iter()
            .map(|(_, index)| *index)
            .collect()
    })
}

/// The first occurrence of `needle` at a word start, else the first anywhere.
fn find(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    let starts: Vec<usize> = (0..=hay.len() - needle.len())
        .filter(|&start| hay[start..start + needle.len()] == *needle)
        .collect();
    let word_start = |start: usize| start == 0 || !hay[start - 1].is_alphanumeric();
    starts
        .iter()
        .copied()
        .find(|&start| word_start(start))
        .or_else(|| starts.first().copied())
}

fn ranges(mut indices: Vec<usize>) -> Vec<Range<usize>> {
    indices.sort_unstable();
    indices.dedup();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for index in indices {
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

    fn marked(text: &str, query: &str) -> String {
        let ranges = highlights(text, query);
        text.chars()
            .enumerate()
            .map(|(index, c)| {
                if ranges.iter().any(|range| range.contains(&index)) {
                    c.to_uppercase().collect::<String>()
                } else {
                    c.to_string()
                }
            })
            .collect()
    }

    #[test]
    fn highlights_the_query_as_written_preferring_word_starts() {
        assert_eq!(marked("visual studio code", "code"), "visual studio CODE");
        assert_eq!(marked("unicode code", "code"), "unicode CODE");
        assert_eq!(marked("barcode", "code"), "barCODE");
        assert_eq!(
            marked("visual studio code", "studio code"),
            "visual STUDIO CODE"
        );
        assert_eq!(
            marked("visual studio code", "code vis"),
            "VISual studio CODE"
        );
        // Full-width and upper-case input match like `rank`.
        assert_eq!(marked("code editor", "ＣＯＤＥ"), "CODE editor");
    }

    #[test]
    fn highlights_matches_that_skip_punctuation_or_use_pinyin() {
        assert_eq!(marked("vs code", "vscode"), "VS CODE");
        assert_eq!(marked("C:\\work\\my-app", "myapp"), "C:\\work\\MY-APP");
        assert_eq!(highlights("记事本", "jishiben"), vec![0..3]);
        assert_eq!(highlights("记事本", "jish"), vec![0..2]);
        assert_eq!(highlights("记事本", "jsb"), vec![0..3]);
        assert_eq!(highlights("打开记事本", "jishi"), vec![2..4]);
        assert_eq!(highlights("以纯文本粘贴剪贴板", "纯文本"), vec![1..4]);
    }

    #[test]
    fn scattered_or_missing_matches_are_not_highlighted() {
        assert!(highlights("Visual Studio Code", "vsc").is_empty());
        assert!(highlights("Paste Clipboard as Plain Text", "纯文本").is_empty());
        assert!(highlights("anything", "").is_empty());
        assert!(highlights("anything", "   ").is_empty());
    }
}
