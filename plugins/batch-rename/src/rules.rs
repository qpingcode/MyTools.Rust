use mytools_plugin_api::i18n::Catalog;
use std::{collections::HashSet, path::Path};

#[derive(Default)]
pub struct Rules<'a> {
    find: &'a str,
    replace: &'a str,
    name: &'a str,
    prefix: &'a str,
    suffix: &'a str,
    number: Option<u64>,
    width: usize,
}

impl<'a> Rules<'a> {
    pub fn parse(input: &'a str, catalog: &Catalog) -> Result<Self, String> {
        let mut rules = Self::default();
        let mut seen = HashSet::new();
        for (index, line) in input.lines().enumerate() {
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let invalid = || catalog.format("rules.invalid", &[("line", &(index + 1).to_string())]);
            let (key, value) = line.split_once('=').ok_or_else(invalid)?;
            let key = key.trim();
            if !seen.insert(key) {
                return Err(invalid());
            }
            match key {
                "find" => rules.find = value,
                "replace" => rules.replace = value,
                "name" => rules.name = value,
                "prefix" => rules.prefix = value,
                "suffix" => rules.suffix = value,
                "number" if value.trim().is_empty() => {}
                "width" if value.trim().is_empty() => {}
                "number" => {
                    rules.number = Some(
                        value
                            .trim()
                            .parse()
                            .map_err(|_| catalog.get("rules.number"))?,
                    );
                }
                "width" => {
                    rules.width = value
                        .trim()
                        .parse()
                        .ok()
                        .filter(|width| (1..=12).contains(width))
                        .ok_or_else(|| catalog.get("rules.number"))?;
                }
                _ => return Err(invalid()),
            }
        }
        if rules.find.is_empty() && !rules.replace.is_empty() {
            return Err(catalog.get("rules.find").into());
        }
        if !rules.name.is_empty() && !rules.find.is_empty() {
            return Err(catalog.get("rules.name_or_find").into());
        }
        Ok(rules)
    }

    pub fn name(
        &self,
        original: &str,
        directory: bool,
        index: usize,
        catalog: &Catalog,
    ) -> Result<String, String> {
        let path = Path::new(original);
        let stem = if directory {
            original
        } else {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(original)
        };
        let stem = if !self.name.is_empty() {
            self.name.to_owned()
        } else if self.find.is_empty() {
            stem.to_owned()
        } else {
            stem.replace(self.find, self.replace)
        };
        let mut name = format!("{}{stem}{}", self.prefix, self.suffix);
        if let Some(start) = self.number {
            let number = start
                .checked_add(index as u64)
                .ok_or_else(|| catalog.get("rules.number"))?;
            let width = self.width;
            name.push_str(&format!(" {number:0width$}"));
        }
        if !directory
            && let Some(extension) = path.extension().and_then(|extension| extension.to_str())
        {
            name.push('.');
            name.push_str(extension);
        }
        if name == original || valid_name(&name) {
            Ok(name)
        } else {
            Err(catalog.format("name.invalid", &[("name", &name)]))
        }
    }
}

fn valid_name(name: &str) -> bool {
    if name.is_empty()
        || name.ends_with([' ', '.'])
        || name.encode_utf16().count() > 255
        || name
            .chars()
            .any(|ch| ch <= '\u{1f}' || "<>:\"/\\|?*".contains(ch))
    {
        return false;
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let numbered_device = ["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(prefix).is_some_and(|number| {
            matches!(
                number,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    !numbered_device
        && !matches!(
            base.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
}
