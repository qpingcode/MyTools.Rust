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

/// Lower ranks are stronger. Unmatched candidates remain available for domain searches.
pub fn rank(title: &str, query: &str) -> u8 {
    let title = normalize(title);
    let query = normalize(query);
    if query.is_empty() {
        return 4;
    }
    if title == query {
        return 0;
    }
    if title.starts_with(&query)
        || title
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word.starts_with(&query))
    {
        return 1;
    }
    if query.split_whitespace().all(|word| title.contains(word)) {
        return 2;
    }
    let compact: String = title.chars().filter(|c| c.is_alphanumeric()).collect();
    let needle: String = query.chars().filter(|c| c.is_alphanumeric()).collect();
    if needle.is_empty() {
        return 4;
    }
    let mut full = String::new();
    let mut initials = String::new();
    let mut boundary = true;
    for ch in title.chars() {
        if let Some(py) = ch.to_pinyin() {
            full.push_str(py.plain());
            initials.push(py.plain().chars().next().unwrap());
            boundary = true;
        } else {
            full.push(ch);
            if ch.is_alphanumeric() && boundary {
                initials.push(ch);
            }
            boundary = !ch.is_alphanumeric();
        }
    }
    let subsequence = |value: &str| {
        let mut chars = value.chars();
        needle
            .chars()
            .all(|ch| chars.by_ref().any(|candidate| candidate == ch))
    };
    if compact.contains(&needle)
        || full.contains(&needle)
        || initials.starts_with(&needle)
        || subsequence(&compact)
    {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_titles_and_transliterations() {
        for (title, query, expected) in [
            ("Code", "ＣＯＤＥ", 0),
            ("Code Editor", "code", 1),
            ("Visual Studio Code", "vsc", 3),
            ("记事本", "jsb", 3),
            ("记事本", "jishiben", 3),
            ("Other", "xyz", 4),
        ] {
            assert_eq!(rank(title, query), expected);
        }
    }
}
