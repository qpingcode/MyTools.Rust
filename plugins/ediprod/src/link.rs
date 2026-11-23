//! Finds the ediProd job hyperlink in copied HTML and writes it back in the formats
//! applications paste: its title as plain text, and a link with that title as HTML and RTF.

const JOB_LINK: &str = "://ediprod.cw.wisetechglobal.com/link/ShowEditForm/";

#[derive(Debug, PartialEq, Eq)]
pub struct JobLink {
    pub url: String,
    /// What the link shows, e.g. "WI01147547 - Automate Elasticsearch Warm-Up for Rollouts".
    pub title: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Problem {
    /// No job link anywhere on the clipboard.
    NoLink,
    /// The job URL alone, e.g. copied from the address bar: nothing names the job.
    NoTitle,
    /// More than one job; which one was meant is unknown.
    SeveralLinks,
}

pub fn is_job_url(url: &str) -> bool {
    url.starts_with("http") && url.contains(JOB_LINK)
}

/// The one job link in `html`; `text` (the plain text) only explains why there is none.
pub fn find(html: Option<&str>, text: Option<&str>) -> Result<JobLink, Problem> {
    let links: Vec<JobLink> = html
        .map(anchors)
        .unwrap_or_default()
        .into_iter()
        .filter(|link| is_job_url(&link.url))
        .collect();
    match links.as_slice() {
        [] if text.is_some_and(|text| text.split_whitespace().any(is_job_url)) => {
            Err(Problem::NoTitle)
        }
        [] => Err(Problem::NoLink),
        [first, rest @ ..] if rest.iter().any(|link| link.url != first.url) => {
            Err(Problem::SeveralLinks)
        }
        [link, ..] if link.title.is_empty() => Err(Problem::NoTitle),
        [_, ..] => Ok(links.into_iter().next().expect("matched a first link")),
    }
}

/// Every `<a href="...">title</a>` in `html`, with entities decoded and inner tags dropped.
fn anchors(html: &str) -> Vec<JobLink> {
    let lower = html.to_ascii_lowercase();
    let mut links = Vec::new();
    let mut position = 0;
    while let Some(start) = lower[position..].find("<a").map(|offset| position + offset) {
        let after = lower.as_bytes().get(start + 2).copied();
        position = start + 2;
        if !matches!(after, Some(b' ' | b'\t' | b'\r' | b'\n')) {
            continue; // <abbr>, <area> and the like.
        }
        let Some(tag_end) = lower[start..].find('>').map(|offset| start + offset) else {
            break;
        };
        let Some(close) = lower[tag_end..].find("</a").map(|offset| tag_end + offset) else {
            break;
        };
        if let Some(url) = attribute(&html[start..tag_end], "href") {
            links.push(JobLink {
                url: decode(&url),
                title: text_content(&html[tag_end + 1..close]),
            });
        }
        position = close;
    }
    links
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find(name).map(|offset| search + offset) {
        search = found + name.len();
        let before = lower[..found].chars().next_back();
        let rest = lower[search..].trim_start();
        if !before.is_some_and(char::is_whitespace) || !rest.starts_with('=') {
            continue;
        }
        let value = tag[tag.len() - rest.len() + 1..].trim_start();
        return Some(match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next()?.to_owned(),
            _ => value
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()?
                .to_owned(),
        });
    }
    None
}

/// The visible text of an HTML fragment, on one line.
fn text_content(fragment: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in fragment.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    decode(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        let entity = rest[1..]
            .find(';')
            .filter(|&end| end <= 10)
            .map(|end| &rest[1..=end]);
        match entity.and_then(character) {
            Some(c) => {
                output.push(c);
                rest = &rest[entity.map_or(0, str::len) + 2..];
            }
            None => {
                output.push('&');
                rest = &rest[1..];
            }
        }
    }
    output.push_str(rest);
    output
}

fn character(entity: &str) -> Option<char> {
    let code = match entity {
        "amp" => return Some('&'),
        "lt" => return Some('<'),
        "gt" => return Some('>'),
        "quot" => return Some('"'),
        "apos" => return Some('\''),
        "nbsp" => return Some(' '),
        _ => entity.strip_prefix('#')?,
    };
    let value = match code.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => code.parse().ok()?,
    };
    char::from_u32(value)
}

/// The link as an HTML fragment.
pub fn html(link: &JobLink) -> String {
    format!(
        "<a href=\"{}\">{}</a>",
        escape_html(&link.url),
        escape_html(&link.title)
    )
}

