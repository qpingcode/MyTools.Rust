//! Lists the projects of a solution and orders projects so each comes after the projects it
//! references, for building them one by one with `--no-dependencies`.
use crate::project::{BUILDABLE_EXTENSIONS, has_extension};
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
};

/// The buildable projects a `.sln`, `.slnx` or `.slnf` names that exist, in its order.
pub fn projects(solution: &Path) -> Result<Vec<PathBuf>, String> {
    let text = std::fs::read_to_string(solution).map_err(|error| error.to_string())?;
    let directory = solution.parent().unwrap_or(Path::new(""));
    let extension = solution
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase());
    let (base, relative) = match extension.as_deref() {
        Some("slnx") => (directory.to_path_buf(), slnx_paths(&text)),
        Some("slnf") => slnf_paths(&text, directory)?,
        _ => (directory.to_path_buf(), sln_paths(&text)),
    };
    Ok(relative
        .iter()
        .map(|path| tidy(&base.join(native(path))))
        .filter(|path| path.is_file() && has_extension(path, BUILDABLE_EXTENSIONS.iter()))
        .collect())
}

/// Solution files write `\`; other systems need their own separator to find the file.
fn native(path: &str) -> String {
    path.replace(['\\', '/'], std::path::MAIN_SEPARATOR_STR)
}

/// Drops `.` and resolves `..` without asking the file system, so paths show as written
/// by hand: `src\App\..\Lib\Lib.csproj` becomes `src\Lib\Lib.csproj`.
fn tidy(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(result.components().next_back(), Some(Component::Normal(_))) =>
            {
                result.pop();
            }
            other => result.push(other),
        }
    }
    result
}

/// `Project("{type}") = "Name", "src\App\App.csproj", "{id}"`; solution folders name
/// themselves instead of a file and are dropped later for having no project extension.
fn sln_paths(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.trim_start().starts_with("Project("))
        .filter_map(|line| line.split_once('=').map(|(_, values)| values))
        .filter_map(|values| values.split(',').nth(1))
        .map(|path| path.trim().trim_matches('"').to_owned())
        .collect()
}

/// `<Project Path="src/App/App.csproj" />`, possibly inside `<Folder>` elements.
fn slnx_paths(text: &str) -> Vec<String> {
    text.split('<')
        .filter(|tag| tag.starts_with("Project ") || tag.starts_with("Project\t"))
        .filter_map(|tag| attribute(tag, "Path"))
        .collect()
}

/// A filter names its solution and the projects it keeps, relative to that solution.
fn slnf_paths(text: &str, directory: &Path) -> Result<(PathBuf, Vec<String>), String> {
    #[derive(serde::Deserialize)]
    struct Filter {
        solution: Solution,
    }
    #[derive(serde::Deserialize)]
    struct Solution {
        path: String,
        #[serde(default)]
        projects: Vec<String>,
    }
    // Visual Studio writes the file with a BOM.
    let filter: Filter = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|error| error.to_string())?;
    let solution = tidy(&directory.join(native(&filter.solution.path)));
    let base = solution.parent().unwrap_or(directory).to_path_buf();
    Ok((base, filter.solution.projects))
}

/// The value of `name="..."` (or single quotes) in the text of one XML tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(index) = rest.find(name) {
        let before = rest[..index].chars().last();
        let after = rest[index + name.len()..].trim_start();
        rest = &rest[index + name.len()..];
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let Some(value) = after.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let value = &value[1..];
        return value.find(quote).map(|end| value[..end].to_owned());
    }
    None
}

/// The projects a project file references with `<ProjectReference Include="..."/>`.
/// References built from MSBuild properties such as `$(SolutionDir)` cannot be resolved
/// here and are left out.
fn references(project: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(project) else {
        return vec![];
    };
    let directory = project.parent().unwrap_or(Path::new(""));
    text.split('<')
        .filter(|tag| tag.starts_with("ProjectReference"))
        .filter_map(|tag| attribute(tag, "Include"))
        .filter(|include| !include.contains("$("))
        .map(|include| tidy(&directory.join(native(&include))))
        .collect()
}

/// The same file under one name, however the paths spell it.
fn key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Drops repeated projects and puts each after the projects it references among them,
/// otherwise keeping the given order. Projects in a reference cycle keep their order.
pub fn build_order(projects: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique: Vec<(PathBuf, PathBuf)> = vec![];
    for project in projects {
        let key = key(&project);
        if !unique.iter().any(|(known, _)| *known == key) {
            unique.push((key, project));
        }
    }
    let index: HashMap<&PathBuf, usize> = unique
        .iter()
        .enumerate()
        .map(|(position, (key, _))| (key, position))
        .collect();
    let needs: Vec<Vec<usize>> = unique
        .iter()
        .map(|(_, project)| {
            references(project)
                .iter()
                .filter_map(|reference| index.get(&key(reference)).copied())
                .collect()
        })
        .collect();
    order(&needs)
        .into_iter()
        .map(|position| unique[position].1.clone())
        .collect()
}

