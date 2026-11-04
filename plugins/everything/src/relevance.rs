//! Relevance ordering on top of Everything 1.4, whose IPC can only sort by fields such as
//! name or date. A plain keyword is sent as several tiered queries so exact and prefix
//! matches are never cut off by the result limit, then all hits are ranked locally.
use crate::Hit;
use mytools_plugin_api::matching::{normalize, rank};
use std::collections::HashSet;

/// Characters that make a query use Everything syntax (wildcards, functions, paths, OR,
/// several terms); such queries are sent unchanged instead of being tiered.
const SYNTAX: &[char] = &[' ', '"', ':', '*', '?', '|', '<', '>', '!', '\\', '/'];

/// The Everything queries to run, strongest tier first, with each tier's result limit.
pub fn tiers(query: &str) -> Vec<(String, u32)> {
    let query = query.trim();
    if query.is_empty() || query.contains(SYNTAX) {
        return vec![(query.to_owned(), 300)];
    }
    vec![
        (format!("wfn:\"{query}\""), 20),
        (format!("startwith:\"{query}\""), 100),
        (query.to_owned(), 300),
    ]
}

/// Removes duplicates across tiers and orders by match quality, then shorter names, then
/// shallower paths. The sort is stable, so Everything's order (recently modified first)
/// breaks the remaining ties.
pub fn rank_hits(hits: Vec<Hit>, query: &str, limit: usize) -> Vec<Hit> {
    let mut seen = HashSet::new();
    let mut hits: Vec<Hit> = hits
        .into_iter()
        .filter(|hit| seen.insert(hit.path.clone()))
        .collect();
    // Wildcard, function or path queries describe what to match, not a name to rank
    // against, so Everything's recently-modified order is kept for them.
    if query
        .trim()
        .contains(|c: char| c != ' ' && SYNTAX.contains(&c))
    {
        hits.truncate(limit);
        return hits;
    }
    hits.sort_by_cached_key(|hit| {
        let name = hit
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| hit.path.to_string_lossy().into_owned());
        let stem = hit
            .path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());
        // "readme" matching "README.md" exactly by stem is as strong as an exact name.
        let quality = if normalize(&stem) == normalize(query) {
            0
        } else {
            rank(&name, query)
        };
        (quality, name.chars().count(), hit.path.components().count())
    });
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn hit(path: &str) -> Hit {
        Hit {
            path: PathBuf::from(path),
            folder: false,
            drive: false,
        }
    }

    fn names(hits: &[Hit]) -> Vec<String> {
        hits.iter()
            .map(|hit| hit.path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn plain_keywords_are_tiered_and_syntax_is_left_alone() {
        let tiers = tiers(" app ");
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].0, "wfn:\"app\"");
        assert_eq!(tiers[2].0, "app");
        assert_eq!(super::tiers("*.sln").len(), 1);
        assert_eq!(super::tiers("app config").len(), 1);
        assert_eq!(super::tiers(r"src\app").len(), 1);
    }

    #[test]
    fn exact_then_prefix_then_contains_then_shorter_and_shallower() {
        let ranked = rank_hits(
            vec![
                hit(r"C:\a\myapp.txt"),
                hit(r"C:\a\b\app.json"),
                hit(r"C:\a\application.md"),
                hit(r"C:\a\app"),
                hit(r"C:\app"),
                hit(r"C:\a\app"),
            ],
            "app",
            10,
        );
        assert_eq!(
            names(&ranked),
            [
                r"C:\app",
                r"C:\a\app",
                r"C:\a\b\app.json",
                r"C:\a\application.md",
                r"C:\a\myapp.txt",
            ]
        );
        assert_eq!(rank_hits(ranked, "app", 2).len(), 2);
    }

    #[test]
    fn wildcard_results_keep_everything_order() {
        let ranked = rank_hits(
            vec![hit(r"C:\deep\path\Recent.sln"), hit(r"C:\A.sln")],
            "*.sln",
            10,
        );
        assert_eq!(names(&ranked), [r"C:\deep\path\Recent.sln", r"C:\A.sln"]);
    }
}
