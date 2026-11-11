//! How well a title matches a query, and which of its characters matched.
//!
//! [`find`] tries the ways a title can match from the strongest to the weakest and stops at
//! the first that applies, so the tier that orders a result and the characters highlighted
//! for it always come from the same match:
//!
//! | Tier | Match | Example |
//! | --- | --- | --- |
//! | 0 | the whole title | `code` → Code |
//! | 1 | the start of the title | `car` → CargoWise PRS |
//! | 2 | the start of a word | `wise` → CargoWise PRS |
//! | 3 | starts of words in order | `cw`, `cwp`, `cp`, `cargop` → CargoWise PRS; `jsb` → 记事本 |
//! | 4 | inside the title, or each query word somewhere | `rgo` → CargoWise |
//! | 5 | ignoring spaces and punctuation, or inside pinyin | `eprs` → CargoWise PRS |
//! | 6 | letters in order anywhere | `cgs` → CargoWise |
//!
//! Words split at spaces and punctuation, at lower-to-upper case changes (CargoWise →
//! Cargo, Wise; XMLParser → XML, Parser), between letters and digits, and around every
//! Chinese character, which also matches by its pinyin.
use pinyin::ToPinyin;
use unicode_normalization::UnicodeNormalization;

pub fn normalize(text: &str) -> String {
    text.nfkc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The tier of no match.
pub const NO_MATCH: u8 = 7;

/// The tier at and below which matched characters are worth highlighting; scattered
/// letters (tier 6) are not.
pub const HIGHLIGHTED: u8 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// 0 (whole title) to 6 (scattered letters); see the module documentation.
    pub tier: u8,
    /// The matched characters as char indices into the title, sorted and distinct.
    pub positions: Vec<usize>,
}

/// The strongest way `title` matches `query`, or `None`.
pub fn find(title: &str, query: &str) -> Option<Match> {
    let query = normalize(query);
    if query.is_empty() {
        return None;
    }
    let text = Text::new(title);
    let needle: Vec<char> = query.chars().filter(|c| c.is_alphanumeric()).collect();
    if !text.may_contain(&needle) {
        return None;
    }
    let attempts: [&dyn Fn() -> Option<Vec<usize>>; 7] = [
        &|| text.whole(&query),
        &|| text.at(&query, |start| start == 0),
        &|| text.at(&query, |start| text.word_start(start)),
        &|| text.word_starts(&needle),
        &|| text.inside(&query),
        &|| {
            text.compact(&needle)
                .or_else(|| text.pinyin_inside(&needle))
        },
        &|| text.scattered(&needle),
    ];
    attempts
        .iter()
        .zip(0u8..)
        .find_map(|(attempt, tier)| attempt().map(|positions| Match { tier, positions }))
        .map(|mut found| {
            found.positions.sort_unstable();
            found.positions.dedup();
            found
        })
}

/// The tier of the strongest match, [`NO_MATCH`] when there is none. Lower is stronger.
pub fn tier(title: &str, query: &str) -> u8 {
    find(title, query).map_or(NO_MATCH, |found| found.tier)
}

/// The match strength on the scale plugins compare with: 0 whole title, 1 its start or a
/// word's start, 2 starts of words in order or inside the title, 3 weaker matches, 4 none.
/// Unmatched candidates remain available for domain searches.
pub fn rank(title: &str, query: &str) -> u8 {
    match tier(title, query) {
        0 => 0,
        1 | 2 => 1,
        3 | 4 => 2,
        5 | 6 => 3,
        _ => 4,
    }
}

/// A title prepared for matching.
struct Text {
    original: Vec<char>,
    /// Each normalized, lower-case character and the index of the title character it is
    /// from; spaces are folded to ' '.
    folded: Vec<(char, usize)>,
    /// The characters of `folded` alone.
    hay: Vec<char>,
    /// Indices into `folded` where a word starts.
    starts: Vec<usize>,
    /// Words in order: how each can be typed (its letters, or a Chinese character's pinyin
    /// and the character itself), and the title characters it covers.
    words: std::cell::OnceCell<Vec<Word>>,
}

struct Word {
    spellings: Vec<Vec<char>>,
    /// The title character of each letter of the first spelling.
    positions: Vec<usize>,
    /// Which space- or punctuation-separated part of the title the word is in; CargoWise
    /// is one part of two words.
    part: usize,
}

/// A way to type the query as starts of words: (skipped words, jumps between parts, first
/// word) and the matched title characters.
type Path = ((u32, u32, usize), Vec<usize>);

