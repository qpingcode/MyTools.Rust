//! Reads hyperlink fields from copied RTF and writes the link as RTF.
use crate::link::JobLink;

/// The link as an RTF hyperlink field, for applications that paste RTF before HTML.
pub fn write(link: &JobLink) -> String {
    format!(
        "{{\\rtf1\\ansi\\uc1{{\\field{{\\*\\fldinst{{HYPERLINK \"{}\"}}}}{{\\fldrslt{{{}}}}}}}\\par}}",
        escape_rtf(&link.url.replace('"', "%22")),
        escape_rtf(&link.title)
    )
}

/// RTF is ASCII: `\`, `{` and `}` are escaped, and other characters written as UTF-16
/// code units (`\uN?`, with `?` for readers that do not know Unicode).
fn escape_rtf(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '{' | '}' => {
                output.push('\\');
                output.push(c);
            }
            ' '..='~' => output.push(c),
            _ => {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    output.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    output
}

/// Every `{\field{\*\fldinst{HYPERLINK "url"}}{\fldrslt title}}` in `rtf`.
pub fn hyperlinks(rtf: &str) -> Vec<JobLink> {
    let mut links = Vec::new();
    let mut rest = rtf;
    while let Some(start) = rest.find("HYPERLINK") {
        rest = &rest[start + "HYPERLINK".len()..];
        let Some(url) = quoted(rest) else {
            continue;
        };
        let Some(result) = rest.find("\\fldrslt") else {
            break;
        };
        rest = &rest[result + "\\fldrslt".len()..];
        links.push(JobLink {
            url,
            title: group_text(rest),
        });
    }
    links
}

/// The `"..."` argument at the start of `text`, as in `HYPERLINK "url"`.
fn quoted(text: &str) -> Option<String> {
    let value = text.trim_start().strip_prefix('"')?;
    Some(value[..value.find('"')?].to_owned())
}

/// The visible text of the group `text` is in, up to the brace that closes it, on one line.
fn group_text(text: &str) -> String {
    let mut reader = Reader {
        chars: text.chars().peekable(),
        units: Vec::new(),
        skip_per_unicode: 1,
        skipping: 0,
    };
    reader.read();
    let text = String::from_utf16_lossy(&reader.units);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Reader<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    /// The text as UTF-16, the unit `\uN` gives, so surrogate pairs join up.
    units: Vec<u16>,
    /// `\ucN`: how many fallback characters follow each `\uN`.
    skip_per_unicode: usize,
    skipping: usize,
}

impl Reader<'_> {
    fn read(&mut self) {
        let mut depth = 0;
        while let Some(c) = self.chars.next() {
            match c {
                '{' if self.starts_destination() => self.skip_group(),
                '{' => depth += 1,
                '}' if depth == 0 => return,
                '}' => depth -= 1,
                '\\' => self.control(),
                '\r' | '\n' => {}
                _ => self.text(c),
            }
        }
    }

    /// Whether the group just opened is `{\*\...}`, an optional destination such as a
    /// bookmark, which holds no visible text.
    fn starts_destination(&self) -> bool {
        let mut ahead = self.chars.clone();
        ahead.next() == Some('\\') && ahead.next() == Some('*')
    }

    fn skip_group(&mut self) {
        let mut depth = 1;
        while let Some(c) = self.chars.next() {
            match c {
                '\\' => {
                    self.chars.next();
                }
                '{' => depth += 1,
                '}' if depth == 1 => return,
                '}' => depth -= 1,
                _ => {}
            }
        }
    }

    fn control(&mut self) {
        match self.chars.next() {
            Some(c @ ('\\' | '{' | '}')) => self.text(c),
            Some('~') => self.text(' '),
            Some('\'') => {
                let hex: String = self.chars.by_ref().take(2).collect();
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    self.text(windows_1252(byte));
                }
            }
            Some(c) if c.is_ascii_alphabetic() => self.word(c),
            _ => {} // `\*`, `\-` and other symbols that show nothing.
        }
    }

    fn word(&mut self, first: char) {
        let mut name = String::from(first);
        while let Some(&c) = self.chars.peek().filter(|c| c.is_ascii_alphabetic()) {
            name.push(c);
            self.chars.next();
        }
        let mut number = String::new();
        if self.chars.peek() == Some(&'-') {
            number.push('-');
            self.chars.next();
        }
        while let Some(&c) = self.chars.peek().filter(|c| c.is_ascii_digit()) {
            number.push(c);
            self.chars.next();
        }
        if self.chars.peek() == Some(&' ') {
            self.chars.next(); // The space that ends a control word is not text.
        }
        match (name.as_str(), number.parse::<i32>().ok()) {
            ("u", Some(code)) => {
                self.units.push(code as i16 as u16);
                self.skipping = self.skip_per_unicode;
            }
            ("uc", Some(count)) => self.skip_per_unicode = count.max(0) as usize,
            ("par" | "line" | "tab", _) => self.text(' '),
            _ => {}
        }
    }

    fn text(&mut self, c: char) {
        if self.skipping > 0 {
            self.skipping -= 1;
            return;
        }
        self.units.extend(c.encode_utf16(&mut [0; 2]).iter());
    }
}

