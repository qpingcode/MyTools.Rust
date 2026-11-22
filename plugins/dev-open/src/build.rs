//! Builds the project or solution of the selected files in a PowerShell window: one
//! `dotnet build ... --no-dependencies` per project, referenced projects first, stopping at
//! the first failure. The window stays open to show the output.
use crate::{project, solution};
use mytools_plugin_api::{i18n::Catalog, paths};
use std::path::{Path, PathBuf};

/// The default command run for each project; `{project}` becomes its path. Override it with
/// `build_command` in the plugin's `config.json`.
pub const DEFAULT_COMMAND: &str = "dotnet build {project} --nologo \
    /p:EnforceCodeStyleInBuild=false /p:CompileUsingReferenceAssemblies=false --no-dependencies";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The project each selected file belongs to.
    Project,
    /// Every project of the solution each selected file belongs to.
    Solution,
}

impl Scope {
    pub fn from_command(command: &str) -> Option<Self> {
        match command {
            "build-project" => Some(Self::Project),
            "build-solution" => Some(Self::Solution),
            _ => None,
        }
    }
}

/// What to build for the selected paths.
#[derive(Debug, PartialEq)]
pub struct Plan {
    /// In build order.
    pub projects: Vec<PathBuf>,
    /// Where the terminal starts: the folder of the first solution or project.
    pub directory: PathBuf,
    /// Selected paths with no project or solution, or a solution without projects.
    pub skipped: usize,
}

/// Finds what to build, or says why there is nothing.
pub fn plan(scope: Scope, selected: &[PathBuf], catalog: &Catalog) -> Result<Plan, String> {
    let mut projects = vec![];
    let mut directory = None;
    let mut skipped = 0;
    let mut first_error = None;
    for path in selected.iter().filter(|path| path.exists()) {
        match targets(scope, path, catalog) {
            Ok((owner, found)) => {
                directory.get_or_insert_with(|| owner.parent().unwrap_or(&owner).to_path_buf());
                projects.extend(found);
            }
            Err(error) => {
                skipped += 1;
                first_error.get_or_insert(error);
            }
        }
    }
    let projects = solution::build_order(projects);
    match (directory, projects.is_empty()) {
        (Some(directory), false) => Ok(Plan {
            projects,
            directory,
            skipped,
        }),
        _ => Err(first_error.unwrap_or_else(|| catalog.get("selection.missing").to_owned())),
    }
}

/// The project or solution file `path` belongs to and the projects to build for it.
fn targets(
    scope: Scope,
    path: &Path,
    catalog: &Catalog,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let shown = paths::display(path);
    match scope {
        Scope::Project => project::owning_project(path)
            .map(|project| (project.clone(), vec![project]))
            .ok_or_else(|| catalog.format("build.no_project", &[("path", &shown)])),
        Scope::Solution => {
            let solution = project::owning_solution(path)
                .ok_or_else(|| catalog.format("build.no_solution", &[("path", &shown)]))?;
            let projects = solution::projects(&solution).unwrap_or_default();
            if projects.is_empty() {
                return Err(catalog.format(
                    "build.empty_solution",
                    &[("solution", &paths::display(&solution))],
                ));
            }
            Ok((solution, projects))
        }
    }
}

/// The PowerShell that builds the projects listed in `list`, one path per line, and then
/// deletes the list. Paths go through a file because a large solution would not fit in a
/// command line.
pub fn script(command: &str, list: &Path, catalog: &Catalog) -> String {
    let command = if command.contains("{project}") {
        command.replace("{project}", "$project")
    } else {
        format!("{command} $project")
    };
    let failed = catalog.format("build.failed", &[("project", "{0}")]);
    let done = catalog.format("build.done", &[("count", "{0}"), ("seconds", "{1:0.0}")]);
    format!(
        "$list = {list}
$projects = @(Get-Content -LiteralPath $list -Encoding UTF8 | Where-Object {{ $_ }})
Remove-Item -LiteralPath $list -ErrorAction SilentlyContinue
$failed = $null
$watch = [Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt $projects.Count; $i++) {{
    $project = $projects[$i]
    Write-Host ('[{{0}}/{{1}}] {{2}}' -f ($i + 1), $projects.Count, $project) -ForegroundColor Cyan
    {command}
    if ($LASTEXITCODE -ne 0) {{ $failed = $project; break }}
}}
if ($failed) {{
    Write-Host ({failed} -f $failed) -ForegroundColor Red
}} else {{
    Write-Host ({done} -f $projects.Count, $watch.Elapsed.TotalSeconds) -ForegroundColor Green
}}
",
        list = quote(&paths::display(list)),
        failed = quote(&escape_format(&failed)),
        done = quote(&escape_format(&done)),
    )
}