impl Text {
    fn new(title: &str) -> Self {
        let original: Vec<char> = title.chars().collect();
        let mut folded = Vec::with_capacity(original.len());
        let mut starts = Vec::new();
        for (index, &c) in original.iter().enumerate() {
            let previous = index.checked_sub(1).map(|i| original[i]);
            let next = original.get(index + 1).copied();
            if c.is_alphanumeric()
                && (is_chinese(c) || previous.is_none_or(|p| starts_after(p, c, next)))
            {
                starts.push(folded.len());
            }
            // Most titles are ASCII: skip the Unicode work for those characters.
            if c.is_ascii() {
                let lower = if c.is_ascii_whitespace() {
                    ' '
                } else {
                    c.to_ascii_lowercase()
                };
                folded.push((lower, index));
            } else {
                folded.extend(
                    c.to_string()
                        .nfkc()
                        .flat_map(char::to_lowercase)
                        .map(|c| (if c.is_whitespace() { ' ' } else { c }, index)),
                );
            }
        }
        let hay = folded.iter().map(|(c, _)| *c).collect();
        Self {
            original,
            folded,
            hay,
            starts,
            words: std::cell::OnceCell::new(),
        }
    }

    /// The title's words, built only for the matches that need them.
    fn words(&self) -> &[Word] {
        self.words.get_or_init(|| {
            let mut words: Vec<Word> = Vec::new();
            let mut at = 0;
            let mut part = 0;
            for (index, &c) in self.original.iter().enumerate() {
                let first = at;
                let mut lower = Vec::new();
                while let Some((letter, _)) = self.folded.get(at).filter(|(_, from)| *from == index)
                {
                    lower.push(*letter);
                    at += 1;
                }
                if !c.is_alphanumeric() {
                    continue;
                }
                let previous = index.checked_sub(1).map(|i| self.original[i]);
                if previous.is_none_or(|p| !p.is_alphanumeric() || is_chinese(p)) || is_chinese(c) {
                    part += 1;
                }
                if let Some(pinyin) = is_chinese(c).then(|| c.to_pinyin()).flatten() {
                    let spelling: Vec<char> = pinyin.plain().chars().collect();
                    words.push(Word {
                        positions: vec![index; spelling.len()],
                        spellings: vec![spelling, lower],
                        part,
                    });
                    continue;
                }
                if self.word_start(first) || words.is_empty() {
                    words.push(Word {
                        spellings: vec![Vec::new()],
                        positions: Vec::new(),
                        part,
                    });
                }
                let word = words.last_mut().expect("a word was just started");
                word.positions
                    .extend(std::iter::repeat_n(index, lower.len()));
                word.spellings[0].extend(lower);
            }
            words
        })
    }

    /// Whether the title might contain every letter of `needle`; when it has no Chinese
    /// (whose pinyin adds letters) and lacks one, nothing can match.
    fn may_contain(&self, needle: &[char]) -> bool {
        self.original.iter().any(|c| is_chinese(*c))
            || needle.iter().all(|letter| self.hay.contains(letter))
    }

    fn word_start(&self, folded_index: usize) -> bool {
        self.starts.binary_search(&folded_index).is_ok()
    }

    fn positions(&self, range: std::ops::Range<usize>) -> Vec<usize> {
        self.folded[range].iter().map(|(_, index)| *index).collect()
    }

    fn whole(&self, query: &str) -> Option<Vec<usize>> {
        let title: String = self.hay.iter().collect();
        (normalize(&title) == query).then(|| self.positions(0..self.folded.len()))
    }

    /// `query` where `accept` allows it to start.
    fn at(&self, query: &str, accept: impl Fn(usize) -> bool) -> Option<Vec<usize>> {
        let needle: Vec<char> = query.chars().collect();
        let start = occurrences(&self.hay, &needle).find(|&start| accept(start))?;
        Some(self.positions(start..start + needle.len()))
    }

    /// The query anywhere, else each of its words somewhere (preferring word starts).
    fn inside(&self, query: &str) -> Option<Vec<usize>> {
        let hay = &self.hay;
        let place = |word: &str| {
            let needle: Vec<char> = word.chars().collect();
            let mut found = occurrences(hay, &needle);
            let first = found.next()?;
            let start = std::iter::once(first)
                .chain(found)
                .find(|&start| self.word_start(start))
                .unwrap_or(first);
            Some(self.positions(start..start + needle.len()))
        };
        place(query).or_else(|| {
            let words: Option<Vec<Vec<usize>>> = query.split_whitespace().map(place).collect();
            words.map(|words| words.concat())
        })
    }

    /// The query typed as the starts of words in order, skipping any words: each piece is
    /// as long as it can be, so `cargop` is Cargo + P rather than C + ….
    fn word_starts(&self, needle: &[char]) -> Option<Vec<usize>> {
        if needle.is_empty() {
            return None;
        }
        let mut memo = std::collections::HashMap::new();
        self.best_from(needle, 0, None, &mut memo)
            .map(|(_, positions)| positions)
    }

