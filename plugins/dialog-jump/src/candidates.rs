//! Places offered for a query: a typed path with its subfolders, folders open in File
//! Explorer ranked by name, and files and folders found by Everything.
use mytools_plugin_api::matching;
use std::path::{Path, PathBuf};

const MAX_SUBFOLDERS: usize = 30;
/// The host shows at most 50 results.
const MAX_RESULTS: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// The typed path or one of its subfolders.
    Typed,
    /// Shown in an open File Explorer window.
    Explorer,
    Everything,
}

/// A place the dialog can jump to; for a file, the folder it is in.
#[derive(Debug, PartialEq)]
pub struct Candidate {
    pub path: PathBuf,
    pub file: bool,
    pub source: Source,
}

/// The query as an absolute path, which is completed from the disk instead of searched.
pub fn typed_path(query: &str) -> Option<&str> {
    let query = query.trim().trim_matches('"');
    (!query.is_empty() && Path::new(query).is_absolute()).then_some(query)
}

/// `indexed` are Everything's hits for a query that is not a path, as `(path, is_file)`.
/// Folders found by Everything come before files, keeping Everything's order otherwise: a
/// folder is what the dialog jumps to, and files with shorter matching names (e.g. under
/// `.git`) would otherwise push it out of sight.
pub fn collect(
    query: &str,
    explorer: Vec<PathBuf>,
    mut indexed: Vec<(PathBuf, bool)>,
) -> Vec<Candidate> {
    indexed.sort_by_key(|&(_, file)| file);
    let found: Vec<(PathBuf, bool, Source)> = match typed_path(query) {
        Some(path) => typed_folders(path)
            .into_iter()
            .map(|path| (path, false, Source::Typed))
            .collect(),
        None => rank_open(explorer, query.trim())
            .into_iter()
            .map(|path| (path, false, Source::Explorer))
            .chain(
                indexed
                    .into_iter()
                    .map(|(path, file)| (path, file, Source::Everything)),
            )
            .collect(),
    };
    let mut candidates: Vec<Candidate> = Vec::new();
    for (path, file, source) in found {
        if candidates.len() < MAX_RESULTS
            && !candidates.iter().any(|known| same_path(&known.path, &path))
        {
            candidates.push(Candidate { path, file, source });
        }
    }
    candidates
}

/// An absolute path typed by the user: the folder itself, and its subfolders when it ends
/// with a separator; otherwise the subfolders of its parent starting with the last part.
fn typed_folders(query: &str) -> Vec<PathBuf> {
    let path = Path::new(query);
    if path.is_dir() {
        let mut folders = vec![path.to_path_buf()];
        if query.ends_with(['\\', '/']) {
            folders.extend(subfolders(path, ""));
        }
        return folders;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(prefix)) => subfolders(parent, &prefix.to_string_lossy()),
        _ => Vec::new(),
    }
}

fn subfolders(parent: &Path, prefix: &str) -> Vec<PathBuf> {
    let prefix = prefix.to_lowercase();
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .starts_with(&prefix)
        })
        .map(|entry| entry.path())
        .collect();
    folders.sort_by_key(|path| path.to_string_lossy().to_lowercase());
    folders.truncate(MAX_SUBFOLDERS);
    folders
}

/// Open folders matching the query by name or path, best first; all of them, in their
/// given order, for an empty query.
fn rank_open(folders: Vec<PathBuf>, query: &str) -> Vec<PathBuf> {
    if query.is_empty() {
        return folders;
    }
    let normalized = matching::normalize(query);
    let mut ranked: Vec<(u8, PathBuf)> = folders
        .into_iter()
        .filter_map(|path| {
            let rank = matching::rank(&display_name(&path), query);
            let in_path = matching::normalize(&path.to_string_lossy()).contains(&normalized);
            match rank {
                4 if in_path => Some((3, path)),
                4 => None,
                rank => Some((rank, path)),
            }
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, path)| path).collect()
}

/// The last part of the path, or the whole path for a drive.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| mytools_plugin_api::paths::display(path))
}

fn same_path(left: &Path, right: &Path) -> bool {
    let key = |path: &Path| {
        path.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    key(left) == key(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_tree() -> PathBuf {
        let root = std::env::temp_dir().join(format!("dialog-jump-{}", std::process::id()));
        for name in ["Alpha", "albums", "Beta"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        std::fs::write(root.join("all.txt"), "").unwrap();
        root
    }

    #[test]
    fn typed_prefix_lists_matching_subfolders_only() {
        let root = temp_tree();
        let query = format!("{}\\al", root.display());
        let names: Vec<String> = collect(&query, vec![], vec![])
            .iter()
            .map(|candidate| display_name(&candidate.path))
            .collect();
        assert_eq!(names, ["albums", "Alpha"]);
        let all = collect(&format!("{}\\", root.display()), vec![], vec![]);
        assert_eq!(all.len(), 4, "the folder itself and its three subfolders");
        assert!(all.iter().all(|candidate| candidate.source == Source::Typed));
    }

    #[test]
    fn open_folders_are_ranked_by_name_and_deduplicated() {
        let open = vec![
            PathBuf::from(r"C:\work\reports-old"),
            PathBuf::from(r"C:\reports"),
            PathBuf::from(r"C:\Reports\"),
            PathBuf::from(r"C:\music"),
        ];
        let found = collect("reports", open, vec![]);
        let paths: Vec<_> = found.iter().map(|candidate| &candidate.path).collect();
        assert_eq!(
            paths,
            [
                &PathBuf::from(r"C:\reports"),
                &PathBuf::from(r"C:\work\reports-old")
            ]
        );
        assert!(
            found
                .iter()
                .all(|candidate| candidate.source == Source::Explorer)
        );
    }

    #[test]
    fn empty_query_keeps_explorer_order_and_paths_match_by_parent() {
        let open = vec![PathBuf::from(r"C:\b"), PathBuf::from(r"C:\a")];
        assert_eq!(collect("", open.clone(), vec![]).len(), 2);
        assert_eq!(collect("", open, vec![])[0].path, PathBuf::from(r"C:\b"));
        let nested = collect("work", vec![PathBuf::from(r"C:\work\docs")], vec![]);
        assert_eq!(nested.len(), 1);
    }

    #[test]
    fn everything_hits_follow_open_folders_without_duplicates() {
        let found = collect(
            "docs",
            vec![PathBuf::from(r"C:\work\docs")],
            vec![
                (PathBuf::from(r"C:\work\docs.txt"), true),
                (PathBuf::from(r"C:\work\docs"), false),
                (PathBuf::from(r"D:\docs"), false),
            ],
        );
        let kinds: Vec<_> = found
            .iter()
            .map(|candidate| (candidate.source, candidate.file))
            .collect();
        assert_eq!(
            kinds,
            [
                (Source::Explorer, false),
                (Source::Everything, false),
                (Source::Everything, true)
            ],
            "folders from Everything come before files"
        );
        assert_eq!(found[1].path, PathBuf::from(r"D:\docs"));
    }

    #[test]
    fn typed_paths_are_completed_from_disk_not_searched() {
        assert_eq!(typed_path(r#" "C:\work" "#), Some(r"C:\work"));
        assert_eq!(typed_path("work"), None);
        let found = collect(
            r"C:\definitely\missing\x",
            vec![],
            vec![(PathBuf::from(r"C:\x"), true)],
        );
        assert!(found.is_empty());
    }
}