/// A PowerShell single-quoted string, which takes everything literally but `'`.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Doubles braces other than the `{0}`-style placeholders, for PowerShell's `-f`.
fn escape_format(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(['{', '}']) {
        result.push_str(&rest[..start]);
        let placeholder = rest[start..]
            .strip_prefix('{')
            .and_then(|tail| tail.find('}').map(|end| &tail[..end]))
            .filter(|inner| {
                inner
                    .split(':')
                    .next()
                    .is_some_and(|n| n.parse::<u8>().is_ok())
            });
        match placeholder {
            Some(inner) => {
                result.push_str(&format!("{{{inner}}}"));
                rest = &rest[start + inner.len() + 2..];
            }
            None => {
                let brace = &rest[start..start + 1];
                result.push_str(brace);
                result.push_str(brace);
                rest = &rest[start + 1..];
            }
        }
    }
    result.push_str(rest);
    result
}

/// Starts the build in a new terminal window.
pub fn run(plan: &Plan, command: &str, catalog: &Catalog) -> Result<(), String> {
    if !cfg!(windows) {
        return Err(catalog.get("build.unsupported").to_owned());
    }
    let list = std::env::temp_dir().join(format!(
        "mytools-dev-open-build-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ));
    let lines: Vec<String> = plan
        .projects
        .iter()
        .map(|path| paths::display(path))
        .collect();
    std::fs::write(&list, lines.join("\n")).map_err(|error| error.to_string())?;
    let result = crate::terminal::open(&script(command, &list, catalog), &plan.directory);
    if result.is_err() {
        let _ = std::fs::remove_file(&list);
    }
    result.map_err(|error| catalog.format("build.launch_failed", &[("error", &error)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::from_json(
            include_str!("../locales/en.json"),
            include_str!("../locales/en.json"),
        )
        .unwrap()
    }

    #[test]
    fn script_runs_the_command_per_project_and_stops_at_a_failure() {
        let script = script(DEFAULT_COMMAND, Path::new("C:/tmp/it's.txt"), &catalog());
        assert!(script.starts_with("$list = 'C:/tmp/it''s.txt'"), "{script}");
        assert!(
            script.contains("    dotnet build $project --nologo /p:EnforceCodeStyleInBuild=false")
        );
        assert!(script.contains("--no-dependencies\n"));
        assert!(!script.contains("maxCpuCount"));
        assert!(script.contains("{ $failed = $project; break }"));
        assert!(script.contains("('Build failed: {0}' -f $failed)"));
        assert!(script.contains("'Built {0} projects in {1:0.0} s'"));
    }

    #[test]
    fn a_command_without_the_placeholder_gets_the_project_appended() {
        let script = script("dotnet build", Path::new("list.txt"), &catalog());
        assert!(script.contains("    dotnet build $project\n"));
    }

    #[test]
    fn format_strings_keep_placeholders_and_double_other_braces() {
        assert_eq!(escape_format("a {0} b {1:0.0}"), "a {0} b {1:0.0}");
        assert_eq!(escape_format("{x} }"), "{{x}} }}");
    }

    #[test]
    fn commands_name_the_scope() {
        assert_eq!(Scope::from_command("build-project"), Some(Scope::Project));
        assert_eq!(Scope::from_command("build-solution"), Some(Scope::Solution));
        assert_eq!(Scope::from_command("rider"), None);
    }

    #[test]
    fn nothing_to_build_says_why() {
        let catalog = catalog();
        let missing = plan(
            Scope::Project,
            &[PathBuf::from("Z:/missing/a.cs")],
            &catalog,
        );
        assert_eq!(missing.unwrap_err(), catalog.get("selection.missing"));
        let root =
            std::env::temp_dir().join(format!("mytools-dev-open-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("App/Code")).unwrap();
        std::fs::write(root.join("App/App.csproj"), "<Project />").unwrap();
        std::fs::write(root.join("App/Code/Program.cs"), "").unwrap();
        let built = plan(
            Scope::Project,
            &[
                root.join("App/Code/Program.cs"),
                root.join("App/App.csproj"),
            ],
            &catalog,
        )
        .unwrap();
        assert_eq!(built.projects, [root.join("App/App.csproj")]);
        assert_eq!(built.directory, root.join("App"));
        assert_eq!(built.skipped, 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