    /// The cheapest way to type `needle[typed..]` as starts of words after `previous`:
    /// fewest skipped words, then fewest jumps between space- or punctuation-separated
    /// parts, then the earliest start. So `cwp` in `C:\work\CargoWise\PRS` is Cargo Wise
    /// PRS, not C, work and PRS.
    fn best_from(
        &self,
        needle: &[char],
        typed: usize,
        previous: Option<usize>,
        memo: &mut std::collections::HashMap<(usize, Option<usize>), Option<Path>>,
    ) -> Option<Path> {
        if typed == needle.len() {
            return Some(((0, 0, 0), Vec::new()));
        }
        if let Some(known) = memo.get(&(typed, previous)) {
            return known.clone();
        }
        let words = self.words();
        let first = previous.map_or(0, |previous| previous + 1);
        let mut best: Option<Path> = None;
        for (index, word) in words.iter().enumerate().skip(first) {
            let (skipped, jumps, start) = match previous {
                Some(previous) => (
                    (index - previous - 1) as u32,
                    u32::from(words[previous].part != word.part),
                    0,
                ),
                None => (0, 0, index),
            };
            for (spelling, length) in longest_prefixes(word, &needle[typed..]) {
                let Some(((more_skipped, more_jumps, _), rest)) =
                    self.best_from(needle, typed + length, Some(index), memo)
                else {
                    continue;
                };
                let cost = (skipped + more_skipped, jumps + more_jumps, start);
                if best.as_ref().is_none_or(|(known, _)| cost < *known) {
                    let mut positions = match spelling {
                        0 => word.positions[..length].to_vec(),
                        _ => vec![word.positions[0]],
                    };
                    positions.extend(rest);
                    best = Some((cost, positions));
                }
            }
        }
        memo.insert((typed, previous), best.clone());
        best
    }

    /// The query's letters and digits in a row once spaces and punctuation are dropped.
    fn compact(&self, needle: &[char]) -> Option<Vec<usize>> {
        let compact: Vec<(char, usize)> = self
            .folded
            .iter()
            .copied()
            .filter(|(c, _)| c.is_alphanumeric())
            .collect();
        let hay: Vec<char> = compact.iter().map(|(c, _)| *c).collect();
        let start = occurrences(&hay, needle).next()?;
        Some(
            compact[start..start + needle.len()]
                .iter()
                .map(|(_, i)| *i)
                .collect(),
        )
    }

    /// The query inside the title's full pinyin, also mid-syllable.
    fn pinyin_inside(&self, needle: &[char]) -> Option<Vec<usize>> {
        if !self.original.iter().any(|c| is_chinese(*c)) {
            return None;
        }
        let spelled: Vec<(char, usize)> = self
            .words()
            .iter()
            .flat_map(|word| {
                word.spellings[0]
                    .iter()
                    .copied()
                    .zip(word.positions.iter().copied())
            })
            .collect();
        let hay: Vec<char> = spelled.iter().map(|(c, _)| *c).collect();
        let start = occurrences(&hay, needle).next()?;
        Some(
            spelled[start..start + needle.len()]
                .iter()
                .map(|(_, i)| *i)
                .collect(),
        )
    }

    /// The query's letters in order anywhere, including pinyin initials.
    fn scattered(&self, needle: &[char]) -> Option<Vec<usize>> {
        if needle.is_empty() {
            return None;
        }
        let mut positions = Vec::new();
        let mut rest = needle.iter().peekable();
        for (c, index) in self.folded.iter().filter(|(c, _)| c.is_alphanumeric()) {
            if rest.peek() == Some(&c) {
                rest.next();
                positions.push(*index);
            }
        }
        rest.peek().is_none().then_some(positions)
    }
}

/// Whether `c` starts a new word after `previous`: after a space or punctuation, at a
/// lower-to-upper change, at the last capital before lower case (XML|Parser), or between
/// letters and digits.
fn starts_after(previous: char, c: char, next: Option<char>) -> bool {
    !previous.is_alphanumeric()
        || is_chinese(previous)
        || (previous.is_lowercase() && c.is_uppercase())
        || (previous.is_uppercase() && c.is_uppercase() && next.is_some_and(char::is_lowercase))
        || (previous.is_numeric() != c.is_numeric())
}

/// A character with pinyin, which is a word of its own.
fn is_chinese(c: char) -> bool {
    !c.is_ascii() && c.to_pinyin().is_some()
}

/// The spellings of `word` that start `rest`, as (spelling, typed length), longest first.
fn longest_prefixes(word: &Word, rest: &[char]) -> Vec<(usize, usize)> {
    let mut found: Vec<(usize, usize)> = Vec::new();
    for (spelling, letters) in word.spellings.iter().enumerate() {
        let common = letters.iter().zip(rest).take_while(|(a, b)| a == b).count();
        found.extend((1..=common).map(|length| (spelling, length)));
    }
    found.sort_by_key(|&(_, length)| std::cmp::Reverse(length));
    found
}

