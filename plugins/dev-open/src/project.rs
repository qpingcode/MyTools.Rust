//! Finds the solution or project that Rider / Visual Studio should open for a path.
use std::path::{Path, PathBuf};

/// Solution formats, most specific first: classic, XML (VS 2022 17.13+), solution filter.
const SOLUTION_EXTENSIONS: &[&str] = &["sln", "slnx", "slnf"];
/// .NET project formats both IDEs open directly when no solution exists.
const PROJECT_EXTENSIONS: &[&str] = &["csproj", "fsproj", "vbproj", "vcxproj", "esproj", "proj"];

/// Walks from `path` up to the root looking for a solution; only when none exists anywhere
/// does it walk again looking for a project file. A solution/project path is used as-is.
pub fn find(path: &Path) -> Option<PathBuf> {
    if path.is_file() && has_extension(path, SOLUTION_EXTENSIONS.iter().chain(PROJECT_EXTENSIONS)) {
        return Some(path.to_path_buf());
    }
    let start = if path.is_dir() { path } else { path.parent()? };
    nearest(start, SOLUTION_EXTENSIONS).or_else(|| nearest(start, PROJECT_EXTENSIONS))
}

fn nearest(start: &Path, extensions: &[&str]) -> Option<PathBuf> {
    start
        .ancestors()
        .find_map(|directory| best_in(directory, extensions))
}

/// Picks one candidate per folder: earlier extensions win, then a file named after the
/// folder, then alphabetical order, so the choice is stable.
fn best_in(directory: &Path, extensions: &[&str]) -> Option<PathBuf> {
    let folder = directory
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase());
    let mut candidates: Vec<_> = std::fs::read_dir(directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && has_extension(path, extensions.iter()))
        .collect();
    candidates.sort_by_key(|path| {
        let extension = path
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let rank = extensions
            .iter()
            .position(|candidate| *candidate == extension)
            .unwrap_or(usize::MAX);
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_lowercase());
        (rank, stem != folder, path.clone())
    });
    candidates.into_iter().next()
}

fn has_extension<'a>(path: &Path, mut extensions: impl Iterator<Item = &'a &'a str>) -> bool {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .is_some_and(|ext| extensions.any(|candidate| *candidate == ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(name: &str, files: &[&str]) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("mytools-dev-open-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            if !file.ends_with('/') {
                std::fs::write(path, b"").unwrap();
            }
        }
        Fixture(root)
    }

    #[test]
    fn solution_in_any_ancestor_beats_nearer_project() {
        let f = fixture(
            "sln",
            &["Repo.sln", "src/App/App.csproj", "src/App/Program.cs"],
        );
        assert_eq!(
            find(&f.0.join("src/App/Program.cs")),
            Some(f.0.join("Repo.sln"))
        );
        assert_eq!(find(&f.0.join("src/App")), Some(f.0.join("Repo.sln")));
    }

    #[test]
    fn falls_back_to_nearest_project_file() {
        let f = fixture("proj", &["src/Lib/Lib.fsproj", "src/Lib/Code/"]);
        assert_eq!(
            find(&f.0.join("src/Lib/Code")),
            Some(f.0.join("src/Lib/Lib.fsproj"))
        );
    }

    #[test]
    fn prefers_sln_over_slnx_then_folder_name() {
        let f = fixture(
            "pick",
            &["Work/Other.sln", "Work/Work.sln", "Work/Work.slnx"],
        );
        assert_eq!(find(&f.0.join("Work")), Some(f.0.join("Work/Work.sln")));
    }

    #[test]
    fn solution_file_is_used_directly_and_missing_project_is_none() {
        let f = fixture("direct", &["a/Direct.slnx", "b/readme.md"]);
        assert_eq!(
            find(&f.0.join("a/Direct.slnx")),
            Some(f.0.join("a/Direct.slnx"))
        );
        // The temp directory itself may hold unrelated solutions, so only check the leaf.
        let found = find(&f.0.join("b/readme.md"));
        assert!(found.is_none_or(|path| !path.starts_with(&f.0)));
    }
}
