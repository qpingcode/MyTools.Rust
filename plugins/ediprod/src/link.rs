//! The ediProd job hyperlink on the clipboard: found in the copied HTML, or in the RTF when
//! the HTML was lost (Remote Desktop forwards only plain text and RTF).
use crate::{html, rtf};

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

/// The one job link in the clipboard's `HTML Format`, or else in its RTF; `text` (the plain
/// text) only explains why there is none.
pub fn find(html: Option<&str>, rtf: Option<&str>, text: Option<&str>) -> Result<JobLink, Problem> {
    let jobs = |links: Vec<JobLink>| -> Vec<JobLink> {
        links
            .into_iter()
            .filter(|link| is_job_url(&link.url))
            .collect()
    };
    let mut links = jobs(html.map(html::anchors).unwrap_or_default());
    if links.is_empty() {
        links = jobs(rtf.map(rtf::hyperlinks).unwrap_or_default());
    }
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
            find(Some(&html), None, Some(URL)),
            Ok(job(
                "WI01147547 - Automate Elasticsearch Warm-Up for Rollouts"
            ))
        );
    }

    #[test]
    fn decodes_entities_and_drops_inner_markup() {
        let html = "<p><A class=x HREF='https://ediprod.cw.wisetechglobal.com/link/ShowEditForm/WorkItem/1?a=1&amp;b=2'>\n<b>CS1 - R&amp;D &lt;x&gt; &#x4E2D;</b></A></p>";
        let link = find(Some(html), None, None).unwrap();
        assert_eq!(
            link.url,
            "https://ediprod.cw.wisetechglobal.com/link/ShowEditForm/WorkItem/1?a=1&b=2"
        );
        assert_eq!(link.title, "CS1 - R&D <x> 中");
    }

    #[test]
    fn explains_why_there_is_no_link() {
        assert_eq!(find(None, None, Some("hello")), Err(Problem::NoLink));
        assert_eq!(find(None, None, Some(URL)), Err(Problem::NoTitle));
        let other = "<a href=\"https://example.com\">Example</a>";
        assert_eq!(find(Some(other), None, None), Err(Problem::NoLink));
        let empty = format!("<a href=\"{URL}\"> </a>");
        assert_eq!(find(Some(&empty), None, None), Err(Problem::NoTitle));
        let two = format!(
            "<a href=\"{URL}\">A</a> <a href=\"{}\">B</a>",
            URL.replace("ad68", "0000")
        );
        assert_eq!(find(Some(&two), None, None), Err(Problem::SeveralLinks));
        // The same job linked twice is still one job.
        let twice = format!("<a href=\"{URL}\">A</a><abbr>x</abbr><a href=\"{URL}\">A</a>");
        assert_eq!(find(Some(&twice), None, None), Ok(job("A")));
    }

    #[test]
    fn falls_back_to_the_rtf_remote_desktop_forwards() {
        // The RTF on the clipboard after copying in CargoWise over Remote Desktop.
        let rtf = format!(
            r#"{{\rtf1\ansi\ansicpg1252\deff0\deflang1033
{{\colortbl ;\red0\green0\blue255;}}
\viewkind4\uc1\pard\sb100\sa100\lang3081\f0\fs24{{\field{{\*\fldinst{{HYPERLINK "{URL}"}}}}{{\fldrslt{{\cf1\ul WI01147547 - Automate Elasticsearch Warm-Up for Rollouts}}}}}}
}}"#
        );
        assert_eq!(
            find(None, Some(&rtf), Some(URL)),
            Ok(job(
                "WI01147547 - Automate Elasticsearch Warm-Up for Rollouts"
            ))
        );
    }
}