fn occurrences<'a>(hay: &'a [char], needle: &'a [char]) -> impl Iterator<Item = usize> + 'a {
    let last = (!needle.is_empty() && needle.len() <= hay.len()).then(|| hay.len() - needle.len());
    last.into_iter()
        .flat_map(|last| 0..=last)
        .filter(move |&start| hay[start..start + needle.len()] == *needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The title with matched characters in upper case, and the tier.
    fn marked(title: &str, query: &str) -> (String, u8) {
        let found = find(title, query).expect("a match");
        let shown = title
            .chars()
            .enumerate()
            .map(|(index, c)| {
                if found.positions.contains(&index) {
                    c.to_uppercase().collect::<String>()
                } else {
                    c.to_lowercase().collect()
                }
            })
            .collect();
        (shown, found.tier)
    }

    #[test]
    fn word_starts_cover_acronyms_camel_case_and_skipped_words() {
        let title = "CargoWise PRS";
        assert_eq!(marked(title, "cw"), ("CargoWise prs".into(), 3));
        assert_eq!(marked(title, "cwp"), ("CargoWise Prs".into(), 3));
        assert_eq!(marked(title, "cp"), ("Cargowise Prs".into(), 3));
        assert_eq!(marked(title, "cargop"), ("CARGOwise Prs".into(), 3));
        assert_eq!(marked(title, "cargo p"), ("CARGOwise Prs".into(), 3));
        assert_eq!(marked(title, "wise"), ("cargoWISE prs".into(), 2));
        assert_eq!(marked("XMLParser", "xp"), ("XmlParser".into(), 3));
        assert_eq!(
            marked("Windows11 Update", "w11"),
            ("Windows11 update".into(), 3)
        );
        assert_eq!(
            marked("Visual Studio Code", "vsc"),
            ("Visual Studio Code".into(), 3)
        );
        // The tightest of several ways wins: no skipped words, no jumps between parts.
        let path = r"C:\work\CargoWise\PRS";
        assert_eq!(marked(path, "cw").0, r"c:\work\CargoWise\prs");
        assert_eq!(marked(path, "cwp").0, r"c:\work\CargoWise\Prs");
        assert_eq!(marked(path, "cp").0, r"c:\work\Cargowise\Prs");
    }

    #[test]
    fn tiers_follow_the_strength_of_the_match() {
        let title = "CargoWise PRS";
        assert_eq!(marked("Code", "code").1, 0);
        assert_eq!(marked(title, "car"), ("CARgowise prs".into(), 1));
        assert_eq!(marked(title, "rgo"), ("caRGOwise prs".into(), 4));
        assert_eq!(marked(title, "prs cargo"), ("CARGOwise PRS".into(), 4));
        assert_eq!(marked(title, "eprs"), ("cargowisE PRS".into(), 5));
        assert_eq!(marked(title, "cgs").1, 6);
        assert_eq!(find(title, "xyz"), None);
        assert_eq!(find(title, "  "), None);
        // Full-width input matches like half-width.
        assert_eq!(marked("Code Editor", "ＣＯＤＥ"), ("CODE editor".into(), 1));
    }

    #[test]
    fn chinese_matches_by_character_pinyin_or_initials() {
        let positions = |title: &str, query: &str| {
            let found = find(title, query).unwrap();
            (found.positions, found.tier)
        };
        assert_eq!(positions("记事本", "jsb"), (vec![0, 1, 2], 3));
        assert_eq!(positions("记事本", "jishiben"), (vec![0, 1, 2], 3));
        assert_eq!(positions("记事本", "jish"), (vec![0, 1], 3));
        assert_eq!(positions("打开记事本", "jishi"), (vec![2, 3], 3));
        // Every Chinese character starts a word, so characters typed as written match there.
        assert_eq!(positions("打开记事本", "记事"), (vec![2, 3], 2));
        assert_eq!(
            positions("以纯文本粘贴剪贴板", "纯文本"),
            (vec![1, 2, 3], 2)
        );
        assert_eq!(positions("记事本", "ishi").1, 5);
        assert_eq!(positions("打开 Code", "dkc"), (vec![0, 1, 3], 3));
    }

    #[test]
    fn rank_keeps_the_scale_plugins_compare_with() {
        for (title, query, expected) in [
            ("Code", "ＣＯＤＥ", 0),
            ("Code Editor", "code", 1),
            ("CargoWise PRS", "wise", 1),
            ("Visual Studio Code", "vsc", 2),
            ("记事本", "jsb", 2),
            ("记事本", "jishiben", 2),
            ("CargoWise PRS", "eprs", 3),
            ("Other", "xyz", 4),
        ] {
            assert_eq!(rank(title, query), expected, "{title} / {query}");
        }
    }
}