/// `\'hh` in an `\ansicpg1252` document: Latin-1, except the punctuation Windows puts in
/// 0x80..0x9F.
fn windows_1252(byte: u8) -> char {
    match byte {
        0x80 => '€',
        0x85 => '…',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x96 => '–',
        0x97 => '—',
        _ => char::from(byte),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://ediprod.cw.wisetechglobal.com/link/ShowEditForm/WorkItem/1";

    fn job(title: &str) -> JobLink {
        JobLink {
            url: URL.into(),
            title: title.into(),
        }
    }

    #[test]
    fn rtf_is_an_escaped_hyperlink_field() {
        let rtf = write(&job("WI1 {x} \\ 中"));
        assert!(rtf.contains(&format!("HYPERLINK \"{URL}\"")));
        assert!(rtf.contains("WI1 \\{x\\} \\\\ \\u20013?"));
        assert!(rtf.is_ascii());
    }

    #[test]
    fn reads_the_title_of_each_hyperlink_field() {
        // As Remote Desktop forwards a link copied in CargoWise: the title in a nested group.
        let remote = format!(
            "{{\\rtf1\\ansi{{\\field{{\\*\\fldinst{{HYPERLINK \"{URL}\"}}}}{{\\fldrslt{{\\cf1\\ul WI1 - Title}}}}}}\r\n}}"
        );
        assert_eq!(hyperlinks(&remote), [job("WI1 - Title")]);
        // As CargoWise copies it locally: the title directly in the result group.
        let local = format!(
            "{{\\field{{\\*\\fldinst HYPERLINK \"{URL}\" }}{{\\fldrslt WI1 - Title}}}}\\par}}"
        );
        assert_eq!(hyperlinks(&local), [job("WI1 - Title")]);
    }

    #[test]
    fn decodes_escapes_and_skips_formatting() {
        let rtf = format!(
            "{{\\field{{\\*\\fldinst{{HYPERLINK \"{URL}\"}}}}{{\\fldrslt{{\\b R\\'26D \\{{x\\}} \\\\ caf\\'e9\\par\r\nnext \\u20013?\\uc0\\u25991{{\\*\\bkmkstart x}} \\'93q\\'94}}}}}}"
        );
        assert_eq!(hyperlinks(&rtf), [job("R&D {x} \\ café next 中文 “q”")]);
    }

    #[test]
    fn what_this_plugin_writes_reads_back() {
        let link = job("WI1 - 中文 {x} \\ “q”");
        assert_eq!(hyperlinks(&write(&link)), [link]);
    }
}
