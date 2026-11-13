//! Web search providers: Tavily's API, or the result pages of Bing and DuckDuckGo, which
//! need no key.
use crate::config::Search;
use scraper::{Html, Selector};
use serde::Serialize;
use std::time::Duration;

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// A browser's user agent, so result pages come as they do for people.
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

pub fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(8))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| mytools_ai::describe(&e))
}

/// Searches with the first provider that answers, and names it; the error lists why each
/// one failed.
pub fn search(config: &Search, query: &str, count: usize) -> Result<(String, Vec<Hit>), String> {
    let client = client()?;
    let count = count.clamp(1, 10);
    let mut failures = Vec::new();
    for provider in &config.providers {
        let result = match provider.as_str() {
            "tavily" => match mytools_ai::environment::variable("TAVILY_API_KEY") {
                Some(key) => tavily(&client, &key, query, count),
                None => Err("TAVILY_API_KEY is not set".into()),
            },
            "bing" => bing(&client, config, query, count),
            "duckduckgo" => duckduckgo(&client, config, query, count),
            other => Err(format!("unknown provider {other}")),
        };
        match result {
            Ok(hits) if !hits.is_empty() => return Ok((provider.clone(), hits)),
            Ok(_) => failures.push(format!("{provider}: no results")),
            Err(error) => failures.push(format!("{provider}: {error}")),
        }
    }
    Err(failures.join("; "))
}

fn tavily(
    client: &reqwest::blocking::Client,
    key: &str,
    query: &str,
    count: usize,
) -> Result<Vec<Hit>, String> {
    let response = client
        .post("https://api.tavily.com/search")
        .bearer_auth(key.trim())
        .json(&serde_json::json!({
            "query": query, "max_results": count, "search_depth": "basic",
        }))
        .send()
        .map_err(|e| mytools_ai::describe(&e))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let body: serde_json::Value = response.json().map_err(|e| mytools_ai::describe(&e))?;
    Ok(body["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|hit| {
            Some(Hit {
                title: hit["title"].as_str()?.trim().into(),
                url: hit["url"].as_str()?.into(),
                snippet: hit["content"].as_str().unwrap_or_default().trim().into(),
            })
        })
        .take(count)
        .collect())
}

fn page(client: &reqwest::blocking::Client, url: reqwest::Url) -> Result<String, String> {
    let response = client
        .get(url)
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .send()
        .map_err(|e| mytools_ai::describe(&e))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response.text().map_err(|e| mytools_ai::describe(&e))
}

fn bing(
    client: &reqwest::blocking::Client,
    config: &Search,
    query: &str,
    count: usize,
) -> Result<Vec<Hit>, String> {
    // A host may carry its scheme, e.g. a local mirror for tests.
    let host = config.bing.host.trim().trim_end_matches('/');
    let base = if host.contains("://") {
        host.to_owned()
    } else {
        format!("https://{host}")
    };
    let url = reqwest::Url::parse_with_params(
        &format!("{base}/search"),
        [("q", query), ("setmkt", config.bing.market.as_str())],
    )
    .map_err(|e| e.to_string())?;
    Ok(parse_bing(&page(client, url)?, count))
}

fn duckduckgo(
    client: &reqwest::blocking::Client,
    config: &Search,
    query: &str,
    count: usize,
) -> Result<Vec<Hit>, String> {
    let url = reqwest::Url::parse_with_params(
        "https://html.duckduckgo.com/html/",
        [("q", query), ("kl", config.duckduckgo.region.as_str())],
    )
    .map_err(|e| e.to_string())?;
    Ok(parse_duckduckgo(&page(client, url)?, count))
}

fn select(css: &str) -> Selector {
    Selector::parse(css).expect("a valid selector")
}

fn text(element: scraper::ElementRef) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Results are `li.b_algo`, each with the link in `h2 a` and the summary in a `p`.
pub fn parse_bing(html: &str, count: usize) -> Vec<Hit> {
    let document = Html::parse_document(html);
    let (result, link, summary) = (select("li.b_algo"), select("h2 a"), select("p"));
    document
        .select(&result)
        .filter_map(|item| {
            let anchor = item.select(&link).next()?;
            let url = bing_target(anchor.value().attr("href")?)?;
            Some(Hit {
                title: text(anchor),
                url,
                snippet: item.select(&summary).next().map(text).unwrap_or_default(),
            })
        })
        .take(count)
        .collect()
}

