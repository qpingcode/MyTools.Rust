//! Reading a web page as text for the model.
use scraper::{Html, Node, Selector};

/// How much of a page the model gets.
const LIMIT: usize = 12_000;

pub fn fetch(url: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("only http and https pages can be read".into());
    }
    let response = super::search::client()?
        .get(parsed)
        .send()
        .map_err(|e| mytools_ai::describe(&e))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let kind = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = response.text().map_err(|e| mytools_ai::describe(&e))?;
    let (title, text) = if kind.contains("html") || kind.is_empty() {
        readable(&body)
    } else if kind.starts_with("text/") || kind.contains("json") || kind.contains("xml") {
        (String::new(), body)
    } else {
        return Err(format!("cannot read {kind}"));
    };
    Ok(format!("Title: {title}\nURL: {url}\n\n{}", truncate(&text)))
}

fn truncate(text: &str) -> String {
    match text.char_indices().nth(LIMIT) {
        Some((end, _)) => format!("{}\n[…]", &text[..end]),
        None => text.to_owned(),
    }
}

/// The page title and the text of its main content (`main` or `article` when there is
/// one), one line per block, without scripts, styles, navigation and the like.
pub fn readable(html: &str) -> (String, String) {
    let document = Html::parse_document(html);
    let title = document
        .select(&Selector::parse("title").unwrap())
        .next()
        .map(|title| title.text().collect::<String>().trim().to_owned())
        .unwrap_or_default();
    let root = ["main", "article", "body"]
        .iter()
        .find_map(|css| document.select(&Selector::parse(css).unwrap()).next());
    let mut lines = Vec::new();
    let mut line = String::new();
    if let Some(root) = root {
        for node in root.descendants() {
            match node.value() {
                Node::Text(text) => {
                    let skipped = node.ancestors().any(|ancestor| {
                        ancestor.value().as_element().is_some_and(|element| {
                            matches!(
                                element.name(),
                                "script"
                                    | "style"
                                    | "noscript"
                                    | "nav"
                                    | "footer"
                                    | "svg"
                                    | "template"
                                    | "header"
                                    | "aside"
                            )
                        })
                    });
                    if !skipped {
                        let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        if !words.is_empty() {
                            if !line.is_empty() {
                                line.push(' ');
                            }
                            line.push_str(&words);
                        }
                    }
                }
                Node::Element(element)
                    if matches!(
                        element.name(),
                        "p" | "div"
                            | "br"
                            | "li"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "tr"
                            | "pre"
                            | "section"
                            | "td"
                    ) && !line.is_empty() =>
                {
                    lines.push(std::mem::take(&mut line));
                }
                _ => {}
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    (title, lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_main_text_and_drops_the_rest() {
        let html = "<html><head><title> Quick sort </title><style>p{}</style></head><body>
            <nav>Home</nav><main><h1>Quicksort</h1><p>Divide and   conquer.</p>
            <script>var x;</script><ul><li>pick</li><li>split</li></ul></main>
            <footer>©</footer></body></html>";
        let (title, text) = readable(html);
        assert_eq!(title, "Quick sort");
        assert_eq!(text, "Quicksort\nDivide and conquer.\npick\nsplit");
        assert_eq!(
            truncate(&"x".repeat(LIMIT + 5)).len(),
            LIMIT + "\n[…]".len()
        );
    }
}
