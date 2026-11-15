use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

pub const LOAD: &str = "load_skill";
const HEADER_LIMIT: u64 = 8 * 1024;
const BODY_LIMIT: u64 = 128 * 1024;

#[derive(Default)]
pub struct Skills(BTreeMap<String, Skill>);

struct Skill {
    description: String,
    path: PathBuf,
}

impl Skills {
    /// Indexes metadata at startup. A user skill with the same name replaces the shipped one.
    pub fn discover(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut skills = BTreeMap::new();
        for root in roots {
            let Ok(entries) = fs::read_dir(root) else {
                continue;
            };
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let path = entry.path().join("SKILL.md");
                match open(&path).and_then(|mut reader| header(&mut reader)) {
                    Ok((name, description)) => {
                        skills.insert(name, Skill { description, path });
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => mytools_plugin_api::log::warn(
                        "skills.header",
                        format!("{}: {error}", path.display()),
                    ),
                }
            }
        }
        Self(skills)
    }

    pub fn prompt(&self) -> Option<String> {
        if self.0.is_empty() {
            return None;
        }
        let index: Vec<_> = self
            .0
            .iter()
            .map(|(name, skill)| json!({"name": name, "description": skill.description}))
            .collect();
        Some(format!(
            "Available skills: {}\nWhen the user's current task matches a skill description, call load_skill with its exact name before taking the workflow steps. Follow the returned instructions for the task.",
            json!(index)
        ))
    }

    pub fn function() -> Value {
        json!({
            "type": "function",
            "function": {
                "name": LOAD,
                "description": "Read the instructions of a skill listed in the system message.",
                "parameters": {
                    "type": "object", "properties": {"name": {"type": "string"}},
                    "required": ["name"], "additionalProperties": false
                }
            }
        })
    }

    pub fn call(&self, arguments: &str) -> String {
        let name = serde_json::from_str::<Value>(arguments)
            .ok()
            .and_then(|value| value["name"].as_str().map(str::to_owned));
        let Some(name) = name else {
            return "Error: expected a skill name.".into();
        };
        match self.load(&name) {
            Ok(instructions) => json!({"name": name, "instructions": instructions}).to_string(),
            Err(error) => format!("Error: {error}"),
        }
    }

    fn load(&self, name: &str) -> io::Result<String> {
        let skill = self
            .0
            .get(name)
            .ok_or_else(|| io::Error::other("unknown skill"))?;
        let mut reader = open(&skill.path).map_err(|_| io::Error::other("cannot open skill"))?;
        let (current, _) = header(&mut reader)?;
        if current != name {
            return Err(io::Error::other(
                "skill name changed; refresh the index by restarting Ask AI",
            ));
        }
        let mut body = String::new();
        reader.take(BODY_LIMIT + 1).read_to_string(&mut body)?;
        if body.len() as u64 > BODY_LIMIT || body.trim().is_empty() {
            return Err(io::Error::other("skill body is empty or exceeds 128 KiB"));
        }
        Ok(body)
    }
}

fn open(path: &Path) -> io::Result<BufReader<File>> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(io::Error::other("expected a regular SKILL.md file"));
    }
    Ok(BufReader::new(File::open(path)?))
}

fn header(reader: &mut impl BufRead) -> io::Result<(String, String)> {
    let invalid = || io::Error::other("invalid SKILL.md header");
    let mut reader = reader.take(HEADER_LIMIT);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim_start_matches('\u{feff}').trim() != "---" {
        return Err(invalid());
    }
    let (mut name, mut description) = (None, None);
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(invalid());
        }
        if line.trim() == "---" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            match key.trim() {
                "name" if name.is_none() => name = Some(field(value)),
                "description" if description.is_none() => description = Some(field(value)),
                "name" | "description" => return Err(invalid()),
                _ => {}
            }
        }
    }
    let name = name
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        .ok_or_else(invalid)?;
    let description = description
        .filter(|description| !description.is_empty() && description.len() <= 1024)
        .ok_or_else(invalid)?;
    Ok((name, description))
}

fn field(value: &str) -> String {
    let value = value.trim();
    if let Some(value) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return value.replace("''", "'");
    }
    serde_json::from_str::<String>(value).unwrap_or_else(|_| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_indexes_only_headers_and_user_skills_override_by_name() {
        let temp = tempfile::tempdir().unwrap();
        let shipped = temp.path().join("shipped");
        let user = temp.path().join("user");
        for (root, label) in [(&shipped, "Shipped"), (&user, "User")] {
            let entry = root.join("test-skill");
            fs::create_dir_all(&entry).unwrap();
            fs::write(
                entry.join("SKILL.md"),
                format!(
                    "---\nname: test-skill\ndescription: {label}\n---\n{}",
                    "x".repeat(BODY_LIMIT as usize + 1)
                ),
            )
            .unwrap();
        }
        let skills = Skills::discover([shipped, user.clone()]);
        let prompt = skills.prompt().unwrap();
        assert!(prompt.contains("User") && !prompt.contains("Shipped"));
        assert!(!prompt.contains("xxxxxxxx"));
        assert!(skills.load("test-skill").is_err());
        fs::write(
            user.join("test-skill/SKILL.md"),
            "---\nname: test-skill\ndescription: User\n---\nUpdated body",
        )
        .unwrap();
        assert_eq!(skills.load("test-skill").unwrap(), "Updated body");
        assert!(skills.load("../test-skill").is_err());
    }
}