/// Bing sometimes links through `bing.com/ck/a?...&u=a1<base64url of the target>`.
fn bing_target(href: &str) -> Option<String> {
    let url = reqwest::Url::parse(href).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let redirect = url
        .host_str()
        .is_some_and(|host| host.ends_with("bing.com"))
        && url.path().starts_with("/ck/");
    if !redirect {
        return Some(href.into());
    }
    let encoded = url
        .query_pairs()
        .find(|(name, _)| name == "u")?
        .1
        .strip_prefix("a1")?
        .to_owned();
    String::from_utf8(base64url(&encoded)?).ok()
}

fn base64url(text: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut count = 0;
    let mut bytes = Vec::new();
    for ch in text.trim_end_matches('=').chars() {
        let value = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            'a'..='z' => ch as u32 - 'a' as u32 + 26,
            '0'..='9' => ch as u32 - '0' as u32 + 52,
            '-' | '+' => 62,
            '_' | '/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    Some(bytes)
}

/// Results are `.result`, with `a.result__a` linking through `/l/?uddg=<target>` and the
/// summary in `.result__snippet`; ads are left out.
pub fn parse_duckduckgo(html: &str, count: usize) -> Vec<Hit> {
    let document = Html::parse_document(html);
    let (result, link, summary) = (
        select(".result:not(.result--ad)"),
        select("a.result__a"),
        select(".result__snippet"),
    );
    document
        .select(&result)
        .filter_map(|item| {
            let anchor = item.select(&link).next()?;
            let href = anchor.value().attr("href")?;
            let absolute = if href.starts_with("//") {
                format!("https:{href}")
            } else {
                href.to_owned()
            };
            let parsed = reqwest::Url::parse(&absolute).ok()?;
            let url = parsed
                .query_pairs()
                .find(|(name, _)| name == "uddg")
                .map(|(_, target)| target.into_owned())
                .unwrap_or(absolute);
            Some(Hit {
                title: text(anchor),
                url,
                snippet: item.select(&summary).next().map(text).unwrap_or_default(),
            })
        })
        .take(count)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bing_results_and_unwraps_redirects() {
        let html = r#"<ol id="b_results">
          <li class="b_algo"><h2><a href="https://rust-lang.org/zh-CN/"><strong>Rust</strong> 程序设计语言</a></h2>
            <div class="b_caption"><p class="b_lineclamp2">生产环境中的 Rust …</p></div></li>
          <li class="b_ad"><h2><a href="https://ad.example">Ad</a></h2></li>
          <li class="b_algo"><h2><a href="https://www.bing.com/ck/a?!&&p=x&u=a1aHR0cHM6Ly9leGFtcGxlLmNvbS9wYWdl&ntb=1">Example</a></h2></li>
        </ol>"#;
        let hits = parse_bing(html, 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Rust 程序设计语言");
        assert_eq!(hits[0].url, "https://rust-lang.org/zh-CN/");
        assert_eq!(hits[0].snippet, "生产环境中的 Rust …");
        assert_eq!(hits[1].url, "https://example.com/page");
        assert_eq!(parse_bing(html, 1).len(), 1);
    }

    #[test]
    fn reads_duckduckgo_results_without_ads() {
        let html = r#"
          <div class="result results_links result--ad"><a class="result__a" href="https://ad.example">Ad</a></div>
          <div class="result results_links"><h2><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fslint.dev%2F&amp;rut=1">Slint
            UI</a></h2><a class="result__snippet" href="x">Declarative <b>GUI</b></a></div>"#;
        let hits = parse_duckduckgo(html, 5);
        assert_eq!(
            hits,
            [Hit {
                title: "Slint UI".into(),
                url: "https://slint.dev/".into(),
                snippet: "Declarative GUI".into(),
            }]
        );
    }
}