/// The `HTML Format` clipboard data: a header giving the UTF-8 byte offsets of the document
/// and of the fragment to paste, followed by the document.
pub fn cf_html(fragment: &str) -> String {
    const START: &str = "<html><body>\r\n<!--StartFragment-->";
    const END: &str = "<!--EndFragment-->\r\n</body></html>";
    let header = |offsets: [usize; 4]| {
        format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n",
            offsets[0], offsets[1], offsets[2], offsets[3]
        )
    };
    let start_html = header([0; 4]).len();
    let start_fragment = start_html + START.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + END.len();
    format!(
        "{}{START}{fragment}{END}",
        header([start_html, end_html, start_fragment, end_fragment])
    )
}

/// The link as an RTF hyperlink field, for applications that paste RTF before HTML.
pub fn rtf(link: &JobLink) -> String {
    format!(
        "{{\\rtf1\\ansi\\uc1{{\\field{{\\*\\fldinst{{HYPERLINK \"{}\"}}}}{{\\fldrslt{{{}}}}}}}\\par}}",
        escape_rtf(&link.url.replace('"', "%22")),
        escape_rtf(&link.title)
    )
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://ediprod.cw.wisetechglobal.com/link/ShowEditForm/WorkItem/ad68d4bc-de1d-41ba-92c4-7f17ff487b75?lang=en-gb";

    fn job(title: &str) -> JobLink {
        JobLink {
            url: URL.into(),
            title: title.into(),
        }
    }

    #[test]
    fn finds_the_link_cargowise_copies() {
        let html = format!(
            "Version:0.9\r\nStartHTML:000000149\r\n<html><body><!--StartFragment--><a href=\"{URL}\">WI01147547 - Automate Elasticsearch Warm-Up for Rollouts</a><!--EndFragment--></body></html>"
        );
        assert_eq!(
            find(Some(&html), Some(URL)),
            Ok(job(
                "WI01147547 - Automate Elasticsearch Warm-Up for Rollouts"
            ))
        );
    }

    #[test]
    fn decodes_entities_and_drops_inner_markup() {
        let html = "<p><A class=x HREF='https://ediprod.cw.wisetechglobal.com/link/ShowEditForm/WorkItem/1?a=1&amp;b=2'>\n<b>CS1 - R&amp;D &lt;x&gt; &#x4E2D;</b></A></p>";
        let link = find(Some(html), None).unwrap();
        assert_eq!(
            link.url,
            "https://ediprod.cw.wisetechglobal.com/link/ShowEditForm/WorkItem/1?a=1&b=2"
        );
        assert_eq!(link.title, "CS1 - R&D <x> 中");
    }

    #[test]
    fn explains_why_there_is_no_link() {
        assert_eq!(find(None, Some("hello")), Err(Problem::NoLink));
        assert_eq!(find(None, Some(URL)), Err(Problem::NoTitle));
        let other = "<a href=\"https://example.com\">Example</a>";
        assert_eq!(find(Some(other), None), Err(Problem::NoLink));
        let empty = format!("<a href=\"{URL}\"> </a>");
        assert_eq!(find(Some(&empty), None), Err(Problem::NoTitle));
        let two = format!(
            "<a href=\"{URL}\">A</a> <a href=\"{}\">B</a>",
            URL.replace("ad68", "0000")
        );
        assert_eq!(find(Some(&two), None), Err(Problem::SeveralLinks));
        // The same job linked twice is still one job.
        let twice = format!("<a href=\"{URL}\">A</a><abbr>x</abbr><a href=\"{URL}\">A</a>");
        assert_eq!(find(Some(&twice), None), Ok(job("A")));
    }

    #[test]
    fn cf_html_offsets_point_at_the_document_and_the_fragment() {
        let fragment = html(&job("WI1 - 中文 & <x>"));
        let data = cf_html(&fragment);
        let offset = |key: &str| -> usize {
            let start = data.find(key).unwrap() + key.len() + 1;
            data[start..start + 10].parse().unwrap()
        };
        assert!(data[offset("StartHTML")..].starts_with("<html>"));
        assert_eq!(
            &data[offset("StartFragment")..offset("EndFragment")],
            fragment
        );
        assert_eq!(offset("EndHTML"), data.len());
        assert!(fragment.contains("WI1 - 中文 &amp; &lt;x&gt;"));
    }

    #[test]
    fn rtf_is_an_escaped_hyperlink_field() {
        let rtf = rtf(&job("WI1 {x} \\ 中"));
        assert!(rtf.contains(&format!("HYPERLINK \"{URL}\"")));
        assert!(rtf.contains("WI1 \\{x\\} \\\\ \\u20013?"));
        assert!(rtf.is_ascii());
    }
}