/// Positions in an order where each comes after the ones it needs; the earliest ready
/// position goes first, and a cycle releases its earliest member.
fn order(needs: &[Vec<usize>]) -> Vec<usize> {
    let mut done = vec![false; needs.len()];
    let mut result = Vec::with_capacity(needs.len());
    while result.len() < needs.len() {
        let ready = (0..needs.len())
            .find(|&position| {
                !done[position]
                    && needs[position]
                        .iter()
                        .all(|&need| done[need] || need == position)
            })
            .or_else(|| (0..needs.len()).find(|&position| !done[position]));
        let Some(position) = ready else { break };
        done[position] = true;
        result.push(position);
    }
    result
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

    fn fixture(name: &str, files: &[(&str, &str)]) -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "mytools-dev-open-sln-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (file, content) in files {
            let path = root.join(native(file));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        Fixture(root)
    }

    fn reference(path: &str) -> String {
        format!(
            r#"<Project><ItemGroup><ProjectReference Include="{path}" /></ItemGroup></Project>"#
        )
    }

    const SLN: &str = r#"
Microsoft Visual Studio Solution File, Format Version 12.00
Project("{2150E333-8FDC-42A3-9474-1A3956D46DE8}") = "src", "src", "{11111111-1111-1111-1111-111111111111}"
EndProject
Project("{9A19103F-16F7-4668-BE54-9A1E7A4F7556}") = "App", "src\App\App.csproj", "{22222222-2222-2222-2222-222222222222}"
EndProject
Project("{9A19103F-16F7-4668-BE54-9A1E7A4F7556}") = "Lib", "src\Lib\Lib.csproj", "{33333333-3333-3333-3333-333333333333}"
EndProject
Project("{8BC9CEB8-8B4A-11D0-8D11-00A0C91BC942}") = "Native", "src\Native\Native.vcxproj", "{44444444-4444-4444-4444-444444444444}"
EndProject
"#;

    #[test]
    fn sln_lists_buildable_projects_and_orders_references_first() {
        let app = reference(r"..\Lib\Lib.csproj");
        let f = fixture(
            "sln",
            &[
                ("Repo.sln", SLN),
                ("src/App/App.csproj", &app),
                ("src/Lib/Lib.csproj", "<Project />"),
                ("src/Native/Native.vcxproj", "<Project />"),
            ],
        );
        let projects = projects(&f.0.join("Repo.sln")).unwrap();
        assert_eq!(
            projects,
            [
                f.0.join(native("src/App/App.csproj")),
                f.0.join(native("src/Lib/Lib.csproj"))
            ]
        );
        assert_eq!(
            build_order(projects),
            [
                f.0.join(native("src/Lib/Lib.csproj")),
                f.0.join(native("src/App/App.csproj"))
            ]
        );
    }

    #[test]
    fn slnx_and_slnf_name_their_projects() {
        let slnx = r#"<Solution>
  <Folder Name="/src/">
    <Project Path="src/App/App.csproj" />
    <Project Type="Classic C#" Path='src/Lib/Lib.csproj'></Project>
  </Folder>
</Solution>"#;
        let slnf = "\u{feff}{ \"solution\": { \"path\": \"..\\\\Repo.slnx\", \"projects\": [ \"src\\\\Lib\\\\Lib.csproj\" ] } }";
        let f = fixture(
            "slnx",
            &[
                ("Repo.slnx", slnx),
                ("filters/Lib.slnf", slnf),
                ("src/App/App.csproj", "<Project />"),
                ("src/Lib/Lib.csproj", "<Project />"),
            ],
        );
        assert_eq!(projects(&f.0.join("Repo.slnx")).unwrap().len(), 2);
        assert_eq!(
            projects(&f.0.join(native("filters/Lib.slnf"))).unwrap(),
            [f.0.join(native("src/Lib/Lib.csproj"))]
        );
    }

    #[test]
    fn order_keeps_given_order_and_survives_cycles() {
        // 0 needs 2, 1 needs nothing, 2 needs 1.
        assert_eq!(order(&[vec![2], vec![], vec![1]]), [1, 2, 0]);
        assert_eq!(order(&[vec![], vec![], vec![]]), [0, 1, 2]);
        // 0 and 1 need each other: the earliest is released.
        assert_eq!(order(&[vec![1], vec![0], vec![]]), [2, 0, 1]);
    }

    #[test]
    fn repeated_projects_are_built_once_and_property_references_are_ignored() {
        let app = reference("$(SolutionDir)src\\Lib\\Lib.csproj");
        let f = fixture(
            "dup",
            &[("App/App.csproj", &app), ("Lib/Lib.csproj", "<Project />")],
        );
        let app = f.0.join(native("App/App.csproj"));
        let lib = f.0.join(native("Lib/Lib.csproj"));
        assert_eq!(
            build_order(vec![app.clone(), lib.clone(), app.clone()]),
            [app, lib]
        );
    }

    #[test]
    fn tidy_resolves_dots_without_the_file_system() {
        assert_eq!(
            tidy(Path::new("/r/filters/../src/./Lib/Lib.csproj")),
            Path::new("/r/src/Lib/Lib.csproj")
        );
        assert_eq!(tidy(Path::new("../a/../b")), Path::new("../b"));
    }

    #[test]
    fn attributes_need_a_whole_name() {
        assert_eq!(
            attribute(r#"Project Type="x" Path="a.csproj""#, "Path").as_deref(),
            Some("a.csproj")
        );
        assert_eq!(attribute(r#"Project XPath="b" "#, "Path"), None);
        assert_eq!(
            attribute(r#"Project Path = 'c.csproj'"#, "Path").as_deref(),
            Some("c.csproj")
        );
    }
}
